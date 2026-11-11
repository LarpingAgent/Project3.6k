use ash::vk;

use super::{Result, VulkanRendererError};

pub(super) struct DepthAttachment {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    pub(super) view: vk::ImageView,
}

pub(super) fn choose_depth_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format> {
    for format in [
        vk::Format::D32_SFLOAT,
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D24_UNORM_S8_UINT,
        vk::Format::D16_UNORM,
    ] {
        // SAFETY: The physical device belongs to the live instance.
        let properties =
            unsafe { instance.get_physical_device_format_properties(physical_device, format) };
        if properties
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
        {
            return Ok(format);
        }
    }
    Err(VulkanRendererError::NoSupportedDepthFormat)
}

impl DepthAttachment {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        extent: vk::Extent2D,
        format: vk::Format,
    ) -> Result<Self> {
        let image_info = vk::ImageCreateInfo::default()
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
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: The extent is nonzero and the image uses an explicitly selected depth format.
        let image = unsafe { device.create_image(&image_info, None)? };
        let mut attachment = Self {
            device: device.clone(),
            image,
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
        };

        // SAFETY: The image is live and owned by this attachment.
        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let memory_type_index = select_device_local_memory_type(memory_properties, &requirements)
            .ok_or(VulkanRendererError::NoCompatibleMemory(
            "device-local depth image",
        ))?;
        let allocation_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: The selected memory type supports this image and has enough bytes.
        attachment.memory = unsafe { device.allocate_memory(&allocation_info, None)? };
        // SAFETY: Image and memory belong to this device and the allocation meets the requirements.
        unsafe { device.bind_image_memory(image, attachment.memory, 0)? };

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: The view describes the full depth subresource of the live image.
        attachment.view = unsafe { device.create_image_view(&view_info, None)? };
        Ok(attachment)
    }
}

fn select_device_local_memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    requirements: &vk::MemoryRequirements,
) -> Option<u32> {
    (0..properties.memory_type_count).find(|index| {
        requirements.memory_type_bits & (1 << index) != 0
            && properties.memory_types[*index as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    })
}

impl Drop for DepthAttachment {
    fn drop(&mut self) {
        // SAFETY: The swapchain waits for device idle and destroys framebuffers before dropping
        // their depth attachments. This guard then releases view, image, and bound memory in order.
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

#[cfg(test)]
mod tests {
    use super::select_device_local_memory_type;
    use ash::vk;

    #[test]
    fn depth_memory_selection_requires_device_local_compatible_type() {
        let memory_types = std::array::from_fn(|index| vk::MemoryType {
            property_flags: match index {
                0 => vk::MemoryPropertyFlags::HOST_VISIBLE,
                1 => vk::MemoryPropertyFlags::DEVICE_LOCAL,
                _ => vk::MemoryPropertyFlags::empty(),
            },
            heap_index: 0,
        });
        let properties = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: 2,
            memory_types,
            ..Default::default()
        };
        let requirements = vk::MemoryRequirements {
            memory_type_bits: 0b11,
            ..Default::default()
        };

        assert_eq!(
            select_device_local_memory_type(&properties, &requirements),
            Some(1)
        );
    }

    #[test]
    fn depth_memory_selection_rejects_incompatible_or_non_local_types() {
        let memory_types = std::array::from_fn(|index| vk::MemoryType {
            property_flags: if index == 0 {
                vk::MemoryPropertyFlags::HOST_VISIBLE
            } else {
                vk::MemoryPropertyFlags::empty()
            },
            heap_index: 0,
        });
        let properties = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: 1,
            memory_types,
            ..Default::default()
        };
        let requirements = vk::MemoryRequirements {
            memory_type_bits: 0b1,
            ..Default::default()
        };

        assert_eq!(
            select_device_local_memory_type(&properties, &requirements),
            None
        );
    }
}
