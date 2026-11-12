use std::{
    collections::HashMap,
    io::Cursor,
    mem::{offset_of, size_of},
    path::PathBuf,
    time::{Duration, Instant},
};

use ash::vk;
use glam::{Mat4, Quat, Vec3, Vec4};
use project3_6k_assets::{
    AlphaMode, AsyncModelLoader, CpuMaterialAsset, ModelImportLimits, TextureAsset, TextureBinding,
    TextureMipMode, import_environment_hdr, import_image,
};
use tracing::info;

use super::{
    FRAMES_IN_FLIGHT, Result, VulkanRendererError,
    forest_mesh::build_forest_geometry,
    gpu_asset_cache::{
        GpuAssetCache, GpuDrawRange, GpuMaterial, GpuModelAsset, GpuModelHandle, GpuVertex,
        upload_gpu_mesh,
    },
    gpu_resources::{BufferAllocation, TextureResource},
    shadow_target::{SHADOW_MAP_EXTENT, ShadowTarget},
};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/forest.vert.spv"));
const FRAGMENT_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/forest.frag.spv"));
const SHADOW_VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shadow.vert.spv"));
const SHADOW_FRAGMENT_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shadow.frag.spv"));
const MATERIAL_TEXTURE_COUNT: usize = 5;
const PUSH_CONSTANT_BYTES: u32 = 128;
const MODEL_IDS: [&str; 5] = [
    "tree_stump_01",
    "rock_09",
    "dead_tree_trunk_02",
    "shrub_03",
    "pine_sapling_small",
];
const SUN_DIRECTION: Vec3 = Vec3::new(-0.43, 0.82, 0.38);
const SUN_TARGET: Vec3 = Vec3::new(0.0, 0.5, -2.5);

#[repr(C, align(16))]
struct SceneUniform {
    view_projection: [[f32; 4]; 4],
    sun_view_projection: [[f32; 4]; 4],
    camera_position: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    ambient_color: [f32; 4],
    irradiance_sh: [[f32; 4]; 9],
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct MaterialPushConstants {
    model: [[f32; 4]; 4],
    base_color_factor: [f32; 4],
    material_params: [f32; 4],
    emissive_factor: [f32; 4],
    material_flags: [f32; 4],
}

struct PlacedModel {
    handle: GpuModelHandle,
    transform: Mat4,
    material_sets: Vec<vk::DescriptorSet>,
}

struct GroundResources {
    vertex_buffer: BufferAllocation,
    index_buffer: BufferAllocation,
    index_count: u32,
    textures: Vec<TextureResource>,
    material: GpuMaterial,
    gpu_bytes: usize,
}

struct EnvironmentResources {
    texture: TextureResource,
    gpu_bytes: usize,
    irradiance_sh: [[f32; 4]; 9],
}

struct FallbackTextures {
    base_color: TextureResource,
    normal: TextureResource,
    data: TextureResource,
    emissive: TextureResource,
}

pub(super) struct ForestSceneInit<'a> {
    pub(super) render_pass: vk::RenderPass,
    pub(super) shadow_render_pass: vk::RenderPass,
    pub(super) shadow_targets: &'a [ShadowTarget],
    pub(super) graphics_queue: vk::Queue,
    pub(super) queue_family_index: u32,
    pub(super) hdr_format: vk::Format,
    pub(super) srgb_format: vk::Format,
    pub(super) linear_format: vk::Format,
}

pub(super) struct ForestScene {
    device: ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
    material_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    scene_descriptor_sets: Vec<vk::DescriptorSet>,
    ground_material_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    shadow_pipeline: vk::Pipeline,
    camera_buffers: Vec<BufferAllocation>,
    ground: Option<GroundResources>,
    environment: Option<TextureResource>,
    irradiance_sh: [[f32; 4]; 9],
    fallbacks: Option<FallbackTextures>,
    gpu_assets: GpuAssetCache,
    models: Vec<PlacedModel>,
}

impl ForestScene {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        init: ForestSceneInit<'_>,
    ) -> Result<Self> {
        let ForestSceneInit {
            render_pass,
            shadow_render_pass,
            shadow_targets,
            graphics_queue,
            queue_family_index,
            hdr_format,
            srgb_format,
            linear_format,
        } = init;
        let mut gpu_assets = GpuAssetCache::new(
            device,
            *memory_properties,
            graphics_queue,
            queue_family_index,
            srgb_format,
            linear_format,
        );
        let model_handles = load_reference_models(&mut gpu_assets)?;
        let mut scene = Self {
            device: device.clone(),
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            material_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            scene_descriptor_sets: Vec::new(),
            ground_material_set: vk::DescriptorSet::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            shadow_pipeline: vk::Pipeline::null(),
            camera_buffers: Vec::new(),
            ground: None,
            environment: None,
            irradiance_sh: [[0.0; 4]; 9],
            fallbacks: None,
            gpu_assets,
            models: Vec::new(),
        };

        scene.ground = Some(load_ground(
            device,
            memory_properties,
            graphics_queue,
            queue_family_index,
            srgb_format,
            linear_format,
        )?);
        let environment = load_environment(
            device,
            memory_properties,
            graphics_queue,
            queue_family_index,
            hdr_format,
        )?;
        let environment_bytes = environment.gpu_bytes;
        scene.irradiance_sh = environment.irradiance_sh;
        scene.environment = Some(environment.texture);
        scene.fallbacks = Some(FallbackTextures::new(
            device,
            memory_properties,
            graphics_queue,
            queue_family_index,
            srgb_format,
            linear_format,
        )?);

        for _ in 0..FRAMES_IN_FLIGHT {
            scene.camera_buffers.push(BufferAllocation::new(
                device,
                memory_properties,
                size_of::<SceneUniform>(),
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                vk::MemoryPropertyFlags::HOST_VISIBLE,
                vk::MemoryPropertyFlags::HOST_COHERENT,
                "host-visible scene uniform",
            )?);
        }

        scene.create_layouts_and_pipeline(render_pass, shadow_render_pass)?;
        scene.create_model_placements(&model_handles)?;
        scene.create_descriptors(shadow_targets)?;

        let model_bytes = scene.gpu_assets.resident_gpu_bytes();
        let ground_bytes = scene.ground.as_ref().map_or(0, |ground| ground.gpu_bytes);
        info!(
            model_count = scene.gpu_assets.model_count(),
            placements = scene.models.len(),
            gpu_model_bytes = model_bytes,
            gpu_ground_bytes = ground_bytes,
            gpu_environment_bytes = environment_bytes,
            shadow_map_resolution = SHADOW_MAP_EXTENT,
            "PBR reference scene uploaded"
        );
        Ok(scene)
    }

    pub(super) fn update_camera(
        &self,
        frame_index: usize,
        view_projection: [[f32; 4]; 4],
        camera_position: [f32; 3],
    ) -> Result<()> {
        let sun_direction = SUN_DIRECTION.normalize();
        let uniform = SceneUniform {
            view_projection,
            sun_view_projection: sun_view_projection().to_cols_array_2d(),
            camera_position: [
                camera_position[0],
                camera_position[1],
                camera_position[2],
                1.0,
            ],
            sun_direction: [sun_direction.x, sun_direction.y, sun_direction.z, 1.0],
            sun_color: [3.2, 2.9, 2.55, 1.0],
            ambient_color: [0.18, 0.22, 0.17, 1.0],
            irradiance_sh: self.irradiance_sh,
        };
        self.camera_buffers[frame_index].upload(struct_bytes(&uniform))
    }

    pub(super) fn record_shadow_draw(
        &self,
        command_buffer: vk::CommandBuffer,
        extent: vk::Extent2D,
        frame_index: usize,
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
        let scene_set = [self.scene_descriptor_sets[frame_index]];
        // SAFETY: The depth-only render pass is active and per-frame resources are fence-safe.
        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.shadow_pipeline,
            );
            self.device.cmd_set_viewport(command_buffer, 0, &viewport);
            self.device.cmd_set_scissor(command_buffer, 0, &scissor);
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &scene_set,
                &[],
            );
        }
        if let Some(ground) = &self.ground {
            self.bind_draw_buffers(command_buffer, &ground.vertex_buffer, &ground.index_buffer);
            let material_set = [self.ground_material_set];
            // SAFETY: The material descriptor and 128-byte transform data match the shared layout.
            unsafe {
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    1,
                    &material_set,
                    &[],
                );
            }
            self.push_material(command_buffer, Mat4::IDENTITY, &ground.material.source);
            // SAFETY: The ground buffers contain a complete indexed plane and remain resident.
            unsafe {
                self.device
                    .cmd_draw_indexed(command_buffer, ground.index_count, 1, 0, 0, 0);
            }
        }
        for placed in &self.models {
            let Some(model) = self.gpu_assets.model(placed.handle) else {
                continue;
            };
            self.bind_draw_buffers(command_buffer, &model.vertex_buffer, &model.index_buffer);
            for instance in &model.instances {
                let model_to_world =
                    placed.transform * Mat4::from_cols_array_2d(&instance.transform);
                for draw in &model.draws {
                    let material = &model.materials[draw.material_index];
                    let descriptor_set = [placed.material_sets[draw.material_index]];
                    // SAFETY: The material set provides its base-color map for alpha masking.
                    unsafe {
                        self.device.cmd_bind_descriptor_sets(
                            command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            self.pipeline_layout,
                            1,
                            &descriptor_set,
                            &[],
                        );
                    }
                    self.push_material(command_buffer, model_to_world, &material.source);
                    // SAFETY: This draw range references validated indices in the live model buffers.
                    unsafe {
                        self.device.cmd_draw_indexed(
                            command_buffer,
                            draw.index_count,
                            1,
                            draw.first_index,
                            draw.vertex_offset,
                            0,
                        );
                    }
                }
            }
        }
    }

    pub(super) fn record_draw(
        &self,
        command_buffer: vk::CommandBuffer,
        extent: vk::Extent2D,
        frame_index: usize,
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
        let scene_set = [self.scene_descriptor_sets[frame_index]];
        // SAFETY: All pipelines, descriptors, buffers and the render pass are live for this draw.
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
                &scene_set,
                &[],
            );
        }

        if let Some(ground) = &self.ground {
            self.bind_draw_buffers(command_buffer, &ground.vertex_buffer, &ground.index_buffer);
            let material_set = [self.ground_material_set];
            // SAFETY: The ground descriptors, index range, pipeline layout, and material are live.
            unsafe {
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    1,
                    &material_set,
                    &[],
                );
                self.push_material(command_buffer, Mat4::IDENTITY, &ground.material.source);
                self.device
                    .cmd_draw_indexed(command_buffer, ground.index_count, 1, 0, 0, 0);
            }
        }

        for placed in &self.models {
            let Some(model) = self.gpu_assets.model(placed.handle) else {
                continue;
            };
            self.bind_draw_buffers(command_buffer, &model.vertex_buffer, &model.index_buffer);
            for instance in &model.instances {
                let local_to_model = Mat4::from_cols_array_2d(&instance.transform);
                let model_to_world = placed.transform * local_to_model;
                for draw in &model.draws {
                    self.draw_model_primitive(
                        command_buffer,
                        model,
                        draw,
                        model_to_world,
                        &placed.material_sets,
                    );
                }
            }
        }
    }

    fn create_layouts_and_pipeline(
        &mut self,
        render_pass: vk::RenderPass,
        shadow_render_pass: vk::RenderPass,
    ) -> Result<()> {
        let scene_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        // SAFETY: The scene set contains its per-frame uniform, shadow map, and environment map.
        self.descriptor_set_layout = unsafe {
            self.device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&scene_bindings),
                None,
            )?
        };
        let material_bindings: [vk::DescriptorSetLayoutBinding<'_>; MATERIAL_TEXTURE_COUNT] =
            std::array::from_fn(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding as u32)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            });
        // SAFETY: Five sampled maps are independently bound to the fragment shader.
        self.material_set_layout = unsafe {
            self.device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&material_bindings),
                None,
            )?
        };
        let layouts = [self.descriptor_set_layout, self.material_set_layout];
        let push_constants = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(PUSH_CONSTANT_BYTES)];
        // SAFETY: Both descriptor layouts outlive the pipeline layout; push data fits Vulkan's
        // guaranteed 128-byte minimum range.
        self.pipeline_layout = unsafe {
            self.device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&layouts)
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
        let vertex_bindings = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: size_of::<GpuVertex>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let vertex_attributes = [
            vertex_attribute(
                0,
                vk::Format::R32G32B32_SFLOAT,
                offset_of!(GpuVertex, position),
            ),
            vertex_attribute(
                1,
                vk::Format::R32G32B32_SFLOAT,
                offset_of!(GpuVertex, normal),
            ),
            vertex_attribute(
                2,
                vk::Format::R32G32B32A32_SFLOAT,
                offset_of!(GpuVertex, tangent),
            ),
            vertex_attribute(3, vk::Format::R32G32_SFLOAT, offset_of!(GpuVertex, uv)),
            vertex_attribute(
                4,
                vk::Format::R32G32B32A32_SFLOAT,
                offset_of!(GpuVertex, color),
            ),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes);
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
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);
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
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(self.pipeline_layout)
            .render_pass(render_pass)
            .subpass(0)];
        // SAFETY: Shader modules, descriptor layouts, render pass, and referenced state are live.
        self.pipeline = match unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_info, None)
        } {
            Ok(mut pipelines) => pipelines.remove(0),
            Err((pipelines, error)) => {
                // SAFETY: Any handles returned on partial pipeline failure belong to this device.
                unsafe {
                    for pipeline in pipelines {
                        self.device.destroy_pipeline(pipeline, None);
                    }
                }
                return Err(error.into());
            }
        };
        self.create_shadow_pipeline(shadow_render_pass)?;
        Ok(())
    }

    fn create_model_placements(&mut self, handles: &HashMap<String, GpuModelHandle>) -> Result<()> {
        let stump = *handles
            .get("tree_stump_01")
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        let rock = *handles
            .get("rock_09")
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        let trunk = *handles
            .get("dead_tree_trunk_02")
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        let shrub = *handles
            .get("shrub_03")
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        let pine = *handles
            .get("pine_sapling_small")
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        self.models = vec![
            placed_model(
                stump,
                Vec3::new(0.0, 0.19, -1.2),
                Quat::from_rotation_y(0.4),
                Vec3::splat(1.0),
            ),
            placed_model(
                rock,
                Vec3::new(-2.1, 0.01, -2.5),
                Quat::from_rotation_y(-0.55),
                Vec3::splat(10.0),
            ),
            placed_model(
                trunk,
                Vec3::new(-1.7, 0.92, -3.5),
                Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_y(-0.22),
                Vec3::splat(0.45),
            ),
            placed_model(
                shrub,
                Vec3::new(1.75, 0.0, -2.7),
                Quat::from_rotation_y(0.9),
                Vec3::splat(4.0),
            ),
            placed_model(
                shrub,
                Vec3::new(3.05, 0.0, -4.2),
                Quat::from_rotation_y(-0.5),
                Vec3::splat(3.4),
            ),
            placed_model(
                rock,
                Vec3::new(2.7, 0.008, -4.6),
                Quat::from_rotation_y(1.2),
                Vec3::splat(7.5),
            ),
            placed_model(
                pine,
                Vec3::new(0.6, 0.0, -8.0),
                Quat::from_rotation_y(0.35),
                Vec3::splat(3.0),
            ),
        ];
        Ok(())
    }

    fn create_shadow_pipeline(&mut self, render_pass: vk::RenderPass) -> Result<()> {
        let vertex_shader = ShaderModule::new(&self.device, SHADOW_VERTEX_SHADER)?;
        let fragment_shader = ShaderModule::new(&self.device, SHADOW_FRAGMENT_SHADER)?;
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
        let vertex_bindings = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: size_of::<GpuVertex>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let vertex_attributes = [
            vertex_attribute(
                0,
                vk::Format::R32G32B32_SFLOAT,
                offset_of!(GpuVertex, position),
            ),
            vertex_attribute(3, vk::Format::R32G32_SFLOAT, offset_of!(GpuVertex, uv)),
            vertex_attribute(
                4,
                vk::Format::R32G32B32A32_SFLOAT,
                offset_of!(GpuVertex, color),
            ),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_bias_enable(true)
            .depth_bias_constant_factor(1.25)
            .depth_bias_slope_factor(1.75)
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);
        let color_blending = vk::PipelineColorBlendStateCreateInfo::default();
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let pipeline_info = [vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport)
            .rasterization_state(&rasterization)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic)
            .layout(self.pipeline_layout)
            .render_pass(render_pass)
            .subpass(0)];
        // SAFETY: The shader interfaces match the shared layouts and the depth-only render pass.
        self.shadow_pipeline = match unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_info, None)
        } {
            Ok(mut pipelines) => pipelines.remove(0),
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

    fn create_descriptors(&mut self, shadow_targets: &[ShadowTarget]) -> Result<()> {
        if shadow_targets.len() != FRAMES_IN_FLIGHT {
            return Err(VulkanRendererError::InvalidShadowTargets);
        }
        let material_set_count = self.models.iter().try_fold(1usize, |sum, placed| {
            let model = self
                .gpu_assets
                .model(placed.handle)
                .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
            sum.checked_add(model.materials.len())
                .ok_or(VulkanRendererError::GpuAssetSizeOverflow)
        })?;
        let material_descriptor_count = material_set_count
            .checked_mul(MATERIAL_TEXTURE_COUNT)
            .and_then(|count| count.checked_add(shadow_targets.len()))
            .and_then(|count| count.checked_add(FRAMES_IN_FLIGHT))
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: FRAMES_IN_FLIGHT as u32,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: material_descriptor_count,
            },
        ];
        let set_count = FRAMES_IN_FLIGHT
            .checked_add(material_set_count)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
        // SAFETY: Pool sizes cover scene uniforms, shadow/environment maps, and five maps per material.
        self.descriptor_pool = unsafe {
            self.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(set_count)
                    .pool_sizes(&pool_sizes),
                None,
            )?
        };
        let scene_layouts = vec![self.descriptor_set_layout; FRAMES_IN_FLIGHT];
        let scene_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&scene_layouts);
        // SAFETY: The descriptor pool and each requested scene set layout are live.
        self.scene_descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&scene_info)? };
        for ((camera_buffer, descriptor_set), shadow_target) in self
            .camera_buffers
            .iter()
            .zip(&self.scene_descriptor_sets)
            .zip(shadow_targets)
        {
            let environment = self
                .environment
                .as_ref()
                .expect("HDR environment is loaded before scene descriptors");
            let buffer_info = [vk::DescriptorBufferInfo::default()
                .buffer(camera_buffer.buffer)
                .offset(0)
                .range(size_of::<SceneUniform>() as u64)];
            let image_info = [vk::DescriptorImageInfo::default()
                .sampler(shadow_target.sampler)
                .image_view(shadow_target.view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
            let environment_info = [vk::DescriptorImageInfo::default()
                .sampler(environment.sampler)
                .image_view(environment.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(*descriptor_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&buffer_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(*descriptor_set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&image_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(*descriptor_set)
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&environment_info),
            ];
            // SAFETY: The set references its frame uniform, fence-safe shadow, and owned environment.
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        }

        let material_layouts = vec![self.material_set_layout; material_set_count];
        let material_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&material_layouts);
        // SAFETY: The pool has capacity and every requested set uses the same live material layout.
        let material_sets = unsafe { self.device.allocate_descriptor_sets(&material_info)? };
        let fallbacks = self
            .fallbacks
            .as_ref()
            .expect("fallback textures are loaded before material descriptors");
        let ground = self
            .ground
            .as_ref()
            .expect("ground material is loaded before material descriptors");
        self.ground_material_set = material_sets[0];
        self.update_material_descriptor(
            self.ground_material_set,
            &ground.material,
            &ground.textures,
            fallbacks,
        );
        let mut next_set = 1;
        let mut sets_per_model = Vec::with_capacity(self.models.len());
        for placed in &self.models {
            let model = self
                .gpu_assets
                .model(placed.handle)
                .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
            let end = next_set + model.materials.len();
            sets_per_model.push(material_sets[next_set..end].to_vec());
            next_set = end;
        }
        for (placed, sets) in self.models.iter_mut().zip(sets_per_model) {
            placed.material_sets = sets;
        }
        for placed in &self.models {
            let model = self
                .gpu_assets
                .model(placed.handle)
                .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
            for (material, set) in model.materials.iter().zip(&placed.material_sets) {
                self.update_material_descriptor(*set, material, &model.textures, fallbacks);
            }
        }
        Ok(())
    }

    fn update_material_descriptor(
        &self,
        descriptor_set: vk::DescriptorSet,
        material: &GpuMaterial,
        textures: &[TextureResource],
        fallbacks: &FallbackTextures,
    ) {
        let image_infos = [
            texture_info(material.base_color_texture, textures, &fallbacks.base_color),
            texture_info(material.normal_texture, textures, &fallbacks.normal),
            texture_info(
                material.metallic_roughness_texture,
                textures,
                &fallbacks.data,
            ),
            texture_info(material.occlusion_texture, textures, &fallbacks.data),
            texture_info(material.emissive_texture, textures, &fallbacks.emissive),
        ];
        let writes = image_infos
            .iter()
            .enumerate()
            .map(|(binding, image_info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(descriptor_set)
                    .dst_binding(binding as u32)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(image_info))
            })
            .collect::<Vec<_>>();
        // SAFETY: Each set is updated before submission and references live shader-readable images.
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
    }

    fn bind_draw_buffers(
        &self,
        command_buffer: vk::CommandBuffer,
        vertex_buffer: &BufferAllocation,
        index_buffer: &BufferAllocation,
    ) {
        let vertex_buffers = [vertex_buffer.buffer];
        let offsets = [0];
        // SAFETY: The buffers are live device-local allocations with compatible usage flags.
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(command_buffer, 0, &vertex_buffers, &offsets);
            self.device.cmd_bind_index_buffer(
                command_buffer,
                index_buffer.buffer,
                0,
                vk::IndexType::UINT32,
            );
        }
    }

    fn draw_model_primitive(
        &self,
        command_buffer: vk::CommandBuffer,
        model: &GpuModelAsset,
        draw: &GpuDrawRange,
        model_to_world: Mat4,
        material_sets: &[vk::DescriptorSet],
    ) {
        let material = &model.materials[draw.material_index];
        let descriptor_set = [material_sets[draw.material_index]];
        // SAFETY: The descriptor set and push constants match this pipeline's second set/layout.
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                1,
                &descriptor_set,
                &[],
            );
            self.push_material(command_buffer, model_to_world, &material.source);
            self.device.cmd_draw_indexed(
                command_buffer,
                draw.index_count,
                1,
                draw.first_index,
                draw.vertex_offset,
                0,
            );
        }
    }

    fn push_material(
        &self,
        command_buffer: vk::CommandBuffer,
        model_to_world: Mat4,
        material: &CpuMaterialAsset,
    ) {
        let alpha_mode = match material.alpha_mode {
            AlphaMode::Opaque => 0.0,
            AlphaMode::Mask => 1.0,
            AlphaMode::Blend => 2.0,
        };
        let constants = MaterialPushConstants {
            model: model_to_world.to_cols_array_2d(),
            base_color_factor: material.base_color_factor,
            material_params: [
                material.metallic_factor,
                material.roughness_factor,
                material.normal_scale,
                material.occlusion_strength,
            ],
            emissive_factor: [
                material.emissive_factor[0],
                material.emissive_factor[1],
                material.emissive_factor[2],
                1.0,
            ],
            material_flags: [
                material.alpha_cutoff,
                alpha_mode,
                material.double_sided as u8 as f32,
                0.0,
            ],
        };
        // SAFETY: This fully initialized, 16-byte-aligned block is exactly the declared 128 bytes.
        // SAFETY: This command buffer is recording inside the live render pass and the declared
        // pipeline range is 128 bytes, matching the fully initialized push-constant block.
        unsafe {
            self.device.cmd_push_constants(
                command_buffer,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                struct_bytes(&constants),
            );
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
            if self.shadow_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.shadow_pipeline, None);
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device
                    .destroy_descriptor_pool(self.descriptor_pool, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
            }
            if self.material_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.material_set_layout, None);
            }
            if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            }
        }
    }
}

impl FallbackTextures {
    fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        queue_family_index: u32,
        srgb_format: vk::Format,
        linear_format: vk::Format,
    ) -> Result<Self> {
        Ok(Self {
            base_color: create_one_pixel_texture(
                device,
                memory_properties,
                queue,
                queue_family_index,
                srgb_format,
                [255, 255, 255, 255],
                TextureMipMode::SrgbColor,
            )?,
            normal: create_one_pixel_texture(
                device,
                memory_properties,
                queue,
                queue_family_index,
                linear_format,
                [128, 128, 255, 255],
                TextureMipMode::NormalMap,
            )?,
            data: create_one_pixel_texture(
                device,
                memory_properties,
                queue,
                queue_family_index,
                linear_format,
                [255, 255, 255, 255],
                TextureMipMode::LinearData,
            )?,
            emissive: create_one_pixel_texture(
                device,
                memory_properties,
                queue,
                queue_family_index,
                srgb_format,
                [0, 0, 0, 255],
                TextureMipMode::SrgbColor,
            )?,
        })
    }
}

fn load_reference_models(
    gpu_assets: &mut GpuAssetCache,
) -> Result<HashMap<String, GpuModelHandle>> {
    let asset_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/reference_scene/polyhaven");
    let mut loader = AsyncModelLoader::new(ModelImportLimits::default(), MODEL_IDS.len())?;
    let mut pending = HashMap::new();
    for id in MODEL_IDS {
        let path = asset_root.join(id).join(format!("{id}-1k.gltf"));
        let request = loader.request(&path)?;
        pending.insert(request, id.to_owned());
    }
    let cpu_started = Instant::now();
    let mut imported = HashMap::with_capacity(MODEL_IDS.len());
    while !pending.is_empty() {
        let event = loader
            .wait_next(Duration::from_secs(120))
            .ok_or(VulkanRendererError::AssetWorkerTimeout)?;
        let id = pending
            .remove(&event.id)
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        imported.insert(id, event.result?);
    }
    drop(loader);
    info!(
        imported_models = imported.len(),
        cpu_import_ms = cpu_started.elapsed().as_secs_f64() * 1000.0,
        "external forest models decoded on asset worker"
    );

    let gpu_started = Instant::now();
    let mut handles = HashMap::with_capacity(MODEL_IDS.len());
    for id in MODEL_IDS {
        let model = imported
            .remove(id)
            .ok_or(VulkanRendererError::InvalidGpuAssetMaterial)?;
        let handle = gpu_assets.insert(&format!("polyhaven/{id}"), model)?;
        handles.insert(id.to_owned(), handle);
    }
    info!(
        models = gpu_assets.model_count(),
        gpu_resident_bytes = gpu_assets.resident_gpu_bytes(),
        gpu_upload_ms = gpu_started.elapsed().as_secs_f64() * 1000.0,
        "external forest models cached on GPU"
    );
    Ok(handles)
}

fn load_ground(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    queue_family_index: u32,
    srgb_format: vk::Format,
    linear_format: vk::Format,
) -> Result<GroundResources> {
    let asset_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/reference_scene/polyhaven/forrest_ground_03");
    let limits = ModelImportLimits::default();
    let diffuse = import_image(asset_root.join("forrest_ground_03_diff_1k.jpg"), limits)?;
    let normal = import_image(asset_root.join("forrest_ground_03_nor_gl_1k.jpg"), limits)?;
    let arm = import_image(asset_root.join("forrest_ground_03_arm_1k.jpg"), limits)?;
    let texture_assets = [
        (
            srgb_format,
            TextureAsset::from_rgba8(
                diffuse.width,
                diffuse.height,
                &diffuse.rgba8,
                TextureMipMode::SrgbColor,
            )?,
        ),
        (
            linear_format,
            TextureAsset::from_rgba8(
                normal.width,
                normal.height,
                &normal.rgba8,
                TextureMipMode::NormalMap,
            )?,
        ),
        (
            linear_format,
            TextureAsset::from_rgba8(
                arm.width,
                arm.height,
                &arm.rgba8,
                TextureMipMode::LinearData,
            )?,
        ),
    ];
    let gpu_bytes = texture_assets
        .iter()
        .try_fold(0usize, |sum, (_, texture)| {
            sum.checked_add(texture.byte_size)
                .ok_or(VulkanRendererError::GpuAssetSizeOverflow)
        })?;
    let mut textures = Vec::with_capacity(texture_assets.len());
    for (format, texture) in &texture_assets {
        textures.push(TextureResource::new(
            device,
            memory_properties,
            queue,
            queue_family_index,
            *format,
            texture,
        )?);
    }
    let geometry = build_forest_geometry();
    let (vertex_buffer, index_buffer) = upload_gpu_mesh(
        device,
        memory_properties,
        queue,
        queue_family_index,
        &geometry.vertices,
        &geometry.indices,
    )?;
    let material = GpuMaterial {
        source: CpuMaterialAsset {
            name: Some("Poly Haven Forest Ground 03".to_owned()),
            base_color_factor: [1.0; 4],
            metallic_factor: 0.0,
            roughness_factor: 1.0,
            emissive_factor: [0.0; 3],
            base_color_texture: Some(TextureBinding {
                image_index: 0,
                tex_coord: 0,
            }),
            metallic_roughness_texture: Some(TextureBinding {
                image_index: 2,
                tex_coord: 0,
            }),
            normal_texture: Some(TextureBinding {
                image_index: 1,
                tex_coord: 0,
            }),
            occlusion_texture: Some(TextureBinding {
                image_index: 2,
                tex_coord: 0,
            }),
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
        },
        base_color_texture: Some(0),
        metallic_roughness_texture: Some(2),
        normal_texture: Some(1),
        occlusion_texture: Some(2),
        emissive_texture: None,
    };
    Ok(GroundResources {
        vertex_buffer,
        index_buffer,
        index_count: geometry.indices.len() as u32,
        textures,
        material,
        gpu_bytes,
    })
}

fn load_environment(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    queue_family_index: u32,
    hdr_format: vk::Format,
) -> Result<EnvironmentResources> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/reference_scene/polyhaven/forest_slope/forest_slope_1k.hdr");
    let started = Instant::now();
    let decoded = import_environment_hdr(&path, ModelImportLimits::default())?;
    let gpu_bytes = decoded
        .byte_size()
        .checked_div(2)
        .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
    let irradiance_sh = decoded.irradiance_sh;
    let texture = TextureResource::new_environment(
        device,
        memory_properties,
        queue,
        queue_family_index,
        hdr_format,
        &decoded,
    )?;
    info!(
        width = decoded.width,
        height = decoded.height,
        mip_levels = decoded.mip_levels.len(),
        cpu_bytes = decoded.byte_size(),
        gpu_bytes,
        import_upload_ms = started.elapsed().as_secs_f64() * 1000.0,
        "external HDR environment imported and uploaded"
    );
    Ok(EnvironmentResources {
        texture,
        gpu_bytes,
        irradiance_sh,
    })
}

fn create_one_pixel_texture(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    queue_family_index: u32,
    format: vk::Format,
    pixel: [u8; 4],
    mode: TextureMipMode,
) -> Result<TextureResource> {
    let texture = TextureAsset::from_rgba8(1, 1, &pixel, mode)?;
    TextureResource::new(
        device,
        memory_properties,
        queue,
        queue_family_index,
        format,
        &texture,
    )
}

fn placed_model(
    handle: GpuModelHandle,
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
) -> PlacedModel {
    PlacedModel {
        handle,
        transform: Mat4::from_scale_rotation_translation(scale, rotation, position),
        material_sets: Vec::new(),
    }
}

fn texture_info(
    index: Option<usize>,
    textures: &[TextureResource],
    fallback: &TextureResource,
) -> vk::DescriptorImageInfo {
    let texture = index
        .and_then(|index| textures.get(index))
        .unwrap_or(fallback);
    vk::DescriptorImageInfo::default()
        .sampler(texture.sampler)
        .image_view(texture.view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
}

fn sun_view_projection() -> Mat4 {
    let direction = SUN_DIRECTION.normalize();
    let eye = SUN_TARGET + direction * 25.0;
    let view = glam::camera::rh::view::look_at_mat4(eye, SUN_TARGET, Vec3::Y);
    let radius = 14.0;
    let near = 0.1;
    let far = 50.0;
    let projection = Mat4::from_cols(
        Vec4::new(1.0 / radius, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -1.0 / radius, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0 / (near - far), 0.0),
        Vec4::new(0.0, 0.0, near / (near - far), 1.0),
    );
    projection * view
}

fn vertex_attribute(
    location: u32,
    format: vk::Format,
    offset: usize,
) -> vk::VertexInputAttributeDescription {
    vk::VertexInputAttributeDescription {
        location,
        binding: 0,
        format,
        offset: offset as u32,
    }
}

fn struct_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: Callers use repr(C, align(16)) structures containing only initialized fixed arrays.
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
        let create_info = vk::ShaderModuleCreateInfo::default().code(&words);
        // SAFETY: `words` contains embedded SPIR-V and remains alive during module creation.
        let handle = unsafe { device.create_shader_module(&create_info, None)? };
        Ok(Self {
            device: device.clone(),
            handle,
        })
    }
}

impl Drop for ShaderModule {
    fn drop(&mut self) {
        // SAFETY: The shader module is owned here and pipelines are created before this local drops.
        unsafe { self.device.destroy_shader_module(self.handle, None) };
    }
}

#[cfg(test)]
mod tests {
    use super::{SUN_TARGET, sun_view_projection};
    use glam::Vec3;

    #[test]
    fn directional_shadow_projection_covers_the_reference_scene_in_vulkan_depth() {
        let projection = sun_view_projection();
        let target = projection.project_point3(SUN_TARGET);
        assert!(target.x.abs() < 0.0001);
        assert!(target.y.abs() < 0.0001);
        assert!((0.0..1.0).contains(&target.z));

        for point in [
            Vec3::new(-4.0, 0.0, -7.0),
            Vec3::new(4.0, 2.2, -7.5),
            Vec3::new(0.0, 0.0, 0.0),
        ] {
            let mapped = projection.project_point3(point);
            assert!((-1.0..=1.0).contains(&mapped.x), "{point:?} -> {mapped:?}");
            assert!((-1.0..=1.0).contains(&mapped.y), "{point:?} -> {mapped:?}");
            assert!((0.0..=1.0).contains(&mapped.z), "{point:?} -> {mapped:?}");
        }
    }
}
