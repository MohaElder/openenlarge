//! Encode a film_core::Image to a base64 PNG/JPEG (8-bit) for the webview.

use base64::Engine;
use film_core::Image;
use image::{ImageBuffer, ImageEncoder, Rgb};

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
}
