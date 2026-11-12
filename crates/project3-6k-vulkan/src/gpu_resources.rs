//! RAII Vulkan allocations shared by the renderer and its GPU asset cache.

use std::ptr;

use ash::vk;
use project3_6k_assets::TextureAsset;

use super::{Result, VulkanRendererError};

pub(super) struct BufferAllocation {
    device: ash::Device,
    pub(super) buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: vk::DeviceSize,
    memory_flags: vk::MemoryPropertyFlags,
}

impl BufferAllocation {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        size: usize,
        usage: vk::BufferUsageFlags,
        required_memory: vk::MemoryPropertyFlags,
        preferred_memory: vk::MemoryPropertyFlags,
        memory_description: &'static str,
    ) -> Result<Self> {
        if size == 0 {
            return Err(VulkanRendererError::InvalidGpuAssetGeometry);
        }
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size as vk::DeviceSize)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: The buffer uses a valid nonzero size and an explicit GPU usage.
        let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let mut allocation = Self {
            device: device.clone(),
            buffer,
            memory: vk::DeviceMemory::null(),
            size: size as vk::DeviceSize,
            memory_flags: vk::MemoryPropertyFlags::empty(),
        };
        // SAFETY: The buffer is live and owned by this allocation.
        let requirements = unsafe { allocation.device.get_buffer_memory_requirements(buffer) };
        let (memory_type_index, memory_flags) = select_memory_type(
            memory_properties,
            &requirements,
            required_memory,
            preferred_memory,
            memory_description,
        )?;
        let allocation_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: The selected memory type is compatible with the buffer and meets its size requirement.
        allocation.memory = unsafe { allocation.device.allocate_memory(&allocation_info, None)? };
        allocation.memory_flags = memory_flags;
        // SAFETY: Buffer and memory belong to the same device and are not bound elsewhere.
        unsafe {
            allocation
                .device
                .bind_buffer_memory(allocation.buffer, allocation.memory, 0)?
        };
        Ok(allocation)
    }

    pub(super) fn upload(&self, data: &[u8]) -> Result<()> {
        if data.len() as vk::DeviceSize > self.size {
            return Err(VulkanRendererError::BufferUploadTooLarge {
                actual: data.len(),
                capacity: self.size,
            });
        }
        if !self
            .memory_flags
            .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
        {
            return Err(VulkanRendererError::NoCompatibleMemory(
                "host-visible upload",
            ));
        }
        // SAFETY: The allocation is host-visible and remains mapped only for this upload.
        let mapped = unsafe {
            self.device
                .map_memory(self.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())?
        };
        // SAFETY: The mapped allocation is large enough and source/destination regions do not overlap.
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), mapped.cast::<u8>(), data.len());
        }
        let flush_result = if !self
            .memory_flags
            .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
        {
            let range = [vk::MappedMemoryRange::default()
                .memory(self.memory)
                .offset(0)
                .size(vk::WHOLE_SIZE)];
            // SAFETY: The whole mapped allocation is flushed, so the range obeys atom alignment.
            unsafe { self.device.flush_mapped_memory_ranges(&range) }
        } else {
            Ok(())
        };
        // SAFETY: The mapping was opened above and is no longer needed after the upload.
        unsafe { self.device.unmap_memory(self.memory) };
        flush_result?;
        Ok(())
    }

    pub(super) fn download(&self, byte_count: usize) -> Result<Vec<u8>> {
        if byte_count as vk::DeviceSize > self.size {
            return Err(VulkanRendererError::BufferUploadTooLarge {
                actual: byte_count,
                capacity: self.size,
            });
        }
        if !self
            .memory_flags
            .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
        {
            return Err(VulkanRendererError::NoCompatibleMemory(
                "host-visible readback",
            ));
        }
        // SAFETY: The allocation is host-visible, and callers wait for its GPU writer first.
        let mapped = unsafe {
            self.device
                .map_memory(self.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())?
        };
        let invalidate_result = if !self
            .memory_flags
            .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
        {
            let range = [vk::MappedMemoryRange::default()
                .memory(self.memory)
                .offset(0)
                .size(vk::WHOLE_SIZE)];
            // SAFETY: The whole mapped allocation is invalidated, satisfying atom alignment.
            unsafe { self.device.invalidate_mapped_memory_ranges(&range) }
        } else {
            Ok(())
        };
        if let Err(error) = invalidate_result {
            // SAFETY: The map operation above succeeded and must be closed on this error path.
            unsafe { self.device.unmap_memory(self.memory) };
            return Err(error.into());
        }
        let mut bytes = vec![0; byte_count];
        // SAFETY: The mapped allocation covers the requested byte count and destination is sized.
        unsafe {
            ptr::copy_nonoverlapping(mapped.cast::<u8>(), bytes.as_mut_ptr(), byte_count);
            self.device.unmap_memory(self.memory);
        }
        Ok(bytes)
    }
}

impl Drop for BufferAllocation {
    fn drop(&mut self) {
        // SAFETY: Owners must wait for work referencing this buffer before dropping it.
        unsafe {
            if self.buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.buffer, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
}

pub(super) struct TextureResource {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    pub(super) view: vk::ImageView,
    pub(super) sampler: vk::Sampler,
    mip_levels: u32,
}

impl TextureResource {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        queue_family_index: u32,
        format: vk::Format,
        decoded: &TextureAsset,
    ) -> Result<Self> {
        if decoded.width == 0 || decoded.height == 0 || decoded.mip_levels.is_empty() {
            return Err(VulkanRendererError::InvalidGpuAssetTexture);
        }
        let mip_levels = u32::try_from(decoded.mip_levels.len())
            .map_err(|_| VulkanRendererError::GpuAssetSizeOverflow)?;
        let mut staging_data = Vec::new();
        let mut copy_regions = Vec::with_capacity(decoded.mip_levels.len());
        for (mip_level, level) in decoded.mip_levels.iter().enumerate() {
            let buffer_offset = staging_data.len() as u64;
            staging_data.extend_from_slice(&level.rgba8);
            copy_regions.push(
                vk::BufferImageCopy::default()
                    .buffer_offset(buffer_offset)
                    .buffer_row_length(0)
                    .buffer_image_height(0)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: mip_level as u32,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                    .image_extent(vk::Extent3D {
                        width: level.width,
                        height: level.height,
                        depth: 1,
                    }),
            );
        }
        let staging = BufferAllocation::new(
            device,
            memory_properties,
            staging_data.len(),
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT,
            "host-visible texture staging",
        )?;
        staging.upload(&staging_data)?;

        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: decoded.width,
                height: decoded.height,
                depth: 1,
            })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: Decoded dimensions are nonzero and the format was checked for sampling support.
        let image = unsafe { device.create_image(&image_info, None)? };
        let mut texture = Self {
            device: device.clone(),
            image,
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            sampler: vk::Sampler::null(),
            mip_levels,
        };
        // SAFETY: The image is live and owned by this texture.
        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let (memory_type_index, _) = select_memory_type(
            memory_properties,
            &requirements,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            "device-local sampled texture",
        )?;
        let allocation_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        // SAFETY: The selected memory type meets the image's size and type-bit requirements.
        texture.memory = unsafe { device.allocate_memory(&allocation_info, None)? };
        // SAFETY: Image and memory are from the same device and meet the allocation requirements.
        unsafe { device.bind_image_memory(image, texture.memory, 0)? };
        texture.upload_mips(queue, queue_family_index, &staging, &copy_regions)?;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: mip_levels,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: The view covers all color mip levels of the live texture image.
        texture.view = unsafe { device.create_image_view(&view_info, None)? };
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .min_lod(0.0)
            .max_lod(mip_levels.saturating_sub(1) as f32)
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK);
        // SAFETY: Linear filtering and normalized repeat addressing are supported by the formats.
        texture.sampler = unsafe { device.create_sampler(&sampler_info, None)? };
        Ok(texture)
    }

    fn upload_mips(
        &self,
        queue: vk::Queue,
        queue_family_index: u32,
        staging: &BufferAllocation,
        copy_regions: &[vk::BufferImageCopy],
    ) -> Result<()> {
        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::TRANSIENT)
            .queue_family_index(queue_family_index);
        // SAFETY: This queue family supports graphics and transfer commands.
        let command_pool = unsafe { self.device.create_command_pool(&pool_info, None)? };
        let result = (|| {
            let allocate_info = vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            // SAFETY: The transient pool owns the allocated primary command buffer.
            let command_buffer =
                unsafe { self.device.allocate_command_buffers(&allocate_info)?[0] };
            let begin_info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            // SAFETY: This command buffer is freshly allocated and not in flight.
            unsafe {
                self.device
                    .begin_command_buffer(command_buffer, &begin_info)?
            };
            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: self.mip_levels,
                base_array_layer: 0,
                layer_count: 1,
            };
            let to_transfer = [vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(self.image)
                .subresource_range(range)];
            // SAFETY: Each mip begins undefined and the image is exclusively owned for upload.
            unsafe {
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &to_transfer,
                );
                self.device.cmd_copy_buffer_to_image(
                    command_buffer,
                    staging.buffer,
                    self.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    copy_regions,
                );
            }
            let to_sample = [vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(self.image)
                .subresource_range(range)];
            // SAFETY: All image levels were copied and are now visible to fragment shader reads.
            unsafe {
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &to_sample,
                );
                self.device.end_command_buffer(command_buffer)?;
            }
            let command_buffers = [command_buffer];
            let submit = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
            // SAFETY: Staging and image remain alive until the queue completes this upload.
            unsafe {
                self.device
                    .queue_submit(queue, &submit, vk::Fence::null())?;
                self.device.queue_wait_idle(queue)?;
            }
            Ok(())
        })();
        if result.is_err() {
            // SAFETY: Ensure no temporary command can still access the texture during teardown.
            let _ = unsafe { self.device.device_wait_idle() };
        }
        // SAFETY: The queue is idle on success and was idled above on failure.
        unsafe { self.device.destroy_command_pool(command_pool, None) };
        result
    }
}

impl Drop for TextureResource {
    fn drop(&mut self) {
        // SAFETY: The owner waits for device idle before releasing scene/cache resources.
        unsafe {
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

pub(super) fn select_memory_type(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    requirements: &vk::MemoryRequirements,
    required: vk::MemoryPropertyFlags,
    preferred: vk::MemoryPropertyFlags,
    description: &'static str,
) -> Result<(u32, vk::MemoryPropertyFlags)> {
    let mut fallback = None;
    for index in 0..memory_properties.memory_type_count {
        if requirements.memory_type_bits & (1 << index) == 0 {
            continue;
        }
        let flags = memory_properties.memory_types[index as usize].property_flags;
        if !flags.contains(required) {
            continue;
        }
        fallback.get_or_insert((index, flags));
        if flags.contains(required | preferred) {
            return Ok((index, flags));
        }
    }
    fallback.ok_or(VulkanRendererError::NoCompatibleMemory(description))
}
