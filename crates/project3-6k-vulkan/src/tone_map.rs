//! Fullscreen presentation pass that maps linear FP16 radiance to the swapchain format.

use std::{io::Cursor, mem::size_of};

use ash::vk;

use super::{Result, VulkanRendererError};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tonemap.vert.spv"));
const FRAGMENT_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tonemap.frag.spv"));

#[repr(C)]
#[derive(Clone, Copy)]
struct ToneMapParameters {
    exposure: f32,
    output_is_srgb: u32,
}

pub(super) struct ToneMapPass {
    device: ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    sampler: vk::Sampler,
    descriptor_sets: Vec<vk::DescriptorSet>,
    parameters: ToneMapParameters,
}

impl ToneMapPass {
    pub(super) fn new(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        hdr_views: &[vk::ImageView],
        swapchain_format: vk::Format,
    ) -> Result<Self> {
        if hdr_views.is_empty() {
            return Err(VulkanRendererError::InvalidToneMapTargets);
        }
        let mut pass = Self {
            device: device.clone(),
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            sampler: vk::Sampler::null(),
            descriptor_sets: Vec::new(),
            parameters: ToneMapParameters {
                exposure: 0.0,
                output_is_srgb: u32::from(is_srgb_format(swapchain_format)),
            },
        };
        pass.initialize(render_pass, hdr_views)?;
        Ok(pass)
    }

    fn initialize(
        &mut self,
        render_pass: vk::RenderPass,
        hdr_views: &[vk::ImageView],
    ) -> Result<()> {
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        // SAFETY: The post-process shader samples one HDR image per swapchain image.
        self.descriptor_set_layout = unsafe {
            self.device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?
        };
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: u32::try_from(hdr_views.len())
                .map_err(|_| VulkanRendererError::GpuAssetSizeOverflow)?,
        }];
        // SAFETY: Exactly one sampled-image descriptor is allocated for each HDR target.
        self.descriptor_pool = unsafe {
            self.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(pool_sizes[0].descriptor_count)
                    .pool_sizes(&pool_sizes),
                None,
            )?
        };
        let layouts = vec![self.descriptor_set_layout; hdr_views.len()];
        let allocation = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        // SAFETY: The descriptor pool and repeated layout are live and correctly sized.
        self.descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&allocation)? };
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .min_lod(0.0)
            .max_lod(0.0)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_BLACK);
        // SAFETY: Linear filtering and edge clamping are supported for the selected FP16 format.
        self.sampler = unsafe { self.device.create_sampler(&sampler_info, None)? };
        for (descriptor_set, image_view) in self.descriptor_sets.iter().zip(hdr_views) {
            let image_info = [vk::DescriptorImageInfo::default()
                .sampler(self.sampler)
                .image_view(*image_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(*descriptor_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(&image_info)];
            // SAFETY: Each descriptor is initialized before the first command buffer uses it.
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        }

        let push_constants = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(size_of::<ToneMapParameters>() as u32)];
        self.pipeline_layout = unsafe {
            self.device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&[self.descriptor_set_layout])
                    .push_constant_ranges(&push_constants),
                None,
            )?
        };
        let vertex_shader = ShaderModule::new(&self.device, VERTEX_SHADER)?;
        let fragment_shader = ShaderModule::new(&self.device, FRAGMENT_SHADER)?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_shader.handle)
                .name(c"main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_shader.handle)
                .name(c"main"),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false);
        let attachments = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blending = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let create_info = [vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport)
            .rasterization_state(&rasterization)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&blending)
            .dynamic_state(&dynamic)
            .layout(self.pipeline_layout)
            .render_pass(render_pass)
            .subpass(0)];
        // SAFETY: The compatible one-color render pass and all pipeline state are live.
        self.pipeline = match unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &create_info, None)
        } {
            Ok(pipelines) => pipelines[0],
            Err((pipelines, error)) => {
                // SAFETY: Partial pipelines returned on failure belong to this logical device.
                unsafe {
                    for pipeline in pipelines {
                        self.device.destroy_pipeline(pipeline, None);
                    }
                }
                return Err(error.into());
            }
        };
        Ok(())
    }

    pub(super) fn record_draw(
        &self,
        command_buffer: vk::CommandBuffer,
        extent: vk::Extent2D,
        image_index: usize,
    ) {
        let viewport = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissor = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        }];
        let descriptor_sets = [self.descriptor_sets[image_index]];
        // SAFETY: The pass render target is active and this image's descriptor references its
        // matching HDR target, which the preceding scene pass transitioned to shader-read-only.
        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            self.device.cmd_set_viewport(command_buffer, 0, &viewport);
            self.device.cmd_set_scissor(command_buffer, 0, &scissor);
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &descriptor_sets,
                &[],
            );
            self.device.cmd_push_constants(
                command_buffer,
                self.pipeline_layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                struct_bytes(&self.parameters),
            );
            self.device.cmd_draw(command_buffer, 3, 1, 0, 0);
        }
    }
}

impl Drop for ToneMapPass {
    fn drop(&mut self) {
        // SAFETY: Swapchain teardown waits for device idle and destroys framebuffers first.
        unsafe {
            if self.pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.pipeline, None);
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device
                    .destroy_descriptor_pool(self.descriptor_pool, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
            }
            if self.sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.sampler, None);
            }
            if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            }
        }
    }
}

fn is_srgb_format(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB
    )
}

fn struct_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: ToneMapParameters is repr(C) and all fields are initialized.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

struct ShaderModule {
    device: ash::Device,
    handle: vk::ShaderModule,
}

impl ShaderModule {
    fn new(device: &ash::Device, bytes: &[u8]) -> Result<Self> {
        let words = ash::util::read_spv(&mut Cursor::new(bytes))
            .map_err(VulkanRendererError::InvalidShader)?;
        // SAFETY: SPIR-V words remain alive through shader-module creation.
        let handle = unsafe {
            device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)?
        };
        Ok(Self {
            device: device.clone(),
            handle,
        })
    }
}

impl Drop for ShaderModule {
    fn drop(&mut self) {
        // SAFETY: The pipeline has been created before this temporary module is dropped.
        unsafe { self.device.destroy_shader_module(self.handle, None) };
    }
}
