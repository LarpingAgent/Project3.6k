//! Bounded Vulkan model cache. CPU decoding stays in `project3-6k-assets`; this module owns
//! device-local mesh buffers and role-correct sampled images.

use std::{
    collections::HashMap,
    mem::{size_of, size_of_val},
};

use ash::vk;
use project3_6k_assets::{
    AlphaMode, CpuMaterialAsset, CpuModelAsset, CpuModelInstance, CpuVertex, TextureAsset,
    TextureBinding, TextureMipMode,
};

use super::{
    Result, VulkanRendererError,
    gpu_resources::{BufferAllocation, TextureResource},
};

const MAX_KEY_BYTES: usize = 256;
const MAX_MODELS: usize = 32;
const MAX_GPU_BYTES: usize = 256 * 1024 * 1024;

/// Vertex layout shared by the generated test ground and imported glTF primitives.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GpuVertex {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
    pub(super) tangent: [f32; 4],
    pub(super) uv: [f32; 2],
    pub(super) color: [f32; 4],
}

impl From<CpuVertex> for GpuVertex {
    fn from(vertex: CpuVertex) -> Self {
        Self {
            position: vertex.position,
            normal: vertex.normal,
            tangent: vertex.tangent,
            uv: vertex.uv0,
            color: vertex.color,
        }
    }
}

/// Generational identity for a resident Vulkan model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct GpuModelHandle {
    slot: usize,
    generation: u64,
}

/// Material texture references and the source glTF scalar data used by the renderer.
#[derive(Debug)]
pub(super) struct GpuMaterial {
    pub(super) source: CpuMaterialAsset,
    pub(super) base_color_texture: Option<usize>,
    pub(super) metallic_roughness_texture: Option<usize>,
    pub(super) normal_texture: Option<usize>,
    pub(super) occlusion_texture: Option<usize>,
    pub(super) emissive_texture: Option<usize>,
}

/// One uploaded triangle range in a model's shared vertex and index buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GpuDrawRange {
    pub(super) first_index: u32,
    pub(super) index_count: u32,
    pub(super) vertex_offset: i32,
    pub(super) material_index: usize,
}

/// All GPU resources for a glTF model, including independent image role variants.
pub(super) struct GpuModelAsset {
    pub(super) vertex_buffer: BufferAllocation,
    pub(super) index_buffer: BufferAllocation,
    pub(super) draws: Vec<GpuDrawRange>,
    pub(super) instances: Vec<CpuModelInstance>,
    pub(super) materials: Vec<GpuMaterial>,
    pub(super) textures: Vec<TextureResource>,
    gpu_bytes: usize,
}

#[derive(Default)]
struct ModelSlot {
    generation: u64,
    model: Option<GpuModelAsset>,
    key: Option<String>,
}

/// Owns bounded model GPU resources and releases them after the owning renderer becomes idle.
pub(super) struct GpuAssetCache {
    device: ash::Device,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    queue_family_index: u32,
    srgb_format: vk::Format,
    linear_format: vk::Format,
    max_models: usize,
    max_gpu_bytes: usize,
    slots: Vec<ModelSlot>,
    free_slots: Vec<usize>,
    by_key: HashMap<String, GpuModelHandle>,
    resident_gpu_bytes: usize,
}

impl GpuAssetCache {
    pub(super) fn new(
        device: &ash::Device,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        queue_family_index: u32,
        srgb_format: vk::Format,
        linear_format: vk::Format,
    ) -> Self {
        Self {
            device: device.clone(),
            memory_properties,
            queue,
            queue_family_index,
            srgb_format,
            linear_format,
            max_models: MAX_MODELS,
            max_gpu_bytes: MAX_GPU_BYTES,
            slots: Vec::new(),
            free_slots: Vec::new(),
            by_key: HashMap::new(),
            resident_gpu_bytes: 0,
        }
    }

    pub(super) fn insert(&mut self, key: &str, model: CpuModelAsset) -> Result<GpuModelHandle> {
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(VulkanRendererError::InvalidGpuAssetKey);
        }
        if let Some(handle) = self.by_key.get(key).copied()
            && self.model(handle).is_some()
        {
            return Ok(handle);
        }
        if self.free_slots.is_empty() && self.slots.len() >= self.max_models {
            return Err(VulkanRendererError::GpuAssetCountExceeded {
                limit: self.max_models,
            });
        }

        let CpuModelAsset {
            meshes,
            instances,
            materials: source_materials,
            images,
            ..
        } = model;
        let source_materials = if source_materials.is_empty() {
            vec![default_material()]
        } else {
            source_materials
        };

        let mut texture_indices = HashMap::<(usize, TextureMipMode), usize>::new();
        let mut prepared_textures = Vec::<(TextureMipMode, TextureAsset)>::new();
        for material in &source_materials {
            for (binding, mode) in material_texture_roles(material) {
                if let Some(binding) = binding {
                    if binding.tex_coord != 0 {
                        return Err(VulkanRendererError::UnsupportedTextureCoordinates(
                            binding.tex_coord,
                        ));
                    }
                    if binding.image_index >= images.len() {
                        return Err(VulkanRendererError::InvalidGpuAssetTexture);
                    }
                    let key = (binding.image_index, mode);
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        texture_indices.entry(key)
                    {
                        let image = &images[binding.image_index];
                        let texture = TextureAsset::from_rgba8(
                            image.width,
                            image.height,
                            &image.rgba8,
                            mode,
                        )?;
                        let index = prepared_textures.len();
                        prepared_textures.push((mode, texture));
                        entry.insert(index);
                    }
                }
            }
        }

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut draws = Vec::new();
        for mesh in meshes {
            for primitive in mesh.primitives {
                if primitive.vertices.is_empty() || primitive.indices.is_empty() {
                    return Err(VulkanRendererError::InvalidGpuAssetGeometry);
                }
                let vertex_offset =
                    i32::try_from(vertices.len()).map_err(|_| VulkanRendererError::MeshTooLarge)?;
                let first_index =
                    u32::try_from(indices.len()).map_err(|_| VulkanRendererError::MeshTooLarge)?;
                for &index in &primitive.indices {
                    if index as usize >= primitive.vertices.len() {
                        return Err(VulkanRendererError::InvalidGpuAssetGeometry);
                    }
                }
                vertices.extend(primitive.vertices.into_iter().map(GpuVertex::from));
                let index_count = u32::try_from(primitive.indices.len())
                    .map_err(|_| VulkanRendererError::MeshTooLarge)?;
                indices.extend(primitive.indices);
                let material_index = primitive.material_index.unwrap_or(0);
                if material_index >= source_materials.len() {
                    return Err(VulkanRendererError::InvalidGpuAssetMaterial);
                }
                if source_materials[material_index].alpha_mode == AlphaMode::Blend {
                    return Err(VulkanRendererError::UnsupportedBlendMaterial);
                }
                draws.push(GpuDrawRange {
                    first_index,
                    index_count,
                    vertex_offset,
                    material_index,
                });
            }
        }
        if draws.is_empty() {
            return Err(VulkanRendererError::InvalidGpuAssetGeometry);
        }

        let vertex_bytes = vertices
            .len()
            .checked_mul(size_of::<GpuVertex>())
            .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
        let index_bytes = indices
            .len()
            .checked_mul(size_of::<u32>())
            .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
        let requested_gpu_bytes = prepared_textures.iter().try_fold(
            vertex_bytes
                .checked_add(index_bytes)
                .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?,
            |sum, (_, texture)| {
                sum.checked_add(texture.byte_size)
                    .ok_or(VulkanRendererError::GpuAssetSizeOverflow)
            },
        )?;
        self.check_memory_budget(requested_gpu_bytes)?;

        let (vertex_buffer, index_buffer) = upload_gpu_mesh(
            &self.device,
            &self.memory_properties,
            self.queue,
            self.queue_family_index,
            &vertices,
            &indices,
        )?;
        let mut textures = Vec::with_capacity(prepared_textures.len());
        for (mode, texture) in &prepared_textures {
            let format = match mode {
                TextureMipMode::SrgbColor => self.srgb_format,
                TextureMipMode::LinearData | TextureMipMode::NormalMap => self.linear_format,
            };
            textures.push(TextureResource::new(
                &self.device,
                &self.memory_properties,
                self.queue,
                self.queue_family_index,
                format,
                texture,
            )?);
        }

        let materials = source_materials
            .into_iter()
            .map(|source| GpuMaterial {
                base_color_texture: texture_index(
                    source.base_color_texture,
                    TextureMipMode::SrgbColor,
                    &texture_indices,
                ),
                metallic_roughness_texture: texture_index(
                    source.metallic_roughness_texture,
                    TextureMipMode::LinearData,
                    &texture_indices,
                ),
                normal_texture: texture_index(
                    source.normal_texture,
                    TextureMipMode::NormalMap,
                    &texture_indices,
                ),
                occlusion_texture: texture_index(
                    source.occlusion_texture,
                    TextureMipMode::LinearData,
                    &texture_indices,
                ),
                emissive_texture: texture_index(
                    source.emissive_texture,
                    TextureMipMode::SrgbColor,
                    &texture_indices,
                ),
                source,
            })
            .collect();
        let gpu_model = GpuModelAsset {
            vertex_buffer,
            index_buffer,
            draws,
            instances,
            materials,
            textures,
            gpu_bytes: requested_gpu_bytes,
        };
        let slot_index = self.free_slots.pop().unwrap_or_else(|| {
            self.slots.push(ModelSlot::default());
            self.slots.len() - 1
        });
        let handle = GpuModelHandle {
            slot: slot_index,
            generation: self.slots[slot_index].generation,
        };
        self.slots[slot_index].key = Some(key.to_owned());
        self.slots[slot_index].model = Some(gpu_model);
        self.by_key.insert(key.to_owned(), handle);
        self.resident_gpu_bytes += requested_gpu_bytes;
        Ok(handle)
    }

    pub(super) fn model(&self, handle: GpuModelHandle) -> Option<&GpuModelAsset> {
        let slot = self.slots.get(handle.slot)?;
        (slot.generation == handle.generation)
            .then_some(slot.model.as_ref())
            .flatten()
    }

    /// Releases a resident model after the caller has waited for the device to become idle.
    pub(super) fn unload_after_device_idle(&mut self, handle: GpuModelHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot) else {
            return false;
        };
        if slot.generation != handle.generation {
            return false;
        }
        let Some(model) = slot.model.take() else {
            return false;
        };
        self.resident_gpu_bytes -= model.gpu_bytes;
        if let Some(key) = slot.key.take() {
            self.by_key.remove(&key);
        }
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            self.free_slots.push(handle.slot);
        }
        drop(model);
        true
    }

    pub(super) fn resident_gpu_bytes(&self) -> usize {
        self.resident_gpu_bytes
    }

    pub(super) fn model_count(&self) -> usize {
        self.by_key.len()
    }

    fn check_memory_budget(&self, requested: usize) -> Result<()> {
        let resident = self
            .resident_gpu_bytes
            .checked_add(requested)
            .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
        if resident > self.max_gpu_bytes {
            return Err(VulkanRendererError::GpuAssetBudgetExceeded {
                resident: self.resident_gpu_bytes,
                requested,
                budget: self.max_gpu_bytes,
            });
        }
        Ok(())
    }
}

impl Drop for GpuAssetCache {
    fn drop(&mut self) {
        // Cache owners only drop this after render work has finished. Upload helpers also wait
        // for the transfer queue before returning, so partially built entries are safe to release.
        for slot_index in 0..self.slots.len() {
            let Some(slot) = self.slots.get(slot_index) else {
                continue;
            };
            if slot.model.is_none() {
                continue;
            }
            let handle = GpuModelHandle {
                slot: slot_index,
                generation: slot.generation,
            };
            let _ = self.unload_after_device_idle(handle);
        }
    }
}

fn material_texture_roles(
    material: &CpuMaterialAsset,
) -> [(Option<TextureBinding>, TextureMipMode); 5] {
    [
        (material.base_color_texture, TextureMipMode::SrgbColor),
        (
            material.metallic_roughness_texture,
            TextureMipMode::LinearData,
        ),
        (material.normal_texture, TextureMipMode::NormalMap),
        (material.occlusion_texture, TextureMipMode::LinearData),
        (material.emissive_texture, TextureMipMode::SrgbColor),
    ]
}

fn texture_index(
    binding: Option<TextureBinding>,
    mode: TextureMipMode,
    indices: &HashMap<(usize, TextureMipMode), usize>,
) -> Option<usize> {
    binding.and_then(|binding| indices.get(&(binding.image_index, mode)).copied())
}

fn default_material() -> CpuMaterialAsset {
    CpuMaterialAsset {
        name: Some("Default".to_owned()),
        base_color_factor: [1.0; 4],
        metallic_factor: 0.0,
        roughness_factor: 1.0,
        emissive_factor: [0.0; 3],
        base_color_texture: None,
        metallic_roughness_texture: None,
        normal_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        alpha_mode: AlphaMode::Opaque,
        alpha_cutoff: 0.5,
        double_sided: false,
        normal_scale: 1.0,
        occlusion_strength: 1.0,
    }
}

pub(super) fn upload_gpu_mesh(
    device: &ash::Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    queue: vk::Queue,
    queue_family_index: u32,
    vertices: &[GpuVertex],
    indices: &[u32],
) -> Result<(BufferAllocation, BufferAllocation)> {
    let vertex_data = vertex_bytes(vertices);
    let index_data = index_bytes(indices);
    let index_offset = align_up_4(vertex_data.len());
    let index_copy_size = align_up_4(index_data.len());
    let staging_size = index_offset
        .checked_add(index_copy_size)
        .ok_or(VulkanRendererError::GpuAssetSizeOverflow)?;
    let mut staging_data = vec![0; staging_size];
    staging_data[..vertex_data.len()].copy_from_slice(vertex_data);
    staging_data[index_offset..index_offset + index_data.len()].copy_from_slice(index_data);
    let staging = BufferAllocation::new(
        device,
        memory_properties,
        staging_size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE,
        vk::MemoryPropertyFlags::HOST_COHERENT,
        "host-visible model staging",
    )?;
    staging.upload(&staging_data)?;
    let vertex_buffer = BufferAllocation::new(
        device,
        memory_properties,
        vertex_data.len(),
        vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::VERTEX_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        "device-local model vertex",
    )?;
    let index_buffer = BufferAllocation::new(
        device,
        memory_properties,
        index_copy_size,
        vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::INDEX_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        "device-local model index",
    )?;
    let pool_info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::TRANSIENT)
        .queue_family_index(queue_family_index);
    // SAFETY: The renderer selected this queue family for graphics and transfer operations.
    let command_pool = unsafe { device.create_command_pool(&pool_info, None)? };
    let result: Result<()> = (|| {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: The transient command pool is live and owns the newly allocated primary buffer.
        let command_buffer = unsafe { device.allocate_command_buffers(&allocate_info)?[0] };
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: This command buffer is freshly allocated and is not in flight.
        unsafe { device.begin_command_buffer(command_buffer, &begin_info)? };
        let vertex_copy = [vk::BufferCopy::default().size(vertex_data.len() as u64)];
        let index_copy = [vk::BufferCopy::default()
            .src_offset(index_offset as u64)
            .size(index_copy_size as u64)];
        // SAFETY: Staging and destination ranges are live, aligned, and large enough for the copies.
        unsafe {
            device.cmd_copy_buffer(
                command_buffer,
                staging.buffer,
                vertex_buffer.buffer,
                &vertex_copy,
            );
            device.cmd_copy_buffer(
                command_buffer,
                staging.buffer,
                index_buffer.buffer,
                &index_copy,
            );
            let barriers = [
                vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::VERTEX_ATTRIBUTE_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(vertex_buffer.buffer)
                    .offset(0)
                    .size(vertex_data.len() as u64),
                vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::INDEX_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(index_buffer.buffer)
                    .offset(0)
                    .size(index_copy_size as u64),
            ];
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::VERTEX_INPUT,
                vk::DependencyFlags::empty(),
                &[],
                &barriers,
                &[],
            );
            device.end_command_buffer(command_buffer)?;
        }
        let command_buffers = [command_buffer];
        let submit = [vk::SubmitInfo::default().command_buffers(&command_buffers)];
        // SAFETY: All referenced buffers remain alive until the queue operation completes.
        unsafe {
            device.queue_submit(queue, &submit, vk::Fence::null())?;
            device.queue_wait_idle(queue)?;
        }
        Ok(())
    })();
    if result.is_err() {
        // SAFETY: Best-effort device idle before destroying the transient command pool on failure.
        let _ = unsafe { device.device_wait_idle() };
    }
    // SAFETY: The queue is idle on success and was idled above on failure.
    unsafe { device.destroy_command_pool(command_pool, None) };
    result?;
    Ok((vertex_buffer, index_buffer))
}

fn vertex_bytes(vertices: &[GpuVertex]) -> &[u8] {
    // SAFETY: GpuVertex is repr(C) and contains only initialized fixed-size f32 arrays.
    unsafe { std::slice::from_raw_parts(vertices.as_ptr().cast::<u8>(), size_of_val(vertices)) }
}

fn index_bytes(indices: &[u32]) -> &[u8] {
    // SAFETY: u32 values have no padding and all elements are initialized.
    unsafe { std::slice::from_raw_parts(indices.as_ptr().cast::<u8>(), size_of_val(indices)) }
}

fn align_up_4(size: usize) -> usize {
    size.saturating_add(3) & !3
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::{CpuVertex, GpuVertex};

    #[test]
    fn gpu_vertex_layout_is_tightly_packed_for_shader_attributes() {
        let vertex = GpuVertex::from(CpuVertex {
            position: [1.0, 2.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            uv0: [0.25, 0.75],
            color: [1.0; 4],
        });
        assert_eq!(size_of::<GpuVertex>(), 16 * size_of::<f32>());
        assert_eq!(vertex.uv, [0.25, 0.75]);
    }
}
