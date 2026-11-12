//! Per-swapchain-image linear FP16 color targets used before tone mapping.

use ash::vk;

use super::{Result, VulkanRendererError, gpu_resources::select_memory_type};

pub(super) struct HdrTarget {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    pub(super) view: vk::ImageView,
}

impl HdrTarget {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        extent: vk::Extent2D,
        format: vk::Format,
    ) -> Result<Self> {
        if extent.width == 0 || extent.height == 0 {
            return Err(VulkanRendererError::InvalidCaptureExtent);
        }
        let create_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: The selected FP16 format supports the image usages and the extent is nonzero.
        let image = unsafe { device.create_image(&create_info, None)? };
        let mut target = Self {
            device: device.clone(),
            image,
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
        };
        // SAFETY: The created image is live and owned by this target.
        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let (memory_type_index, _) = select_memory_type(
            memory_properties,
            &requirements,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            "device-local FP16 HDR target",
        )?;
        let allocation_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: The selected memory type is compatible and large enough for the image.
        target.memory = unsafe { device.allocate_memory(&allocation_info, None)? };
        // SAFETY: The image and allocation were created from the same device.
        unsafe { device.bind_image_memory(image, target.memory, 0)? };
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: The view covers the sole color mip and layer of the owned target image.
        target.view = unsafe { device.create_image_view(&view_info, None)? };
        Ok(target)
    }
}

impl Drop for HdrTarget {
    fn drop(&mut self) {
        // SAFETY: Framebuffers and descriptors are destroyed before targets; device work is idle.
        unsafe {
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
