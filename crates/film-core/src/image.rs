//! f32 linear-RGB image with an optional infrared plane.

/// A linear-light RGB image. `pixels` is row-major, length = width*height,
/// each pixel `[r, g, b]` in linear (not gamma) f32. `ir` (if present) is the
/// V600/SilverFast infrared plane, same length, preserved for future dust removal.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 3]>,
    pub ir: Option<Vec<f32>>,
}

impl Image {
    pub fn new(width: usize, height: usize) -> Self {
        Image { width, height, pixels: vec![[0.0; 3]; width * height], ir: None }
    }

    pub fn len(&self) -> usize {
        self.pixels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_allocates_black_pixels() {
        let img = Image::new(4, 2);
        assert_eq!(img.width, 4);
        assert_eq!(img.height, 2);
        assert_eq!(img.len(), 8);
        assert_eq!(img.pixels[0], [0.0, 0.0, 0.0]);
        assert!(img.ir.is_none());
    }
}
