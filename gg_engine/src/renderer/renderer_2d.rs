use std::cell::RefCell;
use std::sync::Arc;

use ash::vk;

use super::buffer::{
    BufferElement, BufferLayout, DynamicVertexBuffer, IndexBuffer, ShaderDataType,
};
use super::pipeline::{self, Pipeline};
use super::shader::Shader;
use super::texture::Texture2D;
use crate::profiling::ProfileTimer;
use crate::shaders;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_QUADS: usize = 10_000;
const MAX_VERTICES: usize = MAX_QUADS * 4;
const MAX_INDICES: usize = MAX_QUADS * 6;
const MAX_BINDLESS_TEXTURES: u32 = 4096;
const FRAMES_IN_FLIGHT: usize = 2;
/// Max flushes (draw calls) per frame. Sizes the vertex buffer so each flush
/// writes to a distinct region, avoiding overwrites within a command buffer.
const MAX_BATCHES_PER_FRAME: usize = 16;

// ---------------------------------------------------------------------------
// BatchQuadVertex — per-vertex data for batch rendering
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct BatchQuadVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub tex_coord: [f32; 2],
    pub tex_index: f32,
    pub entity_id: i32,
}

/// The canonical buffer layout for batch quad vertices.
fn batch_vertex_layout() -> BufferLayout {
    BufferLayout::new(&[
        BufferElement::new(ShaderDataType::Float3, "a_position"),
        BufferElement::new(ShaderDataType::Float4, "a_color"),
        BufferElement::new(ShaderDataType::Float2, "a_tex_coord"),
        BufferElement::new(ShaderDataType::Float, "a_tex_index"),
        BufferElement::new(ShaderDataType::Int, "a_entity_id"),
    ])
}

// ---------------------------------------------------------------------------
// Renderer2DStats
// ---------------------------------------------------------------------------

/// Statistics from the 2D batch renderer for the current frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Renderer2DStats {
    pub draw_calls: u32,
    pub quad_count: u32,
}

impl Renderer2DStats {
    pub fn total_vertex_count(&self) -> u32 {
        self.quad_count * 4
    }

    pub fn total_index_count(&self) -> u32 {
        self.quad_count * 6
    }
}

// ---------------------------------------------------------------------------
// BatchState — interior-mutable state for the current batch
// ---------------------------------------------------------------------------

struct BatchState {
    vertices: Vec<BatchQuadVertex>,
    quad_count: usize,
    /// Byte offset into the vertex buffer for the next flush.
    vb_write_offset: usize,
    stats: Renderer2DStats,
}

impl BatchState {
    fn new() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_VERTICES),
            quad_count: 0,
            vb_write_offset: 0,
            stats: Renderer2DStats::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Renderer2DData — batch rendering resources (bindless)
// ---------------------------------------------------------------------------

pub(super) struct Renderer2DData {
    _batch_shader: Arc<Shader>,
    batch_pipeline: Arc<Pipeline>,
    offscreen_pipeline: Option<Arc<Pipeline>>,
    use_offscreen: bool,
    vertex_buffers: [DynamicVertexBuffer; FRAMES_IN_FLIGHT],
    index_buffer: IndexBuffer,
    bindless_pool: vk::DescriptorPool,
    bindless_ds_layout: vk::DescriptorSetLayout,
    bindless_ds: [vk::DescriptorSet; FRAMES_IN_FLIGHT],
    next_bindless_index: RefCell<u32>,
    batch: RefCell<BatchState>,
    pub(super) white_texture: Texture2D,
    device: ash::Device,
}

impl Renderer2DData {
    pub(super) fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        camera_ubo_ds_layout: vk::DescriptorSetLayout,
        white_texture: Texture2D,
    ) -> Self {
        let _timer = ProfileTimer::new("Renderer2D::init");

        // -- Shaders --
        // Swapchain shader: 1 color output only (no entity ID).
        let batch_swapchain_shader = Arc::new(Shader::new(
            device,
            "batch_swapchain",
            shaders::BATCH_SWAPCHAIN_VERT_SPV,
            shaders::BATCH_SWAPCHAIN_FRAG_SPV,
        ));
        // Offscreen shader: 2 outputs (color + entity ID for picking).
        let batch_shader = Arc::new(Shader::new(
            device,
            "batch",
            shaders::BATCH_VERT_SPV,
            shaders::BATCH_FRAG_SPV,
        ));

        // -- Vertex layout --
        let layout = batch_vertex_layout();
        let vb_capacity =
            MAX_VERTICES * MAX_BATCHES_PER_FRAME * std::mem::size_of::<BatchQuadVertex>();

        // -- Per-frame-in-flight vertex buffers (persistently mapped) --
        let vertex_buffers = [
            DynamicVertexBuffer::new(
                instance,
                physical_device,
                device,
                vb_capacity,
                layout.clone(),
            ),
            DynamicVertexBuffer::new(
                instance,
                physical_device,
                device,
                vb_capacity,
                layout.clone(),
            ),
        ];

        // -- Static index buffer (pre-generated quad pattern) --
        let mut indices = Vec::with_capacity(MAX_INDICES);
        for i in 0..MAX_QUADS as u32 {
            let base = i * 4;
            indices.push(base);
            indices.push(base + 1);
            indices.push(base + 2);
            indices.push(base + 2);
            indices.push(base + 3);
            indices.push(base);
        }
        let index_buffer = IndexBuffer::new(instance, physical_device, device, &indices);

        // -- Bindless descriptor set layout (UPDATE_AFTER_BIND + PARTIALLY_BOUND) --
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(MAX_BINDLESS_TEXTURES)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);

        let binding_flags = [vk::DescriptorBindingFlags::PARTIALLY_BOUND
            | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND];
        let mut binding_flags_info =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&binding_flags);

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&binding))
            .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
            .push_next(&mut binding_flags_info);

        let bindless_ds_layout = unsafe { device.create_descriptor_set_layout(&layout_info, None) }
            .expect("Failed to create bindless descriptor set layout");

        // -- Pipeline (swapchain: 1 color attachment, no entity ID output) --
        let batch_pipeline = Arc::new(pipeline::create_batch_pipeline(
            device,
            &batch_swapchain_shader,
            vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[bindless_ds_layout],
            1,
        ));

        // -- Bindless descriptor pool (UPDATE_AFTER_BIND) --
        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: MAX_BINDLESS_TEXTURES * FRAMES_IN_FLIGHT as u32,
        };
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND)
            .pool_sizes(std::slice::from_ref(&pool_size))
            .max_sets(FRAMES_IN_FLIGHT as u32);
        let bindless_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }
            .expect("Failed to create bindless descriptor pool");

        // -- Allocate one descriptor set per frame-in-flight --
        let layouts = [bindless_ds_layout; FRAMES_IN_FLIGHT];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(bindless_pool)
            .set_layouts(&layouts);
        let ds_vec = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .expect("Failed to allocate bindless descriptor sets");
        let bindless_ds = [ds_vec[0], ds_vec[1]];

        Self {
            _batch_shader: batch_shader,
            batch_pipeline,
            offscreen_pipeline: None,
            use_offscreen: false,
            vertex_buffers,
            index_buffer,
            bindless_pool,
            bindless_ds_layout,
            bindless_ds,
            next_bindless_index: RefCell::new(0),
            batch: RefCell::new(BatchState::new()),
            white_texture,
            device: device.clone(),
        }
    }

    /// Register a texture in the bindless descriptor array. Writes its
    /// image_view + sampler into both per-frame descriptor sets at the
    /// assigned index. Returns the global bindless index.
    pub(super) fn register_texture(&self, texture: &Texture2D) -> u32 {
        let mut next = self.next_bindless_index.borrow_mut();
        let index = *next;
        assert!(
            index < MAX_BINDLESS_TEXTURES,
            "Exceeded max bindless textures ({})",
            MAX_BINDLESS_TEXTURES
        );
        *next = index + 1;

        let image_info = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(texture.image_view())
            .sampler(texture.sampler());

        // Write to both frame-in-flight descriptor sets.
        for &ds in &self.bindless_ds {
            let write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .dst_array_element(index)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&image_info));

            unsafe {
                self.device.update_descriptor_sets(&[write], &[]);
            }
        }

        index
    }

    /// Reset batch state for a new frame.
    pub(super) fn reset_batch(&self) {
        let mut batch = self.batch.borrow_mut();
        batch.vertices.clear();
        batch.quad_count = 0;
        batch.vb_write_offset = 0;
        batch.stats = Renderer2DStats::default();
    }

    /// Push a quad into the current batch. The 4 vertices should already be
    /// in world space (pre-transformed). Returns false if the batch was full
    /// and a flush is needed first.
    pub(super) fn push_quad(&self, vertices: [BatchQuadVertex; 4]) -> bool {
        let mut batch = self.batch.borrow_mut();
        if batch.quad_count >= MAX_QUADS {
            return false;
        }
        batch.vertices.extend_from_slice(&vertices);
        batch.quad_count += 1;
        true
    }

    /// Returns true if there are quads to flush.
    pub(super) fn has_pending(&self) -> bool {
        self.batch.borrow().quad_count > 0
    }

    /// Flush the current batch: write vertices to GPU, bind the pre-populated
    /// bindless descriptor set, and record draw commands.
    pub(super) fn flush(
        &self,
        cmd_buf: vk::CommandBuffer,
        camera_ubo_ds: vk::DescriptorSet,
        current_frame: usize,
    ) {
        let mut batch = self.batch.borrow_mut();
        if batch.quad_count == 0 {
            return;
        }

        let _timer = ProfileTimer::new("Renderer2D::flush");

        // 1. Copy vertex data to the mapped VB at the current write offset.
        let vertex_data = unsafe {
            std::slice::from_raw_parts(
                batch.vertices.as_ptr() as *const u8,
                batch.vertices.len() * std::mem::size_of::<BatchQuadVertex>(),
            )
        };
        let vb_offset = batch.vb_write_offset;
        self.vertex_buffers[current_frame].write_at(vb_offset, vertex_data);

        // 2. Record Vulkan commands.
        let index_count = (batch.quad_count * 6) as u32;
        let active_pipeline = if self.use_offscreen {
            self.offscreen_pipeline
                .as_ref()
                .unwrap_or(&self.batch_pipeline)
        } else {
            &self.batch_pipeline
        };
        let pipeline = active_pipeline.pipeline();
        let layout = active_pipeline.layout();

        unsafe {
            self.device
                .cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, pipeline);

            // Bind camera UBO (set 0) and bindless textures (set 1) together.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[camera_ubo_ds, self.bindless_ds[current_frame]],
                &[],
            );

            // Bind vertex buffer at this batch's offset so the GPU reads
            // the correct region (each flush writes to a distinct sub-region).
            let vb_handle = self.vertex_buffers[current_frame].handle();
            self.device
                .cmd_bind_vertex_buffers(cmd_buf, 0, &[vb_handle], &[vb_offset as u64]);

            // Bind index buffer.
            self.device.cmd_bind_index_buffer(
                cmd_buf,
                self.index_buffer.buffer(),
                0,
                vk::IndexType::UINT32,
            );

            // Draw!
            self.device
                .cmd_draw_indexed(cmd_buf, index_count, 1, 0, 0, 0);
        }

        // 3. Update stats, advance write offset, and reset vertices for next batch.
        batch.stats.draw_calls += 1;
        batch.stats.quad_count += batch.quad_count as u32;
        batch.vb_write_offset = vb_offset + vertex_data.len();

        batch.vertices.clear();
        batch.quad_count = 0;
    }

    /// Get the accumulated statistics for this frame.
    pub(super) fn stats(&self) -> Renderer2DStats {
        self.batch.borrow().stats
    }

    /// Create an offscreen batch pipeline compatible with a multi-attachment
    /// render pass (e.g. framebuffer with 2 color attachments for picking).
    pub(super) fn create_offscreen_pipeline(
        &mut self,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        camera_ubo_ds_layout: vk::DescriptorSetLayout,
        color_attachment_count: u32,
    ) {
        self.offscreen_pipeline = Some(Arc::new(pipeline::create_batch_pipeline(
            device,
            &self._batch_shader,
            self.vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[self.bindless_ds_layout],
            color_attachment_count,
        )));
    }

    /// Tell the batch renderer to use the offscreen pipeline (or switch back).
    pub(super) fn set_use_offscreen(&mut self, use_it: bool) {
        self.use_offscreen = use_it;
    }
}

impl Drop for Renderer2DData {
    fn drop(&mut self) {
        unsafe {
            // Descriptor sets freed when pool is destroyed.
            self.device
                .destroy_descriptor_pool(self.bindless_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.bindless_ds_layout, None);
        }
        // vertex_buffers, index_buffer, _batch_shader, batch_pipeline — dropped by their own Drop impls.
    }
}
