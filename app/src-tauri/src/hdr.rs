//! HDR gain-map JPEG encoding.
//!
//! Wraps Google's libultrahdr (via the `ultrahdr` crate) to bake an SDR base
//! image plus an HDR rendition into a single ISO 21496-1 / Apple gain-map JPEG.
//! The rest of the HDR export feature depends only on [`encode_gain_map_jpeg`].

use half::f16;
use ultrahdr::{
    ColorGamut, ColorRange, ColorTransfer, Encoder, ImgFormat, ImgLabel, OwnedPackedImage,
};

/// Encode an HDR gain-map JPEG from an SDR base + an HDR rendition (same dims).
///
/// `sdr`/`hdr` are **display-referred, sRGB-encoded** Images (f32), matching the
/// inversion/finish output of `film-core` (already display-encoded; not linear):
/// - `sdr` is in 0..1 (sRGB-encoded display values),
/// - `hdr` is the *same* sRGB encoding but with expanded highlights that may
///   exceed 1.0 (up to ~headroom).
///
/// The SDR base is written straight to 8-bit (no OETF re-applied); the HDR
/// rendition is linearized (sRGB EOTF with a linear continuation above 1.0)
/// before being stored as half-float, so its `UHDR_CT_LINEAR` label is honest.
///
/// Returns JPEG bytes containing an ISO 21496-1 / Apple gain map.
pub fn encode_gain_map_jpeg(
    sdr: &film_core::Image,
    hdr: &film_core::Image,
    quality: u8,
) -> Result<Vec<u8>, String> {
    if sdr.width == 0 || sdr.height == 0 {
        return Err("sdr image has zero dimension".into());
    }
    if sdr.width != hdr.width || sdr.height != hdr.height {
        return Err(format!(
            "sdr/hdr dimension mismatch: {}x{} vs {}x{}",
            sdr.width, sdr.height, hdr.width, hdr.height
        ));
    }
    let expected = sdr.width * sdr.height;
    if sdr.pixels.len() < expected || hdr.pixels.len() < expected {
        return Err("pixel buffer shorter than width*height".into());
    }

    let w = sdr.width as u32;
    let h = sdr.height as u32;

    // SDR base: 8-bit sRGB-encoded RGBA, full-range, BT.709 (sRGB) primaries.
    // Input is already sRGB-encoded display values, so write straight to 8-bit.
    let mut sdr_img = OwnedPackedImage::new(
        ImgFormat::UHDR_IMG_FMT_32bppRGBA8888,
        w,
        h,
        ColorGamut::UHDR_CG_BT_709,
        ColorTransfer::UHDR_CT_SRGB,
        ColorRange::UHDR_CR_FULL_RANGE,
    )
    .map_err(|e| format!("alloc sdr image: {e:?}"))?;
    {
        let buf = sdr_img.buffer();
        for (i, px) in sdr.pixels.iter().take(expected).enumerate() {
            let o = i * 4;
            buf[o] = (px[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            buf[o + 1] = (px[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            buf[o + 2] = (px[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            buf[o + 3] = 255;
        }
    }

    // HDR rendition: 64bpp half-float linear RGBA, full-range, BT.709 primaries.
    // Input is display-referred sRGB (highlights >1.0); linearize before storing
    // so the UHDR_CT_LINEAR label is honest. Half-float preserves the >1.0 values.
    let mut hdr_img = OwnedPackedImage::new(
        ImgFormat::UHDR_IMG_FMT_64bppRGBAHalfFloat,
        w,
        h,
        ColorGamut::UHDR_CG_BT_709,
        ColorTransfer::UHDR_CT_LINEAR,
        ColorRange::UHDR_CR_FULL_RANGE,
    )
    .map_err(|e| format!("alloc hdr image: {e:?}"))?;
    {
        let buf = hdr_img.buffer();
        for (i, px) in hdr.pixels.iter().take(expected).enumerate() {
            let o = i * 8; // 4 channels * 2 bytes
            write_f16(&mut buf[o..o + 2], srgb_to_linear_ext(px[0]));
            write_f16(&mut buf[o + 2..o + 4], srgb_to_linear_ext(px[1]));
            write_f16(&mut buf[o + 4..o + 6], srgb_to_linear_ext(px[2]));
            write_f16(&mut buf[o + 6..o + 8], 1.0);
        }
    }

    let mut enc = Encoder::new().map_err(|e| format!("create encoder: {e:?}"))?;
    let q = quality.min(100) as i32;
    enc.set_raw_owned_image(&mut sdr_img, ImgLabel::UHDR_SDR_IMG)
        .map_err(|e| format!("set sdr image: {e:?}"))?;
    enc.set_raw_owned_image(&mut hdr_img, ImgLabel::UHDR_HDR_IMG)
        .map_err(|e| format!("set hdr image: {e:?}"))?;
    enc.set_quality(q, ImgLabel::UHDR_BASE_IMG)
        .map_err(|e| format!("set base quality: {e:?}"))?;
    enc.encode().map_err(|e| format!("encode: {e:?}"))?;

    let stream = enc
        .encoded_stream()
        .ok_or_else(|| "encoder produced no stream".to_string())?;
    let bytes = stream.bytes().map_err(|e| format!("read stream: {e:?}"))?;
    Ok(bytes.to_vec())
}

/// sRGB EOTF with a linear continuation above 1.0 (display-referred sRGB, where
/// expanded highlights exceed 1.0, → linear light). Continuous + C1 at v=1.0.
pub(crate) fn srgb_to_linear_ext(v: f32) -> f32 {
    if v <= 0.0 {
        0.0
    } else if v <= 0.04045 {
        v / 12.92
    } else if v <= 1.0 {
        ((v + 0.055) / 1.055).powf(2.4)
    } else {
        // slope of the sRGB EOTF at v=1.0 is 2.4/1.055; extend linearly.
        1.0 + (v - 1.0) * (2.4 / 1.055)
    }
}

/// Write a single f32 as little-endian IEEE half-float into a 2-byte slice.
fn write_f16(dst: &mut [u8], v: f32) {
    let bits = f16::from_f32(v.max(0.0)).to_bits().to_le_bytes();
    dst[0] = bits[0];
    dst[1] = bits[1];
}

#[cfg(test)]
mod tests {
    use super::*;
    use film_core::Image;
    fn solid(w: usize, h: usize, c: [f32; 3]) -> Image {
        Image { width: w, height: h, pixels: vec![c; w * h], ir: None }
    }
    #[test]
    fn encode_gain_map_jpeg_emits_a_gain_map() {
        let sdr = solid(64, 64, [0.9, 0.9, 0.9]);
        let hdr = solid(64, 64, [1.8, 1.8, 1.8]);
        let bytes = encode_gain_map_jpeg(&sdr, &hdr, 90).expect("encode");
        assert!(bytes.len() > 1000, "got {} bytes", bytes.len());
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "not a JPEG (SOI)");
        let iso = b"urn:iso";
        let apple = b"hdrgainmap";
        let has = bytes.windows(iso.len()).any(|w| w == iso)
            || bytes.windows(apple.len()).any(|w| w == apple);
        assert!(has, "no gain-map metadata in output");
    }

    #[test]
    fn exif_embedding_preserves_gain_map() {
        use crate::metadata::Metadata;
        let sdr = solid(64, 64, [0.9, 0.9, 0.9]);
        let hdr = solid(64, 64, [1.8, 1.8, 1.8]);
        let bytes = encode_gain_map_jpeg(&sdr, &hdr, 90).expect("encode");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hdr.jpg");
        std::fs::write(&path, &bytes).expect("write jpeg");

        let meta = Metadata {
            camera: Some("TestCam".into()),
            note: Some("hello hdr".into()),
            ..Default::default()
        };
        crate::exif_write::write_exif(&path, &meta).expect("exif embed");

        let after = std::fs::read(&path).expect("read back");
        assert!(
            after.windows(4).any(|w| w == b"Exif"),
            "EXIF marker missing after embed"
        );
        let iso = b"urn:iso";
        let apple = b"hdrgainmap";
        let has_gm = after.windows(iso.len()).any(|w| w == iso)
            || after.windows(apple.len()).any(|w| w == apple);
        assert!(has_gm, "gain-map metadata lost after EXIF embed");
    }
}
