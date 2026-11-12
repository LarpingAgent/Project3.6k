//! Per-frame depth targets sampled by the directional shadow shader.

use ash::vk;

use super::{Result, VulkanRendererError, gpu_resources::select_memory_type};

pub(super) const SHADOW_MAP_EXTENT: u32 = 1024;

pub(super) struct ShadowTarget {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    pub(super) view: vk::ImageView,
    pub(super) sampler: vk::Sampler,
    pub(super) framebuffer: vk::Framebuffer,
}

pub(super) fn choose_shadow_depth_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format> {
    for format in [
        vk::Format::D32_SFLOAT,
        vk::Format::D16_UNORM,
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D24_UNORM_S8_UINT,
    ] {
        // SAFETY: The physical device is enumerated by the live instance.
        let properties =
            unsafe { instance.get_physical_device_format_properties(physical_device, format) };
        let required = vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT
            | vk::FormatFeatureFlags::SAMPLED_IMAGE;
        if properties.optimal_tiling_features.contains(required) {
            return Ok(format);
        }
    }
    Err(VulkanRendererError::NoSupportedShadowFormat)
}

impl ShadowTarget {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        depth_format: vk::Format,
        render_pass: vk::RenderPass,
    ) -> Result<Self> {
        let extent = vk::Extent2D {
            width: SHADOW_MAP_EXTENT,
            height: SHADOW_MAP_EXTENT,
        };
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(depth_format)
            .extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: This extent and usage are valid for a depth attachment sampled by the fragment shader.
        let image = unsafe { device.create_image(&image_info, None)? };
        let mut target = Self {
            device: device.clone(),
            image,
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            sampler: vk::Sampler::null(),
            framebuffer: vk::Framebuffer::null(),
        };
        // SAFETY: The image is live and owned by this target.
        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let (memory_type_index, _) = select_memory_type(
            memory_properties,
            &requirements,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            "device-local directional shadow map",
        )?;
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: The selected memory type is compatible and large enough.
        target.memory = unsafe { device.allocate_memory(&allocation, None)? };
        // SAFETY: Image and allocation were created from the same device.
        unsafe { device.bind_image_memory(image, target.memory, 0)? };
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(depth_format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: The view exposes the complete depth subresource for attachment and sampling.
        target.view = unsafe { device.create_image_view(&view_info, None)? };
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .compare_enable(true)
            .compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_lod(0.0)
            .max_lod(0.0)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE);
        // SAFETY: Nearest depth comparison and an always-lit border support manual PCF taps.
        target.sampler = unsafe { device.create_sampler(&sampler_info, None)? };
        let attachments = [target.view];
        let framebuffer_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&attachments)
            .width(extent.width)
            .height(extent.height)
            .layers(1);
        // SAFETY: The view format and extent match the depth-only shadow render pass.
        target.framebuffer = unsafe { device.create_framebuffer(&framebuffer_info, None)? };
        Ok(target)
    }
}

impl Drop for ShadowTarget {
    fn drop(&mut self) {
        // SAFETY: Device is idle, and the framebuffer is released before its view and image.
        unsafe {
            if self.framebuffer != vk::Framebuffer::null() {
                self.device.destroy_framebuffer(self.framebuffer, None);
            }
            if self.sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.sampler, None);
            }
            if self.view != vk::ImageView::null() {
                self.device.destroy_image_view(self.view, None);
            }
            if self.image != vk::Image::null() {
                self.device.destroy_image(self.image, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
}
