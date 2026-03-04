use std::sync::{Arc, Mutex};

use ash::vk;

pub use gpu_allocator::MemoryLocation;

// ---------------------------------------------------------------------------
// GpuAllocator — wraps gpu_allocator::vulkan::Allocator
// ---------------------------------------------------------------------------

pub struct GpuAllocator {
    inner: gpu_allocator::vulkan::Allocator,
}

impl GpuAllocator {
    pub fn new(
        instance: &ash::Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
    ) -> Self {
        let inner = gpu_allocator::vulkan::Allocator::new(
            &gpu_allocator::vulkan::AllocatorCreateDesc {
                instance: instance.clone(),
                device: device.clone(),
                physical_device,
                debug_settings: gpu_allocator::AllocatorDebugSettings::default(),
                buffer_device_address: false,
                allocation_sizes: gpu_allocator::AllocationSizes::default(),
            },
        )
        .expect("Failed to create GPU allocator");

        log::info!(target: "gg_engine", "GPU sub-allocator initialized");

        Self { inner }
    }

    /// Allocate memory for a buffer. Returns a GpuAllocation that auto-frees on Drop.
    pub fn allocate_for_buffer(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        buffer: vk::Buffer,
        name: &str,
        location: MemoryLocation,
    ) -> Result<GpuAllocation, String> {
        let mem_req = unsafe { device.get_buffer_memory_requirements(buffer) };
        let allocation = {
            let mut alloc = allocator.lock().unwrap();
            alloc
                .inner
                .allocate(&gpu_allocator::vulkan::AllocationCreateDesc {
                    name,
                    requirements: mem_req,
                    location,
                    linear: true,
                    allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(|e| format!("Failed to allocate buffer memory for '{name}': {e}"))?
        };

        unsafe {
            device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
                .map_err(|e| format!("Failed to bind buffer memory for '{name}': {e}"))?;
        }

        Ok(GpuAllocation {
            allocation: Some(allocation),
            allocator: allocator.clone(),
        })
    }

    /// Allocate memory for an image. Returns a GpuAllocation that auto-frees on Drop.
    pub fn allocate_for_image(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        image: vk::Image,
        name: &str,
        location: MemoryLocation,
    ) -> Result<GpuAllocation, String> {
        let mem_req = unsafe { device.get_image_memory_requirements(image) };
        let allocation = {
            let mut alloc = allocator.lock().unwrap();
            alloc
                .inner
                .allocate(&gpu_allocator::vulkan::AllocationCreateDesc {
                    name,
                    requirements: mem_req,
                    location,
                    linear: false,
                    allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(|e| format!("Failed to allocate image memory for '{name}': {e}"))?
        };

        unsafe {
            device
                .bind_image_memory(image, allocation.memory(), allocation.offset())
                .map_err(|e| format!("Failed to bind image memory for '{name}': {e}"))?;
        }

        Ok(GpuAllocation {
            allocation: Some(allocation),
            allocator: allocator.clone(),
        })
    }
}

impl Drop for GpuAllocator {
    fn drop(&mut self) {
        self.inner.report_memory_leaks(log::Level::Warn);
        log::info!(target: "gg_engine", "GPU sub-allocator destroyed");
    }
}

// ---------------------------------------------------------------------------
// GpuAllocation — RAII wrapper that auto-frees on Drop
// ---------------------------------------------------------------------------

pub struct GpuAllocation {
    allocation: Option<gpu_allocator::vulkan::Allocation>,
    allocator: Arc<Mutex<GpuAllocator>>,
}

// Safety: Same contract as the buffers that contain these allocations.
// Access is gated by frame-in-flight fencing.
unsafe impl Send for GpuAllocation {}
unsafe impl Sync for GpuAllocation {}

impl GpuAllocation {
    /// Get the underlying VkDeviceMemory handle.
    pub fn memory(&self) -> vk::DeviceMemory {
        unsafe { self.allocation.as_ref().unwrap().memory() }
    }

    /// Get the byte offset within the device memory block.
    pub fn offset(&self) -> u64 {
        self.allocation.as_ref().unwrap().offset()
    }

    /// Get a mapped pointer if the memory is host-visible.
    /// The pointer already points to this allocation's region (offset applied).
    pub fn mapped_ptr(&self) -> Option<*mut u8> {
        self.allocation
            .as_ref()
            .unwrap()
            .mapped_ptr()
            .map(|p| p.as_ptr() as *mut u8)
    }
}

impl Drop for GpuAllocation {
    fn drop(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            let mut allocator = self.allocator.lock().unwrap();
            if let Err(e) = allocator.inner.free(allocation) {
                log::error!(target: "gg_engine", "Failed to free GPU allocation: {e}");
            }
        }
    }
}
