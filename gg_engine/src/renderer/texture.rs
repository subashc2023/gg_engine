use std::path::Path;

use ash::vk;

use super::buffer::{create_staging_buffer, find_memory_type};

// ---------------------------------------------------------------------------
// Texture2D
// ---------------------------------------------------------------------------

/// A 2D texture backed by a Vulkan image, image view, sampler, and descriptor
/// set. Created via [`Renderer::create_texture_from_file`] or
/// [`Renderer::create_texture_from_rgba8`].
pub struct Texture2D {
    image: vk::Image,
    memory: vk::DeviceMemory,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    descriptor_set: vk::DescriptorSet,
    _width: u32,
    _height: u32,
    device: ash::Device,
}

impl Texture2D {
    /// Load a texture from an image file (PNG, JPEG, etc).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_file(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        graphics_queue: vk::Queue,
        command_pool: vk::CommandPool,
        descriptor_pool: vk::DescriptorPool,
        descriptor_set_layout: vk::DescriptorSetLayout,
        path: &Path,
    ) -> Self {
        let img = image::open(path)
            .unwrap_or_else(|e| panic!("Failed to load texture '{}': {e}", path.display()));
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        let pixels = rgba.into_raw();

        Self::from_rgba8(
            instance,
            physical_device,
            device,
            graphics_queue,
            command_pool,
            descriptor_pool,
            descriptor_set_layout,
            width,
            height,
            &pixels,
        )
    }

    /// Create a texture from raw RGBA8 pixel data.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_rgba8(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        graphics_queue: vk::Queue,
        command_pool: vk::CommandPool,
        descriptor_pool: vk::DescriptorPool,
        descriptor_set_layout: vk::DescriptorSetLayout,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Self {
        let image_size = (width * height * 4) as vk::DeviceSize;
        assert_eq!(pixels.len() as vk::DeviceSize, image_size);

        // 1. Create staging buffer with pixel data.
        let (staging_buffer, staging_memory) =
            create_staging_buffer(instance, physical_device, device, pixels);

        // 2. Create Vulkan image.
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .format(vk::Format::R8G8B8A8_SRGB)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1);

        let image =
            unsafe { device.create_image(&image_info, None) }.expect("Failed to create image");

        // 3. Allocate and bind DEVICE_LOCAL memory.
        let mem_req = unsafe { device.get_image_memory_requirements(image) };
        let mem_type_index = find_memory_type(
            instance,
            physical_device,
            mem_req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_req.size)
            .memory_type_index(mem_type_index);

        let memory = unsafe { device.allocate_memory(&alloc_info, None) }
            .expect("Failed to allocate image memory");
        unsafe { device.bind_image_memory(image, memory, 0) }.expect("Failed to bind image memory");

        // 4. One-shot command buffer: transition + copy + transition.
        execute_one_shot(device, command_pool, graphics_queue, |cmd_buf| {
            // UNDEFINED -> TRANSFER_DST_OPTIMAL
            transition_image_layout(
                device,
                cmd_buf,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            );

            // Copy buffer to image.
            let region = vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            };

            unsafe {
                device.cmd_copy_buffer_to_image(
                    cmd_buf,
                    staging_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            // TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL
            transition_image_layout(
                device,
                cmd_buf,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        });

        // 5. Destroy staging buffer.
        unsafe {
            device.free_memory(staging_memory, None);
            device.destroy_buffer(staging_buffer, None);
        }

        // 6. Create image view.
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });

        let image_view = unsafe { device.create_image_view(&view_info, None) }
            .expect("Failed to create image view");

        // 7. Create sampler.
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .anisotropy_enable(true)
            .max_anisotropy(16.0)
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .mip_lod_bias(0.0)
            .min_lod(0.0)
            .max_lod(0.0);

        let sampler = unsafe { device.create_sampler(&sampler_info, None) }
            .expect("Failed to create sampler");

        // 8. Allocate descriptor set and write combined image sampler.
        let layouts = [descriptor_set_layout];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);

        let descriptor_set = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .expect("Failed to allocate descriptor set")[0];

        let image_info_ds = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(image_view)
            .sampler(sampler);

        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(std::slice::from_ref(&image_info_ds));

        unsafe {
            device.update_descriptor_sets(&[write], &[]);
        }

        Self {
            image,
            memory,
            image_view,
            sampler,
            descriptor_set,
            _width: width,
            _height: height,
            device: device.clone(),
        }
    }

    /// The descriptor set for binding this texture in a draw call.
    pub fn descriptor_set(&self) -> vk::DescriptorSet {
        self.descriptor_set
    }
}

impl Drop for Texture2D {
    fn drop(&mut self) {
        unsafe {
            // Descriptor set is freed when the pool is destroyed/reset.
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Record, submit, and wait for a one-shot command buffer.
fn execute_one_shot(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    record: impl FnOnce(vk::CommandBuffer),
) {
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(command_pool)
        .command_buffer_count(1);

    let cmd_buf = unsafe { device.allocate_command_buffers(&alloc_info) }
        .expect("Failed to allocate one-shot command buffer")[0];

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

    unsafe {
        device
            .begin_command_buffer(cmd_buf, &begin_info)
            .expect("Failed to begin one-shot command buffer");
    }

    record(cmd_buf);

    unsafe {
        device
            .end_command_buffer(cmd_buf)
            .expect("Failed to end one-shot command buffer");

        let cmd_bufs = [cmd_buf];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
        device
            .queue_submit(queue, &[submit_info], vk::Fence::null())
            .expect("Failed to submit one-shot command buffer");
        device
            .queue_wait_idle(queue)
            .expect("Failed to wait for queue idle");

        device.free_command_buffers(command_pool, &[cmd_buf]);
    }
}

/// Insert a pipeline barrier to transition an image between layouts.
fn transition_image_layout(
    device: &ash::Device,
    cmd_buf: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
) {
    let (src_access, dst_access, src_stage, dst_stage) = match (old_layout, new_layout) {
        (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        ),
        (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) => (
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
        _ => panic!("Unsupported image layout transition: {old_layout:?} -> {new_layout:?}"),
    };

    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old_layout)
        .new_layout(new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        })
        .src_access_mask(src_access)
        .dst_access_mask(dst_access);

    unsafe {
        device.cmd_pipeline_barrier(
            cmd_buf,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}
