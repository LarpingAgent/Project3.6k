//! Bounded, renderer-independent loading and caching for CPU-side texture assets.

use std::collections::HashMap;
use std::io::Cursor;

use png::{ColorType, Transformations};
use thiserror::Error;

/// Limits applied before decoded textures are retained by an [`AssetManager`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssetLimits {
    /// Maximum decoder output for one PNG before RGBA conversion.
    pub max_decoded_bytes: usize,
    /// Maximum CPU pixel bytes held by all cached mip chains.
    pub max_resident_bytes: usize,
    /// Maximum number of cached texture keys.
    pub max_assets: usize,
    /// Maximum UTF-8 byte length for an asset key.
    pub max_key_bytes: usize,
}

impl Default for AssetLimits {
    fn default() -> Self {
        Self {
            max_decoded_bytes: 16 * 1024 * 1024,
            max_resident_bytes: 64 * 1024 * 1024,
            max_assets: 1024,
            max_key_bytes: 256,
        }
    }
}

/// Opaque slot-and-generation identity for a cached texture.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextureHandle {
    slot: usize,
    generation: u64,
}

/// One RGBA8 level of a CPU-side texture mip chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextureMipLevel {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Packed row-major RGBA8 pixels.
    pub rgba8: Vec<u8>,
}

/// Decoded texture pixels and the complete CPU-generated mip chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextureAsset {
    /// Base-level width in texels.
    pub width: u32,
    /// Base-level height in texels.
    pub height: u32,
    /// Levels start at full resolution and end at 1×1.
    pub mip_levels: Vec<TextureMipLevel>,
    /// Total bytes retained by all levels.
    pub byte_size: usize,
}

/// Interpretation used while reducing mip texels for an imported glTF image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextureMipMode {
    /// Decode and average RGB in linear light, then encode back to sRGB.
    SrgbColor,
    /// Average channels directly for roughness, metallic, AO, and other data textures.
    LinearData,
    /// Average decoded tangent-space vectors and renormalize the resulting vector.
    NormalMap,
}

impl TextureAsset {
    /// Builds a complete mip chain from decoded RGBA8 pixels using the map's data role.
    pub fn from_rgba8(
        width: u32,
        height: u32,
        rgba8: &[u8],
        mip_mode: TextureMipMode,
    ) -> Result<Self, AssetError> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(AssetError::InvalidDimensions)?;
        if width == 0 || height == 0 || rgba8.len() != expected {
            return Err(AssetError::InvalidDimensions);
        }
        let mip_levels = generate_mip_chain(
            TextureMipLevel {
                width,
                height,
                rgba8: rgba8.to_vec(),
            },
            mip_mode,
        )?;
        let byte_size = mip_levels.iter().try_fold(0usize, |sum, level| {
            sum.checked_add(level.rgba8.len())
                .ok_or(AssetError::TextureSizeOverflow)
        })?;
        Ok(Self {
            width,
            height,
            mip_levels,
            byte_size,
        })
    }
}

/// Errors returned by PNG decoding and bounded asset caching.
#[derive(Debug, Error)]
pub enum AssetError {
    #[error("asset key must contain 1..={max_bytes} UTF-8 bytes")]
    InvalidKey { max_bytes: usize },
    #[error("PNG decoding failed: {0}")]
    PngDecode(#[from] png::DecodingError),
    #[error("decoded PNG exceeds the {limit}-byte limit")]
    DecodedImageTooLarge { limit: usize },
    #[error("PNG dimensions or decoded pixel size are invalid")]
    InvalidDimensions,
    #[error("texture mip-chain size overflows addressable memory")]
    TextureSizeOverflow,
    #[error("asset memory budget exceeded: {resident} + {requested} > {budget} bytes")]
    MemoryBudgetExceeded {
        resident: usize,
        requested: usize,
        budget: usize,
    },
    #[error("asset count limit reached ({limit})")]
    AssetCountExceeded { limit: usize },
}

#[derive(Debug, Default)]
struct TextureSlot {
    generation: u64,
    asset: Option<TextureAsset>,
    key: Option<String>,
}

/// Loads and caches PNG textures with explicit decoded-memory and asset-count budgets.
#[derive(Debug)]
pub struct AssetManager {
    limits: AssetLimits,
    slots: Vec<TextureSlot>,
    free_slots: Vec<usize>,
    by_key: HashMap<String, TextureHandle>,
    resident_bytes: usize,
}

impl Default for AssetManager {
    fn default() -> Self {
        Self::new(AssetLimits::default())
    }
}

impl AssetManager {
    /// Creates an empty manager with caller-selected decode and residency limits.
    pub fn new(limits: AssetLimits) -> Self {
        Self {
            limits,
            slots: Vec::new(),
            free_slots: Vec::new(),
            by_key: HashMap::new(),
            resident_bytes: 0,
        }
    }

    /// Decodes, mipmaps, and caches PNG bytes under `key`; repeated keys reuse the live handle.
    pub fn load_png(&mut self, key: &str, bytes: &[u8]) -> Result<TextureHandle, AssetError> {
        if key.is_empty() || key.len() > self.limits.max_key_bytes {
            return Err(AssetError::InvalidKey {
                max_bytes: self.limits.max_key_bytes,
            });
        }
        if let Some(handle) = self.by_key.get(key).copied() {
            if self.texture(handle).is_some() {
                return Ok(handle);
            }
            self.by_key.remove(key);
        }

        let base = decode_png(bytes, self.limits.max_decoded_bytes)?;
        let mip_levels = generate_mip_chain(base, TextureMipMode::SrgbColor)?;
        let byte_size = mip_levels.iter().try_fold(0usize, |sum, mip| {
            sum.checked_add(mip.rgba8.len())
                .ok_or(AssetError::TextureSizeOverflow)
        })?;
        let Some(new_resident_bytes) = self.resident_bytes.checked_add(byte_size) else {
            return Err(AssetError::TextureSizeOverflow);
        };
        if new_resident_bytes > self.limits.max_resident_bytes {
            return Err(AssetError::MemoryBudgetExceeded {
                resident: self.resident_bytes,
                requested: byte_size,
                budget: self.limits.max_resident_bytes,
            });
        }
        if self.free_slots.is_empty() && self.slots.len() >= self.limits.max_assets {
            return Err(AssetError::AssetCountExceeded {
                limit: self.limits.max_assets,
            });
        }

        let texture = TextureAsset {
            width: mip_levels[0].width,
            height: mip_levels[0].height,
            mip_levels,
            byte_size,
        };
        let slot_index = if let Some(slot) = self.free_slots.pop() {
            slot
        } else {
            self.slots.push(TextureSlot::default());
            self.slots.len() - 1
        };
        let handle = TextureHandle {
            slot: slot_index,
            generation: self.slots[slot_index].generation,
        };
        self.slots[slot_index].asset = Some(texture);
        self.slots[slot_index].key = Some(key.to_owned());
        self.by_key.insert(key.to_owned(), handle);
        self.resident_bytes = new_resident_bytes;
        Ok(handle)
    }

    /// Returns a cached texture only while the handle generation is current.
    pub fn texture(&self, handle: TextureHandle) -> Option<&TextureAsset> {
        let slot = self.slots.get(handle.slot)?;
        (slot.generation == handle.generation)
            .then_some(slot.asset.as_ref())
            .flatten()
    }

    /// Removes a cached texture and invalidates its handle. Returns false for a stale handle.
    pub fn unload(&mut self, handle: TextureHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot) else {
            return false;
        };
        if slot.generation != handle.generation {
            return false;
        }
        let Some(asset) = slot.asset.take() else {
            return false;
        };
        self.resident_bytes -= asset.byte_size;
        if let Some(key) = slot.key.take() {
            self.by_key.remove(&key);
        }
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            self.free_slots.push(handle.slot);
        }
        true
    }

    /// Number of live cached textures.
    pub fn asset_count(&self) -> usize {
        self.by_key.len()
    }

    /// CPU bytes retained by all live texture mip chains.
    pub fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }

    /// Returns the limits used by this manager.
    pub fn limits(&self) -> AssetLimits {
        self.limits
    }
}

fn decode_png(bytes: &[u8], max_decoded_bytes: usize) -> Result<TextureMipLevel, AssetError> {
    let limits = png::Limits {
        bytes: max_decoded_bytes,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let output_size = reader
        .output_buffer_size()
        .ok_or(AssetError::InvalidDimensions)?;
    if output_size > max_decoded_bytes {
        return Err(AssetError::DecodedImageTooLarge {
            limit: max_decoded_bytes,
        });
    }
    let mut decoded = vec![0; output_size];
    let info = reader.next_frame(&mut decoded)?;
    decoded.truncate(info.buffer_size());
    if info.width == 0 || info.height == 0 {
        return Err(AssetError::InvalidDimensions);
    }
    let rgba8 = to_rgba8(info.color_type, info.width, info.height, &decoded)?;
    Ok(TextureMipLevel {
        width: info.width,
        height: info.height,
        rgba8,
    })
}

fn to_rgba8(
    color_type: ColorType,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<Vec<u8>, AssetError> {
    let width = usize::try_from(width).map_err(|_| AssetError::InvalidDimensions)?;
    let height = usize::try_from(height).map_err(|_| AssetError::InvalidDimensions)?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or(AssetError::InvalidDimensions)?;
    let rgba_bytes = pixel_count
        .checked_mul(4)
        .ok_or(AssetError::InvalidDimensions)?;
    let mut rgba = Vec::with_capacity(rgba_bytes);
    match color_type {
        ColorType::Rgba => {
            if pixels.len() != rgba_bytes {
                return Err(AssetError::InvalidDimensions);
            }
            rgba.extend_from_slice(pixels);
        }
        ColorType::Rgb => {
            if pixels.len() != pixel_count * 3 {
                return Err(AssetError::InvalidDimensions);
            }
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        ColorType::Grayscale => {
            if pixels.len() != pixel_count {
                return Err(AssetError::InvalidDimensions);
            }
            for gray in pixels {
                rgba.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            if pixels.len() != pixel_count * 2 {
                return Err(AssetError::InvalidDimensions);
            }
            for pixel in pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        ColorType::Indexed => return Err(AssetError::InvalidDimensions),
    }
    Ok(rgba)
}

fn generate_mip_chain(
    base: TextureMipLevel,
    mode: TextureMipMode,
) -> Result<Vec<TextureMipLevel>, AssetError> {
    let mut levels = vec![base];
    while levels
        .last()
        .is_some_and(|level| level.width > 1 || level.height > 1)
    {
        let source = levels.last().expect("base mip is present");
        let width = source.width.div_ceil(2).max(1);
        let height = source.height.div_ceil(2).max(1);
        let byte_len = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(AssetError::TextureSizeOverflow)?;
        let mut rgba8 = vec![0; byte_len];
        for y in 0..height {
            for x in 0..width {
                let target_offset = ((y * width + x) * 4) as usize;
                if mode == TextureMipMode::NormalMap {
                    let mut normal_sum = glam::Vec3::ZERO;
                    let mut alpha_sum = 0.0f32;
                    for sample_y in 0..2 {
                        for sample_x in 0..2 {
                            let source_x = (x * 2 + sample_x).min(source.width - 1);
                            let source_y = (y * 2 + sample_y).min(source.height - 1);
                            let source_offset = ((source_y * source.width + source_x) * 4) as usize;
                            let normal = glam::Vec3::new(
                                source.rgba8[source_offset] as f32,
                                source.rgba8[source_offset + 1] as f32,
                                source.rgba8[source_offset + 2] as f32,
                            ) / 127.5
                                - glam::Vec3::ONE;
                            normal_sum += normal;
                            alpha_sum += source.rgba8[source_offset + 3] as f32 / 255.0;
                        }
                    }
                    let normal = normal_sum.try_normalize().unwrap_or(glam::Vec3::Z);
                    for channel in 0..3 {
                        let encoded = (normal[channel] + 1.0) * 0.5;
                        rgba8[target_offset + channel] =
                            (encoded * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                    }
                    rgba8[target_offset + 3] =
                        (alpha_sum * 0.25 * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                    continue;
                }
                for channel in 0..4usize {
                    let mut sum = 0.0f32;
                    for sample_y in 0..2 {
                        for sample_x in 0..2 {
                            let source_x = (x * 2 + sample_x).min(source.width - 1);
                            let source_y = (y * 2 + sample_y).min(source.height - 1);
                            let source_offset =
                                ((source_y * source.width + source_x) * 4) as usize + channel;
                            let value = source.rgba8[source_offset] as f32 / 255.0;
                            match mode {
                                TextureMipMode::SrgbColor => {
                                    sum += if channel < 3 {
                                        srgb_to_linear(value)
                                    } else {
                                        value
                                    };
                                }
                                TextureMipMode::LinearData => sum += value,
                                TextureMipMode::NormalMap => unreachable!(
                                    "normal maps were reduced as complete vectors above"
                                ),
                            }
                        }
                    }
                    let channel_offset = target_offset + channel;
                    let average = match mode {
                        TextureMipMode::SrgbColor => {
                            let linear = sum * 0.25;
                            if channel < 3 {
                                linear_to_srgb(linear)
                            } else {
                                linear
                            }
                        }
                        TextureMipMode::LinearData => sum * 0.25,
                        TextureMipMode::NormalMap => {
                            unreachable!("normal maps were reduced as complete vectors above")
                        }
                    };
                    rgba8[channel_offset] = (average * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                }
            }
        }
        levels.push(TextureMipLevel {
            width,
            height,
            rgba8,
        });
    }
    Ok(levels)
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssetError, AssetLimits, AssetManager, TextureMipLevel, TextureMipMode, generate_mip_chain,
    };

    const TEST_PNG: &[u8] = include_bytes!("../../../assets/forest_noise.png");

    #[test]
    fn png_assets_are_cached_by_key_and_report_resident_bytes() {
        let mut assets = AssetManager::default();
        let first = assets.load_png("forest/noise", TEST_PNG).unwrap();
        let bytes = assets.resident_bytes();
        let duplicate = assets
            .load_png("forest/noise", b"not decoded again")
            .unwrap();

        assert_eq!(first, duplicate);
        assert_eq!(assets.asset_count(), 1);
        assert_eq!(assets.resident_bytes(), bytes);
        assert_eq!(
            assets
                .texture(first)
                .unwrap()
                .mip_levels
                .last()
                .unwrap()
                .width,
            1
        );
    }

    #[test]
    fn unloading_recycles_slots_without_revalidating_stale_handles() {
        let mut assets = AssetManager::default();
        let old = assets.load_png("forest/noise", TEST_PNG).unwrap();
        let old_bytes = assets.resident_bytes();
        assert!(assets.unload(old));
        assert!(assets.texture(old).is_none());
        assert_eq!(assets.resident_bytes(), 0);

        let new = assets.load_png("forest/noise", TEST_PNG).unwrap();
        assert_eq!(old.slot, new.slot);
        assert_ne!(old.generation, new.generation);
        assert!(!assets.unload(old));
        assert_eq!(assets.resident_bytes(), old_bytes);
    }

    #[test]
    fn memory_and_asset_count_limits_are_enforced() {
        let limits = AssetLimits {
            max_resident_bytes: 4,
            max_assets: 1,
            ..AssetLimits::default()
        };
        let mut assets = AssetManager::new(limits);
        assert!(matches!(
            assets.load_png("forest/noise", TEST_PNG),
            Err(AssetError::MemoryBudgetExceeded { .. })
        ));
        assert_eq!(assets.resident_bytes(), 0);
        assert!(matches!(
            assets.load_png("", TEST_PNG),
            Err(AssetError::InvalidKey { .. })
        ));

        let mut assets = AssetManager::new(AssetLimits {
            max_assets: 1,
            ..AssetLimits::default()
        });
        assets.load_png("forest/noise", TEST_PNG).unwrap();
        assert!(matches!(
            assets.load_png("forest/second", TEST_PNG),
            Err(AssetError::AssetCountExceeded { limit: 1 })
        ));
    }

    #[test]
    fn mip_chain_averages_color_in_linear_light_and_preserves_odd_edges() {
        let base = TextureMipLevel {
            width: 3,
            height: 3,
            rgba8: vec![
                0, 0, 0, 255, 10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255, 40, 40, 40, 255,
                50, 50, 50, 255, 60, 60, 60, 255, 70, 70, 70, 255, 80, 80, 80, 255,
            ],
        };
        let levels = generate_mip_chain(base, TextureMipMode::SrgbColor).unwrap();
        assert_eq!((levels[1].width, levels[1].height), (2, 2));
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
        assert!(levels[1].rgba8[12] > 60);
    }

    #[test]
    fn linear_and_normal_maps_use_role_aware_mip_reduction() {
        let pixels = [
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let linear =
            super::TextureAsset::from_rgba8(2, 2, &pixels, TextureMipMode::LinearData).unwrap();
        assert_eq!(linear.mip_levels[1].rgba8, [128, 128, 128, 255]);

        let normals = [
            255, 128, 128, 255, 255, 128, 128, 255, 255, 128, 128, 255, 255, 128, 128, 255,
        ];
        let normal =
            super::TextureAsset::from_rgba8(2, 2, &normals, TextureMipMode::NormalMap).unwrap();
        assert_eq!(normal.mip_levels[1].rgba8, [255, 128, 128, 255]);
        assert!(matches!(
            super::TextureAsset::from_rgba8(2, 2, &[0; 3], TextureMipMode::SrgbColor),
            Err(AssetError::InvalidDimensions)
        ));
    }
}
