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

/// Crop a rectangle (in pixels) from the image, clamped to its bounds. Returns a
/// new Image; `ir` is dropped (previews don't need it).
pub fn crop(img: &Image, x: usize, y: usize, w: usize, h: usize) -> Image {
    let x = x.min(img.width);
    let y = y.min(img.height);
    let x2 = (x + w).min(img.width);
    let y2 = (y + h).min(img.height);
    let (cw, ch) = (x2 - x, y2 - y);
    let mut pixels = Vec::with_capacity(cw * ch);
    for yy in y..y2 {
        let row = yy * img.width;
        for xx in x..x2 {
            pixels.push(img.pixels[row + xx]);
        }
    }
    Image { width: cw, height: ch, pixels, ir: None }
}

/// Oriented dimensions after `rot90` clockwise quarter-turns.
pub fn orient_dims(w: usize, h: usize, rot90: u8) -> (usize, usize) {
    if rot90 % 2 == 1 { (h, w) } else { (w, h) }
}

fn flip_h(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut px = vec![[0.0_f32; 3]; w * h];
    for y in 0..h { for x in 0..w { px[y * w + x] = img.pixels[y * w + (w - 1 - x)]; } }
    Image { width: w, height: h, pixels: px, ir: None }
}
fn flip_v(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut px = vec![[0.0_f32; 3]; w * h];
    for y in 0..h { for x in 0..w { px[y * w + x] = img.pixels[(h - 1 - y) * w + x]; } }
    Image { width: w, height: h, pixels: px, ir: None }
}
fn rotate_cw(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let (nw, nh) = (h, w);
    let mut px = vec![[0.0_f32; 3]; nw * nh];
    for ny in 0..nh { for nx in 0..nw {
        let ox = ny; let oy = h - 1 - nx;
        px[ny * nw + nx] = img.pixels[oy * w + ox];
    } }
    Image { width: nw, height: nh, pixels: px, ir: None }
}

/// Lossless orientation: flip-H, flip-V, then `rot90` clockwise quarter-turns.
pub fn orient(img: &Image, rot90: u8, flip_horizontal: bool, flip_vertical: bool) -> Image {
    let mut o = img.clone();
    if flip_horizontal { o = flip_h(&o); }
    if flip_vertical { o = flip_v(&o); }
    for _ in 0..(rot90 % 4) { o = rotate_cw(&o); }
    o
}

fn sample_bilinear(img: &Image, sx: f32, sy: f32) -> [f32; 3] {
    let (w, h) = (img.width as i32, img.height as i32);
    // Return black for any coordinate outside the image pixel space.
    if sx < 0.0 || sy < 0.0 || sx >= w as f32 || sy >= h as f32 {
        return [0.0, 0.0, 0.0];
    }
    let x0 = sx.floor() as i32; let y0 = sy.floor() as i32;
    let fx = sx - x0 as f32; let fy = sy - y0 as f32;
    let get = |x: i32, y: i32| -> [f32; 3] {
        let xc = x.clamp(0, w - 1) as usize; let yc = y.clamp(0, h - 1) as usize;
        img.pixels[yc * img.width + xc]
    };
    let p00 = get(x0, y0); let p10 = get(x0 + 1, y0);
    let p01 = get(x0, y0 + 1); let p11 = get(x0 + 1, y0 + 1);
    std::array::from_fn(|c| {
        let a = p00[c] * (1.0 - fx) + p10[c] * fx;
        let b = p01[c] * (1.0 - fx) + p11[c] * fx;
        a * (1.0 - fy) + b * fy
    })
}

/// Straighten: rotate clockwise by `deg` about the centre into a same-size canvas.
/// Out-of-bounds samples are black. No-op below 1e-4 deg.
pub fn rotate(img: &Image, deg: f32) -> Image {
    if deg.abs() < 1e-4 { return img.clone(); }
    let (w, h) = (img.width, img.height);
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let cx = w as f32 / 2.0; let cy = h as f32 / 2.0;
    let mut px = vec![[0.0_f32; 3]; w * h];
    for oy in 0..h { for ox in 0..w {
        let dx = ox as f32 + 0.5 - cx;
        let dy = oy as f32 + 0.5 - cy;
        let sx = cos * dx + sin * dy + cx - 0.5;
        let sy = -sin * dx + cos * dy + cy - 0.5;
        px[oy * w + ox] = sample_bilinear(img, sx, sy);
    } }
    Image { width: w, height: h, pixels: px, ir: None }
}

/// Resize to exactly `w x h` (Triangle filter). No-op if already that size.
pub fn resize_to(img: &Image, w: u32, h: u32) -> Image {
    if img.width as u32 == w && img.height as u32 == h {
        return img.clone();
    }
    let buf = to_rgb32f(img);
    let r = image::imageops::resize(&buf, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
    from_rgb32f(&r)
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

    #[test]
    fn crop_extracts_subrectangle() {
        let mut img = Image { width: 4, height: 4, pixels: vec![[0.0; 3]; 16], ir: None };
        for y in 0..4 {
            for x in 0..4 {
                img.pixels[y * 4 + x] = [x as f32 / 10.0, y as f32 / 10.0, 0.0];
            }
        }
        let c = crop(&img, 1, 2, 2, 1);
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(c.pixels[0], [0.1, 0.2, 0.0]);
        assert_eq!(c.pixels[1], [0.2, 0.2, 0.0]);
    }

    #[test]
    fn crop_clamps_to_bounds_without_panic() {
        let img = solid(4, 4, [0.5, 0.5, 0.5]);
        let c = crop(&img, 3, 3, 10, 10);
        assert_eq!((c.width, c.height), (1, 1));
        let z = crop(&img, 9, 9, 2, 2);
        assert_eq!((z.width, z.height), (0, 0));
    }

    #[test]
    fn resize_to_hits_target_dims_and_keeps_color() {
        let img = solid(10, 8, [0.2, 0.4, 0.6]);
        let r = resize_to(&img, 5, 4);
        assert_eq!((r.width, r.height), (5, 4));
        for c in 0..3 {
            assert!((r.pixels[0][c] - img.pixels[0][c]).abs() < 1e-3);
        }
    }

    fn pattern() -> Image {
        let mut img = Image { width: 2, height: 3, pixels: vec![[0.0; 3]; 6], ir: None };
        for y in 0..3 { for x in 0..2 { img.pixels[y * 2 + x] = [x as f32 / 10.0, y as f32 / 10.0, 0.0]; } }
        img
    }
    #[test]
    fn orient_identity() {
        let p = pattern();
        assert_eq!(orient(&p, 0, false, false).pixels, p.pixels);
    }
    #[test]
    fn orient_dims_swaps_on_quarter_turns() {
        assert_eq!(orient_dims(2, 3, 0), (2, 3));
        assert_eq!(orient_dims(2, 3, 1), (3, 2));
        assert_eq!(orient_dims(2, 3, 2), (2, 3));
        assert_eq!(orient_dims(2, 3, 3), (3, 2));
    }
    #[test]
    fn orient_flip_h_mirrors_x() {
        let p = pattern();
        let f = orient(&p, 0, true, false);
        assert_eq!(f.pixels[0], p.pixels[1]);
        assert_eq!(f.pixels[1], p.pixels[0]);
    }
    #[test]
    fn orient_rot90_cw_maps_topleft_to_topright() {
        let p = pattern();
        let r = orient(&p, 1, false, false);
        assert_eq!((r.width, r.height), (3, 2));
        assert_eq!(r.pixels[0 * 3 + 2], p.pixels[0]);
    }
    #[test]
    fn rotate_zero_is_identity() {
        let p = pattern();
        assert_eq!(rotate(&p, 0.0).pixels, p.pixels);
    }
    #[test]
    fn rotate_90_on_square_matches_orient_interior() {
        let mut s = Image { width: 3, height: 3, pixels: vec![[0.0; 3]; 9], ir: None };
        for y in 0..3 { for x in 0..3 { s.pixels[y * 3 + x] = [x as f32 / 10.0, y as f32 / 10.0, 0.0]; } }
        let a = rotate(&s, 90.0);
        let b = orient(&s, 1, false, false);
        assert!((a.pixels[1 * 3 + 1][0] - b.pixels[1 * 3 + 1][0]).abs() < 1e-3);
        assert!((a.pixels[1 * 3 + 1][1] - b.pixels[1 * 3 + 1][1]).abs() < 1e-3);
    }
    #[test]
    fn rotate_blacks_out_of_bounds_corners() {
        let p = pattern();
        let r = rotate(&p, 30.0);
        assert_eq!(r.pixels[0], [0.0, 0.0, 0.0]);
    }
}
