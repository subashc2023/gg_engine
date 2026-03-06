use std::sync::{Arc, Mutex};

use ash::vk;

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Cast a typed slice to raw bytes for uploading into a vertex buffer.
///
/// # Safety
///
/// The caller must ensure `T` is `#[repr(C)]` with no padding bytes. Types
/// with internal padding contain uninitialized memory, and reading those
/// bytes is undefined behavior. All built-in vertex types (`BatchQuadVertex`,
/// `BatchCircleVertex`, etc.) satisfy this requirement.
pub fn as_bytes<T: Copy>(data: &[T]) -> &[u8] {
    // Safety: T: Copy ensures no drop glue. Caller guarantees #[repr(C)], no padding.
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

// ---------------------------------------------------------------------------
// ShaderDataType
// ---------------------------------------------------------------------------

/// Cross-API shader data type descriptor.
///
/// Used by [`BufferElement`] to describe the type of each vertex attribute.
/// Naming follows HLSL convention (Float3 rather than Vec3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderDataType {
    Float,
    Float2,
    Float3,
    Float4,
    Int,
    Int2,
    Int3,
    Int4,
    Mat3,
    Mat4,
    Bool,
}

impl ShaderDataType {
    /// Size in bytes of this type.
    pub fn size(self) -> u32 {
        match self {
            Self::Float => 4,
            Self::Float2 => 4 * 2,
            Self::Float3 => 4 * 3,
            Self::Float4 => 4 * 4,
            Self::Int => 4,
            Self::Int2 => 4 * 2,
            Self::Int3 => 4 * 3,
            Self::Int4 => 4 * 4,
            Self::Mat3 => 4 * 3 * 3,
            Self::Mat4 => 4 * 4 * 4,
            Self::Bool => 4,
        }
    }

    /// Number of scalar components (e.g. Float3 → 3).
    pub fn component_count(self) -> u32 {
        match self {
            Self::Float | Self::Int | Self::Bool => 1,
            Self::Float2 | Self::Int2 => 2,
            Self::Float3 | Self::Int3 => 3,
            Self::Float4 | Self::Int4 => 4,
            Self::Mat3 => 3 * 3,
            Self::Mat4 => 4 * 4,
        }
    }

    /// Convert to the corresponding Vulkan vertex attribute format.
    pub fn to_vk_format(self) -> vk::Format {
        match self {
            Self::Float => vk::Format::R32_SFLOAT,
            Self::Float2 => vk::Format::R32G32_SFLOAT,
            Self::Float3 => vk::Format::R32G32B32_SFLOAT,
            Self::Float4 => vk::Format::R32G32B32A32_SFLOAT,
            Self::Int => vk::Format::R32_SINT,
            Self::Int2 => vk::Format::R32G32_SINT,
            Self::Int3 => vk::Format::R32G32B32_SINT,
            Self::Int4 => vk::Format::R32G32B32A32_SINT,
            Self::Bool => vk::Format::R32_SINT,
            Self::Mat3 | Self::Mat4 => {
                panic!(
                    "ShaderDataType::{:?} cannot be represented as a single vertex attribute; \
                     matrix types require one attribute per column (not yet implemented)",
                    self
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BufferElement
// ---------------------------------------------------------------------------

/// A single element (attribute) inside a [`BufferLayout`].
#[derive(Debug, Clone)]
pub struct BufferElement {
    pub name: String,
    pub data_type: ShaderDataType,
    pub size: u32,
    pub offset: u32,
    pub normalized: bool,
}

impl BufferElement {
    pub fn new(data_type: ShaderDataType, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            data_type,
            size: data_type.size(),
            offset: 0,
            normalized: false,
        }
    }

    pub fn normalized(mut self, normalized: bool) -> Self {
        self.normalized = normalized;
        self
    }
}

// ---------------------------------------------------------------------------
// BufferLayout
// ---------------------------------------------------------------------------

/// Describes the layout of interleaved vertex data inside a vertex buffer.
///
/// # Example
/// ```ignore
/// let layout = BufferLayout::new(&[
///     BufferElement::new(ShaderDataType::Float3, "a_position"),
///     BufferElement::new(ShaderDataType::Float4, "a_color"),
/// ]);
/// ```
///
/// Offsets and stride are computed automatically.
#[derive(Debug, Clone)]
pub struct BufferLayout {
    elements: Vec<BufferElement>,
    stride: u32,
}

impl BufferLayout {
    pub fn new(elements: &[BufferElement]) -> Self {
        let mut layout = Self {
            elements: elements.to_vec(),
            stride: 0,
        };
        layout.calculate_offsets_and_stride();
        layout
    }

    pub fn stride(&self) -> u32 {
        self.stride
    }

    pub fn elements(&self) -> &[BufferElement] {
        &self.elements
    }

    /// Generate a Vulkan vertex input binding description for this layout.
    pub fn vk_binding_description(&self, binding: u32) -> vk::VertexInputBindingDescription {
        vk::VertexInputBindingDescription {
            binding,
            stride: self.stride,
            input_rate: vk::VertexInputRate::VERTEX,
        }
    }

    /// Generate Vulkan vertex input attribute descriptions for this layout.
    pub fn vk_attribute_descriptions(
        &self,
        binding: u32,
    ) -> Vec<vk::VertexInputAttributeDescription> {
        self.elements
            .iter()
            .enumerate()
            .map(|(location, elem)| vk::VertexInputAttributeDescription {
                location: location as u32,
                binding,
                format: elem.data_type.to_vk_format(),
                offset: elem.offset,
            })
            .collect()
    }

    fn calculate_offsets_and_stride(&mut self) {
        let mut offset = 0u32;
        for elem in &mut self.elements {
            elem.offset = offset;
            offset += elem.size;
        }
        self.stride = offset;
    }
}

// ---------------------------------------------------------------------------
// VertexBuffer
// ---------------------------------------------------------------------------

/// GPU vertex buffer. Created via [`Renderer::create_vertex_buffer`](super::Renderer::create_vertex_buffer).
pub struct VertexBuffer {
    buffer: vk::Buffer,
    _allocation: GpuAllocation,
    layout: Option<BufferLayout>,
    device: ash::Device,
}

impl VertexBuffer {
    pub(crate) fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        data: &[u8],
    ) -> Result<Self, String> {
        let _timer = ProfileTimer::new("VertexBuffer::new");
        let size = data.len() as vk::DeviceSize;

        let (buffer, allocation) =
            create_buffer_with_allocation(allocator, device, size, vk::BufferUsageFlags::VERTEX_BUFFER, "VertexBuffer")?;

        // Copy data via mapped pointer.
        let ptr = allocation
            .mapped_ptr()
            .expect("VertexBuffer allocation must be host-visible");
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        }

        Ok(Self {
            buffer,
            _allocation: allocation,
            layout: None,
            device: device.clone(),
        })
    }

    pub fn set_layout(&mut self, layout: BufferLayout) {
        self.layout = Some(layout);
    }

    pub fn layout(&self) -> Option<&BufferLayout> {
        self.layout.as_ref()
    }

    pub(crate) fn handle(&self) -> vk::Buffer {
        self.buffer
    }
}

impl Drop for VertexBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }
        // GpuAllocation auto-frees memory on drop.
    }
}

// ---------------------------------------------------------------------------
// IndexBuffer
// ---------------------------------------------------------------------------

/// GPU index buffer. Created via [`Renderer::create_index_buffer`](super::Renderer::create_index_buffer).
pub struct IndexBuffer {
    buffer: vk::Buffer,
    _allocation: GpuAllocation,
    count: u32,
    device: ash::Device,
}

impl IndexBuffer {
    pub(crate) fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        indices: &[u32],
    ) -> Result<Self, String> {
        let _timer = ProfileTimer::new("IndexBuffer::new");
        let size = std::mem::size_of_val(indices) as vk::DeviceSize;

        let (buffer, allocation) =
            create_buffer_with_allocation(allocator, device, size, vk::BufferUsageFlags::INDEX_BUFFER, "IndexBuffer")?;

        // Copy data via mapped pointer.
        let ptr = allocation
            .mapped_ptr()
            .expect("IndexBuffer allocation must be host-visible");
        unsafe {
            std::ptr::copy_nonoverlapping(
                indices.as_ptr() as *const u8,
                ptr,
                std::mem::size_of_val(indices),
            );
        }

        Ok(Self {
            buffer,
            _allocation: allocation,
            count: indices.len() as u32,
            device: device.clone(),
        })
    }

    pub(crate) fn bind(&self, device: &ash::Device, cmd_buf: vk::CommandBuffer) {
        unsafe {
            device.cmd_bind_index_buffer(cmd_buf, self.buffer, 0, vk::IndexType::UINT32);
        }
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub(crate) fn buffer(&self) -> vk::Buffer {
        self.buffer
    }
}

impl Drop for IndexBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }
    }
}

// ---------------------------------------------------------------------------
// DynamicVertexBuffer (persistently mapped, for batch rendering)
// ---------------------------------------------------------------------------

/// GPU vertex buffer with persistent host mapping for per-frame streaming.
///
/// Created with `HOST_VISIBLE | HOST_COHERENT` memory that stays mapped for
/// the lifetime of the buffer. Use `write()` to copy vertex data each frame.
pub(crate) struct DynamicVertexBuffer {
    buffer: vk::Buffer,
    allocation: GpuAllocation,
    capacity: usize,
    layout: BufferLayout,
    device: ash::Device,
}

impl DynamicVertexBuffer {
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        capacity: usize,
        layout: BufferLayout,
    ) -> Result<Self, String> {
        let (buffer, allocation) = create_buffer_with_allocation(
            allocator,
            device,
            capacity as vk::DeviceSize,
            vk::BufferUsageFlags::VERTEX_BUFFER,
            "DynamicVertexBuffer",
        )?;

        Ok(Self {
            buffer,
            allocation,
            capacity,
            layout,
            device: device.clone(),
        })
    }

    /// Copy vertex data into the persistently mapped buffer at a byte offset.
    ///
    /// # Panics
    /// Panics if `offset + data.len()` exceeds the buffer's capacity.
    pub fn write_at(&self, offset: usize, data: &[u8]) {
        assert!(
            offset + data.len() <= self.capacity,
            "DynamicVertexBuffer::write_at: offset ({}) + data ({} bytes) exceeds capacity ({} bytes)",
            offset,
            data.len(),
            self.capacity
        );
        let base_ptr = self
            .allocation
            .mapped_ptr()
            .expect("DynamicVertexBuffer must be persistently mapped");
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), base_ptr.add(offset), data.len());
        }
    }

    pub fn handle(&self) -> vk::Buffer {
        self.buffer
    }

    pub fn layout(&self) -> &BufferLayout {
        &self.layout
    }
}

impl Drop for DynamicVertexBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }
        // GpuAllocation auto-frees and auto-unmaps on drop.
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a buffer and allocate host-visible memory for it via the sub-allocator.
pub(super) fn create_buffer_with_allocation(
    allocator: &Arc<Mutex<GpuAllocator>>,
    device: &ash::Device,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    name: &str,
) -> Result<(vk::Buffer, GpuAllocation), String> {
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer =
        unsafe { device.create_buffer(&buffer_info, None) }
            .map_err(|e| format!("Failed to create {name} buffer: {e}"))?;

    let allocation =
        GpuAllocator::allocate_for_buffer(allocator, device, buffer, name, MemoryLocation::CpuToGpu)?;

    Ok((buffer, allocation))
}

/// Create a staging buffer (TRANSFER_SRC, host-visible) and copy data into it.
pub(super) fn create_staging_buffer(
    allocator: &Arc<Mutex<GpuAllocator>>,
    device: &ash::Device,
    data: &[u8],
) -> Result<(vk::Buffer, GpuAllocation), String> {
    let size = data.len() as vk::DeviceSize;

    let (buffer, allocation) = create_buffer_with_allocation(
        allocator,
        device,
        size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        "StagingBuffer",
    )?;

    let ptr = allocation
        .mapped_ptr()
        .expect("Staging buffer must be host-visible");
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
    }

    Ok((buffer, allocation))
}
