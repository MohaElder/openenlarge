//! Decode scan files into a linear-RGB Image.
//!
//! `decode_tiff` handles plain 8/16-bit RGB TIFF and scanner *linear* DNGs that
//! the `tiff` crate can read directly. `decode_raw` handles Bayer RAF/DNG via
//! rawler (demosaiced, linear light, no white-balance, no gamma).

use crate::Image;
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::ColorType;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("tiff error: {0}")]
    Tiff(#[from] tiff::TiffError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported color type: {0:?}")]
    UnsupportedColor(ColorType),
    #[error("raw decode error: {0}")]
    Raw(String),
    #[error("image decode error: {0}")]
    Image(#[from] ::image::ImageError),
}

/// sRGB electro-optical transfer function: gamma-encoded sRGB → linear light.
/// Input and output are normalized to [0, 1].
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Decode an 8- or 16-bit RGB(A) TIFF / linear DNG into a normalized f32 Image.
/// A 4th (alpha/IR) channel, if present, is captured into `ir`.
pub fn decode_tiff(path: &Path) -> Result<Image, DecodeError> {
    let file = std::fs::File::open(path)?;
    let mut dec = Decoder::new(file)?;
    let (w, h) = dec.dimensions()?;
    let color = dec.colortype()?;
    let (channels, max) = match color {
        ColorType::RGB(8) => (3usize, 255.0f32),
        ColorType::RGB(16) => (3, 65535.0),
        ColorType::RGBA(8) => (4, 255.0),
        ColorType::RGBA(16) => (4, 65535.0),
        other => return Err(DecodeError::UnsupportedColor(other)),
    };
    let result = dec.read_image()?;
    let floats: Vec<f32> = match result {
        DecodingResult::U8(v) => v.into_iter().map(|x| x as f32 / max).collect(),
        DecodingResult::U16(v) => v.into_iter().map(|x| x as f32 / max).collect(),
        _ => return Err(DecodeError::UnsupportedColor(color)),
    };
    let n = (w as usize) * (h as usize);
    let mut pixels = Vec::with_capacity(n);
    let mut ir: Option<Vec<f32>> = if channels == 4 {
        Some(Vec::with_capacity(n))
    } else {
        None
    };
    for i in 0..n {
        let base = i * channels;
        pixels.push([floats[base], floats[base + 1], floats[base + 2]]);
        if let Some(ir) = ir.as_mut() {
            ir.push(floats[base + 3]);
        }
    }
    Ok(Image {
        width: w as usize,
        height: h as usize,
        pixels,
        ir,
    })
}

/// Decode a camera RAW file (Fujifilm `.raf`, `.dng`, or any rawler-supported
/// format) into a demosaiced, linear-light RGB `Image`.
///
/// # Processing pipeline
/// We run rawler's `RawDevelop` with only the steps needed for a clean linear
/// decode:
///   - `Rescale` — applies black/white level correction, scaling raw u16 data
///     to f32 in [0, 1] before demosaic.
///   - `Demosaic` — PPG demosaic for standard RGB Bayer; bilinear for 4-channel
///     CFAs. Output remains in [0, 1] linear camera-native light.
///   - `CropActiveArea` — crops the optical black borders used during demosaic.
///   - `CropDefault` — applies the camera's default image crop.
///
/// Deliberately excluded:
///   - `WhiteBalance` — the inversion engine does its own channel balancing.
///   - `Calibrate` — skips the XYZ→camera color matrix; we want raw camera-
///     native values, not a rendering colorspace transform.
///   - `SRgb` — no gamma/tone curve; output stays linear.
///
/// # Normalization
/// `rawler`'s `Rescale` step subtracts per-channel black levels and divides by
/// (white_level − black_level), producing f32 values nominally in [0, 1].
/// After demosaic the values remain in that range (bilinear/PPG only
/// interpolate; they don't amplify). We clamp to [0, 1] as a safety net in
/// case of hot pixels or sensor artefacts slightly above white level.
pub fn decode_raw(path: &Path) -> Result<Image, DecodeError> {
    use rawler::imgop::develop::Intermediate;
    use rawler::imgop::develop::{ProcessingStep, RawDevelop};

    // Step 1: decode the raw file into a mosaic RawImage (integer u16 data,
    // not yet demosaiced).
    let raw = rawler::decode_file(path).map_err(|e| DecodeError::Raw(e.to_string()))?;

    // Step 2: develop with only linear steps (no WB, no colour matrix, no gamma).
    let develop = RawDevelop {
        steps: vec![
            ProcessingStep::Rescale,
            ProcessingStep::Demosaic,
            ProcessingStep::CropActiveArea,
            ProcessingStep::CropDefault,
        ],
    };
    let intermediate = develop
        .develop_intermediate(&raw)
        .map_err(|e| DecodeError::Raw(e.to_string()))?;

    // Step 3: extract the three-channel f32 pixel data.
    // After Rescale the data is in [0,1]; after Demosaic it stays in [0,1].
    // Clamp to guard against hot pixels that exceed white level.
    let (width, height, pixels) = match intermediate {
        Intermediate::ThreeColor(color2d) => {
            let w = color2d.width;
            let h = color2d.height;
            // color2d.data is Vec<[f32;3]> — exactly our Image::pixels type.
            let clamped: Vec<[f32; 3]> = color2d
                .data
                .into_iter()
                .map(|[r, g, b]| [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)])
                .collect();
            (w, h, clamped)
        }
        Intermediate::FourColor(color2d) => {
            // Some exotic CFAs produce a 4-channel intermediate; collapse to RGB
            // by dropping the 4th channel (which is typically a second green or
            // near-IR channel — not meaningful for film inversion).
            let w = color2d.width;
            let h = color2d.height;
            let clamped: Vec<[f32; 3]> = color2d
                .data
                .into_iter()
                .map(|[r, g, b, _]| [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)])
                .collect();
            (w, h, clamped)
        }
        Intermediate::Monochrome(pix) => {
            // Monochrome sensor: replicate the single channel into R=G=B.
            let w = pix.width;
            let h = pix.height;
            let clamped: Vec<[f32; 3]> = pix
                .data
                .into_iter()
                .map(|v| {
                    let c = v.clamp(0.0, 1.0);
                    [c, c, c]
                })
                .collect();
            (w, h, clamped)
        }
    };

    Ok(Image {
        width,
        height,
        pixels,
        ir: None,
    })
}

/// Decode a gamma-encoded LDR image (JPEG / PNG) into a linear-light RGB `Image`.
///
/// Unlike camera RAW and scanner TIFFs — which the pipeline treats as already
/// linear — JPEG/PNG are almost always **sRGB gamma-encoded**. We apply the sRGB
/// EOTF (`srgb_to_linear`) so the decoded values land in the same linear-light
/// domain the inversion engine expects. Any alpha channel is dropped; 16-bit PNGs
/// are supported (decoded at full precision before normalizing).
///
/// Note: 8-bit JPEG is lossy and low-bit-depth, so density-domain inversion has
/// less headroom than with a 16-bit RAW/TIFF scan — quality will be lower.
pub fn decode_ldr(path: &Path) -> Result<Image, DecodeError> {
    let img = ::image::open(path)?;
    let rgb = img.to_rgb32f(); // normalized [0,1] f32, alpha dropped
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let pixels: Vec<[f32; 3]> = rgb
        .pixels()
        .map(|p| {
            [
                srgb_to_linear(p[0]),
                srgb_to_linear(p[1]),
                srgb_to_linear(p[2]),
            ]
        })
        .collect();
    Ok(Image {
        width: w,
        height: h,
        pixels,
        ir: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_to_linear_endpoints_and_midtone() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-6);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        // 128/255 sRGB ≈ 0.50196 encodes to ≈ 0.2159 linear.
        let mid = srgb_to_linear(128.0 / 255.0);
        assert!((mid - 0.2159).abs() < 1e-3, "got {mid}");
    }

    #[test]
    fn decode_ldr_png_linearizes() {
        // 2x1 PNG: black, white. Decoded linear values must be 0.0 and 1.0.
        let mut buf: ::image::RgbImage = ::image::ImageBuffer::new(2, 1);
        buf.put_pixel(0, 0, ::image::Rgb([0, 0, 0]));
        buf.put_pixel(1, 0, ::image::Rgb([255, 255, 255]));
        let dir = std::env::temp_dir();
        let path = dir.join("filmrev_decode_ldr_test.png");
        buf.save(&path).unwrap();

        let img = decode_ldr(&path).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert!(img.ir.is_none());
        assert!((img.pixels[0][0] - 0.0).abs() < 1e-6);
        assert!((img.pixels[1][0] - 1.0).abs() < 1e-6);

        let _ = std::fs::remove_file(&path);
    }
}
