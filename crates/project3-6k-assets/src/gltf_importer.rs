//! Bounded CPU import for static glTF 2.0 and GLB assets.

use std::{
    fs,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Component, Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use glam::{Mat4, Vec2, Vec3, Vec4};
use thiserror::Error;

/// Resource limits applied while importing one model and its referenced files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelImportLimits {
    /// Maximum glTF JSON or GLB container size.
    pub max_source_bytes: usize,
    /// Maximum size of any external buffer or encoded image.
    pub max_external_file_bytes: usize,
    /// Maximum combined bytes read from external buffers and encoded images.
    pub max_external_bytes: usize,
    /// Maximum combined decoded buffers, images, vertices, and indices during import.
    pub max_total_cpu_bytes: usize,
    /// Maximum decoded width or height of an image.
    pub max_image_dimension: u32,
    /// Maximum vertices retained by one model.
    pub max_vertices: usize,
    /// Maximum indices retained by one model.
    pub max_indices: usize,
    /// Maximum meshes retained by one model.
    pub max_meshes: usize,
    /// Maximum mesh primitives retained by one model.
    pub max_primitives: usize,
    /// Maximum images decoded by one model.
    pub max_images: usize,
    /// Maximum texture objects referenced by one model.
    pub max_textures: usize,
    /// Maximum materials retained by one model.
    pub max_materials: usize,
    /// Maximum buffers referenced by one model.
    pub max_buffers: usize,
    /// Maximum nodes traversed in the selected glTF scene.
    pub max_nodes: usize,
    /// Maximum hierarchy depth accepted by the importer.
    pub max_node_depth: usize,
}

impl Default for ModelImportLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 32 * 1024 * 1024,
            max_external_file_bytes: 64 * 1024 * 1024,
            max_external_bytes: 128 * 1024 * 1024,
            max_total_cpu_bytes: 256 * 1024 * 1024,
            max_image_dimension: 8192,
            max_vertices: 2_000_000,
            max_indices: 6_000_000,
            max_meshes: 16_384,
            max_primitives: 65_536,
            max_images: 8192,
            max_textures: 16_384,
            max_materials: 16_384,
            max_buffers: 1024,
            max_nodes: 65_536,
            max_node_depth: 256,
        }
    }
}

/// Vertex data decoded from glTF into a stable CPU representation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpuVertex {
    /// Local-space position.
    pub position: [f32; 3],
    /// Local-space normal.
    pub normal: [f32; 3],
    /// Local-space tangent and bitangent handedness.
    pub tangent: [f32; 4],
    /// First texture coordinate set; defaults to zero when absent.
    pub uv0: [f32; 2],
    /// Linear vertex color; defaults to white when absent.
    pub color: [f32; 4],
}

/// One indexed triangle primitive and its optional material reference.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuPrimitiveAsset {
    /// Local-space vertices.
    pub vertices: Vec<CpuVertex>,
    /// Triangle-list indices into [`Self::vertices`].
    pub indices: Vec<u32>,
    /// Index into [`CpuModelAsset::materials`], or `None` for the glTF default material.
    pub material_index: Option<usize>,
}

/// A glTF mesh containing one or more renderable primitives.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuMeshAsset {
    /// Optional authored mesh name.
    pub name: Option<String>,
    /// Triangle primitives in authored order.
    pub primitives: Vec<CpuPrimitiveAsset>,
}

/// Image data shared by material texture bindings. Raw decoded channel values are retained;
/// the renderer chooses sRGB or linear GPU formats based on the binding's material role.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuImageAsset {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Decoded row-major RGBA8 pixels.
    pub rgba8: Vec<u8>,
}

/// Reference to an imported image and texture coordinate set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextureBinding {
    /// Index into [`CpuModelAsset::images`].
    pub image_index: usize,
    /// Texture coordinate set selected by the glTF material.
    pub tex_coord: u32,
}

/// glTF alpha rendering mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlphaMode {
    /// Fully opaque material.
    Opaque,
    /// Alpha cutoff material.
    Mask,
    /// Alpha blended material.
    Blend,
}

/// CPU PBR material data using the glTF metallic-roughness convention.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuMaterialAsset {
    /// Optional authored material name.
    pub name: Option<String>,
    /// Linear base color multiplier.
    pub base_color_factor: [f32; 4],
    /// Metallic factor in [0, 1].
    pub metallic_factor: f32,
    /// Roughness factor in [0, 1].
    pub roughness_factor: f32,
    /// Emissive factor in linear RGB.
    pub emissive_factor: [f32; 3],
    /// Base-color map (sampled as sRGB by the renderer).
    pub base_color_texture: Option<TextureBinding>,
    /// Metallic B / roughness G map (sampled as linear data).
    pub metallic_roughness_texture: Option<TextureBinding>,
    /// Tangent-space normal map and scale.
    pub normal_texture: Option<TextureBinding>,
    /// Ambient occlusion map and strength.
    pub occlusion_texture: Option<TextureBinding>,
    /// Emissive map (sampled as sRGB by the renderer).
    pub emissive_texture: Option<TextureBinding>,
    /// Alpha interpretation.
    pub alpha_mode: AlphaMode,
    /// Alpha cutoff used by mask materials.
    pub alpha_cutoff: f32,
    /// Whether back-face culling should be disabled for this material.
    pub double_sided: bool,
    /// Normal map intensity.
    pub normal_scale: f32,
    /// Ambient occlusion intensity.
    pub occlusion_strength: f32,
}

/// One node instance referencing a reusable mesh and a baked parent-to-world transform.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuModelInstance {
    /// Index into [`CpuModelAsset::meshes`].
    pub mesh_index: usize,
    /// Column-major local-to-world matrix.
    pub transform: [[f32; 4]; 4],
    /// Optional authored node name.
    pub name: Option<String>,
}

/// Fully decoded static model data, independent of Vulkan resources.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuModelAsset {
    /// Optional model name from the selected scene or source file.
    pub name: Option<String>,
    /// Reusable mesh data.
    pub meshes: Vec<CpuMeshAsset>,
    /// Mesh instances in the selected scene.
    pub instances: Vec<CpuModelInstance>,
    /// PBR materials; primitive material indices refer to this list.
    pub materials: Vec<CpuMaterialAsset>,
    /// Decoded source images, shared by texture bindings.
    pub images: Vec<CpuImageAsset>,
    /// Retained CPU bytes for model metadata, geometry, and decoded images.
    pub(crate) byte_size: usize,
}

impl CpuModelAsset {
    /// Returns CPU bytes retained by the imported geometry and image arrays.
    pub fn byte_size(&self) -> usize {
        self.byte_size
    }
}

/// Errors raised by glTF parsing, resource validation, and CPU decoding.
#[derive(Debug, Error)]
pub enum ModelImportError {
    #[error("failed to read model or referenced resource {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("glTF parsing failed: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("image decoding failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("image decoder panicked while processing the source data")]
    ImageDecoderPanicked,
    #[error("unsupported or unsafe external resource URI")]
    InvalidUri,
    #[error("data URI must use base64 encoding and contain valid payload data")]
    InvalidDataUri,
    #[error("resource limit exceeded for {kind}: {actual} > {limit}")]
    LimitExceeded {
        kind: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("glTF buffer {index} is missing its binary payload")]
    MissingBuffer { index: usize },
    #[error("glTF buffer {index} contains {actual} bytes but requires {expected}")]
    BufferTooSmall {
        index: usize,
        actual: usize,
        expected: usize,
    },
    #[error("glTF image {index} references a missing buffer or invalid byte range")]
    InvalidImageRange { index: usize },
    #[error("glTF model has no scene to import")]
    MissingScene,
    #[error("unsupported non-triangle primitive mode {0:?}")]
    UnsupportedPrimitive(gltf::mesh::Mode),
    #[error("unsupported skinned or morphed mesh data")]
    UnsupportedDeformation,
    #[error("glTF primitive has no POSITION attribute")]
    MissingPositions,
    #[error("glTF primitive has inconsistent or invalid vertex attributes")]
    InvalidVertexData,
    #[error("glTF image dimensions or decoded pixel data are invalid")]
    InvalidImageData,
    #[error("glTF model has no usable triangle indices")]
    InvalidTriangleData,
    #[error("glTF hierarchy exceeded its configured bounds")]
    InvalidHierarchy,
}

/// Imports static glTF 2.0 (`.gltf`) or binary glTF (`.glb`) into bounded CPU-owned data.
/// External references must resolve inside the source model's directory.
pub fn import_gltf(
    path: impl AsRef<Path>,
    limits: ModelImportLimits,
) -> Result<CpuModelAsset, ModelImportError> {
    let source_path = path
        .as_ref()
        .canonicalize()
        .map_err(|source| ModelImportError::Io {
            path: path.as_ref().to_owned(),
            source,
        })?;
    let source_len = file_len(&source_path)?;
    check_limit("model source bytes", source_len, limits.max_source_bytes)?;
    let source_bytes = fs::read(&source_path).map_err(|source| ModelImportError::Io {
        path: source_path.clone(),
        source,
    })?;
    check_limit(
        "model source bytes",
        source_bytes.len(),
        limits.max_source_bytes,
    )?;
    let source_size = source_bytes.len();
    let gltf = gltf::Gltf::from_slice(&source_bytes)?;
    drop(source_bytes);
    let base_dir = source_path.parent().ok_or(ModelImportError::InvalidUri)?;
    let mut total_bytes = source_size;
    let mut external_bytes = 0usize;
    let blob = gltf.blob.map(std::sync::Arc::<[u8]>::from);

    check_limit(
        "buffer count",
        gltf.document.buffers().len(),
        limits.max_buffers,
    )?;
    check_limit(
        "image count",
        gltf.document.images().len(),
        limits.max_images,
    )?;
    check_limit(
        "material count",
        gltf.document.materials().len(),
        limits.max_materials,
    )?;
    check_limit(
        "texture count",
        gltf.document.textures().len(),
        limits.max_textures,
    )?;
    let mut buffers = Vec::with_capacity(gltf.document.buffers().len());
    for buffer in gltf.document.buffers() {
        let bytes = match buffer.source() {
            gltf::buffer::Source::Bin => {
                blob.as_ref()
                    .cloned()
                    .ok_or(ModelImportError::MissingBuffer {
                        index: buffer.index(),
                    })?
            }
            gltf::buffer::Source::Uri(uri) => {
                let data = read_uri(uri, base_dir, limits.max_external_file_bytes)?;
                external_bytes = checked_add(external_bytes, data.len())?;
                check_limit(
                    "external resource bytes",
                    external_bytes,
                    limits.max_external_bytes,
                )?;
                total_bytes = checked_add(total_bytes, data.len())?;
                check_limit("total CPU bytes", total_bytes, limits.max_total_cpu_bytes)?;
                std::sync::Arc::<[u8]>::from(data)
            }
        };
        let expected = buffer.length();
        if bytes.len() < expected {
            return Err(ModelImportError::BufferTooSmall {
                index: buffer.index(),
                actual: bytes.len(),
                expected,
            });
        }
        buffers.push(bytes);
    }

    let mut images = Vec::with_capacity(gltf.document.images().len());
    for image in gltf.document.images() {
        let encoded = match image.source() {
            gltf::image::Source::Uri { uri, .. } => {
                let bytes = read_uri(uri, base_dir, limits.max_external_file_bytes)?;
                external_bytes = checked_add(external_bytes, bytes.len())?;
                check_limit(
                    "external resource bytes",
                    external_bytes,
                    limits.max_external_bytes,
                )?;
                total_bytes = checked_add(total_bytes, bytes.len())?;
                check_limit("total CPU bytes", total_bytes, limits.max_total_cpu_bytes)?;
                bytes
            }
            gltf::image::Source::View { view, .. } => {
                let buffer = buffers.get(view.buffer().index()).ok_or(
                    ModelImportError::InvalidImageRange {
                        index: image.index(),
                    },
                )?;
                let begin = view.offset();
                let end = begin.checked_add(view.length()).ok_or(
                    ModelImportError::InvalidImageRange {
                        index: image.index(),
                    },
                )?;
                buffer
                    .get(begin..end)
                    .ok_or(ModelImportError::InvalidImageRange {
                        index: image.index(),
                    })?
                    .to_vec()
            }
        };
        check_limit(
            "encoded image bytes",
            encoded.len(),
            limits.max_external_file_bytes,
        )?;
        let decoded = decode_image(encoded, limits, &source_path)?;
        let width = decoded.width;
        let height = decoded.height;
        let rgba8 = decoded.rgba8;
        total_bytes = checked_add(total_bytes, rgba8.len())?;
        check_limit("total CPU bytes", total_bytes, limits.max_total_cpu_bytes)?;
        images.push(CpuImageAsset {
            width,
            height,
            rgba8,
        });
    }

    let materials = gltf
        .document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            let texture_binding = |info: gltf::texture::Info<'_>| TextureBinding {
                image_index: info.texture().source().index(),
                tex_coord: info.tex_coord(),
            };
            let normal = material.normal_texture();
            let occlusion = material.occlusion_texture();
            let alpha_mode = match material.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            };
            CpuMaterialAsset {
                name: material.name().map(str::to_owned),
                base_color_factor: pbr.base_color_factor(),
                metallic_factor: pbr.metallic_factor(),
                roughness_factor: pbr.roughness_factor(),
                emissive_factor: material.emissive_factor(),
                base_color_texture: pbr.base_color_texture().map(texture_binding),
                metallic_roughness_texture: pbr.metallic_roughness_texture().map(texture_binding),
                normal_texture: normal.as_ref().map(|info| TextureBinding {
                    image_index: info.texture().source().index(),
                    tex_coord: info.tex_coord(),
                }),
                occlusion_texture: occlusion.as_ref().map(|info| TextureBinding {
                    image_index: info.texture().source().index(),
                    tex_coord: info.tex_coord(),
                }),
                emissive_texture: material.emissive_texture().map(texture_binding),
                alpha_mode,
                alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
                double_sided: material.double_sided(),
                normal_scale: normal.map_or(1.0, |info| info.scale()),
                occlusion_strength: occlusion.map_or(1.0, |info| info.strength()),
            }
        })
        .collect::<Vec<_>>();

    check_limit(
        "mesh count",
        gltf.document.meshes().len(),
        limits.max_meshes,
    )?;
    let mut primitive_count = 0usize;
    let mut meshes = Vec::with_capacity(gltf.document.meshes().len());
    let mut vertex_count = 0usize;
    let mut index_count = 0usize;
    for mesh in gltf.document.meshes() {
        primitive_count = checked_add(primitive_count, mesh.primitives().len())?;
        check_limit("primitive count", primitive_count, limits.max_primitives)?;
        let mut primitives = Vec::with_capacity(mesh.primitives().len());
        for primitive in mesh.primitives() {
            if !primitive.morph_targets().next().is_none() {
                return Err(ModelImportError::UnsupportedDeformation);
            }
            let reader =
                primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| data.as_ref()));
            let position_reader = reader
                .read_positions()
                .ok_or(ModelImportError::MissingPositions)?;
            let count = position_reader.len();
            let next_vertex_count = checked_add(vertex_count, count)?;
            check_limit("vertex count", next_vertex_count, limits.max_vertices)?;
            let positions = position_reader.collect::<Vec<_>>();
            let local_indices = if let Some(values) = reader.read_indices() {
                let values = values.into_u32();
                check_limit("source index count", values.len(), limits.max_indices)?;
                values.collect::<Vec<_>>()
            } else {
                check_limit("source index count", count, limits.max_indices)?;
                (0..count)
                    .map(|index| {
                        u32::try_from(index).map_err(|_| ModelImportError::InvalidTriangleData)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            if local_indices.is_empty() {
                return Err(ModelImportError::InvalidTriangleData);
            }
            let remaining_indices = limits.max_indices.saturating_sub(index_count);
            let indices = triangulate(primitive.mode(), &local_indices, remaining_indices)?;
            if indices
                .iter()
                .any(|&index| index as usize >= positions.len())
            {
                return Err(ModelImportError::InvalidTriangleData);
            }
            let mut normals = match reader.read_normals() {
                Some(values) if values.len() == count => values.collect::<Vec<_>>(),
                Some(_) => return Err(ModelImportError::InvalidVertexData),
                None => Vec::new(),
            };
            if normals.is_empty() {
                normals = generate_normals(&positions, &indices);
            }
            let mut tangents = match reader.read_tangents() {
                Some(values) if values.len() == count => values.collect::<Vec<_>>(),
                Some(_) => return Err(ModelImportError::InvalidVertexData),
                None => Vec::new(),
            };
            let uv0 = match reader.read_tex_coords(0) {
                Some(values) => {
                    let values = values.into_f32();
                    if values.len() != count {
                        return Err(ModelImportError::InvalidVertexData);
                    }
                    values.collect::<Vec<_>>()
                }
                None => vec![[0.0, 0.0]; count],
            };
            let colors = match reader.read_colors(0) {
                Some(values) => {
                    let values = values.into_rgba_f32();
                    if values.len() != count {
                        return Err(ModelImportError::InvalidVertexData);
                    }
                    values.collect::<Vec<_>>()
                }
                None => vec![[1.0, 1.0, 1.0, 1.0]; count],
            };
            if tangents.is_empty() {
                tangents = generate_tangents(&positions, &normals, &uv0, &indices);
            }
            if tangents.len() != count
                || positions
                    .iter()
                    .flatten()
                    .chain(normals.iter().flatten())
                    .chain(tangents.iter().flatten())
                    .chain(uv0.iter().flatten())
                    .chain(colors.iter().flatten())
                    .any(|value| !value.is_finite())
            {
                return Err(ModelImportError::InvalidVertexData);
            }
            vertex_count = next_vertex_count;
            index_count = checked_add(index_count, indices.len())?;
            check_limit("vertex count", vertex_count, limits.max_vertices)?;
            check_limit("index count", index_count, limits.max_indices)?;
            let vertices = positions
                .into_iter()
                .zip(normals)
                .zip(tangents)
                .zip(uv0)
                .zip(colors)
                .map(|((((position, normal), tangent), uv0), color)| CpuVertex {
                    position,
                    normal,
                    tangent,
                    uv0,
                    color,
                })
                .collect::<Vec<_>>();
            let material_index = primitive.material().index();
            if material_index.is_some_and(|index| index >= materials.len()) {
                return Err(ModelImportError::InvalidVertexData);
            }
            let geometry_bytes = vertices
                .len()
                .checked_mul(std::mem::size_of::<CpuVertex>())
                .and_then(|bytes| {
                    indices
                        .len()
                        .checked_mul(std::mem::size_of::<u32>())
                        .and_then(|index_bytes| bytes.checked_add(index_bytes))
                })
                .ok_or(ModelImportError::InvalidTriangleData)?;
            total_bytes = checked_add(total_bytes, geometry_bytes)?;
            check_limit("total CPU bytes", total_bytes, limits.max_total_cpu_bytes)?;
            primitives.push(CpuPrimitiveAsset {
                vertices,
                indices,
                material_index,
            });
        }
        meshes.push(CpuMeshAsset {
            name: mesh.name().map(str::to_owned),
            primitives,
        });
    }

    let scene = gltf
        .document
        .default_scene()
        .or_else(|| gltf.document.scenes().next())
        .ok_or(ModelImportError::MissingScene)?;
    let root_node_count = scene.nodes().len();
    check_limit("root node count", root_node_count, limits.max_nodes)?;
    let mut instances = Vec::new();
    let mut stack = Vec::with_capacity(root_node_count);
    stack.extend(scene.nodes().map(|node| (node, Mat4::IDENTITY, 0usize)));
    let mut visited = 0usize;
    while let Some((node, parent_transform, depth)) = stack.pop() {
        visited = checked_add(visited, 1)?;
        check_limit("node count", visited, limits.max_nodes)?;
        if depth > limits.max_node_depth {
            return Err(ModelImportError::InvalidHierarchy);
        }
        if node.skin().is_some() || node.weights().is_some() {
            return Err(ModelImportError::UnsupportedDeformation);
        }
        let local = Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent_transform * local;
        if !world.is_finite() {
            return Err(ModelImportError::InvalidHierarchy);
        }
        if let Some(mesh) = node.mesh() {
            instances.push(CpuModelInstance {
                mesh_index: mesh.index(),
                transform: world.to_cols_array_2d(),
                name: node.name().map(str::to_owned),
            });
        }
        for child in node.children() {
            let scheduled_nodes = checked_add(visited, stack.len())?;
            check_limit(
                "scheduled node count",
                checked_add(scheduled_nodes, 1)?,
                limits.max_nodes,
            )?;
            stack.push((child, world, depth + 1));
        }
    }
    if instances
        .iter()
        .any(|instance| instance.mesh_index >= meshes.len())
    {
        return Err(ModelImportError::InvalidHierarchy);
    }
    let model_name = scene.name().map(str::to_owned);
    let image_bytes = images
        .iter()
        .try_fold(0usize, |sum, image| checked_add(sum, image.rgba8.len()))?;
    let geometry_bytes =
        meshes
            .iter()
            .flat_map(|mesh| &mesh.primitives)
            .try_fold(0usize, |sum, primitive| {
                let vertex_bytes = primitive
                    .vertices
                    .len()
                    .checked_mul(std::mem::size_of::<CpuVertex>())
                    .ok_or(ModelImportError::InvalidTriangleData)?;
                let index_bytes = primitive
                    .indices
                    .len()
                    .checked_mul(std::mem::size_of::<u32>())
                    .ok_or(ModelImportError::InvalidTriangleData)?;
                checked_add(sum, checked_add(vertex_bytes, index_bytes)?)
            })?;
    let metadata_bytes = [
        std::mem::size_of::<CpuModelAsset>(),
        meshes
            .len()
            .checked_mul(std::mem::size_of::<CpuMeshAsset>())
            .ok_or(ModelImportError::InvalidTriangleData)?,
        primitive_count
            .checked_mul(std::mem::size_of::<CpuPrimitiveAsset>())
            .ok_or(ModelImportError::InvalidTriangleData)?,
        instances
            .len()
            .checked_mul(std::mem::size_of::<CpuModelInstance>())
            .ok_or(ModelImportError::InvalidTriangleData)?,
        materials
            .len()
            .checked_mul(std::mem::size_of::<CpuMaterialAsset>())
            .ok_or(ModelImportError::InvalidTriangleData)?,
        images
            .len()
            .checked_mul(std::mem::size_of::<CpuImageAsset>())
            .ok_or(ModelImportError::InvalidTriangleData)?,
    ]
    .into_iter()
    .try_fold(0usize, checked_add)?;
    let name_bytes = meshes
        .iter()
        .filter_map(|mesh| mesh.name.as_ref())
        .map(String::len)
        .chain(
            instances
                .iter()
                .filter_map(|instance| instance.name.as_ref())
                .map(String::len),
        )
        .chain(
            materials
                .iter()
                .filter_map(|material| material.name.as_ref())
                .map(String::len),
        )
        .try_fold(model_name.as_ref().map_or(0, String::len), checked_add)?;
    let metadata_bytes = checked_add(metadata_bytes, name_bytes)?;
    total_bytes = checked_add(total_bytes, metadata_bytes)?;
    check_limit("total CPU bytes", total_bytes, limits.max_total_cpu_bytes)?;
    let retained_bytes = checked_add(checked_add(image_bytes, geometry_bytes)?, metadata_bytes)?;
    check_limit(
        "retained CPU bytes",
        retained_bytes,
        limits.max_total_cpu_bytes,
    )?;

    Ok(CpuModelAsset {
        name: model_name,
        meshes,
        instances,
        materials,
        images,
        byte_size: retained_bytes,
    })
}

/// Loads a common raster texture format into bounded CPU-owned RGBA8 data.
pub fn import_image(
    path: impl AsRef<Path>,
    limits: ModelImportLimits,
) -> Result<CpuImageAsset, ModelImportError> {
    let path = path
        .as_ref()
        .canonicalize()
        .map_err(|source| ModelImportError::Io {
            path: path.as_ref().to_owned(),
            source,
        })?;
    check_limit(
        "image source bytes",
        file_len(&path)?,
        limits.max_external_file_bytes,
    )?;
    let encoded = fs::read(&path).map_err(|source| ModelImportError::Io {
        path: path.clone(),
        source,
    })?;
    check_limit(
        "image source bytes",
        encoded.len(),
        limits.max_external_file_bytes,
    )?;
    decode_image(encoded, limits, &path)
}

fn decode_image(
    encoded: Vec<u8>,
    limits: ModelImportLimits,
    source_path: &Path,
) -> Result<CpuImageAsset, ModelImportError> {
    check_limit(
        "encoded image bytes",
        encoded.len(),
        limits.max_external_file_bytes,
    )?;
    let mut reader = image::ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(|source| ModelImportError::Io {
            path: source_path.to_owned(),
            source,
        })?;
    let mut decode_limits = image::Limits::default();
    decode_limits.max_image_width = Some(limits.max_image_dimension);
    decode_limits.max_image_height = Some(limits.max_image_dimension);
    decode_limits.max_alloc = Some(limits.max_total_cpu_bytes as u64);
    reader.limits(decode_limits);
    let decoded = catch_unwind(AssertUnwindSafe(|| reader.decode()))
        .map_err(|_| ModelImportError::ImageDecoderPanicked)??;
    let width = decoded.width();
    let height = decoded.height();
    if width == 0
        || height == 0
        || width > limits.max_image_dimension
        || height > limits.max_image_dimension
    {
        return Err(ModelImportError::InvalidImageData);
    }
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ModelImportError::InvalidImageData)?;
    check_limit("decoded image bytes", expected, limits.max_total_cpu_bytes)?;
    let rgba8 = decoded.into_rgba8().into_raw();
    if rgba8.len() != expected {
        return Err(ModelImportError::InvalidImageData);
    }
    check_limit(
        "decoded image bytes",
        rgba8.len(),
        limits.max_total_cpu_bytes,
    )?;
    Ok(CpuImageAsset {
        width,
        height,
        rgba8,
    })
}

fn triangulate(
    mode: gltf::mesh::Mode,
    source: &[u32],
    max_output_indices: usize,
) -> Result<Vec<u32>, ModelImportError> {
    match mode {
        gltf::mesh::Mode::Triangles => {
            if source.len() < 3 || source.len() % 3 != 0 {
                return Err(ModelImportError::InvalidTriangleData);
            }
            check_limit("index count", source.len(), max_output_indices)?;
            Ok(source.to_vec())
        }
        gltf::mesh::Mode::TriangleStrip => {
            if source.len() < 3 {
                return Err(ModelImportError::InvalidTriangleData);
            }
            let capacity = (source.len() - 2)
                .checked_mul(3)
                .ok_or(ModelImportError::InvalidTriangleData)?;
            check_limit("index count", capacity, max_output_indices)?;
            let mut result = Vec::with_capacity(capacity);
            for index in 2..source.len() {
                let (a, b) = if index % 2 == 0 {
                    (source[index - 2], source[index - 1])
                } else {
                    (source[index - 1], source[index - 2])
                };
                let c = source[index];
                if a != b && b != c && a != c {
                    result.extend_from_slice(&[a, b, c]);
                }
            }
            if result.is_empty() {
                return Err(ModelImportError::InvalidTriangleData);
            }
            Ok(result)
        }
        gltf::mesh::Mode::TriangleFan => {
            if source.len() < 3 {
                return Err(ModelImportError::InvalidTriangleData);
            }
            let capacity = (source.len() - 2)
                .checked_mul(3)
                .ok_or(ModelImportError::InvalidTriangleData)?;
            check_limit("index count", capacity, max_output_indices)?;
            let mut result = Vec::with_capacity(capacity);
            for index in 2..source.len() {
                let (a, b, c) = (source[0], source[index - 1], source[index]);
                if a != b && b != c && a != c {
                    result.extend_from_slice(&[a, b, c]);
                }
            }
            if result.is_empty() {
                return Err(ModelImportError::InvalidTriangleData);
            }
            Ok(result)
        }
        unsupported => Err(ModelImportError::UnsupportedPrimitive(unsupported)),
    }
}

fn generate_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let a = Vec3::from_array(positions[triangle[0] as usize]);
        let b = Vec3::from_array(positions[triangle[1] as usize]);
        let c = Vec3::from_array(positions[triangle[2] as usize]);
        let face_normal = (b - a).cross(c - a);
        if face_normal.is_finite() {
            for &index in triangle {
                normals[index as usize] += face_normal;
            }
        }
    }
    normals
        .into_iter()
        .map(|normal| {
            let normal = normal.normalize_or_zero();
            if normal == Vec3::ZERO {
                Vec3::Y.to_array()
            } else {
                normal.to_array()
            }
        })
        .collect()
}

fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uv0: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let mut tangent_sum = vec![Vec3::ZERO; normals.len()];
    let mut bitangent_sum = vec![Vec3::ZERO; normals.len()];
    for triangle in indices.chunks_exact(3) {
        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;
        let p0 = Vec3::from_array(positions[i0]);
        let p1 = Vec3::from_array(positions[i1]);
        let p2 = Vec3::from_array(positions[i2]);
        let uv0a = Vec2::from_array(uv0[i0]);
        let uv0b = Vec2::from_array(uv0[i1]);
        let uv0c = Vec2::from_array(uv0[i2]);
        let duv1 = uv0b - uv0a;
        let duv2 = uv0c - uv0a;
        let determinant = duv1.x * duv2.y - duv1.y * duv2.x;
        if determinant.abs() <= 1.0e-8 {
            continue;
        }
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let tangent = (edge1 * duv2.y - edge2 * duv1.y) / determinant;
        let bitangent = (edge2 * duv1.x - edge1 * duv2.x) / determinant;
        for &index in triangle {
            tangent_sum[index as usize] += tangent;
            bitangent_sum[index as usize] += bitangent;
        }
    }
    normals
        .iter()
        .enumerate()
        .map(|(index, normal)| {
            let normal = Vec3::from_array(*normal).normalize_or_zero();
            let tangent =
                (tangent_sum[index] - normal * normal.dot(tangent_sum[index])).normalize_or_zero();
            let tangent = if tangent == Vec3::ZERO {
                let axis = if normal.y.abs() < 0.9 {
                    Vec3::Y
                } else {
                    Vec3::X
                };
                normal.cross(axis).normalize_or_zero()
            } else {
                tangent
            };
            let handedness = if normal.cross(tangent).dot(bitangent_sum[index]) < 0.0 {
                -1.0
            } else {
                1.0
            };
            Vec4::new(tangent.x, tangent.y, tangent.z, handedness).to_array()
        })
        .collect()
}

fn read_uri(uri: &str, base_dir: &Path, max_bytes: usize) -> Result<Vec<u8>, ModelImportError> {
    if uri.starts_with("data:") {
        let (_, data) = uri
            .split_once(',')
            .ok_or(ModelImportError::InvalidDataUri)?;
        let (metadata, _) = uri.split_once(',').expect("comma checked above");
        if !metadata.ends_with(";base64") {
            return Err(ModelImportError::InvalidDataUri);
        }
        let estimated = data.len().saturating_mul(3) / 4;
        check_limit("embedded resource bytes", estimated, max_bytes)?;
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| ModelImportError::InvalidDataUri)?;
        check_limit("embedded resource bytes", bytes.len(), max_bytes)?;
        return Ok(bytes);
    }
    if uri.contains(':') || uri.contains('?') || uri.contains('#') {
        return Err(ModelImportError::InvalidUri);
    }
    let decoded = percent_decode(uri)?;
    let relative = Path::new(&decoded);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ModelImportError::InvalidUri);
    }
    let candidate = base_dir.join(relative);
    let canonical = candidate
        .canonicalize()
        .map_err(|source| ModelImportError::Io {
            path: candidate.clone(),
            source,
        })?;
    if !canonical.starts_with(base_dir) {
        return Err(ModelImportError::InvalidUri);
    }
    let length = file_len(&canonical)?;
    check_limit("external resource bytes", length, max_bytes)?;
    let bytes = fs::read(&canonical).map_err(|source| ModelImportError::Io {
        path: canonical,
        source,
    })?;
    check_limit("external resource bytes", bytes.len(), max_bytes)?;
    Ok(bytes)
}

fn percent_decode(value: &str) -> Result<String, ModelImportError> {
    let input = value.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            if index + 2 >= input.len() {
                return Err(ModelImportError::InvalidUri);
            }
            let high = hex_digit(input[index + 1]).ok_or(ModelImportError::InvalidUri)?;
            let low = hex_digit(input[index + 2]).ok_or(ModelImportError::InvalidUri)?;
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| ModelImportError::InvalidUri)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn file_len(path: &Path) -> Result<usize, ModelImportError> {
    let metadata = fs::metadata(path).map_err(|source| ModelImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    usize::try_from(metadata.len()).map_err(|_| ModelImportError::LimitExceeded {
        kind: "file size",
        actual: usize::MAX,
        limit: usize::MAX,
    })
}

fn checked_add(left: usize, right: usize) -> Result<usize, ModelImportError> {
    left.checked_add(right)
        .ok_or(ModelImportError::LimitExceeded {
            kind: "size overflow",
            actual: usize::MAX,
            limit: usize::MAX - 1,
        })
}

fn check_limit(kind: &'static str, actual: usize, limit: usize) -> Result<(), ModelImportError> {
    if actual > limit {
        return Err(ModelImportError::LimitExceeded {
            kind,
            actual,
            limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{ModelImportError, ModelImportLimits, import_gltf, import_image, read_uri};

    static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    fn fixture_dir() -> PathBuf {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("project36k-gltf-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn triangle_buffer() -> Vec<u8> {
        let positions = [[-1.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let normals = [[0.0f32, 0.0, 1.0]; 3];
        let uvs = [[0.0f32, 0.0], [1.0, 0.0], [0.5, 1.0]];
        let indices = [0u16, 1, 2];
        let mut bytes = Vec::new();
        for value in positions
            .iter()
            .flatten()
            .chain(normals.iter().flatten())
            .chain(uvs.iter().flatten())
        {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in indices {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn model_json(buffer_uri: Option<&str>, byte_len: usize, image_uri: Option<&str>) -> String {
        let (material, images, textures) = if let Some(uri) = image_uri {
            (
                String::from(
                    "{\"pbrMetallicRoughness\":{\"baseColorFactor\":[0.8,0.6,0.4,1.0],\"metallicFactor\":0.35,\"roughnessFactor\":0.72,\"baseColorTexture\":{\"index\":0},\"metallicRoughnessTexture\":{\"index\":0}},\"normalTexture\":{\"index\":0},\"occlusionTexture\":{\"index\":0}}",
                ),
                format!(",\"images\":[{{\"uri\":\"{uri}\"}}]"),
                ",\"textures\":[{\"source\":0}]".to_owned(),
            )
        } else {
            (
                String::from(
                    "{\"pbrMetallicRoughness\":{\"baseColorFactor\":[0.8,0.6,0.4,1.0],\"metallicFactor\":0.35,\"roughnessFactor\":0.72}}",
                ),
                String::new(),
                String::new(),
            )
        };
        let buffer = buffer_uri.map_or_else(
            || format!("{{\"byteLength\":{byte_len}}}"),
            |uri| format!("{{\"uri\":\"{uri}\",\"byteLength\":{byte_len}}}"),
        );
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"name\":\"test scene\",\"nodes\":[0]}}],\"nodes\":[{{\"name\":\"triangle\",\"mesh\":0,\"translation\":[1,2,3]}}],\"meshes\":[{{\"name\":\"triangle mesh\",\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1,\"TEXCOORD_0\":2}},\"indices\":3,\"material\":0}}]}}],\"materials\":[{material}],\"buffers\":[{buffer}],\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36}},{{\"buffer\":0,\"byteOffset\":36,\"byteLength\":36}},{{\"buffer\":0,\"byteOffset\":72,\"byteLength\":24}},{{\"buffer\":0,\"byteOffset\":96,\"byteLength\":6}}],\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":[-1,0,0],\"max\":[1,2,0]}},{{\"bufferView\":1,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},{{\"bufferView\":2,\"componentType\":5126,\"count\":3,\"type\":\"VEC2\"}},{{\"bufferView\":3,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}]{images}{textures}}}"
        )
    }

    fn make_glb(document: &str, binary: &[u8]) -> Vec<u8> {
        let mut json = document.as_bytes().to_vec();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bin = binary.to_vec();
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let total_len = 12 + 8 + json.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total_len);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total_len as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E4942u32.to_le_bytes());
        glb.extend_from_slice(&bin);
        glb
    }

    #[test]
    fn imports_gltf_geometry_pbr_maps_and_node_transform() {
        let dir = fixture_dir();
        let buffer = triangle_buffer();
        let buffer_uri = format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(&buffer)
        );
        let png = include_bytes!("../../../assets/forest_noise.png");
        let image_uri = format!("data:image/png;base64,{}", STANDARD.encode(png));
        let source = model_json(Some(&buffer_uri), buffer.len(), Some(&image_uri));
        let model_path = dir.join("forest.gltf");
        fs::write(&model_path, source).unwrap();

        let model = import_gltf(&model_path, ModelImportLimits::default()).unwrap();
        assert_eq!(model.meshes.len(), 1);
        assert_eq!(model.instances.len(), 1);
        assert_eq!(model.instances[0].transform[3], [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(model.materials[0].metallic_factor, 0.35);
        assert_eq!(model.materials[0].roughness_factor, 0.72);
        assert!(model.materials[0].base_color_texture.is_some());
        assert!(model.materials[0].metallic_roughness_texture.is_some());
        assert!(model.materials[0].normal_texture.is_some());
        assert!(model.materials[0].occlusion_texture.is_some());
        assert_eq!(model.images.len(), 1);
        assert_eq!(model.meshes[0].primitives[0].indices, [0, 1, 2]);
        assert!(model.meshes[0].primitives[0].vertices[0].tangent[0].is_finite());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_glb_binary_buffer_and_scene() {
        let dir = fixture_dir();
        let buffer = triangle_buffer();
        let source = model_json(None, buffer.len(), None);
        let glb_path = dir.join("forest.glb");
        fs::write(&glb_path, make_glb(&source, &buffer)).unwrap();

        let model = import_gltf(&glb_path, ModelImportLimits::default()).unwrap();
        assert_eq!(model.meshes.len(), 1);
        assert_eq!(model.instances.len(), 1);
        assert_eq!(model.meshes[0].primitives[0].vertices.len(), 3);
        assert_eq!(model.meshes[0].primitives[0].indices, [0, 1, 2]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generates_missing_normals_and_rejects_index_budget_overflow() {
        let dir = fixture_dir();
        let buffer = triangle_buffer();
        let buffer_uri = format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(&buffer)
        );
        let source = model_json(Some(&buffer_uri), buffer.len(), None).replace("\"NORMAL\":1,", "");
        let path = dir.join("normal-missing.gltf");
        fs::write(&path, source).unwrap();

        let model = import_gltf(&path, ModelImportLimits::default()).unwrap();
        assert_eq!(
            model.meshes[0].primitives[0].vertices[0].normal,
            [0.0, 0.0, 1.0]
        );
        let error = import_gltf(
            &path,
            ModelImportLimits {
                max_indices: 2,
                ..ModelImportLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ModelImportError::LimitExceeded {
                kind: "source index count",
                ..
            }
        ));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_references_cannot_escape_the_model_directory() {
        let dir = fixture_dir();
        let error = read_uri("../outside.bin", &dir, 1024).unwrap_err();
        assert!(matches!(error, ModelImportError::InvalidUri));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn importer_rejects_source_exceeding_limits_before_parsing() {
        let dir = fixture_dir();
        let path = dir.join("oversized.gltf");
        fs::write(&path, vec![b' '; 128]).unwrap();
        let error = import_gltf(
            &path,
            ModelImportLimits {
                max_source_bytes: 16,
                ..ModelImportLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ModelImportError::LimitExceeded {
                kind: "model source bytes",
                ..
            }
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_standalone_png_texture_with_dimension_limits() {
        let dir = fixture_dir();
        let path = dir.join("surface.png");
        fs::write(&path, include_bytes!("../../../assets/forest_noise.png")).unwrap();
        let image = import_image(&path, ModelImportLimits::default()).unwrap();
        assert!(image.width > 0);
        assert!(image.height > 0);
        assert_eq!(
            image.rgba8.len(),
            image.width as usize * image.height as usize * 4
        );

        let error = import_image(
            &path,
            ModelImportLimits {
                max_image_dimension: 1,
                ..ModelImportLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, ModelImportError::Image(_)));
        fs::remove_dir_all(dir).unwrap();
    }
}
