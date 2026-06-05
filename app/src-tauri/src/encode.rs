//! Encode a film_core::Image — base64 PNG/JPEG for the webview, and PNG/TIFF/JPEG
//! files (8 or 16-bit) for export.

use base64::Engine;
use film_core::Image;
use image::{ImageBuffer, ImageEncoder, Rgb};
use std::path::Path;

/// Build an 8-bit RGB buffer from the linear/toned image, optionally applying
/// ~sRGB display gamma.
fn to_rgb8(img: &Image, apply_gamma: bool) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let g = if apply_gamma { 1.0 / 2.2 } else { 1.0 };
    let mut buf: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::new(img.width as u32, img.height as u32);
    for (i, px) in img.pixels.iter().enumerate() {
        let x = (i % img.width) as u32;
        let y = (i / img.width) as u32;
        let enc = |v: f32| -> u8 { (v.clamp(0.0, 1.0).powf(g) * 255.0).round() as u8 };
        buf.put_pixel(x, y, Rgb([enc(px[0]), enc(px[1]), enc(px[2])]));
    }
    buf
}

/// Build a 16-bit RGB buffer from the toned image (no gamma — output is already
/// display-encoded, matching the TIFF/preview path).
fn to_rgb16(img: &Image) -> ImageBuffer<Rgb<u16>, Vec<u16>> {
    let mut buf: ImageBuffer<Rgb<u16>, Vec<u16>> =
        ImageBuffer::new(img.width as u32, img.height as u32);
    for (i, px) in img.pixels.iter().enumerate() {
        let x = (i % img.width) as u32;
        let y = (i / img.width) as u32;
        let enc = |v: f32| -> u16 { (v.clamp(0.0, 1.0) * 65535.0).round() as u16 };
        buf.put_pixel(x, y, Rgb([enc(px[0]), enc(px[1]), enc(px[2])]));
    }
    buf
}

/// Write a PNG file. bits == 16 → 16-bit PNG; any other value → 8-bit PNG.
/// (Callers only pass 8 or 16; the format is inferred from the .png path.)
pub fn write_png(img: &Image, path: &Path, bits: u8) -> Result<(), String> {
    if bits == 16 {
        to_rgb16(img).save(path).map_err(|e| format!("png16 write: {e}"))
    } else {
        to_rgb8(img, false).save(path).map_err(|e| format!("png8 write: {e}"))
    }
}

/// Write an 8-bit RGB TIFF (16-bit goes through film_core::export::write_tiff16).
pub fn write_tiff8(img: &Image, path: &Path) -> Result<(), String> {
    to_rgb8(img, false).save(path).map_err(|e| format!("tiff8 write: {e}"))
}

/// Encode the toned image to in-memory JPEG bytes at the given quality (1–100).
pub fn encode_jpeg_bytes(img: &Image, quality: u8) -> Result<Vec<u8>, String> {
    let buf = to_rgb8(img, false);
    let mut bytes: Vec<u8> = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality.clamp(1, 100))
        .encode(buf.as_raw(), buf.width(), buf.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("jpeg encode: {e}"))?;
    Ok(bytes)
}

/// Write a JPEG file. Encodes at `quality`; if `max_bytes` is set and the result
/// exceeds it, binary-searches quality downward to the largest value that fits
/// (floor at quality 1).
pub fn write_jpeg(img: &Image, path: &Path, quality: u8, max_bytes: Option<u64>) -> Result<(), String> {
    let ceil = quality.clamp(1, 100);
    let bytes = match max_bytes {
        None => encode_jpeg_bytes(img, ceil)?,
        Some(cap) => {
            let mut lo: u8 = 1;
            let mut hi: u8 = ceil;
            let mut best = encode_jpeg_bytes(img, 1)?; // q=1 fallback if nothing fits
            while lo <= hi {
                let mid = lo + (hi - lo) / 2;
                let candidate = encode_jpeg_bytes(img, mid)?;
                if (candidate.len() as u64) <= cap {
                    best = candidate;
                    if mid == 100 { break; }
                    lo = mid + 1;
                } else {
                    if mid == 1 { break; }
                    hi = mid - 1;
                }
            }
            best
        }
    };
    std::fs::write(path, &bytes).map_err(|e| format!("jpeg write: {e}"))
}

/// Encode to a base64 JPEG data URI (quality 0–100). Much faster to encode and
/// far smaller over IPC than PNG — used for live previews.
pub fn to_jpeg_b64(img: &Image, apply_gamma: bool, quality: u8) -> Result<String, String> {
    let buf = to_rgb8(img, apply_gamma);
    let mut bytes: Vec<u8> = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode(buf.as_raw(), buf.width(), buf.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("jpeg encode: {e}"))?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// Encode to base64 PNG data URI. If `apply_gamma`, apply ~sRGB display gamma
/// (1/2.2) — use for raw (linear) previews; pass false for engine output that is
/// already tone-mapped.
pub fn to_png_b64(img: &Image, apply_gamma: bool) -> Result<String, String> {
    let buf = to_rgb8(img, apply_gamma);
    let mut bytes: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&buf, img.width as u32, img.height as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("png encode: {e}"))?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn produces_decodable_png_data_uri() {
        let img = Image { width: 2, height: 1, pixels: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], ir: None };
        let uri = to_png_b64(&img, false).unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        let b64 = uri.strip_prefix("data:image/png;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
    }
    #[test]
    fn gamma_changes_encoding() {
        let img = Image { width: 1, height: 1, pixels: vec![[0.25, 0.25, 0.25]], ir: None };
        assert_ne!(to_png_b64(&img, false).unwrap(), to_png_b64(&img, true).unwrap());
    }

    fn gradient(w: usize, h: usize) -> Image {
        let mut px = Vec::with_capacity(w * h);
        for i in 0..w * h {
            let t = (i as f32) / ((w * h) as f32);
            px.push([t, 1.0 - t, (t * 2.0).fract()]);
        }
        Image { width: w, height: h, pixels: px, ir: None }
    }

    #[test]
    fn png8_writes_decodable_file() {
        let p = std::env::temp_dir().join("filmrev_t1_png8.png");
        write_png(&gradient(8, 4), &p, 8).unwrap();
        let d = image::open(&p).unwrap();
        assert_eq!((d.width(), d.height()), (8, 4));
    }

    #[test]
    fn png16_writes_decodable_file() {
        let p = std::env::temp_dir().join("filmrev_t1_png16.png");
        write_png(&gradient(8, 4), &p, 16).unwrap();
        let d = image::open(&p).unwrap();
        assert_eq!((d.width(), d.height()), (8, 4));
        assert_eq!(d.color().bits_per_pixel(), 48); // 16-bit RGB
    }

    #[test]
    fn tiff8_writes_decodable_file() {
        let p = std::env::temp_dir().join("filmrev_t1_tiff8.tiff");
        write_tiff8(&gradient(6, 3), &p).unwrap();
        let d = image::open(&p).unwrap();
        assert_eq!((d.width(), d.height()), (6, 3));
    }

    #[test]
    fn jpeg_quality_is_monotonic() {
        let g = gradient(64, 64);
        let lo = encode_jpeg_bytes(&g, 20).unwrap().len();
        let hi = encode_jpeg_bytes(&g, 95).unwrap().len();
        assert!(hi >= lo, "hi {hi} should be >= lo {lo}");
    }

    #[test]
    fn jpeg_respects_max_bytes() {
        let g = gradient(64, 64);
        let big = encode_jpeg_bytes(&g, 95).unwrap().len() as u64;
        let floor = encode_jpeg_bytes(&g, 1).unwrap().len() as u64;
        let cap = (big / 4).max(1);
        let p = std::env::temp_dir().join("filmrev_t1_cap.jpg");
        write_jpeg(&g, &p, 95, Some(cap)).unwrap();
        let got = std::fs::metadata(&p).unwrap().len();
        assert!(got <= cap || got == floor, "got {got} cap {cap} floor {floor}");
    }
}
