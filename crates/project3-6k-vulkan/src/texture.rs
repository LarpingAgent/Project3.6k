use std::io::Cursor;

use ash::vk;
use png::{ColorType, Transformations};

use super::{Result, VulkanRendererError};

pub(super) const FOREST_TEXTURE_PNG: &[u8] = include_bytes!("../assets/forest_noise.png");
const MAX_DECODE_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn choose_texture_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format> {
    let format = vk::Format::R8G8B8A8_SRGB;
    // SAFETY: The physical device belongs to the live instance.
    let properties =
        unsafe { instance.get_physical_device_format_properties(physical_device, format) };
    let required =
        vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
    if properties.optimal_tiling_features.contains(required) {
        Ok(format)
    } else {
        Err(VulkanRendererError::NoSupportedTextureFormat)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DecodedTexture {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba8: Vec<u8>,
}

pub(super) fn decode_png(bytes: &[u8]) -> Result<DecodedTexture> {
    let limits = png::Limits {
        bytes: MAX_DECODE_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(VulkanRendererError::TextureDecode)?;
    let output_size = reader
        .output_buffer_size()
        .ok_or(VulkanRendererError::InvalidTexture(
            "decoded image size overflows",
        ))?;
    if output_size > MAX_DECODE_BYTES {
        return Err(VulkanRendererError::InvalidTexture(
            "decoded image exceeds the 16 MiB limit",
        ));
    }
    let mut decoded = vec![0; output_size];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(VulkanRendererError::TextureDecode)?;
    decoded.truncate(info.buffer_size());
    let rgba8 = to_rgba8(info.color_type, info.width, info.height, &decoded)?;
    Ok(DecodedTexture {
        width: info.width,
        height: info.height,
        rgba8,
    })
}

fn to_rgba8(color_type: ColorType, width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let pixel_count = (width as usize).checked_mul(height as usize).ok_or(
        VulkanRendererError::InvalidTexture("image dimensions overflow"),
    )?;
    let rgba_bytes = pixel_count
        .checked_mul(4)
        .ok_or(VulkanRendererError::InvalidTexture(
            "RGBA image size overflows",
        ))?;
    let mut rgba = Vec::with_capacity(rgba_bytes);
    match color_type {
        ColorType::Rgba => {
            if pixels.len() != rgba_bytes {
                return Err(VulkanRendererError::InvalidTexture(
                    "decoded RGBA byte count does not match dimensions",
                ));
            }
            rgba.extend_from_slice(pixels);
        }
        ColorType::Rgb => {
            if pixels.len() != pixel_count * 3 {
                return Err(VulkanRendererError::InvalidTexture(
                    "decoded RGB byte count does not match dimensions",
                ));
            }
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        ColorType::Grayscale => {
            if pixels.len() != pixel_count {
                return Err(VulkanRendererError::InvalidTexture(
                    "decoded grayscale byte count does not match dimensions",
                ));
            }
            for gray in pixels {
                rgba.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            if pixels.len() != pixel_count * 2 {
                return Err(VulkanRendererError::InvalidTexture(
                    "decoded grayscale-alpha byte count does not match dimensions",
                ));
            }
            for pixel in pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        ColorType::Indexed => {
            return Err(VulkanRendererError::InvalidTexture(
                "palette image was not expanded by the PNG decoder",
            ));
        }
    }
    Ok(rgba)
}

pub(super) fn generate_mip_chain(base: &DecodedTexture) -> Vec<DecodedTexture> {
    let mut levels = vec![base.clone()];
    while levels
        .last()
        .is_some_and(|level| level.width > 1 || level.height > 1)
    {
        let source = levels.last().expect("the base level is always present");
        let width = source.width.div_ceil(2).max(1);
        let height = source.height.div_ceil(2).max(1);
        let mut rgba8 = vec![0; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                for channel in 0..4 {
                    let mut sum = 0.0f32;
                    for sample_y in 0..2 {
                        for sample_x in 0..2 {
                            let source_x = (x * 2 + sample_x).min(source.width - 1);
                            let source_y = (y * 2 + sample_y).min(source.height - 1);
                            let source_offset =
                                ((source_y * source.width + source_x) * 4 + channel) as usize;
                            let value = source.rgba8[source_offset] as f32 / 255.0;
                            sum += if channel < 3 {
                                srgb_to_linear(value)
                            } else {
                                value
                            };
                        }
                    }
                    let target_offset = ((y * width + x) * 4 + channel) as usize;
                    let average = sum * 0.25;
                    let encoded = if channel < 3 {
                        linear_to_srgb(average)
                    } else {
                        average
                    };
                    rgba8[target_offset] = (encoded * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                }
            }
        }
        levels.push(DecodedTexture {
            width,
            height,
            rgba8,
        });
    }
    levels
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
    use super::{DecodedTexture, FOREST_TEXTURE_PNG, decode_png, generate_mip_chain};

    #[test]
    fn bundled_texture_decodes_to_bounded_rgba8_data() {
        let texture = decode_png(FOREST_TEXTURE_PNG).unwrap();
        assert!(texture.width > 1);
        assert!(texture.height > 1);
        assert_eq!(
            texture.rgba8.len(),
            texture.width as usize * texture.height as usize * 4
        );
    }

    #[test]
    fn mip_chain_reaches_one_texel_and_averages_all_channels() {
        let texture = DecodedTexture {
            width: 2,
            height: 2,
            rgba8: vec![
                0, 10, 20, 30, 100, 110, 120, 130, 200, 210, 220, 230, 255, 245, 235, 225,
            ],
        };
        let levels = generate_mip_chain(&texture);

        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1].width, 1);
        assert_eq!(levels[1].height, 1);
        assert!(levels[1].rgba8[0] > 139);
        assert!(levels[1].rgba8[1] > 144);
        assert!(levels[1].rgba8[2] > 149);
        assert_eq!(levels[1].rgba8[3], 154);
    }

    #[test]
    fn one_pixel_texture_does_not_generate_empty_mip_levels() {
        let texture = DecodedTexture {
            width: 1,
            height: 1,
            rgba8: vec![64, 128, 192, 255],
        };
        assert_eq!(generate_mip_chain(&texture), vec![texture]);
    }

    #[test]
    fn odd_sized_texture_keeps_the_last_row_and_column_in_mips() {
        let texture = DecodedTexture {
            width: 3,
            height: 3,
            rgba8: vec![
                0, 0, 0, 255, 10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255, 40, 40, 40, 255,
                50, 50, 50, 255, 60, 60, 60, 255, 70, 70, 70, 255, 80, 80, 80, 255,
            ],
        };

        let levels = generate_mip_chain(&texture);

        assert_eq!((levels[1].width, levels[1].height), (2, 2));
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
        assert!(levels[1].rgba8[12] > 60);
    }
}
