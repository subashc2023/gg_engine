use std::sync::{Arc, Mutex};

use ash::vk;

use super::buffer::create_buffer_with_location;
use super::compute::{self, ComputePipeline, ComputeShader};
use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::renderer_2d::Renderer2DData;
use super::MAX_FRAMES_IN_FLIGHT;
use crate::particle_system::ParticleProps;
use crate::profiling::ProfileTimer;
use crate::shaders;

const FRAMES: usize = MAX_FRAMES_IN_FLIGHT;

// ---------------------------------------------------------------------------
// GpuParticle — per-particle simulation state on the GPU
// ---------------------------------------------------------------------------

/// Must match the GLSL `GpuParticle` struct in particle_sim.glsl (std430 layout).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuParticle {
    position: [f32; 2],
    velocity: [f32; 2],
    rotation: f32,
    rotation_speed: f32,
    size_begin: f32,
    size_end: f32,
    color_begin: [f32; 4],
    color_end: [f32; 4],
    lifetime: f32,
    life_remaining: f32,
    is_active: u32,
    _pad: u32,
}
// 80 bytes

impl Default for GpuParticle {
    fn default() -> Self {
        Self {
            position: [0.0; 2],
            velocity: [0.0; 2],
            rotation: 0.0,
            rotation_speed: 0.0,
            size_begin: 0.0,
            size_end: 0.0,
            color_begin: [1.0; 4],
            color_end: [1.0; 4],
            lifetime: 1.0,
            life_remaining: 0.0,
            is_active: 0,
            _pad: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VkDrawIndexedIndirectCommand layout
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct DrawIndexedIndirectCommand {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    vertex_offset: i32,
    first_instance: u32,
}

// ---------------------------------------------------------------------------
// Push constants for the compute shader
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct ParticleSimPush {
    dt: f32,
    max_particles: u32,
}

// ---------------------------------------------------------------------------
// Simple xorshift32 PRNG (same as particle_system.rs)
// ---------------------------------------------------------------------------

struct Rng {
    state: u32,
}

impl Rng {
    fn from_time() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let stack_addr = &nanos as *const _ as usize as u32;
        let seed = nanos ^ stack_addr;
        Self {
            state: if seed == 0 { 0xDEAD_BEEF } else { seed },
        }
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    fn random(&mut self) -> f32 {
        (self.next_u32() as f64 / u32::MAX as f64) as f32
    }

    fn random_signed(&mut self) -> f32 {
        self.random() * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------------------
// GpuParticleSystem
// ---------------------------------------------------------------------------

/// GPU-driven particle system using a compute shader for simulation and
/// instanced rendering for drawing.
///
/// # Usage
/// ```ignore
/// // Create (in on_attach):
/// renderer.create_gpu_particle_system(100_000);
///
/// // Emit (in on_render):
/// renderer.emit_particles(&props);
///
/// // Render (in on_render, after emit):
/// renderer.render_gpu_particles();
/// ```
pub struct GpuParticleSystem {
    // -- Compute pipeline --
    _compute_shader: ComputeShader,
    compute_pipeline: ComputePipeline,
    compute_ds_layout: vk::DescriptorSetLayout,
    compute_ds_pool: vk::DescriptorPool,
    compute_ds: [vk::DescriptorSet; FRAMES],

    // -- Particle state SSBO (single, shared across frames) --
    state_buffer: vk::Buffer,
    _state_allocation: GpuAllocation,

    // -- Instance output SSBOs (per-frame, also used as vertex buffer) --
    instance_buffers: [vk::Buffer; FRAMES],
    _instance_allocations: [Option<GpuAllocation>; FRAMES],

    // -- Indirect draw buffers (per-frame) --
    indirect_buffers: [vk::Buffer; FRAMES],
    _indirect_allocations: [Option<GpuAllocation>; FRAMES],
    indirect_ptrs: [*mut u8; FRAMES],

    // -- Emission queue (CPU-side, flushed before compute dispatch) --
    emission_queue: Vec<GpuParticle>,
    pool_index: u32,
    max_particles: u32,

    // -- RNG for emission variation --
    rng: Rng,

    device: ash::Device,
}

// Safety: Buffers are accessed with proper frame-in-flight synchronization.
unsafe impl Send for GpuParticleSystem {}
unsafe impl Sync for GpuParticleSystem {}

impl GpuParticleSystem {
    pub(super) fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        max_particles: u32,
        pipeline_cache: vk::PipelineCache,
    ) -> Result<Self, String> {
        let _timer = ProfileTimer::new("GpuParticleSystem::new");

        let particle_size = std::mem::size_of::<GpuParticle>() as u64;
        let instance_size = std::mem::size_of::<super::renderer_2d::SpriteInstanceData>() as u64;
        let indirect_size = std::mem::size_of::<DrawIndexedIndirectCommand>() as u64;

        // -- Particle state SSBO --
        let (state_buffer, state_allocation) = create_buffer_with_location(
            allocator,
            device,
            particle_size * max_particles as u64,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            "ParticleState",
            MemoryLocation::CpuToGpu,
        )?;

        // Zero-initialize particle state (all inactive).
        if let Some(ptr) = state_allocation.mapped_ptr() {
            unsafe {
                std::ptr::write_bytes(ptr, 0, (particle_size * max_particles as u64) as usize);
            }
        }

        // -- Instance output SSBOs (per-frame) --
        let mut instance_buffers = [vk::Buffer::null(); FRAMES];
        let mut instance_allocations: [Option<GpuAllocation>; FRAMES] = [None, None];
        for i in 0..FRAMES {
            let (buf, alloc) = create_buffer_with_location(
                allocator,
                device,
                instance_size * max_particles as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::VERTEX_BUFFER,
                "ParticleInstanceOutput",
                MemoryLocation::CpuToGpu,
            )?;
            instance_buffers[i] = buf;
            instance_allocations[i] = Some(alloc);
        }

        // -- Indirect draw buffers (per-frame) --
        let mut indirect_buffers = [vk::Buffer::null(); FRAMES];
        let mut indirect_allocations: [Option<GpuAllocation>; FRAMES] = [None, None];
        let mut indirect_ptrs = [std::ptr::null_mut(); FRAMES];
        for i in 0..FRAMES {
            let (buf, alloc) = create_buffer_with_location(
                allocator,
                device,
                indirect_size,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER,
                "ParticleIndirect",
                MemoryLocation::CpuToGpu,
            )?;
            let ptr = alloc
                .mapped_ptr()
                .expect("Indirect buffer must be host-visible");
            // Initialize: indexCount=6, instanceCount=0, rest=0
            let cmd = DrawIndexedIndirectCommand {
                index_count: 6,
                instance_count: 0,
                first_index: 0,
                vertex_offset: 0,
                first_instance: 0,
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &cmd as *const _ as *const u8,
                    ptr,
                    std::mem::size_of::<DrawIndexedIndirectCommand>(),
                );
            }
            indirect_buffers[i] = buf;
            indirect_allocations[i] = Some(alloc);
            indirect_ptrs[i] = ptr;
        }

        // -- Descriptor set layout for compute --
        let bindings = [
            // Binding 0: Particle state (read-write)
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            // Binding 1: Instance output (write-only)
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            // Binding 2: Indirect draw command (read-write)
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let compute_ds_layout =
            unsafe { device.create_descriptor_set_layout(&ds_layout_info, None) }
                .map_err(|e| format!("Failed to create compute DS layout: {e}"))?;

        // -- Descriptor pool --
        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 3 * FRAMES as u32,
        };
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(std::slice::from_ref(&pool_size))
            .max_sets(FRAMES as u32);
        let compute_ds_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("Failed to create compute descriptor pool: {e}"))?;

        // -- Allocate descriptor sets --
        let layouts = [compute_ds_layout; FRAMES];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(compute_ds_pool)
            .set_layouts(&layouts);
        let ds_vec = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .map_err(|e| format!("Failed to allocate compute descriptor sets: {e}"))?;
        let compute_ds = [ds_vec[0], ds_vec[1]];

        // -- Write descriptor sets --
        for frame in 0..FRAMES {
            let state_info = vk::DescriptorBufferInfo::default()
                .buffer(state_buffer)
                .offset(0)
                .range(particle_size * max_particles as u64);

            let instance_info = vk::DescriptorBufferInfo::default()
                .buffer(instance_buffers[frame])
                .offset(0)
                .range(instance_size * max_particles as u64);

            let indirect_info = vk::DescriptorBufferInfo::default()
                .buffer(indirect_buffers[frame])
                .offset(0)
                .range(indirect_size);

            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(compute_ds[frame])
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(std::slice::from_ref(&state_info)),
                vk::WriteDescriptorSet::default()
                    .dst_set(compute_ds[frame])
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(std::slice::from_ref(&instance_info)),
                vk::WriteDescriptorSet::default()
                    .dst_set(compute_ds[frame])
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(std::slice::from_ref(&indirect_info)),
            ];
            unsafe {
                device.update_descriptor_sets(&writes, &[]);
            }
        }

        // -- Compute shader and pipeline --
        let compute_shader =
            ComputeShader::new(device, "particle_sim", shaders::PARTICLE_SIM_COMP_SPV)?;
        let compute_pipeline = compute::create_compute_pipeline(
            device,
            &compute_shader,
            &[compute_ds_layout],
            std::mem::size_of::<ParticleSimPush>() as u32,
            pipeline_cache,
        )?;

        Ok(Self {
            _compute_shader: compute_shader,
            compute_pipeline,
            compute_ds_layout,
            compute_ds_pool,
            compute_ds,
            state_buffer,
            _state_allocation: state_allocation,
            instance_buffers,
            _instance_allocations: instance_allocations,
            indirect_buffers,
            _indirect_allocations: indirect_allocations,
            indirect_ptrs,
            emission_queue: Vec::with_capacity(1024),
            pool_index: 0,
            max_particles,
            rng: Rng::from_time(),
            device: device.clone(),
        })
    }

    // -----------------------------------------------------------------------
    // Emission (CPU-side, queued for next dispatch)
    // -----------------------------------------------------------------------

    /// Queue a particle emission. The particle will be spawned on the GPU
    /// during the next compute dispatch (1-frame latency).
    pub(super) fn emit(&mut self, props: &ParticleProps) {
        let angle = self.rng.random() * std::f32::consts::TAU;
        let radius = self.rng.random().sqrt();

        let particle = GpuParticle {
            position: [props.position.x, props.position.y],
            velocity: [
                props.velocity.x + angle.cos() * radius * props.velocity_variation.x,
                props.velocity.y + angle.sin() * radius * props.velocity_variation.y,
            ],
            rotation: self.rng.random() * std::f32::consts::TAU,
            rotation_speed: self.rng.random_signed() * 4.0,
            size_begin: (props.size_begin + self.rng.random_signed() * props.size_variation)
                .max(0.01),
            size_end: props.size_end,
            color_begin: [
                props.color_begin.x,
                props.color_begin.y,
                props.color_begin.z,
                props.color_begin.w,
            ],
            color_end: [
                props.color_end.x,
                props.color_end.y,
                props.color_end.z,
                props.color_end.w,
            ],
            lifetime: props.lifetime,
            life_remaining: props.lifetime,
            is_active: 1,
            _pad: 0,
        };

        self.emission_queue.push(particle);
        self.pool_index = (self.pool_index + 1) % self.max_particles;
    }

    // -----------------------------------------------------------------------
    // Compute dispatch (before render pass)
    // -----------------------------------------------------------------------

    /// Record compute commands into the command buffer. Must be called
    /// OUTSIDE a render pass, before any draw calls that render particles.
    pub(super) fn dispatch(&mut self, cmd_buf: vk::CommandBuffer, current_frame: usize, dt: f32) {
        let _timer = ProfileTimer::new("GpuParticleSystem::dispatch");

        // 1. Reset indirect buffer (CPU write, safe after fence wait).
        let cmd = DrawIndexedIndirectCommand {
            index_count: 6,
            instance_count: 0,
            first_index: 0,
            vertex_offset: 0,
            first_instance: 0,
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                &cmd as *const _ as *const u8,
                self.indirect_ptrs[current_frame],
                std::mem::size_of::<DrawIndexedIndirectCommand>(),
            );
        }

        // 2. Barrier: ensure previous compute writes to state SSBO are visible
        //    before we write emission data via vkCmdUpdateBuffer.
        let state_barrier = vk::BufferMemoryBarrier::default()
            .buffer(self.state_buffer)
            .offset(0)
            .size(vk::WHOLE_SIZE)
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[state_barrier],
                &[],
            );
        }

        // 3. Write emission data via vkCmdUpdateBuffer.
        if !self.emission_queue.is_empty() {
            let particle_size = std::mem::size_of::<GpuParticle>();
            // Compute starting pool index for this batch of emissions.
            let emit_count = self.emission_queue.len() as u32;
            let start_index =
                (self.pool_index + self.max_particles - emit_count) % self.max_particles;

            // Handle wrap-around: split into at most two contiguous regions.
            if start_index + emit_count <= self.max_particles {
                // No wrap — single vkCmdUpdateBuffer.
                let offset = start_index as u64 * particle_size as u64;
                let data = unsafe {
                    std::slice::from_raw_parts(
                        self.emission_queue.as_ptr() as *const u8,
                        self.emission_queue.len() * particle_size,
                    )
                };
                // vkCmdUpdateBuffer limit: 65536 bytes per call.
                for chunk_start in (0..data.len()).step_by(65536) {
                    let chunk_end = (chunk_start + 65536).min(data.len());
                    let chunk = &data[chunk_start..chunk_end];
                    unsafe {
                        self.device.cmd_update_buffer(
                            cmd_buf,
                            self.state_buffer,
                            offset + chunk_start as u64,
                            chunk,
                        );
                    }
                }
            } else {
                // Wrap-around: first region = [start_index .. max_particles)
                let first_count = (self.max_particles - start_index) as usize;
                let first_offset = start_index as u64 * particle_size as u64;
                let first_data = unsafe {
                    std::slice::from_raw_parts(
                        self.emission_queue.as_ptr() as *const u8,
                        first_count * particle_size,
                    )
                };
                for chunk_start in (0..first_data.len()).step_by(65536) {
                    let chunk_end = (chunk_start + 65536).min(first_data.len());
                    let chunk = &first_data[chunk_start..chunk_end];
                    unsafe {
                        self.device.cmd_update_buffer(
                            cmd_buf,
                            self.state_buffer,
                            first_offset + chunk_start as u64,
                            chunk,
                        );
                    }
                }

                // Second region = [0 .. remaining)
                let second_count = self.emission_queue.len() - first_count;
                let second_data = unsafe {
                    std::slice::from_raw_parts(
                        (self.emission_queue.as_ptr() as *const u8)
                            .add(first_count * particle_size),
                        second_count * particle_size,
                    )
                };
                for chunk_start in (0..second_data.len()).step_by(65536) {
                    let chunk_end = (chunk_start + 65536).min(second_data.len());
                    let chunk = &second_data[chunk_start..chunk_end];
                    unsafe {
                        self.device.cmd_update_buffer(
                            cmd_buf,
                            self.state_buffer,
                            chunk_start as u64,
                            chunk,
                        );
                    }
                }
            }

            self.emission_queue.clear();
        }

        // 4. Barrier: transfer writes visible to compute shader.
        let transfer_to_compute = vk::BufferMemoryBarrier::default()
            .buffer(self.state_buffer)
            .offset(0)
            .size(vk::WHOLE_SIZE)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE);

        // Also barrier for indirect buffer (CPU write → compute read/write).
        let indirect_barrier = vk::BufferMemoryBarrier::default()
            .buffer(self.indirect_buffers[current_frame])
            .offset(0)
            .size(vk::WHOLE_SIZE)
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE);

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::TRANSFER | vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[transfer_to_compute, indirect_barrier],
                &[],
            );
        }

        // 5. Bind compute pipeline and dispatch.
        let push = ParticleSimPush {
            dt,
            max_particles: self.max_particles,
        };

        unsafe {
            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::COMPUTE,
                self.compute_pipeline.pipeline(),
            );

            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::COMPUTE,
                self.compute_pipeline.layout(),
                0,
                &[self.compute_ds[current_frame]],
                &[],
            );

            self.device.cmd_push_constants(
                cmd_buf,
                self.compute_pipeline.layout(),
                vk::ShaderStageFlags::COMPUTE,
                0,
                std::slice::from_raw_parts(
                    &push as *const ParticleSimPush as *const u8,
                    std::mem::size_of::<ParticleSimPush>(),
                ),
            );

            let workgroups = self.max_particles.div_ceil(256);
            self.device.cmd_dispatch(cmd_buf, workgroups, 1, 1);
        }

        // 6. Barrier: compute writes → vertex input + indirect draw reads.
        let instance_barrier = vk::BufferMemoryBarrier::default()
            .buffer(self.instance_buffers[current_frame])
            .offset(0)
            .size(vk::WHOLE_SIZE)
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(vk::AccessFlags::VERTEX_ATTRIBUTE_READ);

        let indirect_read_barrier = vk::BufferMemoryBarrier::default()
            .buffer(self.indirect_buffers[current_frame])
            .offset(0)
            .size(vk::WHOLE_SIZE)
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(vk::AccessFlags::INDIRECT_COMMAND_READ);

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::VERTEX_INPUT | vk::PipelineStageFlags::DRAW_INDIRECT,
                vk::DependencyFlags::empty(),
                &[],
                &[instance_barrier, indirect_read_barrier],
                &[],
            );
        }
    }

    // -----------------------------------------------------------------------
    // Rendering (inside render pass)
    // -----------------------------------------------------------------------

    /// Record instanced draw commands for GPU particles. Must be called
    /// INSIDE a render pass, after `dispatch()` has been called for this frame.
    pub(super) fn render(
        &self,
        cmd_buf: vk::CommandBuffer,
        current_frame: usize,
        camera_ubo_ds: vk::DescriptorSet,
        r2d: &Renderer2DData,
    ) {
        let pipeline = r2d.active_instance_pipeline();
        let pl = pipeline.pipeline();
        let layout = pipeline.layout();

        unsafe {
            self.device
                .cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, pl);

            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[camera_ubo_ds, r2d.bindless_descriptor_set(current_frame)],
                &[],
            );

            // Bind vertex buffers: binding 0 = unit quad, binding 1 = particle instances.
            let uq_handle = r2d.unit_quad_vb_handle();
            let inst_handle = self.instance_buffers[current_frame];
            self.device
                .cmd_bind_vertex_buffers(cmd_buf, 0, &[uq_handle, inst_handle], &[0, 0]);

            // Bind index buffer (unit quad: 6 indices).
            self.device.cmd_bind_index_buffer(
                cmd_buf,
                r2d.unit_quad_ib_buffer(),
                0,
                vk::IndexType::UINT32,
            );

            // Indirect draw: GPU-driven instance count.
            self.device.cmd_draw_indexed_indirect(
                cmd_buf,
                self.indirect_buffers[current_frame],
                0,
                1,
                0,
            );
        }
    }
}

impl Drop for GpuParticleSystem {
    fn drop(&mut self) {
        unsafe {
            // Buffers
            self.device.destroy_buffer(self.state_buffer, None);
            for buf in &self.instance_buffers {
                self.device.destroy_buffer(*buf, None);
            }
            for buf in &self.indirect_buffers {
                self.device.destroy_buffer(*buf, None);
            }
            // Descriptor pool (frees all sets allocated from it)
            self.device
                .destroy_descriptor_pool(self.compute_ds_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.compute_ds_layout, None);
        }
        // GpuAllocations auto-free on drop (state_allocation, instance_allocations, indirect_allocations).
    }
}
