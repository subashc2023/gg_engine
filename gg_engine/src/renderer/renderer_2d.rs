use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use ash::vk;

use super::buffer::{
    BufferElement, BufferLayout, DynamicVertexBuffer, IndexBuffer, ShaderDataType,
};
use super::gpu_allocation::GpuAllocator;
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
// BatchQuadVertex — per-vertex data for quad batch rendering
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
fn batch_quad_vertex_layout() -> BufferLayout {
    BufferLayout::new(&[
        BufferElement::new(ShaderDataType::Float3, "a_position"),
        BufferElement::new(ShaderDataType::Float4, "a_color"),
        BufferElement::new(ShaderDataType::Float2, "a_tex_coord"),
        BufferElement::new(ShaderDataType::Float, "a_tex_index"),
        BufferElement::new(ShaderDataType::Int, "a_entity_id"),
    ])
}

// ---------------------------------------------------------------------------
// BatchCircleVertex — per-vertex data for circle batch rendering
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct BatchCircleVertex {
    pub world_position: [f32; 3],
    pub local_position: [f32; 3],
    pub color: [f32; 4],
    pub thickness: f32,
    pub fade: f32,
    pub entity_id: i32,
}

/// The canonical buffer layout for batch circle vertices.
fn batch_circle_vertex_layout() -> BufferLayout {
    BufferLayout::new(&[
        BufferElement::new(ShaderDataType::Float3, "a_world_position"),
        BufferElement::new(ShaderDataType::Float3, "a_local_position"),
        BufferElement::new(ShaderDataType::Float4, "a_color"),
        BufferElement::new(ShaderDataType::Float, "a_thickness"),
        BufferElement::new(ShaderDataType::Float, "a_fade"),
        BufferElement::new(ShaderDataType::Int, "a_entity_id"),
    ])
}

// ---------------------------------------------------------------------------
// BatchLineVertex — per-vertex data for line batch rendering
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct BatchLineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub entity_id: i32,
}

/// The canonical buffer layout for batch line vertices.
fn batch_line_vertex_layout() -> BufferLayout {
    BufferLayout::new(&[
        BufferElement::new(ShaderDataType::Float3, "a_position"),
        BufferElement::new(ShaderDataType::Float4, "a_color"),
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

struct QuadBatchState {
    vertices: Vec<BatchQuadVertex>,
    quad_count: usize,
    /// Byte offset into the vertex buffer for the next flush.
    vb_write_offset: usize,
    stats: Renderer2DStats,
}

impl QuadBatchState {
    fn new() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_VERTICES),
            quad_count: 0,
            vb_write_offset: 0,
            stats: Renderer2DStats::default(),
        }
    }
}

struct CircleBatchState {
    vertices: Vec<BatchCircleVertex>,
    quad_count: usize,
    /// Byte offset into the circle vertex buffer for the next flush.
    vb_write_offset: usize,
    stats: Renderer2DStats,
}

impl CircleBatchState {
    fn new() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_VERTICES),
            quad_count: 0,
            vb_write_offset: 0,
            stats: Renderer2DStats::default(),
        }
    }
}

const MAX_LINES: usize = 10_000;
const MAX_LINE_VERTICES: usize = MAX_LINES * 2;

struct LineBatchState {
    vertices: Vec<BatchLineVertex>,
    line_count: usize,
    /// Byte offset into the line vertex buffer for the next flush.
    vb_write_offset: usize,
    stats: Renderer2DStats,
}

impl LineBatchState {
    fn new() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_LINE_VERTICES),
            line_count: 0,
            vb_write_offset: 0,
            stats: Renderer2DStats::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// TextBatchState — same vertex layout as quads, different shader (MSDF)
// ---------------------------------------------------------------------------

struct TextBatchState {
    vertices: Vec<BatchQuadVertex>,
    quad_count: usize,
    /// Byte offset into the text vertex buffer for the next flush.
    vb_write_offset: usize,
    stats: Renderer2DStats,
}

impl TextBatchState {
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
    // -- Quad batch resources --
    _batch_shader: Arc<Shader>,
    batch_pipeline: Arc<Pipeline>,
    offscreen_pipeline: Option<Arc<Pipeline>>,
    use_offscreen: bool,
    vertex_buffers: [DynamicVertexBuffer; FRAMES_IN_FLIGHT],
    index_buffer: IndexBuffer,
    quad_batch: RefCell<QuadBatchState>,

    // -- Circle batch resources --
    _circle_shader: Arc<Shader>,
    circle_pipeline: Arc<Pipeline>,
    circle_offscreen_pipeline: Option<Arc<Pipeline>>,
    circle_vertex_buffers: [DynamicVertexBuffer; FRAMES_IN_FLIGHT],
    // Circle reuses the same index_buffer (identical quad topology).
    circle_batch: RefCell<CircleBatchState>,

    // -- Line batch resources --
    _line_shader: Arc<Shader>,
    line_pipeline: Arc<Pipeline>,
    line_offscreen_pipeline: Option<Arc<Pipeline>>,
    line_vertex_buffers: [DynamicVertexBuffer; FRAMES_IN_FLIGHT],
    // Lines don't use an index buffer (drawn with vkCmdDraw).
    line_batch: RefCell<LineBatchState>,

    // -- Text batch resources (same vertex format as quads, MSDF shader) --
    _text_shader: Arc<Shader>,
    text_pipeline: Arc<Pipeline>,
    text_offscreen_pipeline: Option<Arc<Pipeline>>,
    text_vertex_buffers: [DynamicVertexBuffer; FRAMES_IN_FLIGHT],
    // Text reuses the same index_buffer (identical quad topology).
    text_batch: RefCell<TextBatchState>,

    // -- Shared resources --
    bindless_pool: vk::DescriptorPool,
    bindless_ds_layout: vk::DescriptorSetLayout,
    bindless_ds: [vk::DescriptorSet; FRAMES_IN_FLIGHT],
    /// Free-list allocator for bindless texture slots. Slots are returned via
    /// `unregister_texture` and reused on the next `register_texture` call.
    bindless_free_list: RefCell<Vec<u32>>,
    next_bindless_index: RefCell<u32>,
    pub(super) white_texture: Texture2D,
    device: ash::Device,
}

impl Renderer2DData {
    pub(super) fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        camera_ubo_ds_layout: vk::DescriptorSetLayout,
        white_texture: Texture2D,
        pipeline_cache: vk::PipelineCache,
    ) -> Self {
        let _timer = ProfileTimer::new("Renderer2D::init");

        // -- Quad Shaders --
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

        // -- Circle Shaders --
        let circle_swapchain_shader = Arc::new(Shader::new(
            device,
            "circle_swapchain",
            shaders::CIRCLE_SWAPCHAIN_VERT_SPV,
            shaders::CIRCLE_SWAPCHAIN_FRAG_SPV,
        ));
        let circle_shader = Arc::new(Shader::new(
            device,
            "circle",
            shaders::CIRCLE_VERT_SPV,
            shaders::CIRCLE_FRAG_SPV,
        ));

        // -- Line Shaders --
        let line_swapchain_shader = Arc::new(Shader::new(
            device,
            "line_swapchain",
            shaders::LINE_SWAPCHAIN_VERT_SPV,
            shaders::LINE_SWAPCHAIN_FRAG_SPV,
        ));
        let line_shader = Arc::new(Shader::new(
            device,
            "line",
            shaders::LINE_VERT_SPV,
            shaders::LINE_FRAG_SPV,
        ));

        // -- Text Shaders --
        let text_swapchain_shader = Arc::new(Shader::new(
            device,
            "text_swapchain",
            shaders::TEXT_SWAPCHAIN_VERT_SPV,
            shaders::TEXT_SWAPCHAIN_FRAG_SPV,
        ));
        let text_shader = Arc::new(Shader::new(
            device,
            "text",
            shaders::TEXT_VERT_SPV,
            shaders::TEXT_FRAG_SPV,
        ));

        // -- Quad Vertex layout --
        let quad_layout = batch_quad_vertex_layout();
        let quad_vb_capacity =
            MAX_VERTICES * MAX_BATCHES_PER_FRAME * std::mem::size_of::<BatchQuadVertex>();

        // -- Circle Vertex layout --
        let circle_layout = batch_circle_vertex_layout();
        let circle_vb_capacity =
            MAX_VERTICES * MAX_BATCHES_PER_FRAME * std::mem::size_of::<BatchCircleVertex>();

        // -- Per-frame-in-flight quad vertex buffers (persistently mapped) --
        let vertex_buffers = [
            DynamicVertexBuffer::new(allocator, device, quad_vb_capacity, quad_layout.clone()),
            DynamicVertexBuffer::new(allocator, device, quad_vb_capacity, quad_layout.clone()),
        ];

        // -- Per-frame-in-flight circle vertex buffers (persistently mapped) --
        let circle_vertex_buffers = [
            DynamicVertexBuffer::new(allocator, device, circle_vb_capacity, circle_layout.clone()),
            DynamicVertexBuffer::new(allocator, device, circle_vb_capacity, circle_layout.clone()),
        ];

        // -- Line Vertex layout --
        let line_layout = batch_line_vertex_layout();
        let line_vb_capacity =
            MAX_LINE_VERTICES * MAX_BATCHES_PER_FRAME * std::mem::size_of::<BatchLineVertex>();

        // -- Per-frame-in-flight line vertex buffers (persistently mapped) --
        let line_vertex_buffers = [
            DynamicVertexBuffer::new(allocator, device, line_vb_capacity, line_layout.clone()),
            DynamicVertexBuffer::new(allocator, device, line_vb_capacity, line_layout.clone()),
        ];

        // -- Per-frame-in-flight text vertex buffers (same layout as quads) --
        let text_vertex_buffers = [
            DynamicVertexBuffer::new(allocator, device, quad_vb_capacity, quad_layout.clone()),
            DynamicVertexBuffer::new(allocator, device, quad_vb_capacity, quad_layout.clone()),
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
        let index_buffer = IndexBuffer::new(allocator, device, &indices);

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

        // -- Quad Pipeline (swapchain: 1 color attachment, no entity ID output) --
        let batch_pipeline = Arc::new(pipeline::create_batch_pipeline(
            device,
            &batch_swapchain_shader,
            vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[bindless_ds_layout],
            1,
            pipeline_cache,
        ));

        // -- Circle Pipeline (swapchain: 1 color attachment, no entity ID output) --
        // Circles don't use textures, so no bindless descriptor set needed.
        let circle_pipeline = Arc::new(pipeline::create_batch_pipeline(
            device,
            &circle_swapchain_shader,
            circle_vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[],
            1,
            pipeline_cache,
        ));

        // -- Line Pipeline (swapchain: 1 color attachment, no entity ID output) --
        // Lines don't use textures, so no bindless descriptor set needed.
        let line_pipeline = Arc::new(pipeline::create_line_batch_pipeline(
            device,
            &line_swapchain_shader,
            line_vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            1,
            pipeline_cache,
        ));

        // -- Text Pipeline (swapchain: 1 color attachment, uses bindless textures for font atlas) --
        // Text uses the same vertex layout as quads but a different (MSDF) fragment shader.
        let text_pipeline = Arc::new(pipeline::create_batch_pipeline(
            device,
            &text_swapchain_shader,
            vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[bindless_ds_layout],
            1,
            pipeline_cache,
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
            quad_batch: RefCell::new(QuadBatchState::new()),

            _circle_shader: circle_shader,
            circle_pipeline,
            circle_offscreen_pipeline: None,
            circle_vertex_buffers,
            circle_batch: RefCell::new(CircleBatchState::new()),

            _line_shader: line_shader,
            line_pipeline,
            line_offscreen_pipeline: None,
            line_vertex_buffers,
            line_batch: RefCell::new(LineBatchState::new()),

            _text_shader: text_shader,
            text_pipeline,
            text_offscreen_pipeline: None,
            text_vertex_buffers,
            text_batch: RefCell::new(TextBatchState::new()),

            bindless_pool,
            bindless_ds_layout,
            bindless_ds,
            bindless_free_list: RefCell::new(Vec::new()),
            next_bindless_index: RefCell::new(0),
            white_texture,
            device: device.clone(),
        }
    }

    /// Register a texture in the bindless descriptor array. Writes its
    /// image_view + sampler into both per-frame descriptor sets at the
    /// assigned index. Returns the global bindless index.
    ///
    /// Recycles slots returned by [`unregister_texture`] before allocating new ones.
    pub(super) fn register_texture(&self, texture: &Texture2D) -> u32 {
        // Try to reuse a freed slot first.
        let index = if let Some(recycled) = self.bindless_free_list.borrow_mut().pop() {
            recycled
        } else {
            let mut next = self.next_bindless_index.borrow_mut();
            let index = *next;
            assert!(
                index < MAX_BINDLESS_TEXTURES,
                "Exceeded max bindless textures ({})",
                MAX_BINDLESS_TEXTURES
            );
            *next = index + 1;
            index
        };

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

    /// Return a bindless texture slot to the free-list for reuse.
    ///
    /// The descriptor array entry is not cleared — it will be overwritten
    /// on the next `register_texture` call that recycles this slot.
    /// Slot 0 (white texture) should never be unregistered.
    pub(super) fn unregister_texture(&self, index: u32) {
        if index == 0 {
            return; // Never free the white texture slot.
        }
        self.bindless_free_list.borrow_mut().push(index);
    }

    /// Reset batch state for a new frame.
    pub(super) fn reset_batch(&self) {
        // Reset quad batch.
        {
            let mut batch = self.quad_batch.borrow_mut();
            batch.vertices.clear();
            batch.quad_count = 0;
            batch.vb_write_offset = 0;
            batch.stats = Renderer2DStats::default();
        }
        // Reset circle batch.
        {
            let mut batch = self.circle_batch.borrow_mut();
            batch.vertices.clear();
            batch.quad_count = 0;
            batch.vb_write_offset = 0;
            batch.stats = Renderer2DStats::default();
        }
        // Reset line batch.
        {
            let mut batch = self.line_batch.borrow_mut();
            batch.vertices.clear();
            batch.line_count = 0;
            batch.vb_write_offset = 0;
            batch.stats = Renderer2DStats::default();
        }
        // Reset text batch.
        {
            let mut batch = self.text_batch.borrow_mut();
            batch.vertices.clear();
            batch.quad_count = 0;
            batch.vb_write_offset = 0;
            batch.stats = Renderer2DStats::default();
        }
    }

    // -- Quad batch operations --

    /// Push a quad into the current batch. The 4 vertices should already be
    /// in world space (pre-transformed). Returns false if the batch was full
    /// and a flush is needed first.
    pub(super) fn push_quad(&self, vertices: [BatchQuadVertex; 4]) -> bool {
        let mut batch = self.quad_batch.borrow_mut();
        if batch.quad_count >= MAX_QUADS {
            return false;
        }
        batch.vertices.extend_from_slice(&vertices);
        batch.quad_count += 1;
        true
    }

    /// Returns true if there are quads to flush.
    pub(super) fn has_pending_quads(&self) -> bool {
        self.quad_batch.borrow().quad_count > 0
    }

    /// Flush the current quad batch: write vertices to GPU, bind the pre-populated
    /// bindless descriptor set, and record draw commands.
    pub(super) fn flush_quads(
        &self,
        cmd_buf: vk::CommandBuffer,
        camera_ubo_ds: vk::DescriptorSet,
        current_frame: usize,
    ) {
        let mut batch = self.quad_batch.borrow_mut();
        if batch.quad_count == 0 {
            return;
        }

        let _timer = ProfileTimer::new("Renderer2D::flush_quads");

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

    // -- Circle batch operations --

    /// Push a circle (rendered as a quad) into the current circle batch.
    /// The 4 vertices should already be in world space (pre-transformed).
    /// Returns false if the batch was full and a flush is needed first.
    pub(super) fn push_circle(&self, vertices: [BatchCircleVertex; 4]) -> bool {
        let mut batch = self.circle_batch.borrow_mut();
        if batch.quad_count >= MAX_QUADS {
            return false;
        }
        batch.vertices.extend_from_slice(&vertices);
        batch.quad_count += 1;
        true
    }

    /// Returns true if there are circles to flush.
    pub(super) fn has_pending_circles(&self) -> bool {
        self.circle_batch.borrow().quad_count > 0
    }

    /// Flush the current circle batch: write vertices to GPU, bind the circle
    /// pipeline, and record draw commands.
    pub(super) fn flush_circles(
        &self,
        cmd_buf: vk::CommandBuffer,
        camera_ubo_ds: vk::DescriptorSet,
        current_frame: usize,
    ) {
        let mut batch = self.circle_batch.borrow_mut();
        if batch.quad_count == 0 {
            return;
        }

        let _timer = ProfileTimer::new("Renderer2D::flush_circles");

        // 1. Copy vertex data to the mapped VB at the current write offset.
        let vertex_data = unsafe {
            std::slice::from_raw_parts(
                batch.vertices.as_ptr() as *const u8,
                batch.vertices.len() * std::mem::size_of::<BatchCircleVertex>(),
            )
        };
        let vb_offset = batch.vb_write_offset;
        self.circle_vertex_buffers[current_frame].write_at(vb_offset, vertex_data);

        // 2. Record Vulkan commands.
        let index_count = (batch.quad_count * 6) as u32;
        let active_pipeline = if self.use_offscreen {
            self.circle_offscreen_pipeline
                .as_ref()
                .unwrap_or(&self.circle_pipeline)
        } else {
            &self.circle_pipeline
        };
        let pipeline = active_pipeline.pipeline();
        let layout = active_pipeline.layout();

        unsafe {
            self.device
                .cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, pipeline);

            // Bind camera UBO (set 0) only — circles don't use textures.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[camera_ubo_ds],
                &[],
            );

            // Bind circle vertex buffer at this batch's offset.
            let vb_handle = self.circle_vertex_buffers[current_frame].handle();
            self.device
                .cmd_bind_vertex_buffers(cmd_buf, 0, &[vb_handle], &[vb_offset as u64]);

            // Bind index buffer (shared with quads — same topology).
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

    /// Get the accumulated quad statistics for this frame.
    pub(super) fn quad_stats(&self) -> Renderer2DStats {
        self.quad_batch.borrow().stats
    }

    /// Get the accumulated circle statistics for this frame.
    pub(super) fn circle_stats(&self) -> Renderer2DStats {
        self.circle_batch.borrow().stats
    }

    // -- Line batch operations --

    /// Push a line (2 vertices) into the current line batch.
    /// Returns false if the batch was full and a flush is needed first.
    pub(super) fn push_line(&self, vertices: [BatchLineVertex; 2]) -> bool {
        let mut batch = self.line_batch.borrow_mut();
        if batch.line_count >= MAX_LINES {
            return false;
        }
        batch.vertices.extend_from_slice(&vertices);
        batch.line_count += 1;
        true
    }

    /// Returns true if there are lines to flush.
    pub(super) fn has_pending_lines(&self) -> bool {
        self.line_batch.borrow().line_count > 0
    }

    /// Flush the current line batch: write vertices to GPU, bind the line
    /// pipeline, set line width, and record draw commands.
    pub(super) fn flush_lines(
        &self,
        cmd_buf: vk::CommandBuffer,
        camera_ubo_ds: vk::DescriptorSet,
        current_frame: usize,
        line_width: f32,
    ) {
        let mut batch = self.line_batch.borrow_mut();
        if batch.line_count == 0 {
            return;
        }

        let _timer = ProfileTimer::new("Renderer2D::flush_lines");

        // 1. Copy vertex data to the mapped VB at the current write offset.
        let vertex_data = unsafe {
            std::slice::from_raw_parts(
                batch.vertices.as_ptr() as *const u8,
                batch.vertices.len() * std::mem::size_of::<BatchLineVertex>(),
            )
        };
        let vb_offset = batch.vb_write_offset;
        self.line_vertex_buffers[current_frame].write_at(vb_offset, vertex_data);

        // 2. Record Vulkan commands.
        let vertex_count = (batch.line_count * 2) as u32;
        let active_pipeline = if self.use_offscreen {
            self.line_offscreen_pipeline
                .as_ref()
                .unwrap_or(&self.line_pipeline)
        } else {
            &self.line_pipeline
        };
        let pipeline = active_pipeline.pipeline();
        let layout = active_pipeline.layout();

        unsafe {
            self.device
                .cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, pipeline);

            // Set line width (dynamic state).
            self.device.cmd_set_line_width(cmd_buf, line_width);

            // Bind camera UBO (set 0) only — lines don't use textures.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[camera_ubo_ds],
                &[],
            );

            // Bind line vertex buffer at this batch's offset.
            let vb_handle = self.line_vertex_buffers[current_frame].handle();
            self.device
                .cmd_bind_vertex_buffers(cmd_buf, 0, &[vb_handle], &[vb_offset as u64]);

            // Draw! Lines use cmd_draw (non-indexed).
            self.device.cmd_draw(cmd_buf, vertex_count, 1, 0, 0);
        }

        // 3. Update stats, advance write offset, and reset vertices for next batch.
        batch.stats.draw_calls += 1;
        batch.stats.quad_count += batch.line_count as u32;
        batch.vb_write_offset = vb_offset + vertex_data.len();

        batch.vertices.clear();
        batch.line_count = 0;
    }

    /// Get the accumulated line statistics for this frame.
    pub(super) fn line_stats(&self) -> Renderer2DStats {
        self.line_batch.borrow().stats
    }

    // -- Text batch operations --

    /// Push a text quad into the current text batch.
    /// Returns false if the batch was full and a flush is needed first.
    pub(super) fn push_text_quad(&self, vertices: [BatchQuadVertex; 4]) -> bool {
        let mut batch = self.text_batch.borrow_mut();
        if batch.quad_count >= MAX_QUADS {
            return false;
        }
        batch.vertices.extend_from_slice(&vertices);
        batch.quad_count += 1;
        true
    }

    /// Returns true if there are text quads to flush.
    pub(super) fn has_pending_text(&self) -> bool {
        self.text_batch.borrow().quad_count > 0
    }

    /// Flush the current text batch: write vertices to GPU, bind the MSDF text
    /// pipeline, and record draw commands.
    pub(super) fn flush_text(
        &self,
        cmd_buf: vk::CommandBuffer,
        camera_ubo_ds: vk::DescriptorSet,
        current_frame: usize,
    ) {
        let mut batch = self.text_batch.borrow_mut();
        if batch.quad_count == 0 {
            return;
        }

        let _timer = ProfileTimer::new("Renderer2D::flush_text");

        // 1. Copy vertex data to the mapped VB at the current write offset.
        let vertex_data = unsafe {
            std::slice::from_raw_parts(
                batch.vertices.as_ptr() as *const u8,
                batch.vertices.len() * std::mem::size_of::<BatchQuadVertex>(),
            )
        };
        let vb_offset = batch.vb_write_offset;
        self.text_vertex_buffers[current_frame].write_at(vb_offset, vertex_data);

        // 2. Record Vulkan commands.
        let index_count = (batch.quad_count * 6) as u32;
        let active_pipeline = if self.use_offscreen {
            self.text_offscreen_pipeline
                .as_ref()
                .unwrap_or(&self.text_pipeline)
        } else {
            &self.text_pipeline
        };
        let pipeline = active_pipeline.pipeline();
        let layout = active_pipeline.layout();

        unsafe {
            self.device
                .cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, pipeline);

            // Bind camera UBO (set 0) and bindless textures (set 1).
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[camera_ubo_ds, self.bindless_ds[current_frame]],
                &[],
            );

            // Bind text vertex buffer at this batch's offset.
            let vb_handle = self.text_vertex_buffers[current_frame].handle();
            self.device
                .cmd_bind_vertex_buffers(cmd_buf, 0, &[vb_handle], &[vb_offset as u64]);

            // Bind index buffer (shared with quads — same topology).
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

    /// Get the accumulated text statistics for this frame.
    pub(super) fn text_stats(&self) -> Renderer2DStats {
        self.text_batch.borrow().stats
    }

    /// Create offscreen batch pipelines compatible with a multi-attachment
    /// render pass (e.g. framebuffer with 2 color attachments for picking).
    pub(super) fn create_offscreen_pipeline(
        &mut self,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        camera_ubo_ds_layout: vk::DescriptorSetLayout,
        color_attachment_count: u32,
        pipeline_cache: vk::PipelineCache,
    ) {
        // Quad offscreen pipeline (with bindless textures at set 1).
        self.offscreen_pipeline = Some(Arc::new(pipeline::create_batch_pipeline(
            device,
            &self._batch_shader,
            self.vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[self.bindless_ds_layout],
            color_attachment_count,
            pipeline_cache,
        )));

        // Circle offscreen pipeline (no textures).
        self.circle_offscreen_pipeline = Some(Arc::new(pipeline::create_batch_pipeline(
            device,
            &self._circle_shader,
            self.circle_vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[],
            color_attachment_count,
            pipeline_cache,
        )));

        // Line offscreen pipeline (no textures, LINE_LIST topology).
        self.line_offscreen_pipeline = Some(Arc::new(pipeline::create_line_batch_pipeline(
            device,
            &self._line_shader,
            self.line_vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            color_attachment_count,
            pipeline_cache,
        )));

        // Text offscreen pipeline (with bindless textures at set 1, MSDF shader).
        self.text_offscreen_pipeline = Some(Arc::new(pipeline::create_batch_pipeline(
            device,
            &self._text_shader,
            self.text_vertex_buffers[0].layout(),
            render_pass,
            camera_ubo_ds_layout,
            &[self.bindless_ds_layout],
            color_attachment_count,
            pipeline_cache,
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
