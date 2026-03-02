use std::ffi::CStr;
use std::sync::Arc;

use ash::khr::surface;
use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum VulkanInitError {
    LoadFailed(String),
    NoDisplayHandle,
    NoWindowHandle,
    SurfaceExtensions(vk::Result),
    InstanceCreation(vk::Result),
    #[cfg(debug_assertions)]
    DebugMessenger(vk::Result),
    SurfaceCreation(vk::Result),
    EnumerateDevices(vk::Result),
    NoSuitableGpu,
    DeviceCreation(vk::Result),
}

impl std::fmt::Display for VulkanInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadFailed(e) => write!(f, "Failed to load Vulkan library: {e}"),
            Self::NoDisplayHandle => write!(f, "Could not obtain display handle"),
            Self::NoWindowHandle => write!(f, "Could not obtain window handle"),
            Self::SurfaceExtensions(e) => {
                write!(f, "Failed to enumerate surface extensions: {e}")
            }
            Self::InstanceCreation(e) => write!(f, "Failed to create Vulkan instance: {e}"),
            #[cfg(debug_assertions)]
            Self::DebugMessenger(e) => write!(f, "Failed to create debug messenger: {e}"),
            Self::SurfaceCreation(e) => write!(f, "Failed to create Vulkan surface: {e}"),
            Self::EnumerateDevices(e) => {
                write!(f, "Failed to enumerate physical devices: {e}")
            }
            Self::NoSuitableGpu => write!(f, "No suitable GPU found"),
            Self::DeviceCreation(e) => write!(f, "Failed to create logical device: {e}"),
        }
    }
}

impl std::error::Error for VulkanInitError {}

// ---------------------------------------------------------------------------
// VulkanContext
// ---------------------------------------------------------------------------

/// Holds all Vulkan state needed for rendering.
/// Fields are ordered for correct drop order (reverse of creation).
pub struct VulkanContext {
    device: ash::Device,
    graphics_queue: vk::Queue,
    graphics_queue_family_index: u32,

    surface_loader: surface::Instance,
    surface: vk::SurfaceKHR,

    physical_device: vk::PhysicalDevice,
    physical_device_properties: vk::PhysicalDeviceProperties,

    #[cfg(debug_assertions)]
    debug_utils_loader: ash::ext::debug_utils::Instance,
    #[cfg(debug_assertions)]
    debug_messenger: vk::DebugUtilsMessengerEXT,

    instance: ash::Instance,

    // Must outlive instance (Entry::load returns a dynamically loaded entry).
    _entry: ash::Entry,
}

impl VulkanContext {
    /// Create a Vulkan context tied to the given window.
    pub fn new(window: &Arc<Window>) -> Result<Self, VulkanInitError> {
        // Step 1: Load Vulkan library
        let entry = unsafe { ash::Entry::load() }
            .map_err(|e| VulkanInitError::LoadFailed(e.to_string()))?;
        log::info!(target: "gg_engine", "Vulkan library loaded");

        // Step 2: Determine required extensions
        let display_handle = window
            .display_handle()
            .map_err(|_| VulkanInitError::NoDisplayHandle)?;
        let raw_display = display_handle.as_raw();

        #[allow(unused_mut)] // mut only used in debug builds (validation layer ext)
        let mut required_extensions = ash_window::enumerate_required_extensions(raw_display)
            .map_err(VulkanInitError::SurfaceExtensions)?
            .to_vec();

        #[cfg(debug_assertions)]
        required_extensions.push(ash::ext::debug_utils::NAME.as_ptr());

        // Step 3: Validation layers (debug only)
        #[cfg(debug_assertions)]
        let layer_names = [c"VK_LAYER_KHRONOS_validation"];
        #[cfg(debug_assertions)]
        let layer_name_ptrs: Vec<*const std::ffi::c_char> =
            layer_names.iter().map(|n| n.as_ptr()).collect();

        #[cfg(not(debug_assertions))]
        let layer_name_ptrs: Vec<*const std::ffi::c_char> = vec![];

        // Step 4: Create instance
        let app_info = vk::ApplicationInfo::default()
            .application_name(c"GGEngine Application")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"GGEngine")
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);

        let instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&required_extensions)
            .enabled_layer_names(&layer_name_ptrs);

        let instance = unsafe { entry.create_instance(&instance_create_info, None) }
            .map_err(VulkanInitError::InstanceCreation)?;
        log::info!(target: "gg_engine", "Vulkan instance created");

        // Step 5: Debug messenger (debug only)
        #[cfg(debug_assertions)]
        let (debug_utils_loader, debug_messenger) = {
            let debug_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                        | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(vulkan_debug_callback));

            let loader = ash::ext::debug_utils::Instance::new(&entry, &instance);
            let messenger = unsafe { loader.create_debug_utils_messenger(&debug_info, None) }
                .map_err(VulkanInitError::DebugMessenger)?;
            log::info!(target: "gg_engine", "Vulkan debug messenger created");
            (loader, messenger)
        };

        // Step 6: Create surface
        let window_handle = window
            .window_handle()
            .map_err(|_| VulkanInitError::NoWindowHandle)?;
        let raw_window = window_handle.as_raw();

        let surface =
            unsafe { ash_window::create_surface(&entry, &instance, raw_display, raw_window, None) }
                .map_err(VulkanInitError::SurfaceCreation)?;
        let surface_loader = surface::Instance::new(&entry, &instance);
        log::info!(target: "gg_engine", "Vulkan surface created");

        // Step 7: Pick physical device
        let (physical_device, physical_device_properties, graphics_queue_family_index) =
            pick_physical_device(&instance, &surface_loader, surface)?;

        let device_name =
            unsafe { CStr::from_ptr(physical_device_properties.device_name.as_ptr()) };
        let api_version = physical_device_properties.api_version;
        log::info!(
            target: "gg_engine",
            "Vulkan GPU: {} (API {}.{}.{})",
            device_name.to_string_lossy(),
            vk::api_version_major(api_version),
            vk::api_version_minor(api_version),
            vk::api_version_patch(api_version),
        );

        // Step 8: Create logical device with graphics queue
        let queue_priorities = [1.0_f32];
        let queue_create_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(graphics_queue_family_index)
            .queue_priorities(&queue_priorities);

        let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];

        let features = vk::PhysicalDeviceFeatures::default().sampler_anisotropy(true);

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_create_info))
            .enabled_extension_names(&device_extensions)
            .enabled_features(&features);

        let device = unsafe { instance.create_device(physical_device, &device_create_info, None) }
            .map_err(VulkanInitError::DeviceCreation)?;

        let graphics_queue = unsafe { device.get_device_queue(graphics_queue_family_index, 0) };

        log::info!(
            target: "gg_engine",
            "Vulkan logical device created (queue family: {graphics_queue_family_index})"
        );

        Ok(Self {
            device,
            graphics_queue,
            graphics_queue_family_index,
            surface_loader,
            surface,
            physical_device,
            physical_device_properties,
            #[cfg(debug_assertions)]
            debug_utils_loader,
            #[cfg(debug_assertions)]
            debug_messenger,
            instance,
            _entry: entry,
        })
    }

    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    pub fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    pub fn instance(&self) -> &ash::Instance {
        &self.instance
    }

    pub fn surface(&self) -> vk::SurfaceKHR {
        self.surface
    }

    pub fn surface_loader(&self) -> &surface::Instance {
        &self.surface_loader
    }

    pub fn graphics_queue(&self) -> vk::Queue {
        self.graphics_queue
    }

    pub fn graphics_queue_family_index(&self) -> u32 {
        self.graphics_queue_family_index
    }

    pub fn physical_device_properties(&self) -> &vk::PhysicalDeviceProperties {
        &self.physical_device_properties
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);

            #[cfg(debug_assertions)]
            self.debug_utils_loader
                .destroy_debug_utils_messenger(self.debug_messenger, None);

            self.instance.destroy_instance(None);
        }
        log::info!(target: "gg_engine", "Vulkan context destroyed");
    }
}

// ---------------------------------------------------------------------------
// Physical device selection
// ---------------------------------------------------------------------------

fn pick_physical_device(
    instance: &ash::Instance,
    surface_loader: &surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, vk::PhysicalDeviceProperties, u32), VulkanInitError> {
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(VulkanInitError::EnumerateDevices)?;

    if devices.is_empty() {
        return Err(VulkanInitError::NoSuitableGpu);
    }

    // First pass: prefer discrete GPU with graphics + present support.
    for &device in &devices {
        let properties = unsafe { instance.get_physical_device_properties(device) };

        if properties.device_type != vk::PhysicalDeviceType::DISCRETE_GPU {
            continue;
        }

        if let Some(index) = find_graphics_present_queue(instance, surface_loader, surface, device)
        {
            return Ok((device, properties, index));
        }
    }

    // Second pass: accept any device with graphics + present support.
    for &device in &devices {
        let properties = unsafe { instance.get_physical_device_properties(device) };

        if let Some(index) = find_graphics_present_queue(instance, surface_loader, surface, device)
        {
            return Ok((device, properties, index));
        }
    }

    Err(VulkanInitError::NoSuitableGpu)
}

fn find_graphics_present_queue(
    instance: &ash::Instance,
    surface_loader: &surface::Instance,
    surface: vk::SurfaceKHR,
    device: vk::PhysicalDevice,
) -> Option<u32> {
    let queue_families = unsafe { instance.get_physical_device_queue_family_properties(device) };

    for (index, family) in queue_families.iter().enumerate() {
        let supports_graphics = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);
        let supports_present = unsafe {
            surface_loader.get_physical_device_surface_support(device, index as u32, surface)
        }
        .unwrap_or(false);

        if supports_graphics && supports_present {
            return Some(index as u32);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Debug callback
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
    let message = unsafe { CStr::from_ptr((*p_callback_data).p_message) };
    let message_str = message.to_string_lossy();

    match message_severity {
        vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
            log::error!(target: "gg_engine", "[Vulkan] {message_str}");
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
            log::warn!(target: "gg_engine", "[Vulkan] {message_str}");
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::INFO => {
            log::info!(target: "gg_engine", "[Vulkan] {message_str}");
        }
        _ => {
            log::trace!(target: "gg_engine", "[Vulkan] {message_str}");
        }
    }

    vk::FALSE
}
