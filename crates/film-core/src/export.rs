//! Write an Image to a 16-bit RGB TIFF.

use crate::Image;
use std::path::Path;
use tiff::encoder::{colortype, TiffEncoder};

/// Encode a linear (or already-toned) Image as 16-bit RGB TIFF. Values are
/// clamped to [0,1] and scaled to u16. IR plane is not written (preserved only
/// in-memory for future use).
pub fn write_tiff16(img: &Image, path: &Path) -> Result<(), tiff::TiffError> {
    let mut file = std::fs::File::create(path).map_err(tiff::TiffError::IoError)?;
    let mut enc = TiffEncoder::new(&mut file)?;
    let mut data: Vec<u16> = Vec::with_capacity(img.len() * 3);
    for px in &img.pixels {
        for c in 0..3 {
            let v = (px[c].clamp(0.0, 1.0) * 65535.0).round() as u16;
            data.push(v);
        }
    }
    enc.write_image::<colortype::RGB16>(img.width as u32, img.height as u32, &data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode_tiff;

    #[test]
    fn roundtrip_tiff16() {
        let mut img = Image::new(2, 1);
        img.pixels[0] = [1.0, 0.0, 0.5];
        img.pixels[1] = [0.25, 0.75, 0.0];
        let dir = std::env::temp_dir();
        let path = dir.join("filmrev_roundtrip.tiff");
        write_tiff16(&img, &path).unwrap();
        let back = decode_tiff(&path).unwrap();
        assert_eq!(back.width, 2);
        assert_eq!(back.height, 1);
        assert!((back.pixels[0][0] - 1.0).abs() < 1e-3);
        assert!((back.pixels[0][2] - 0.5).abs() < 1e-3);
        assert!((back.pixels[1][1] - 0.75).abs() < 1e-3);
    }
}
