//! CPU-side asset decoding and bounded caches. This crate deliberately has no Vulkan dependency.

mod async_loader;
mod environment;
mod gltf_importer;
mod model_cache;
mod texture;

pub use async_loader::{AsyncLoadError, AsyncModelLoader, ModelLoadEvent, ModelLoadId};
pub use environment::{
    CpuEnvironmentMap, EnvironmentImportError, EnvironmentMipLevel, import_environment_hdr,
};
pub use gltf_importer::{
    AlphaMode, CpuImageAsset, CpuMaterialAsset, CpuMeshAsset, CpuModelAsset, CpuModelInstance,
    CpuPrimitiveAsset, CpuVertex, ModelImportError, ModelImportLimits, TextureBinding, import_gltf,
    import_image,
};
pub use model_cache::{ModelAssetCache, ModelCacheError, ModelHandle};
pub use texture::{
    AssetError, AssetLimits, AssetManager, TextureAsset, TextureHandle, TextureMipLevel,
    TextureMipMode,
};
