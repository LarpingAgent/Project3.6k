use std::{io::Cursor, mem::size_of, ptr};

use ash::vk;

use super::{Result, VulkanRendererError};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/forest.vert.spv"));
const FRAGMENT_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/forest.frag.spv"));

#[repr(C)]
#[derive(Clone, Copy)]
struct ForestVertex {
    position: [f32; 2],
    color: [f32; 3],
}

const VERTICES: [ForestVertex; 31] = [
    // Forest floor.
    ForestVertex {
        position: [-1.0, -0.56],
        color: [0.09, 0.22, 0.11],
    },
    ForestVertex {
        position: [1.0, -0.56],
        color: [0.09, 0.22, 0.11],
    },
    ForestVertex {
        position: [1.0, -1.0],
        color: [0.035, 0.12, 0.065],
    },
    ForestVertex {
        position: [-1.0, -1.0],
        color: [0.035, 0.12, 0.065],
    },
    // Distant ridge.
    ForestVertex {
        position: [-1.0, -0.38],
        color: [0.08, 0.19, 0.14],
    },
    ForestVertex {
        position: [-0.52, 0.3],
        color: [0.08, 0.19, 0.14],
    },
    ForestVertex {
        position: [-0.12, -0.38],
        color: [0.08, 0.19, 0.14],
    },
    ForestVertex {
        position: [-0.38, -0.38],
        color: [0.07, 0.17, 0.12],
    },
    ForestVertex {
        position: [0.12, 0.43],
        color: [0.07, 0.17, 0.12],
    },
    ForestVertex {
        position: [0.62, -0.38],
        color: [0.07, 0.17, 0.12],
    },
    // Left tree canopy and trunk.
    ForestVertex {
        position: [-0.84, -0.06],
        color: [0.12, 0.34, 0.16],
    },
    ForestVertex {
        position: [-0.48, -0.06],
        color: [0.12, 0.34, 0.16],
    },
    ForestVertex {
        position: [-0.66, 0.62],
        color: [0.2, 0.46, 0.2],
    },
    ForestVertex {
        position: [-0.71, -0.07],
        color: [0.3, 0.19, 0.1],
    },
    ForestVertex {
        position: [-0.61, -0.07],
        color: [0.3, 0.19, 0.1],
    },
    ForestVertex {
        position: [-0.61, -0.56],
        color: [0.2, 0.12, 0.07],
    },
    ForestVertex {
        position: [-0.71, -0.56],
        color: [0.2, 0.12, 0.07],
    },
    // Center tree.
    ForestVertex {
        position: [-0.22, -0.02],
        color: [0.09, 0.29, 0.13],
    },
    ForestVertex {
        position: [0.22, -0.02],
        color: [0.09, 0.29, 0.13],
    },
    ForestVertex {
        position: [0.0, 0.86],
        color: [0.17, 0.42, 0.18],
    },
    ForestVertex {
        position: [-0.055, -0.03],
        color: [0.32, 0.2, 0.1],
    },
    ForestVertex {
        position: [0.055, -0.03],
        color: [0.32, 0.2, 0.1],
    },
    ForestVertex {
        position: [0.055, -0.56],
        color: [0.21, 0.13, 0.07],
    },
    ForestVertex {
        position: [-0.055, -0.56],
        color: [0.21, 0.13, 0.07],
    },
    // Right tree canopy.
    ForestVertex {
        position: [0.45, -0.04],
        color: [0.11, 0.31, 0.14],
    },
    ForestVertex {
        position: [0.81, -0.04],
        color: [0.11, 0.31, 0.14],
    },
    ForestVertex {
        position: [0.63, 0.62],
        color: [0.19, 0.43, 0.18],
    },
    ForestVertex {
        position: [0.58, -0.05],
        color: [0.3, 0.19, 0.1],
    },
    ForestVertex {
        position: [0.68, -0.05],
        color: [0.3, 0.19, 0.1],
    },
    ForestVertex {
        position: [0.68, -0.56],
        color: [0.2, 0.12, 0.07],
    },
    ForestVertex {
        position: [0.58, -0.56],
        color: [0.2, 0.12, 0.07],
    },
];

const INDICES: [u16; 39] = [
    0, 1, 2, 0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 13, 15, 16, 17, 18, 19, 20, 21, 22,
    20, 22, 23, 24, 25, 26, 27, 28, 29, 27, 29, 30,
];

pub(super) struct ForestScene {
    device: ash::Device,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    vertex_buffer: Option<BufferAllocation>,
    index_buffer: Option<BufferAllocation>,
}

impl ForestScene {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        render_pass: vk::RenderPass,
        graphics_queue: vk::Queue,
        queue_family_index: u32,
    ) -> Result<Self> {
        let mut scene = Self {
            device: device.clone(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            vertex_buffer: None,
            index_buffer: None,
        };

        let vertex_shader = ShaderModule::new(device, VERTEX_SHADER)?;
        let fragment_shader = ShaderModule::new(device, FRAGMENT_SHADER)?;
        let layout_info = vk::PipelineLayoutCreateInfo::default();
        // SAFETY: The pipeline layout has no descriptors or push constants and owns no borrowed data.
        scene.pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };

        let entry_point = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_shader.handle)
                .name(entry_point),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_shader.handle)
                .name(entry_point),
        ];
        let bindings = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: size_of::<ForestVertex>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let attributes = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: (size_of::<[f32; 2]>()) as u32,
            },
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let color_blending =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let pipeline_info = [vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisampling)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(scene.pipeline_layout)
            .render_pass(render_pass)
            .subpass(0)];
        // SAFETY: Shader modules, pipeline layout, and render pass stay alive through creation;
        // all state structures reference local arrays that outlive the call.
        scene.pipeline = match unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_info, None)
        } {
            Ok(mut pipelines) => pipelines.remove(0),
            Err((pipelines, error)) => {
                // SAFETY: Any handles returned on a partial creation failure belong to this device.
                unsafe {
                    for pipeline in pipelines {
                        device.destroy_pipeline(pipeline, None);
                    }
                }
                return Err(error.into());
            }
        };

        let vertex_data = bytemuck_bytes(&VERTICES);
        let index_data = index_bytes(&INDICES);
        let vertex_size = vertex_data.len();
        let index_offset = align_up_4(vertex_size);
        let index_copy_size = align_up_4(index_data.len());
        let mut staging_data = vec![0; index_offset + index_copy_size];
        staging_data[..vertex_size].copy_from_slice(vertex_data);
        staging_data[index_offset..index_offset + index_data.len()].copy_from_slice(index_data);

        let staging = BufferAllocation::new(
            device,
            memory_properties,
            staging_data.len(),
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT,
            "host-visible staging",
        )?;
        staging.upload(&staging_data)?;

        let vertex_buffer = BufferAllocation::new(
            device,
            memory_properties,
            vertex_size,
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            "device-local vertex",
        )?;
        let index_buffer = BufferAllocation::new(
            device,
            memory_properties,
            index_copy_size,
            vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            "device-local index",
        )?;
        upload_static_mesh(
            device,
            graphics_queue,
            queue_family_index,
            &staging,
            &vertex_buffer,
            &index_buffer,
            MeshUploadLayout {
                vertex_size: vertex_size as u64,
                index_offset: index_offset as u64,
                index_copy_size: index_copy_size as u64,
            },
        )?;
        scene.vertex_buffer = Some(vertex_buffer);
        scene.index_buffer = Some(index_buffer);
        Ok(scene)
    }

    pub(super) fn record_draw(&self, command_buffer: vk::CommandBuffer, extent: vk::Extent2D) {
        let vertex_buffer = self
            .vertex_buffer
            .as_ref()
            .expect("scene vertex buffer initialized");
        let index_buffer = self
            .index_buffer
            .as_ref()
            .expect("scene index buffer initialized");
        let viewports = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissors = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        }];
        let vertex_buffers = [vertex_buffer.buffer];
        let offsets = [0];
        // SAFETY: The command buffer is recording inside the compatible render pass. The
        // pipeline, buffers, viewport, and index range are owned by this live scene.
        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            self.device.cmd_set_viewport(command_buffer, 0, &viewports);
            self.device.cmd_set_scissor(command_buffer, 0, &scissors);
            self.device
                .cmd_bind_vertex_buffers(command_buffer, 0, &vertex_buffers, &offsets);
            self.device.cmd_bind_index_buffer(
                command_buffer,
                index_buffer.buffer,
                0,
                vk::IndexType::UINT16,
            );
            self.device
                .cmd_draw_indexed(command_buffer, INDICES.len() as u32, 1, 0, 0, 0);
        }
    }
}

impl Drop for ForestScene {
    fn drop(&mut self) {
        // SAFETY: The owning swapchain waits for device idle before releasing this scene.
        unsafe {
            if self.pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.pipeline, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
            }
        }
    }
}

struct ShaderModule {
    device: ash::Device,
    handle: vk::ShaderModule,
}

impl ShaderModule {
    fn new(device: &ash::Device, bytes: &[u8]) -> Result<Self> {
        let words = ash::util::read_spv(&mut Cursor::new(bytes))
            .map_err(VulkanRendererError::InvalidShader)?;
        let create_info = vk::ShaderModuleCreateInfo::default().code(&words);
        // SAFETY: `words` contains the embedded SPIR-V and remains alive during module creation.
        let handle = unsafe { device.create_shader_module(&create_info, None)? };
        Ok(Self {
            device: device.clone(),
            handle,
        })
    }
}

impl Drop for ShaderModule {
    fn drop(&mut self) {
        // SAFETY: This guard owns the shader module and all pipelines have already been created.
        unsafe { self.device.destroy_shader_module(self.handle, None) };
    }
}

struct BufferAllocation {
    device: ash::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: vk::DeviceSize,
    memory_flags: vk::MemoryPropertyFlags,
}

impl BufferAllocation {
    fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        size: usize,
        usage: vk::BufferUsageFlags,
        required_memory: vk::MemoryPropertyFlags,
        preferred_memory: vk::MemoryPropertyFlags,
        memory_description: &'static str,
    ) -> Result<Self> {
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

    fn upload(&self, data: &[u8]) -> Result<()> {
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
        // SAFETY: The mapped allocation is at least `requirements.size`, `data` is valid for
        // `data.len()` bytes, and the source/destination regions do not overlap.
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
}

fn select_memory_type(
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

struct MeshUploadLayout {
    vertex_size: u64,
    index_offset: u64,
    index_copy_size: u64,
}

fn upload_static_mesh(
    device: &ash::Device,
    queue: vk::Queue,
    queue_family_index: u32,
    staging: &BufferAllocation,
    vertex_buffer: &BufferAllocation,
    index_buffer: &BufferAllocation,
    layout: MeshUploadLayout,
) -> Result<()> {
    let pool_info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::TRANSIENT)
        .queue_family_index(queue_family_index);
    // SAFETY: The queue family supports graphics and therefore transfer commands.
    let command_pool = unsafe { device.create_command_pool(&pool_info, None)? };
    let result = (|| {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: The transient pool is live and allocated for this queue family.
        let command_buffers = unsafe { device.allocate_command_buffers(&allocate_info)? };
        let command_buffer = command_buffers[0];
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: The command buffer is freshly allocated and not in flight.
        unsafe { device.begin_command_buffer(command_buffer, &begin_info)? };
        let copies = [
            vk::BufferCopy::default().size(layout.vertex_size),
            vk::BufferCopy::default()
                .src_offset(layout.index_offset)
                .size(layout.index_copy_size),
        ];
        // SAFETY: Source and destination buffers are live, the copied ranges fit, and both
        // copy sizes and offsets are four-byte aligned as required by vkCmdCopyBuffer.
        unsafe {
            device.cmd_copy_buffer(
                command_buffer,
                staging.buffer,
                vertex_buffer.buffer,
                &copies[..1],
            );
            device.cmd_copy_buffer(
                command_buffer,
                staging.buffer,
                index_buffer.buffer,
                &copies[1..],
            );
            let buffer_barriers = [
                vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::VERTEX_ATTRIBUTE_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(vertex_buffer.buffer)
                    .offset(0)
                    .size(layout.vertex_size),
                vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::INDEX_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(index_buffer.buffer)
                    .offset(0)
                    .size(layout.index_copy_size),
            ];
            // SAFETY: The buffer ranges were written by the preceding transfer operations and
            // the destination access masks match the later vertex-input reads on this queue.
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::VERTEX_INPUT,
                vk::DependencyFlags::empty(),
                &[],
                &buffer_barriers,
                &[],
            );
            device.end_command_buffer(command_buffer)?;
        }
        let command_buffers = [command_buffer];
        let submissions = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
        // SAFETY: This queue is idle during initialization/recreation and the command buffer
        // references only live buffers that remain owned until queue_wait_idle completes.
        unsafe {
            device.queue_submit(queue, &submissions, vk::Fence::null())?;
            device.queue_wait_idle(queue)?;
        }
        Ok(())
    })();
    if result.is_err() {
        // SAFETY: Best-effort idle before destroying the transient pool on an upload error.
        let _ = unsafe { device.device_wait_idle() };
    }
    // SAFETY: The queue has been idled on success; on error the device was idled above.
    unsafe { device.destroy_command_pool(command_pool, None) };
    result
}

fn align_up_4(size: usize) -> usize {
    (size + 3) & !3
}

impl Drop for BufferAllocation {
    fn drop(&mut self) {
        // SAFETY: The scene owner idles the device before destruction or drops this allocation before
        // any command using it has been submitted.
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

fn bytemuck_bytes(values: &[ForestVertex]) -> &[u8] {
    // SAFETY: `ForestVertex` is repr(C) and consists of five initialized f32 values, so it
    // has no padding bytes. The returned slice is bounded by the source allocation.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

fn index_bytes(values: &[u16]) -> &[u8] {
    // SAFETY: Every u16 is initialized and has no padding; the output covers the same allocation.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

#[cfg(test)]
mod tests {
    use super::{INDICES, VERTICES, VulkanRendererError, align_up_4, select_memory_type};
    use ash::vk;

    #[test]
    fn forest_mesh_indices_reference_complete_triangles() {
        assert_eq!(INDICES.len() % 3, 0);
        assert!(
            INDICES
                .iter()
                .all(|index| (*index as usize) < VERTICES.len())
        );
    }

    #[test]
    fn memory_selection_prefers_coherent_host_memory() {
        let memory_types = std::array::from_fn(|index| {
            let property_flags = match index {
                0 => vk::MemoryPropertyFlags::HOST_VISIBLE,
                1 => vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                _ => vk::MemoryPropertyFlags::empty(),
            };
            vk::MemoryType {
                property_flags,
                heap_index: 0,
            }
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

        let (index, flags) = select_memory_type(
            &properties,
            &requirements,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
            vk::MemoryPropertyFlags::HOST_COHERENT,
            "test",
        )
        .unwrap();
        assert_eq!(index, 1);
        assert!(flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT));
    }

    #[test]
    fn memory_selection_reports_missing_required_properties() {
        let properties = vk::PhysicalDeviceMemoryProperties::default();
        let requirements = vk::MemoryRequirements {
            memory_type_bits: 1,
            ..Default::default()
        };

        assert!(matches!(
            select_memory_type(
                &properties,
                &requirements,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::MemoryPropertyFlags::empty(),
                "device-local vertex",
            ),
            Err(VulkanRendererError::NoCompatibleMemory(
                "device-local vertex"
            ))
        ));
    }

    #[test]
    fn staging_offsets_and_copy_sizes_are_four_byte_aligned() {
        assert_eq!(align_up_4(0), 0);
        assert_eq!(align_up_4(1), 4);
        assert_eq!(align_up_4(620), 620);
        assert_eq!(align_up_4(78), 80);
    }
}
