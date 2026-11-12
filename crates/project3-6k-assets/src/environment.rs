//! CPU-side HDR environment import and bounded mip generation.

use std::{
    fs,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::ModelImportLimits;

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
    /// Lambertian diffuse irradiance in real spherical harmonics, normalized by PI.
    pub irradiance_sh: [[f32; 4]; 9],
    /// Retained CPU bytes across all mip levels.
    byte_size: usize,
}

impl CpuEnvironmentMap {
    /// Returns retained CPU bytes across all mip levels.
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

    Ok(CpuEnvironmentMap {
        width,
        height,
        mip_levels,
        irradiance_sh,
        byte_size: total_bytes,
    })
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
        assert_eq!(
            environment.byte_size(),
            environment
                .mip_levels
                .iter()
                .map(|level| level.rgba32f.len() * 16)
                .sum::<usize>()
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
}
