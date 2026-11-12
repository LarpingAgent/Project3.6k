use std::{
    ffi::CStr,
    path::{Path, PathBuf},
};

use ash::{Entry, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use thiserror::Error;
use tracing::{info, warn};
use winit::window::Window;

#[path = "validation.rs"]
mod validation;

use validation::ValidationState;
pub use validation::{ValidationMonitor, ValidationReport};
#[path = "depth_buffer.rs"]
mod depth_buffer;
#[path = "forest_mesh.rs"]
mod forest_mesh;
#[path = "forest_scene.rs"]
mod forest_scene;
#[path = "gpu_asset_cache.rs"]
mod gpu_asset_cache;
#[path = "gpu_resources.rs"]
mod gpu_resources;
#[path = "hdr_target.rs"]
mod hdr_target;
#[path = "tone_map.rs"]
mod tone_map;

use depth_buffer::{DepthAttachment, choose_depth_format};
use forest_scene::ForestScene;
use hdr_target::HdrTarget;
use tone_map::ToneMapPass;

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";
const DEBUG_UTILS_EXTENSION: &CStr = c"VK_EXT_debug_utils";
const SWAPCHAIN_EXTENSION: &CStr = c"VK_KHR_swapchain";
pub(super) const FRAMES_IN_FLIGHT: usize = 2;
const MAX_CAPTURE_BYTES: usize = 256 * 1024 * 1024;

/// Errors returned by Vulkan initialization and frame submission.
#[derive(Debug, Error)]
pub enum VulkanRendererError {
    #[error("could not load the Vulkan loader: {0}")]
    Loader(#[from] ash::LoadingError),
    #[error("Vulkan call failed: {0}")]
    Vulkan(#[from] vk::Result),
    #[error("could not read a native window handle: {0}")]
    WindowHandle(#[from] raw_window_handle::HandleError),
    #[error("no Vulkan device supports graphics, presentation, and VK_KHR_swapchain")]
    NoSuitableDevice,
    #[error("the presentation surface has no supported image formats")]
    NoSurfaceFormats,
    #[error("the presentation surface has no supported present modes")]
    NoPresentModes,
    #[error("the presentation surface does not support color-attachment images")]
    UnsupportedSurfaceUsage,
    #[error("the selected Vulkan device has no supported depth attachment format")]
    NoSupportedDepthFormat,
    #[error("the selected Vulkan device has no filterable FP16 color-attachment format for HDR")]
    NoSupportedHdrFormat,
    #[error("the selected Vulkan device has no linearly-filterable sRGB texture format")]
    NoSupportedTextureFormat,
    #[error("the selected Vulkan device has no linearly-filterable RGBA8 data texture format")]
    NoSupportedLinearTextureFormat,
    #[error("could not load a renderer texture asset: {0}")]
    Asset(#[from] project3_6k_assets::AssetError),
    #[error("could not import an external glTF model: {0}")]
    ModelImport(#[from] project3_6k_assets::ModelImportError),
    #[error("could not build the scene: {0}")]
    Scene(#[from] project3_6k_core::SceneError),
    #[error("the renderer could not resolve a texture immediately after loading it")]
    MissingTextureAsset,
    #[error("static forest geometry exceeds the 16-bit indexed mesh limit")]
    MeshTooLarge,
    #[error("Vulkan validation was requested but {0} is unavailable")]
    ValidationUnavailable(&'static str),
    #[error("could not start the CPU asset worker: {0}")]
    AssetWorker(#[from] std::io::Error),
    #[error("could not queue a CPU asset import: {0}")]
    AssetQueue(#[from] project3_6k_assets::AsyncLoadError),
    #[error("CPU asset worker did not return a requested model before its timeout")]
    AssetWorkerTimeout,
    #[error("the embedded GLSL shader could not be decoded: {0}")]
    InvalidShader(#[source] std::io::Error),
    #[error("the selected Vulkan device has no compatible {0} memory type")]
    NoCompatibleMemory(&'static str),
    #[error("mesh upload contains {actual} bytes but the buffer holds only {capacity} bytes")]
    BufferUploadTooLarge {
        actual: usize,
        capacity: vk::DeviceSize,
    },
    #[error("the presentation surface does not support transfer-source swapchain images")]
    UnsupportedCaptureSurface,
    #[error("swapchain pixel format {0:?} is not supported by PNG frame capture")]
    UnsupportedCaptureFormat(vk::Format),
    #[error("frame capture requires {actual} bytes, above the {limit}-byte limit")]
    CaptureTooLarge { actual: usize, limit: usize },
    #[error("frame capture received an empty swapchain extent")]
    InvalidCaptureExtent,
    #[error("frame capture output path is empty")]
    EmptyCapturePath,
    #[error("a frame capture is already scheduled")]
    CaptureAlreadyScheduled,
    #[error("could not create screenshot {path}: {source}")]
    CaptureIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not encode screenshot PNG: {0}")]
    CapturePng(#[from] png::EncodingError),
    #[error("GPU asset key must contain between 1 and 256 UTF-8 bytes")]
    InvalidGpuAssetKey,
    #[error("GPU asset cache limit reached ({limit} models)")]
    GpuAssetCountExceeded { limit: usize },
    #[error("GPU asset memory budget exceeded: {resident} + {requested} > {budget} bytes")]
    GpuAssetBudgetExceeded {
        resident: usize,
        requested: usize,
        budget: usize,
    },
    #[error("GPU asset byte accounting overflowed")]
    GpuAssetSizeOverflow,
    #[error("GPU model contains invalid or empty triangle geometry")]
    InvalidGpuAssetGeometry,
    #[error("GPU model references a missing material")]
    InvalidGpuAssetMaterial,
    #[error("GPU model references a missing image")]
    InvalidGpuAssetTexture,
    #[error("GPU renderer currently requires TEXCOORD_0; texture coordinate set {0} was requested")]
    UnsupportedTextureCoordinates(u32),
    #[error("GPU renderer does not yet support alpha-blended glTF materials")]
    UnsupportedBlendMaterial,
    #[error("the tone-mapping pass received no HDR targets")]
    InvalidToneMapTargets,
}

type Result<T> = std::result::Result<T, VulkanRendererError>;

/// Owns the Vulkan instance, window surface, and selected logical device.
///
/// The platform layer must keep the `Window` alive until this value is dropped.
struct VulkanContext {
    _entry: Entry,
    instance: ash::Instance,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    debug_utils: Option<ash::ext::debug_utils::Instance>,
    debug_messenger: vk::DebugUtilsMessengerEXT,
    device: Option<ash::Device>,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_index: u32,
    physical_device: vk::PhysicalDevice,
    validation_enabled: bool,
    validation_state: std::sync::Arc<ValidationState>,
}

impl VulkanContext {
    fn new(window: &Window, force_validation: bool) -> Result<Self> {
        // SAFETY: Loading the system Vulkan loader is required before making Vulkan calls.
        // `Entry` owns the loaded function pointers and remains alive for this context.
        let entry = unsafe { Entry::load()? };
        let display_handle = window.display_handle()?;
        let window_handle = window.window_handle()?;
        // `ash-window` returns the platform-specific instance extensions required for the
        // supplied display handle. Both handles remain valid while `window` lives.
        let required_extensions =
            ash_window::enumerate_required_extensions(display_handle.as_raw())?;

        // SAFETY: These queries only inspect loader-owned metadata.
        let available_layers = unsafe { entry.enumerate_instance_layer_properties()? };
        // SAFETY: Same as above; the global extension query uses a null layer name.
        let available_extensions = unsafe { entry.enumerate_instance_extension_properties(None)? };
        let validation_layer_available = available_layers.iter().any(|layer| {
            // SAFETY: Vulkan guarantees that `layer_name` is a null-terminated fixed-size field.
            unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) == VALIDATION_LAYER }
        });
        let debug_utils_available = available_extensions.iter().any(|extension| {
            // SAFETY: Vulkan guarantees that `extension_name` is null-terminated.
            unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) == DEBUG_UTILS_EXTENSION }
        });
        let validation_enabled = cfg!(debug_assertions) || force_validation;
        if validation_enabled && !validation_layer_available {
            return Err(VulkanRendererError::ValidationUnavailable(
                "VK_LAYER_KHRONOS_validation",
            ));
        }
        if validation_enabled && !debug_utils_available {
            return Err(VulkanRendererError::ValidationUnavailable(
                "VK_EXT_debug_utils",
            ));
        }
        let validation_state = std::sync::Arc::new(ValidationState::default());

        let mut extension_names = required_extensions.to_vec();
        if validation_enabled {
            extension_names.push(DEBUG_UTILS_EXTENSION.as_ptr());
        }
        let layer_names = if validation_enabled {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };
        let application_info = vk::ApplicationInfo::default()
            .application_name(c"Project3.6k")
            .engine_name(c"Project3.6k Engine")
            // The foundation uses Vulkan 1.0 features and therefore has a broad fallback.
            .api_version(vk::API_VERSION_1_0);
        let mut debug_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(vulkan_debug_callback));
        if validation_enabled {
            // SAFETY: `validation_state` is stored in the context before the messenger is
            // created and remains alive until after the messenger is destroyed.
            debug_create_info = debug_create_info
                .user_data(std::sync::Arc::as_ptr(&validation_state) as *mut std::ffi::c_void);
        }
        let mut instance_info = vk::InstanceCreateInfo::default()
            .application_info(&application_info)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);
        if validation_enabled {
            instance_info = instance_info.push_next(&mut debug_create_info);
        }

        // SAFETY: Extension and layer name pointers above point to C strings that outlive
        // this call. The application-info structures also remain alive through creation.
        let instance = unsafe { entry.create_instance(&instance_info, None)? };
        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        // SAFETY: `window` owns the display and native window handles for the full lifetime
        // of the renderer. The context is explicitly dropped before the window by the app.
        let surface = match unsafe {
            ash_window::create_surface(
                &entry,
                &instance,
                display_handle.as_raw(),
                window_handle.as_raw(),
                None,
            )
        } {
            Ok(surface) => surface,
            Err(error) => {
                // SAFETY: No surface was created, and no device uses this instance yet.
                unsafe { instance.destroy_instance(None) };
                return Err(error.into());
            }
        };

        let mut context = Self {
            _entry: entry,
            instance,
            surface_loader,
            surface,
            debug_utils: None,
            debug_messenger: vk::DebugUtilsMessengerEXT::null(),
            device: None,
            graphics_queue: vk::Queue::null(),
            present_queue: vk::Queue::null(),
            queue_family_index: 0,
            physical_device: vk::PhysicalDevice::null(),
            validation_enabled,
            validation_state,
        };

        if validation_enabled {
            let debug_utils =
                ash::ext::debug_utils::Instance::new(&context._entry, &context.instance);
            // SAFETY: Debug-utils was enabled on this instance and the callback is valid for
            // the lifetime of the messenger. The callback does not retain the user pointer.
            context.debug_messenger =
                unsafe { debug_utils.create_debug_utils_messenger(&debug_create_info, None)? };
            context.debug_utils = Some(debug_utils);
        }

        let (physical_device, queue_family_index) = context.select_physical_device()?;
        let queue_priorities = [1.0_f32];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priorities)];
        let device_extensions = [SWAPCHAIN_EXTENSION.as_ptr()];
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&device_extensions);
        // SAFETY: The selected queue family supports both graphics and this surface, and
        // VK_KHR_swapchain was verified before the logical device is created.
        let device = unsafe {
            context
                .instance
                .create_device(physical_device, &device_info, None)?
        };
        // SAFETY: Queue index zero was requested in the queue-create info above.
        let graphics_queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        let present_queue = graphics_queue;
        // SAFETY: Device properties are read from a valid physical-device handle.
        let properties = unsafe {
            context
                .instance
                .get_physical_device_properties(physical_device)
        };
        // SAFETY: `device_name` is specified by Vulkan as a null-terminated fixed-size field.
        let device_name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) };
        info!(
            gpu = %device_name.to_string_lossy(),
            validation = validation_enabled,
            "Vulkan device initialized"
        );

        context.device = Some(device);
        context.graphics_queue = graphics_queue;
        context.present_queue = present_queue;
        context.queue_family_index = queue_family_index;
        context.physical_device = physical_device;
        Ok(context)
    }

    fn device(&self) -> &ash::Device {
        self.device
            .as_ref()
            .expect("device is initialized before use")
    }

    fn select_physical_device(&self) -> Result<(vk::PhysicalDevice, u32)> {
        // SAFETY: Instance is live and physical-device enumeration is read-only.
        let physical_devices = unsafe { self.instance.enumerate_physical_devices()? };
        let mut candidates = Vec::new();

        for physical_device in physical_devices {
            // SAFETY: The physical device came from this live instance.
            let extensions = unsafe {
                self.instance
                    .enumerate_device_extension_properties(physical_device)?
            };
            let supports_swapchain = extensions.iter().any(|extension| {
                // SAFETY: Vulkan guarantees a null-terminated extension name.
                unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) == SWAPCHAIN_EXTENSION }
            });
            if !supports_swapchain {
                continue;
            }

            // SAFETY: Queue-family properties are read from a valid physical device.
            let queue_families = unsafe {
                self.instance
                    .get_physical_device_queue_family_properties(physical_device)
            };
            for (index, queue_family) in queue_families.iter().enumerate() {
                if queue_family.queue_count == 0
                    || !queue_family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                {
                    continue;
                }
                // SAFETY: The family index comes from the queried properties and the surface
                // was created from a live native window for this instance.
                let presents = unsafe {
                    self.surface_loader.get_physical_device_surface_support(
                        physical_device,
                        index as u32,
                        self.surface,
                    )?
                };
                if presents {
                    // SAFETY: Querying device properties is valid for this enumerated handle.
                    let properties = unsafe {
                        self.instance
                            .get_physical_device_properties(physical_device)
                    };
                    let score = match properties.device_type {
                        vk::PhysicalDeviceType::DISCRETE_GPU => 3,
                        vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
                        _ => 1,
                    };
                    candidates.push((score, physical_device, index as u32));
                    break;
                }
            }
        }

        candidates
            .into_iter()
            .max_by_key(|(score, _, _)| *score)
            .map(|(_, physical_device, queue_family)| (physical_device, queue_family))
            .ok_or(VulkanRendererError::NoSuitableDevice)
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        // SAFETY: This context owns these handles. The renderer destroys swapchain resources
        // before this context is dropped, so no submitted work references the device here.
        unsafe {
            if let Some(device) = self.device.take() {
                let _ = device.device_wait_idle();
                device.destroy_device(None);
            }
            if self.debug_messenger != vk::DebugUtilsMessengerEXT::null() {
                if let Some(debug_utils) = &self.debug_utils {
                    debug_utils.destroy_debug_utils_messenger(self.debug_messenger, None);
                }
                self.debug_messenger = vk::DebugUtilsMessengerEXT::null();
            }
            if self.surface != vk::SurfaceKHR::null() {
                self.surface_loader.destroy_surface(self.surface, None);
                self.surface = vk::SurfaceKHR::null();
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// Owns swapchain images, the static forest scene, and frame submission resources.
struct SwapchainState {
    loader: ash::khr::swapchain::Device,
    device: ash::Device,
    swapchain: vk::SwapchainKHR,
    format: vk::Format,
    hdr_format: vk::Format,
    depth_format: vk::Format,
    extent: vk::Extent2D,
    readback_supported: bool,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    hdr_render_pass: vk::RenderPass,
    present_render_pass: vk::RenderPass,
    depth_images: Vec<DepthAttachment>,
    hdr_targets: Vec<HdrTarget>,
    forest_scene: Option<ForestScene>,
    tone_map: Option<ToneMapPass>,
    hdr_framebuffers: Vec<vk::Framebuffer>,
    present_framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    frames: Vec<FrameSync>,
    /// Per-swapchain-image semaphores are not reused until acquisition guarantees the prior
    /// presentation wait for that image has completed.
    render_finished: Vec<vk::Semaphore>,
    images_in_flight: Vec<vk::Fence>,
    current_frame: usize,
}

#[derive(Default)]
struct FrameSync {
    image_available: vk::Semaphore,
    in_flight: vk::Fence,
}

impl SwapchainState {
    fn new(context: &VulkanContext, window: &Window) -> Result<Self> {
        let device = context.device().clone();
        let mut state = Self {
            loader: ash::khr::swapchain::Device::new(&context.instance, &device),
            device,
            swapchain: vk::SwapchainKHR::null(),
            format: vk::Format::UNDEFINED,
            hdr_format: vk::Format::UNDEFINED,
            depth_format: vk::Format::UNDEFINED,
            extent: vk::Extent2D::default(),
            readback_supported: false,
            images: Vec::new(),
            image_views: Vec::new(),
            hdr_render_pass: vk::RenderPass::null(),
            present_render_pass: vk::RenderPass::null(),
            depth_images: Vec::new(),
            hdr_targets: Vec::new(),
            forest_scene: None,
            tone_map: None,
            hdr_framebuffers: Vec::new(),
            present_framebuffers: Vec::new(),
            command_pool: vk::CommandPool::null(),
            command_buffers: Vec::new(),
            frames: Vec::new(),
            render_finished: Vec::new(),
            images_in_flight: Vec::new(),
            current_frame: 0,
        };
        state.initialize(context, window)?;
        Ok(state)
    }

    fn initialize(&mut self, context: &VulkanContext, window: &Window) -> Result<()> {
        // SAFETY: The context owns a valid surface and physical-device handle.
        let capabilities = unsafe {
            context
                .surface_loader
                .get_physical_device_surface_capabilities(
                    context.physical_device,
                    context.surface,
                )?
        };
        // SAFETY: Same live physical device and surface.
        let formats = unsafe {
            context
                .surface_loader
                .get_physical_device_surface_formats(context.physical_device, context.surface)?
        };
        // SAFETY: Same live physical device and surface.
        let present_modes = unsafe {
            context
                .surface_loader
                .get_physical_device_surface_present_modes(
                    context.physical_device,
                    context.surface,
                )?
        };
        if !capabilities
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        {
            return Err(VulkanRendererError::UnsupportedSurfaceUsage);
        }
        let surface_format = choose_surface_format(&formats)?;
        let present_mode = choose_present_mode(&present_modes)?;
        self.format = surface_format.format;
        self.readback_supported = capabilities
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_SRC);
        self.depth_format = choose_depth_format(&context.instance, context.physical_device)?;
        self.extent = choose_extent(&capabilities, window);

        let mut image_count = capabilities.min_image_count.saturating_add(1);
        if capabilities.max_image_count != 0 {
            image_count = image_count.min(capabilities.max_image_count);
        }
        let mut image_usage = vk::ImageUsageFlags::COLOR_ATTACHMENT;
        if self.readback_supported {
            image_usage |= vk::ImageUsageFlags::TRANSFER_SRC;
        }
        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(context.surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(self.extent)
            .image_array_layers(1)
            .image_usage(image_usage)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(choose_composite_alpha(
                capabilities.supported_composite_alpha,
            ))
            .present_mode(present_mode)
            .clipped(true);
        // SAFETY: Every choice was queried from the surface capabilities, format, and modes.
        self.swapchain = unsafe { self.loader.create_swapchain(&create_info, None)? };
        // SAFETY: The new swapchain is live and owned by this state.
        self.images = unsafe { self.loader.get_swapchain_images(self.swapchain)? };

        self.hdr_format = choose_hdr_format(&context.instance, context.physical_device)?;
        self.hdr_render_pass =
            create_hdr_render_pass(&self.device, self.hdr_format, self.depth_format)?;
        self.present_render_pass = create_present_render_pass(&self.device, self.format)?;
        // SAFETY: The physical device belongs to the live Vulkan instance.
        let memory_properties = unsafe {
            context
                .instance
                .get_physical_device_memory_properties(context.physical_device)
        };
        let texture_format = choose_texture_format(&context.instance, context.physical_device)?;
        let linear_texture_format =
            choose_linear_texture_format(&context.instance, context.physical_device)?;
        for _ in &self.images {
            self.depth_images.push(DepthAttachment::new(
                &self.device,
                &memory_properties,
                self.extent,
                self.depth_format,
            )?);
            self.hdr_targets.push(HdrTarget::new(
                &self.device,
                &memory_properties,
                self.extent,
                self.hdr_format,
            )?);
        }
        self.forest_scene = Some(ForestScene::new(
            &self.device,
            &memory_properties,
            self.hdr_render_pass,
            context.graphics_queue,
            context.queue_family_index,
            texture_format,
            linear_texture_format,
        )?);
        for image in &self.images {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(*image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.format)
                .components(vk::ComponentMapping::default())
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            // SAFETY: The image belongs to this swapchain and the view uses its chosen format.
            self.image_views
                .push(unsafe { self.device.create_image_view(&view_info, None)? });
        }
        for (hdr_target, depth_image) in self.hdr_targets.iter().zip(&self.depth_images) {
            let attachments = [hdr_target.view, depth_image.view];
            let framebuffer_info = vk::FramebufferCreateInfo::default()
                .render_pass(self.hdr_render_pass)
                .attachments(&attachments)
                .width(self.extent.width)
                .height(self.extent.height)
                .layers(1);
            // SAFETY: The image view is compatible with the render pass and extent.
            self.hdr_framebuffers
                .push(unsafe { self.device.create_framebuffer(&framebuffer_info, None)? });
        }
        for image_view in &self.image_views {
            let attachments = [*image_view];
            let framebuffer_info = vk::FramebufferCreateInfo::default()
                .render_pass(self.present_render_pass)
                .attachments(&attachments)
                .width(self.extent.width)
                .height(self.extent.height)
                .layers(1);
            // SAFETY: The swapchain image view is compatible with the one-color presentation pass.
            self.present_framebuffers
                .push(unsafe { self.device.create_framebuffer(&framebuffer_info, None)? });
        }
        self.tone_map = Some(ToneMapPass::new(
            &self.device,
            self.present_render_pass,
            &self
                .hdr_targets
                .iter()
                .map(|target| target.view)
                .collect::<Vec<_>>(),
            self.format,
        )?);

        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(context.queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: The selected queue family supports graphics commands.
        self.command_pool = unsafe { self.device.create_command_pool(&pool_info, None)? };
        let command_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.images.len() as u32);
        // SAFETY: The command pool is live and the image count is nonzero for a valid surface.
        self.command_buffers = unsafe { self.device.allocate_command_buffers(&command_info)? };
        self.images_in_flight = vec![vk::Fence::null(); self.images.len()];

        for _ in &self.images {
            self.render_finished.push(vk::Semaphore::null());
            let semaphore = self
                .render_finished
                .last_mut()
                .expect("just inserted semaphore");
            // SAFETY: The logical device is live; default semaphore create info is valid.
            *semaphore = unsafe {
                self.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?
            };
        }

        for _ in 0..FRAMES_IN_FLIGHT {
            self.frames.push(FrameSync::default());
            let frame = self.frames.last_mut().expect("just inserted frame sync");
            // SAFETY: The logical device is live; default semaphore create info is valid.
            frame.image_available = unsafe {
                self.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?
            };
            let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
            // SAFETY: The logical device is live; this fence starts signaled for the first wait.
            frame.in_flight = unsafe { self.device.create_fence(&fence_info, None)? };
        }

        info!(
            width = self.extent.width,
            height = self.extent.height,
            images = self.images.len(),
            "Vulkan swapchain created"
        );
        Ok(())
    }

    fn draw(
        &mut self,
        context: &VulkanContext,
        view_projection: [[f32; 4]; 4],
        camera_position: [f32; 3],
        capture_path: Option<&Path>,
    ) -> Result<FrameDrawOutcome> {
        let frame = &self.frames[self.current_frame];
        let image_available = frame.image_available;
        let in_flight = frame.in_flight;
        // SAFETY: This fence belongs to the current frame and was signaled on its last use.
        unsafe { self.device.wait_for_fences(&[in_flight], true, u64::MAX)? };
        self.forest_scene
            .as_ref()
            .expect("forest scene is initialized with the render pass")
            .update_camera(self.current_frame, view_projection, camera_position)?;
        // SAFETY: The swapchain and semaphore are live. The timeout is intentionally unbounded;
        // acquisition will complete once a presentable image becomes available.
        let (image_index, was_suboptimal) = match unsafe {
            self.loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                image_available,
                vk::Fence::null(),
            )
        } {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                return Ok(FrameDrawOutcome {
                    submitted: false,
                    recreate_swapchain: true,
                    captured: false,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let image_index = image_index as usize;

        let previous_fence = self.images_in_flight[image_index];
        if previous_fence != vk::Fence::null() {
            // SAFETY: This fence tracks the prior submission for the acquired swapchain image.
            unsafe {
                self.device
                    .wait_for_fences(&[previous_fence], true, u64::MAX)?
            };
        }
        self.images_in_flight[image_index] = in_flight;
        let render_finished = self.render_finished[image_index];

        let capture_size = capture_path
            .map(|_| capture_byte_count(self.extent))
            .transpose()?;
        let readback_buffer = if let Some(size) = capture_size {
            // SAFETY: The physical device belongs to the live instance and its device is active.
            let memory_properties = unsafe {
                context
                    .instance
                    .get_physical_device_memory_properties(context.physical_device)
            };
            Some(gpu_resources::BufferAllocation::new(
                &self.device,
                &memory_properties,
                size,
                vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::HOST_VISIBLE,
                vk::MemoryPropertyFlags::HOST_COHERENT,
                "host-visible swapchain readback",
            )?)
        } else {
            None
        };

        let command_buffer = self.command_buffers[image_index];
        record_scene_pass(
            &self.device,
            command_buffer,
            ScenePassTarget {
                hdr_render_pass: self.hdr_render_pass,
                hdr_framebuffer: self.hdr_framebuffers[image_index],
                present_render_pass: self.present_render_pass,
                present_framebuffer: self.present_framebuffers[image_index],
                swapchain_image: self.images[image_index],
                readback_buffer: readback_buffer.as_ref().map(|buffer| buffer.buffer),
                frame_index: self.current_frame,
                image_index,
            },
            self.extent,
            self.forest_scene
                .as_ref()
                .expect("forest scene is initialized with the HDR render pass"),
            self.tone_map
                .as_ref()
                .expect("tone-mapping pass is initialized with the swapchain"),
        )?;

        // SAFETY: Reset only after the current frame's previous work has completed.
        unsafe { self.device.reset_fences(&[in_flight])? };
        let wait_semaphores = [image_available];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [command_buffer];
        let signal_semaphores = [render_finished];
        let submit = [vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores)];
        // SAFETY: Command buffer and semaphores are valid and owned by this state. The fence is
        // reset immediately before submission and will signal on completion.
        unsafe {
            self.device
                .queue_submit(context.graphics_queue, &submit, in_flight)?
        };

        let swapchains = [self.swapchain];
        let image_indices = [image_index as u32];
        let present_wait = [render_finished];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&present_wait)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        // SAFETY: The image was acquired from this swapchain; the render-finished semaphore
        // is signaled by the just-submitted queue operation.
        let present_result = unsafe {
            self.loader
                .queue_present(context.present_queue, &present_info)
        };
        if let (Some(path), Some(buffer), Some(size)) =
            (capture_path, readback_buffer.as_ref(), capture_size)
        {
            // SAFETY: This fence covers the submitted image-to-buffer copy, so the mapped
            // readback allocation is no longer being written by the GPU.
            unsafe { self.device.wait_for_fences(&[in_flight], true, u64::MAX)? };
            let pixels = buffer.download(size)?;
            write_capture_png(path, self.extent, self.format, pixels)?;
            info!(
                path = %path.display(),
                width = self.extent.width,
                height = self.extent.height,
                bytes = size,
                "captured Vulkan swapchain frame"
            );
        }
        let recreate_swapchain = match present_result {
            Ok(suboptimal) => suboptimal || was_suboptimal,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => true,
            Err(error) => return Err(error.into()),
        };
        self.current_frame = (self.current_frame + 1) % self.frames.len();
        Ok(FrameDrawOutcome {
            submitted: true,
            recreate_swapchain,
            captured: capture_path.is_some(),
        })
    }

    fn recreate(&mut self, context: &VulkanContext, window: &Window) -> Result<bool> {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(false);
        }
        // SAFETY: No swapchain resource is released until all queued work is idle.
        unsafe { self.device.device_wait_idle()? };
        self.destroy_resources();
        self.initialize(context, window)?;
        Ok(true)
    }

    fn destroy_resources(&mut self) {
        // SAFETY: This method is called while the device is alive, either during recreation
        // after a wait-idle or from Drop where waiting is repeated defensively.
        unsafe {
            let _ = self.device.device_wait_idle();
            for frame in self.frames.drain(..).rev() {
                if frame.in_flight != vk::Fence::null() {
                    self.device.destroy_fence(frame.in_flight, None);
                }
                if frame.image_available != vk::Semaphore::null() {
                    self.device.destroy_semaphore(frame.image_available, None);
                }
            }
            for semaphore in self.render_finished.drain(..).rev() {
                if semaphore != vk::Semaphore::null() {
                    self.device.destroy_semaphore(semaphore, None);
                }
            }
            if self.command_pool != vk::CommandPool::null() {
                self.device.destroy_command_pool(self.command_pool, None);
                self.command_pool = vk::CommandPool::null();
            }
            self.command_buffers.clear();
            for framebuffer in self.present_framebuffers.drain(..).rev() {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            for framebuffer in self.hdr_framebuffers.drain(..).rev() {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            self.tone_map.take();
            self.forest_scene.take();
            self.hdr_targets.clear();
            self.depth_images.clear();
            for image_view in self.image_views.drain(..).rev() {
                self.device.destroy_image_view(image_view, None);
            }
            if self.present_render_pass != vk::RenderPass::null() {
                self.device
                    .destroy_render_pass(self.present_render_pass, None);
                self.present_render_pass = vk::RenderPass::null();
            }
            if self.hdr_render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.hdr_render_pass, None);
                self.hdr_render_pass = vk::RenderPass::null();
            }
            if self.swapchain != vk::SwapchainKHR::null() {
                self.loader.destroy_swapchain(self.swapchain, None);
                self.swapchain = vk::SwapchainKHR::null();
            }
            self.images.clear();
            self.images_in_flight.clear();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameDrawOutcome {
    submitted: bool,
    recreate_swapchain: bool,
    captured: bool,
}

impl Drop for SwapchainState {
    fn drop(&mut self) {
        self.destroy_resources();
    }
}

/// Initializes Vulkan and renders a vertex-colored forest scene to a native window.
///
/// The renderer owns its instance, surface, device, swapchain, framebuffers, command buffers,
/// synchronization primitives, and presentation state. The platform must outlive it.
pub struct VulkanRenderer {
    // Field drop order is intentional: dependent swapchain objects must be destroyed before
    // the context tears down the logical device, surface, and instance.
    swapchain: SwapchainState,
    context: VulkanContext,
    resized: bool,
    suspended: bool,
    capture_path: Option<PathBuf>,
}

impl VulkanRenderer {
    /// Creates the Vulkan instance, chooses a graphics/present device, and creates a swapchain.
    pub fn new(window: &Window) -> Result<Self> {
        Self::new_with_validation(window, false)
    }

    /// Creates the renderer and can require validation layers in release builds.
    pub fn new_with_validation(window: &Window, force_validation: bool) -> Result<Self> {
        let context = VulkanContext::new(window, force_validation)?;
        let swapchain = SwapchainState::new(&context, window)?;
        Ok(Self {
            swapchain,
            context,
            resized: false,
            suspended: false,
            capture_path: None,
        })
    }

    /// Returns a monitor that remains readable after the renderer has been dropped.
    pub fn validation_monitor(&self) -> Option<ValidationMonitor> {
        self.context
            .validation_enabled
            .then(|| self.context.validation_state.monitor())
    }

    /// Captures the next successfully submitted swapchain frame to a PNG file.
    pub fn capture_next_frame(&mut self, path: impl Into<PathBuf>) -> Result<()> {
        if !self.swapchain.readback_supported {
            return Err(VulkanRendererError::UnsupportedCaptureSurface);
        }
        if self.capture_path.is_some() {
            return Err(VulkanRendererError::CaptureAlreadyScheduled);
        }
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(VulkanRendererError::EmptyCapturePath);
        }
        if !matches!(
            self.swapchain.format,
            vk::Format::R8G8B8A8_UNORM
                | vk::Format::R8G8B8A8_SRGB
                | vk::Format::B8G8R8A8_UNORM
                | vk::Format::B8G8R8A8_SRGB
        ) {
            return Err(VulkanRendererError::UnsupportedCaptureFormat(
                self.swapchain.format,
            ));
        }
        self.capture_path = Some(path);
        Ok(())
    }

    /// Marks the swapchain for recreation after a native window resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.suspended = width == 0 || height == 0;
        if !self.suspended {
            self.resized = true;
        }
    }

    /// Acquires, records the forest scene, submits, and presents one frame.
    pub fn draw(
        &mut self,
        window: &Window,
        view_projection: [[f32; 4]; 4],
        camera_position: [f32; 3],
    ) -> Result<bool> {
        if self.suspended {
            let size = window.inner_size();
            self.suspended = size.width == 0 || size.height == 0;
            if self.suspended {
                return Ok(false);
            }
            self.resized = true;
        }
        if self.resized {
            self.resized = !self.swapchain.recreate(&self.context, window)?;
            if self.resized {
                return Ok(false);
            }
        }
        let outcome = self.swapchain.draw(
            &self.context,
            view_projection,
            camera_position,
            self.capture_path.as_deref(),
        )?;
        if outcome.captured {
            self.capture_path = None;
        }
        if outcome.recreate_swapchain {
            self.resized = !self.swapchain.recreate(&self.context, window)?;
        }
        Ok(outcome.submitted)
    }
}

fn choose_texture_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format> {
    let format = vk::Format::R8G8B8A8_SRGB;
    // SAFETY: The physical device belongs to the live instance.
    let properties =
        unsafe { instance.get_physical_device_format_properties(physical_device, format) };
    let required =
        vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
    if properties.optimal_tiling_features.contains(required) {
        Ok(format)
    } else {
        Err(VulkanRendererError::NoSupportedTextureFormat)
    }
}

fn choose_hdr_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format> {
    let format = vk::Format::R16G16B16A16_SFLOAT;
    // SAFETY: The physical device belongs to the live Vulkan instance.
    let properties =
        unsafe { instance.get_physical_device_format_properties(physical_device, format) };
    let required = vk::FormatFeatureFlags::COLOR_ATTACHMENT
        | vk::FormatFeatureFlags::SAMPLED_IMAGE
        | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
    if properties.optimal_tiling_features.contains(required) {
        Ok(format)
    } else {
        Err(VulkanRendererError::NoSupportedHdrFormat)
    }
}

fn choose_linear_texture_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format> {
    let format = vk::Format::R8G8B8A8_UNORM;
    // SAFETY: The physical device belongs to the live instance.
    let properties =
        unsafe { instance.get_physical_device_format_properties(physical_device, format) };
    let required =
        vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
    if properties.optimal_tiling_features.contains(required) {
        Ok(format)
    } else {
        Err(VulkanRendererError::NoSupportedLinearTextureFormat)
    }
}

fn choose_surface_format(formats: &[vk::SurfaceFormatKHR]) -> Result<vk::SurfaceFormatKHR> {
    if formats.len() == 1 && formats[0].format == vk::Format::UNDEFINED {
        return Ok(vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_SRGB,
            color_space: formats[0].color_space,
        });
    }
    formats
        .iter()
        .copied()
        .find(|format| {
            format.format == vk::Format::B8G8R8A8_SRGB
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .or_else(|| formats.first().copied())
        .ok_or(VulkanRendererError::NoSurfaceFormats)
}

fn choose_present_mode(modes: &[vk::PresentModeKHR]) -> Result<vk::PresentModeKHR> {
    if modes.is_empty() {
        return Err(VulkanRendererError::NoPresentModes);
    }
    // FIFO is guaranteed by Vulkan and avoids an unbounded busy presentation loop.
    Ok(vk::PresentModeKHR::FIFO)
}

fn choose_extent(capabilities: &vk::SurfaceCapabilitiesKHR, window: &Window) -> vk::Extent2D {
    if capabilities.current_extent.width != u32::MAX {
        return capabilities.current_extent;
    }
    let size = window.inner_size();
    vk::Extent2D {
        width: size.width.clamp(
            capabilities.min_image_extent.width,
            capabilities.max_image_extent.width,
        ),
        height: size.height.clamp(
            capabilities.min_image_extent.height,
            capabilities.max_image_extent.height,
        ),
    }
}

fn choose_composite_alpha(supported: vk::CompositeAlphaFlagsKHR) -> vk::CompositeAlphaFlagsKHR {
    [
        vk::CompositeAlphaFlagsKHR::OPAQUE,
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::INHERIT,
    ]
    .into_iter()
    .find(|mode| supported.contains(*mode))
    .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE)
}

fn create_hdr_render_pass(
    device: &ash::Device,
    hdr_format: vk::Format,
    depth_format: vk::Format,
) -> Result<vk::RenderPass> {
    let attachments = [
        vk::AttachmentDescription::default()
            .format(hdr_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
        vk::AttachmentDescription::default()
            .format(depth_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
    ];
    let color_attachment = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let depth_attachment = vk::AttachmentReference {
        attachment: 1,
        layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
    };
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_attachment)
        .depth_stencil_attachment(&depth_attachment)];
    let dependencies = [
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            ),
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
            )
            .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ),
    ];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);
    // SAFETY: The attachment, subpass, and dependency descriptions are internally consistent.
    Ok(unsafe { device.create_render_pass(&info, None)? })
}

fn create_present_render_pass(device: &ash::Device, format: vk::Format) -> Result<vk::RenderPass> {
    let attachments = [vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
    let color = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color)];
    let dependencies = [
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::TRANSFER)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ),
    ];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);
    // SAFETY: The pass contains one sampled-tone-mapped swapchain color attachment.
    Ok(unsafe { device.create_render_pass(&info, None)? })
}

struct ScenePassTarget {
    hdr_render_pass: vk::RenderPass,
    hdr_framebuffer: vk::Framebuffer,
    present_render_pass: vk::RenderPass,
    present_framebuffer: vk::Framebuffer,
    swapchain_image: vk::Image,
    readback_buffer: Option<vk::Buffer>,
    frame_index: usize,
    image_index: usize,
}

fn record_scene_pass(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    target: ScenePassTarget,
    extent: vk::Extent2D,
    scene: &ForestScene,
    tone_map: &ToneMapPass,
) -> Result<()> {
    // SAFETY: This command buffer is owned by the graphics command pool and is not in flight.
    unsafe {
        device.reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())?;
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(command_buffer, &begin_info)?;
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.04, 0.075, 0.06, 1.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];
        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(target.hdr_render_pass)
            .framebuffer(target.hdr_framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(&clear_values);
        device.cmd_begin_render_pass(
            command_buffer,
            &render_pass_info,
            vk::SubpassContents::INLINE,
        );
        scene.record_draw(command_buffer, extent, target.frame_index);
        device.cmd_end_render_pass(command_buffer);
        let present_clear = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        }];
        let present_info = vk::RenderPassBeginInfo::default()
            .render_pass(target.present_render_pass)
            .framebuffer(target.present_framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(&present_clear);
        device.cmd_begin_render_pass(command_buffer, &present_info, vk::SubpassContents::INLINE);
        tone_map.record_draw(command_buffer, extent, target.image_index);
        device.cmd_end_render_pass(command_buffer);
        if let Some(readback_buffer) = target.readback_buffer {
            let to_transfer = [vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(target.swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })];
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_transfer,
            );
            device.cmd_copy_image_to_buffer(
                command_buffer,
                target.swapchain_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback_buffer,
                &[vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    })],
            );
            let to_present = [vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                .dst_access_mask(vk::AccessFlags::empty())
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(target.swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })];
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_present,
            );
        }
        device.end_command_buffer(command_buffer)?;
    }
    Ok(())
}

fn capture_byte_count(extent: vk::Extent2D) -> Result<usize> {
    if extent.width == 0 || extent.height == 0 {
        return Err(VulkanRendererError::InvalidCaptureExtent);
    }
    let byte_count = u64::from(extent.width)
        .checked_mul(u64::from(extent.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(VulkanRendererError::CaptureTooLarge {
            actual: usize::MAX,
            limit: MAX_CAPTURE_BYTES,
        })?;
    if byte_count > MAX_CAPTURE_BYTES {
        return Err(VulkanRendererError::CaptureTooLarge {
            actual: byte_count,
            limit: MAX_CAPTURE_BYTES,
        });
    }
    Ok(byte_count)
}

fn write_capture_png(
    path: &Path,
    extent: vk::Extent2D,
    format: vk::Format,
    mut pixels: Vec<u8>,
) -> Result<()> {
    if matches!(
        format,
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
    ) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    } else if !matches!(
        format,
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB
    ) {
        return Err(VulkanRendererError::UnsupportedCaptureFormat(format));
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| VulkanRendererError::CaptureIo {
            path: path.to_owned(),
            source,
        })?;
    }
    let file = std::fs::File::create(path).map_err(|source| VulkanRendererError::CaptureIo {
        path: path.to_owned(),
        source,
    })?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), extent.width, extent.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(&pixels)
        .map_err(VulkanRendererError::CapturePng)
}

unsafe extern "system" fn vulkan_debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
    let message = if callback_data.is_null() {
        "<empty Vulkan validation message>".into()
    } else {
        // SAFETY: Vulkan provides callback data for the duration of the callback invocation.
        let message_pointer = unsafe { (*callback_data).p_message };
        if message_pointer.is_null() {
            "<empty Vulkan validation message>".into()
        } else {
            // SAFETY: Vulkan specifies a null-terminated message string for this callback.
            unsafe { CStr::from_ptr(message_pointer) }
                .to_string_lossy()
                .into_owned()
        }
    };
    if !user_data.is_null() {
        // SAFETY: The callback's user pointer references the `ValidationState` Arc allocation
        // retained by `VulkanContext` until after its debug messenger has been destroyed.
        let state = unsafe { &*user_data.cast::<ValidationState>() };
        if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
            state.record_warning(message.clone());
        }
        if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
            state.record_error(message.clone());
        }
    }
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        tracing::error!(target: "vulkan::validation", "{message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        warn!(target: "vulkan::validation", "{message}");
    }
    vk::FALSE
}

#[cfg(test)]
mod tests {
    use super::{MAX_CAPTURE_BYTES, VulkanRendererError, capture_byte_count};
    use ash::vk;

    #[test]
    fn swapchain_capture_size_is_checked_before_allocation() {
        assert_eq!(
            capture_byte_count(vk::Extent2D {
                width: 1280,
                height: 720,
            })
            .unwrap(),
            1280 * 720 * 4
        );
        assert!(matches!(
            capture_byte_count(vk::Extent2D {
                width: 0,
                height: 720,
            }),
            Err(VulkanRendererError::InvalidCaptureExtent)
        ));
        assert!(matches!(
            capture_byte_count(vk::Extent2D {
                width: 16384,
                height: 16384,
            }),
            Err(VulkanRendererError::CaptureTooLarge {
                limit: MAX_CAPTURE_BYTES,
                ..
            })
        ));
    }
}
