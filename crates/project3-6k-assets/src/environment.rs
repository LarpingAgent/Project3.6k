//! CPU-side HDR environment import and bounded mip generation.

use std::{
    fs,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::ModelImportLimits;

const SPECULAR_BASE_WIDTH: u32 = 256;
const SPECULAR_BASE_HEIGHT: u32 = 128;
const SPECULAR_SAMPLE_COUNT: u32 = 64;
const BRDF_LUT_WIDTH: u32 = 128;
const BRDF_LUT_HEIGHT: u32 = 128;
const BRDF_SAMPLE_COUNT: u32 = 128;

/// One linear-light RGBA32F level of an equirectangular environment texture.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentMipLevel {
    /// Pixel width at this level.
    pub width: u32,
    /// Pixel height at this level.
    pub height: u32,
    /// Row-major linear radiance with alpha fixed to one.
    pub rgba32f: Vec<[f32; 4]>,
}

/// Fully decoded CPU environment data. This type has no Vulkan dependency.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuEnvironmentMap {
    /// Base-level pixel width.
    pub width: u32,
    /// Base-level pixel height.
    pub height: u32,
    /// Base level followed by linearly averaged levels down to 1x1.
    pub mip_levels: Vec<EnvironmentMipLevel>,
    /// GGX-prefiltered specular radiance, with mip level encoding roughness.
    pub specular_mip_levels: Vec<EnvironmentMipLevel>,
    /// Split-sum BRDF integration; red is the scale term and green is the bias term.
    pub brdf_lut: EnvironmentMipLevel,
    /// Lambertian diffuse irradiance in real spherical harmonics, normalized by PI.
    pub irradiance_sh: [[f32; 4]; 9],
    /// Retained CPU bytes across the sky, specular, and BRDF lookup data.
    byte_size: usize,
}

impl CpuEnvironmentMap {
    /// Returns retained CPU bytes across all environment rendering data.
    pub fn byte_size(&self) -> usize {
        self.byte_size
    }
}

/// Imports an equirectangular HDR panorama as bounded linear-light RGBA32F data.
pub fn import_environment_hdr(
    path: impl AsRef<Path>,
    limits: ModelImportLimits,
) -> Result<CpuEnvironmentMap, EnvironmentImportError> {
    let requested_path = path.as_ref();
    let path = requested_path
        .canonicalize()
        .map_err(|source| EnvironmentImportError::Io {
            path: requested_path.to_owned(),
            source,
        })?;
    let file_len = fs::metadata(&path)
        .map_err(|source| EnvironmentImportError::Io {
            path: path.clone(),
            source,
        })?
        .len();
    check_limit(
        "environment source bytes",
        usize::try_from(file_len).unwrap_or(usize::MAX),
        limits.max_external_file_bytes,
    )?;
    let encoded = fs::read(&path).map_err(|source| EnvironmentImportError::Io {
        path: path.clone(),
        source,
    })?;
    check_limit(
        "environment source bytes",
        encoded.len(),
        limits.max_external_file_bytes,
    )?;

    let mut reader = image::ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(|source| EnvironmentImportError::Io {
            path: path.clone(),
            source,
        })?;
    if reader.format() != Some(image::ImageFormat::Hdr) {
        return Err(EnvironmentImportError::UnsupportedFormat);
    }
    let mut decode_limits = image::Limits::default();
    decode_limits.max_image_width = Some(limits.max_image_dimension);
    decode_limits.max_image_height = Some(limits.max_image_dimension);
    decode_limits.max_alloc = Some(limits.max_total_cpu_bytes as u64);
    reader.limits(decode_limits);
    let decoded = catch_unwind(AssertUnwindSafe(|| reader.decode()))
        .map_err(|_| EnvironmentImportError::DecoderPanicked)??;
    let width = decoded.width();
    let height = decoded.height();
    if width == 0
        || height == 0
        || width > limits.max_image_dimension
        || height > limits.max_image_dimension
    {
        return Err(EnvironmentImportError::InvalidDimensions { width, height });
    }

    let pixel_count = checked_pixel_count(width, height)?;
    let decoded_bytes = pixel_count
        .checked_mul(3 * size_of::<f32>())
        .ok_or(EnvironmentImportError::SizeOverflow)?;
    check_limit(
        "decoded environment bytes",
        decoded_bytes,
        limits.max_total_cpu_bytes,
    )?;
    let decoded = decoded.into_rgb32f();
    if decoded.as_raw().len() != pixel_count * 3 {
        return Err(EnvironmentImportError::InvalidPixels);
    }
    let irradiance_sh = integrate_irradiance_sh(&decoded);

    let mut mip_levels = Vec::new();
    let mut base = Vec::with_capacity(pixel_count);
    for rgb in decoded.pixels() {
        base.push([
            sanitize(rgb.0[0]),
            sanitize(rgb.0[1]),
            sanitize(rgb.0[2]),
            1.0,
        ]);
    }
    if base.len() != pixel_count {
        return Err(EnvironmentImportError::InvalidPixels);
    }
    let mut total_bytes = base
        .len()
        .checked_mul(size_of::<[f32; 4]>())
        .ok_or(EnvironmentImportError::SizeOverflow)?;
    check_limit(
        "environment mip bytes",
        total_bytes
            .checked_add(decoded_bytes)
            .ok_or(EnvironmentImportError::SizeOverflow)?,
        limits.max_total_cpu_bytes,
    )?;
    mip_levels.push(EnvironmentMipLevel {
        width,
        height,
        rgba32f: base,
    });

    while mip_levels
        .last()
        .is_some_and(|level| level.width > 1 || level.height > 1)
    {
        let source = mip_levels.last().expect("base environment mip exists");
        let next_width = (source.width / 2).max(1);
        let next_height = (source.height / 2).max(1);
        let next_pixels = checked_pixel_count(next_width, next_height)?;
        let next_bytes = next_pixels
            .checked_mul(size_of::<[f32; 4]>())
            .ok_or(EnvironmentImportError::SizeOverflow)?;
        total_bytes = total_bytes
            .checked_add(next_bytes)
            .ok_or(EnvironmentImportError::SizeOverflow)?;
        check_limit(
            "environment mip bytes",
            total_bytes
                .checked_add(decoded_bytes)
                .ok_or(EnvironmentImportError::SizeOverflow)?,
            limits.max_total_cpu_bytes,
        )?;

        let mut pixels = Vec::with_capacity(next_pixels);
        for y in 0..next_height {
            for x in 0..next_width {
                let x0 = (x * 2).min(source.width - 1);
                let x1 = (x0 + 1).min(source.width - 1);
                let y0 = (y * 2).min(source.height - 1);
                let y1 = (y0 + 1).min(source.height - 1);
                let taps = [
                    source.rgba32f[(y0 * source.width + x0) as usize],
                    source.rgba32f[(y0 * source.width + x1) as usize],
                    source.rgba32f[(y1 * source.width + x0) as usize],
                    source.rgba32f[(y1 * source.width + x1) as usize],
                ];
                pixels.push([
                    (taps[0][0] + taps[1][0] + taps[2][0] + taps[3][0]) * 0.25,
                    (taps[0][1] + taps[1][1] + taps[2][1] + taps[3][1]) * 0.25,
                    (taps[0][2] + taps[1][2] + taps[2][2] + taps[3][2]) * 0.25,
                    1.0,
                ]);
            }
        }
        mip_levels.push(EnvironmentMipLevel {
            width: next_width,
            height: next_height,
            rgba32f: pixels,
        });
    }

    let specular_base = mip_levels
        .iter()
        .find(|level| level.width <= SPECULAR_BASE_WIDTH && level.height <= SPECULAR_BASE_HEIGHT)
        .or_else(|| mip_levels.last())
        .expect("environment mip chain contains a base level");
    let specular_base_bytes = specular_base
        .rgba32f
        .len()
        .checked_mul(size_of::<[f32; 4]>())
        .ok_or(EnvironmentImportError::SizeOverflow)?;
    let mut retained_bytes = total_bytes
        .checked_add(specular_base_bytes)
        .ok_or(EnvironmentImportError::SizeOverflow)?;
    check_limit(
        "environment lighting bytes",
        retained_bytes
            .checked_add(decoded_bytes)
            .ok_or(EnvironmentImportError::SizeOverflow)?,
        limits.max_total_cpu_bytes,
    )?;
    let mut specular_mip_levels = vec![specular_base.clone()];
    while specular_mip_levels
        .last()
        .is_some_and(|level| level.width > 1 || level.height > 1)
    {
        let source = specular_mip_levels
            .last()
            .expect("specular base mip exists");
        let width = (source.width / 2).max(1);
        let height = (source.height / 2).max(1);
        let pixel_count = checked_pixel_count(width, height)?;
        let level_bytes = pixel_count
            .checked_mul(size_of::<[f32; 4]>())
            .ok_or(EnvironmentImportError::SizeOverflow)?;
        retained_bytes = retained_bytes
            .checked_add(level_bytes)
            .ok_or(EnvironmentImportError::SizeOverflow)?;
        check_limit(
            "environment lighting bytes",
            retained_bytes
                .checked_add(decoded_bytes)
                .ok_or(EnvironmentImportError::SizeOverflow)?,
            limits.max_total_cpu_bytes,
        )?;
        let roughness = specular_mip_levels.len() as f32
            / ((specular_base.width.max(specular_base.height).ilog2()) as f32);
        let pixels = prefilter_specular_level(
            &mip_levels,
            width,
            height,
            roughness.clamp(0.0, 1.0),
            SPECULAR_SAMPLE_COUNT,
        );
        specular_mip_levels.push(EnvironmentMipLevel {
            width,
            height,
            rgba32f: pixels,
        });
    }
    let brdf_pixel_count = checked_pixel_count(BRDF_LUT_WIDTH, BRDF_LUT_HEIGHT)?;
    let brdf_bytes = brdf_pixel_count
        .checked_mul(size_of::<[f32; 4]>())
        .ok_or(EnvironmentImportError::SizeOverflow)?;
    retained_bytes = retained_bytes
        .checked_add(brdf_bytes)
        .ok_or(EnvironmentImportError::SizeOverflow)?;
    check_limit(
        "environment lighting bytes",
        retained_bytes
            .checked_add(decoded_bytes)
            .ok_or(EnvironmentImportError::SizeOverflow)?,
        limits.max_total_cpu_bytes,
    )?;
    let brdf_lut = integrate_brdf_lut(BRDF_LUT_WIDTH, BRDF_LUT_HEIGHT, BRDF_SAMPLE_COUNT);

    Ok(CpuEnvironmentMap {
        width,
        height,
        mip_levels,
        specular_mip_levels,
        brdf_lut,
        irradiance_sh,
        byte_size: retained_bytes,
    })
}

fn prefilter_specular_level(
    environment_mips: &[EnvironmentMipLevel],
    width: u32,
    height: u32,
    roughness: f32,
    sample_count: u32,
) -> Vec<[f32; 4]> {
    let samples = hammersley_samples(sample_count);
    let alpha = roughness.max(0.001).powi(2);
    let alpha_squared = alpha * alpha;
    let source_texel_solid_angle = 4.0 * std::f32::consts::PI
        / (environment_mips[0].width * environment_mips[0].height) as f32;
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let theta = std::f32::consts::PI * (y as f32 + 0.5) / height as f32;
        for x in 0..width {
            let phi = std::f32::consts::TAU * ((x as f32 + 0.5) / width as f32 - 0.5);
            let normal = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            let up = if normal[1].abs() < 0.999 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let tangent = normalize3(cross3(up, normal));
            let bitangent = cross3(normal, tangent);
            let mut sum = [0.0; 3];
            let mut weight_sum = 0.0;
            for sample in &samples {
                let sample_phi = std::f32::consts::TAU * sample[0];
                let cos_theta =
                    ((1.0 - sample[1]) / (1.0 + (alpha_squared - 1.0) * sample[1])).sqrt();
                let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
                let half_tangent = [
                    sin_theta * sample_phi.cos(),
                    sin_theta * sample_phi.sin(),
                    cos_theta,
                ];
                let half_vector = normalize3(add3(
                    add3(
                        scale3(tangent, half_tangent[0]),
                        scale3(bitangent, half_tangent[1]),
                    ),
                    scale3(normal, half_tangent[2]),
                ));
                let n_dot_h = dot3(normal, half_vector).max(0.0);
                let view_dot_h = n_dot_h;
                let light = normalize3(sub3(scale3(half_vector, 2.0 * view_dot_h), normal));
                let n_dot_l = dot3(normal, light).max(0.0);
                if n_dot_l <= 0.0 || view_dot_h <= 0.000001 {
                    continue;
                }
                let denominator = n_dot_h * n_dot_h * (alpha_squared - 1.0) + 1.0;
                let distribution = alpha_squared
                    / (std::f32::consts::PI * denominator * denominator).max(0.000001);
                let pdf = (distribution * n_dot_h / (4.0 * view_dot_h)).max(0.000001);
                let sample_solid_angle = 1.0 / (sample_count as f32 * pdf);
                let source_lod = (0.5 * (sample_solid_angle / source_texel_solid_angle).log2())
                    .clamp(0.0, (environment_mips.len() - 1) as f32);
                let radiance = sample_environment_lod(environment_mips, light, source_lod);
                for channel in 0..3 {
                    sum[channel] += radiance[channel] * n_dot_l;
                }
                weight_sum += n_dot_l;
            }
            let inverse_weight = 1.0 / weight_sum.max(0.000001);
            pixels.push([
                sum[0] * inverse_weight,
                sum[1] * inverse_weight,
                sum[2] * inverse_weight,
                1.0,
            ]);
        }
    }
    pixels
}

fn integrate_brdf_lut(width: u32, height: u32, sample_count: u32) -> EnvironmentMipLevel {
    let samples = hammersley_samples(sample_count);
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let roughness = (y as f32 + 0.5) / height as f32;
        let alpha = roughness * roughness;
        let alpha_squared = alpha * alpha;
        let k = roughness * roughness * 0.5;
        for x in 0..width {
            let n_dot_v = (x as f32 + 0.5) / width as f32;
            let view = [(1.0 - n_dot_v * n_dot_v).sqrt(), 0.0, n_dot_v];
            let mut scale = 0.0;
            let mut bias = 0.0;
            for sample in &samples {
                let phi = std::f32::consts::TAU * sample[0];
                let cos_theta =
                    ((1.0 - sample[1]) / (1.0 + (alpha_squared - 1.0) * sample[1])).sqrt();
                let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
                let half_vector = [sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta];
                let view_dot_h = dot3(view, half_vector).max(0.0);
                let light = normalize3(sub3(scale3(half_vector, 2.0 * view_dot_h), view));
                let n_dot_l = light[2].max(0.0);
                let n_dot_h = half_vector[2].max(0.0);
                if n_dot_l <= 0.0 || n_dot_h <= 0.0 || view_dot_h <= 0.0 {
                    continue;
                }
                let geometry_view = n_dot_v / (n_dot_v * (1.0 - k) + k).max(0.000001);
                let geometry_light = n_dot_l / (n_dot_l * (1.0 - k) + k).max(0.000001);
                let visibility =
                    geometry_view * geometry_light * view_dot_h / (n_dot_h * n_dot_v).max(0.000001);
                let fresnel = (1.0 - view_dot_h).clamp(0.0, 1.0).powi(5);
                scale += (1.0 - fresnel) * visibility;
                bias += fresnel * visibility;
            }
            pixels.push([
                scale / sample_count as f32,
                bias / sample_count as f32,
                0.0,
                1.0,
            ]);
        }
    }
    EnvironmentMipLevel {
        width,
        height,
        rgba32f: pixels,
    }
}

fn hammersley_samples(sample_count: u32) -> Vec<[f32; 2]> {
    (0..sample_count)
        .map(|index| {
            [
                index as f32 / sample_count as f32,
                index.reverse_bits() as f32 * 2.328_306_4e-10,
            ]
        })
        .collect()
}

fn sample_environment_lod(
    levels: &[EnvironmentMipLevel],
    direction: [f32; 3],
    lod: f32,
) -> [f32; 3] {
    let low = lod.floor() as usize;
    let high = (low + 1).min(levels.len() - 1);
    let blend = lod.fract();
    let a = sample_environment(&levels[low], direction);
    let b = sample_environment(&levels[high], direction);
    std::array::from_fn(|channel| a[channel] + (b[channel] - a[channel]) * blend)
}

fn sample_environment(level: &EnvironmentMipLevel, direction: [f32; 3]) -> [f32; 3] {
    let direction = normalize3(direction);
    let u = direction[2].atan2(direction[0]) / std::f32::consts::TAU + 0.5;
    let v = direction[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    let x = u * level.width as f32 - 0.5;
    let y = v * level.height as f32 - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = x - x.floor();
    let ty = y - y.floor();
    let x_indices = [
        x0.rem_euclid(level.width as i32),
        (x0 + 1).rem_euclid(level.width as i32),
    ];
    let y_indices = [
        y0.clamp(0, level.height as i32 - 1),
        (y0 + 1).clamp(0, level.height as i32 - 1),
    ];
    let taps = [
        level.rgba32f[(y_indices[0] * level.width as i32 + x_indices[0]) as usize],
        level.rgba32f[(y_indices[0] * level.width as i32 + x_indices[1]) as usize],
        level.rgba32f[(y_indices[1] * level.width as i32 + x_indices[0]) as usize],
        level.rgba32f[(y_indices[1] * level.width as i32 + x_indices[1]) as usize],
    ];
    std::array::from_fn(|channel| {
        let top = taps[0][channel] + (taps[1][channel] - taps[0][channel]) * tx;
        let bottom = taps[2][channel] + (taps[3][channel] - taps[2][channel]) * tx;
        top + (bottom - top) * ty
    })
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(vector: [f32; 3], scale: f32) -> [f32; 3] {
    [vector[0] * scale, vector[1] * scale, vector[2] * scale]
}

fn normalize3(vector: [f32; 3]) -> [f32; 3] {
    let inverse_length = dot3(vector, vector).max(0.000001).sqrt().recip();
    scale3(vector, inverse_length)
}

#[derive(Debug, Error)]
pub enum EnvironmentImportError {
    #[error("failed to read HDR environment {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("HDR environment decoding failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("HDR decoder panicked while processing source data")]
    DecoderPanicked,
    #[error("environment source is not a Radiance HDR image")]
    UnsupportedFormat,
    #[error("resource limit exceeded for {kind}: {actual} > {limit}")]
    LimitExceeded {
        kind: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("invalid HDR environment dimensions {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("HDR environment pixel data has an unexpected size")]
    InvalidPixels,
    #[error("HDR environment dimensions overflow addressable memory")]
    SizeOverflow,
}

fn check_limit(
    kind: &'static str,
    actual: usize,
    limit: usize,
) -> Result<(), EnvironmentImportError> {
    if actual > limit {
        return Err(EnvironmentImportError::LimitExceeded {
            kind,
            actual,
            limit,
        });
    }
    Ok(())
}

fn checked_pixel_count(width: u32, height: u32) -> Result<usize, EnvironmentImportError> {
    usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .ok_or(EnvironmentImportError::SizeOverflow)
}

fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn integrate_irradiance_sh(image: &image::Rgb32FImage) -> [[f32; 4]; 9] {
    let mut coefficients = [[0.0_f32; 3]; 9];
    let width = image.width();
    let height = image.height();
    let delta_phi = std::f32::consts::TAU / width as f32;
    for y in 0..height {
        let theta0 = std::f32::consts::PI * y as f32 / height as f32;
        let theta1 = std::f32::consts::PI * (y + 1) as f32 / height as f32;
        let theta = (theta0 + theta1) * 0.5;
        let row_solid_angle = delta_phi * (theta0.cos() - theta1.cos());
        let sin_theta = theta.sin();
        for x in 0..width {
            let phi = std::f32::consts::TAU * ((x as f32 + 0.5) / width as f32 - 0.5);
            let direction = [sin_theta * phi.cos(), theta.cos(), sin_theta * phi.sin()];
            let [nx, ny, nz] = direction;
            let basis = [
                0.282_095,
                0.488_603 * ny,
                0.488_603 * nz,
                0.488_603 * nx,
                1.092_548 * nx * ny,
                1.092_548 * ny * nz,
                0.315_392 * (3.0 * nz * nz - 1.0),
                1.092_548 * nx * nz,
                0.546_274 * (nx * nx - ny * ny),
            ];
            let pixel = image.get_pixel(x, y).0.map(sanitize);
            let weight = row_solid_angle;
            for index in 0..9 {
                let factor = basis[index] * weight;
                for channel in 0..3 {
                    coefficients[index][channel] += pixel[channel] * factor;
                }
            }
        }
    }

    let band_convolution = [
        1.0,
        2.0 / 3.0,
        2.0 / 3.0,
        2.0 / 3.0,
        0.25,
        0.25,
        0.25,
        0.25,
        0.25,
    ];
    std::array::from_fn(|index| {
        let factor = band_convolution[index];
        [
            coefficients[index][0] * factor,
            coefficients[index][1] * factor,
            coefficients[index][2] * factor,
            0.0,
        ]
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn hdri_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/reference_scene/polyhaven/forest_slope/forest_slope_2k.hdr")
    }

    #[test]
    fn imports_linear_hdr_environment_and_generates_mip_chain() {
        let environment = import_environment_hdr(hdri_path(), ModelImportLimits::default())
            .expect("reference HDRI imports");
        assert_eq!((environment.width, environment.height), (2048, 1024));
        assert_eq!(
            environment
                .mip_levels
                .last()
                .map(|level| (level.width, level.height)),
            Some((1, 1))
        );
        assert!(environment.mip_levels.len() > 10);
        assert!(
            environment
                .irradiance_sh
                .iter()
                .flatten()
                .all(|coefficient| coefficient.is_finite())
        );
        assert!(
            environment
                .mip_levels
                .iter()
                .flat_map(|level| &level.rgba32f)
                .any(|pixel| pixel[0] > 1.0 || pixel[1] > 1.0 || pixel[2] > 1.0)
        );
        let retained_bytes = environment
            .mip_levels
            .iter()
            .chain(&environment.specular_mip_levels)
            .map(|level| level.rgba32f.len() * 16)
            .sum::<usize>()
            + environment.brdf_lut.rgba32f.len() * 16;
        assert_eq!(environment.byte_size(), retained_bytes);
        assert_eq!(
            environment
                .specular_mip_levels
                .first()
                .map(|level| (level.width, level.height)),
            Some((256, 128))
        );
        assert_eq!(
            environment
                .specular_mip_levels
                .last()
                .map(|level| (level.width, level.height)),
            Some((1, 1))
        );
        assert_eq!(
            (environment.brdf_lut.width, environment.brdf_lut.height),
            (128, 128)
        );
        assert!(
            environment
                .specular_mip_levels
                .iter()
                .flat_map(|level| &level.rgba32f)
                .chain(&environment.brdf_lut.rgba32f)
                .flatten()
                .all(|value| value.is_finite())
        );
    }

    #[test]
    fn rejects_environment_above_external_file_limit() {
        let limits = ModelImportLimits {
            max_external_file_bytes: 128,
            ..ModelImportLimits::default()
        };
        assert!(matches!(
            import_environment_hdr(hdri_path(), limits),
            Err(EnvironmentImportError::LimitExceeded {
                kind: "environment source bytes",
                ..
            })
        ));
    }

    #[test]
    fn constant_environment_produces_constant_lambertian_irradiance() {
        let image = image::Rgb32FImage::from_pixel(64, 32, image::Rgb([0.75, 0.5, 0.25]));
        let coefficients = integrate_irradiance_sh(&image);
        let constant_basis = 0.282_095;
        assert!((coefficients[0][0] * constant_basis - 0.75).abs() < 0.001);
        assert!((coefficients[0][1] * constant_basis - 0.5).abs() < 0.001);
        assert!((coefficients[0][2] * constant_basis - 0.25).abs() < 0.001);
        assert!(
            coefficients[1..]
                .iter()
                .flatten()
                .all(|coefficient| coefficient.abs() < 0.002)
        );
    }

    #[test]
    fn ggx_prefilter_preserves_a_constant_environment() {
        let pixels = vec![[0.75, 0.5, 0.25, 1.0]; 32 * 16];
        let environment = [EnvironmentMipLevel {
            width: 32,
            height: 16,
            rgba32f: pixels,
        }];
        let filtered = prefilter_specular_level(&environment, 8, 4, 0.6, 32);
        assert!(filtered.iter().all(|pixel| {
            (pixel[0] - 0.75).abs() < 0.0001
                && (pixel[1] - 0.5).abs() < 0.0001
                && (pixel[2] - 0.25).abs() < 0.0001
        }));
    }

    #[test]
    fn brdf_lut_contains_finite_bounded_split_sum_terms() {
        let lut = integrate_brdf_lut(16, 16, 64);
        assert!(lut.rgba32f.iter().all(|pixel| {
            pixel[0].is_finite()
                && pixel[1].is_finite()
                && (0.0..=1.0).contains(&pixel[0])
                && (0.0..=1.0).contains(&pixel[1])
        }));
    }
}
