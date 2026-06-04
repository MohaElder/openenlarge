//! Convert between film_core::Image (f32 linear RGB) and the `image` crate,
//! and downscale to a preview proxy.

use film_core::Image;
use image::{ImageBuffer, Rgb};

pub fn to_rgb32f(img: &Image) -> ImageBuffer<Rgb<f32>, Vec<f32>> {
    let mut buf = ImageBuffer::new(img.width as u32, img.height as u32);
    for (i, px) in img.pixels.iter().enumerate() {
        let x = (i % img.width) as u32;
        let y = (i / img.width) as u32;
        buf.put_pixel(x, y, Rgb([px[0], px[1], px[2]]));
    }
    buf
}

pub fn from_rgb32f(buf: &ImageBuffer<Rgb<f32>, Vec<f32>>) -> Image {
    let (w, h) = (buf.width() as usize, buf.height() as usize);
    let pixels = buf.pixels().map(|p| [p[0], p[1], p[2]]).collect();
    Image { width: w, height: h, pixels, ir: None }
}

/// Downscale so the long edge is at most `max_edge` px (preserving aspect).
pub fn proxy(img: &Image, max_edge: u32) -> Image {
    let long = img.width.max(img.height) as u32;
    if long <= max_edge {
        return img.clone();
    }
    let scale = max_edge as f32 / long as f32;
    let nw = (img.width as f32 * scale).round().max(1.0) as u32;
    let nh = (img.height as f32 * scale).round().max(1.0) as u32;
    let buf = to_rgb32f(img);
    let resized = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
    from_rgb32f(&resized)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn solid(w: usize, h: usize, c: [f32; 3]) -> Image {
        Image { width: w, height: h, pixels: vec![c; w * h], ir: None }
    }
    #[test]
    fn roundtrip_preserves_pixels() {
        let img = solid(3, 2, [0.25, 0.5, 0.75]);
        let back = from_rgb32f(&to_rgb32f(&img));
        assert_eq!(back.width, 3);
        assert_eq!(back.height, 2);
        assert_eq!(back.pixels[0], [0.25, 0.5, 0.75]);
    }
    #[test]
    fn proxy_caps_long_edge_and_keeps_aspect() {
        let img = solid(4000, 2000, [0.4, 0.4, 0.4]);
        let p = proxy(&img, 2048);
        assert_eq!(p.width, 2048);
        assert_eq!(p.height, 1024);
    }
    #[test]
    fn proxy_noop_when_small() {
        let img = solid(100, 80, [0.1, 0.2, 0.3]);
        let p = proxy(&img, 2048);
        assert_eq!((p.width, p.height), (100, 80));
    }
}
