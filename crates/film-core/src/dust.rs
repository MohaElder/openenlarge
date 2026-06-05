//! Manual dust removal: rasterize brush stamps to a windowed mask, then Telea-inpaint.

use crate::Image;
use ndarray::{Array2, Array3};

/// A brush dab in image PIXEL coordinates. `r` is the radius in pixels.
#[derive(Debug, Clone, Copy)]
pub struct Stamp {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
}

/// A binary mask confined to a window (origin `x0,y0`, size `w*h`) of the image.
/// `bits[y*w + x]` is true where a pixel should be inpainted. Empty → `w==0 || h==0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub x0: usize,
    pub y0: usize,
    pub w: usize,
    pub h: usize,
    pub bits: Vec<bool>,
}

/// Rasterize `stamps` (pixel coords) into a windowed mask on a `img_w`×`img_h` image.
/// Each dab is grown by `grow` px (soft dilation). The window is padded by `pad` px
/// beyond the dabs (clamped to the image) so the inpainter has known source pixels
/// around the hole. Returns an empty mask if nothing lands inside the image.
pub fn rasterize(img_w: usize, img_h: usize, stamps: &[Stamp], grow: f32, pad: usize) -> Mask {
    let empty = Mask { x0: 0, y0: 0, w: 0, h: 0, bits: Vec::new() };
    if img_w == 0 || img_h == 0 || stamps.is_empty() {
        return empty;
    }
    // Union bounds of all grown dabs (float), then intersect with the image.
    let mut minx = f32::INFINITY;
    let mut miny = f32::INFINITY;
    let mut maxx = f32::NEG_INFINITY;
    let mut maxy = f32::NEG_INFINITY;
    for s in stamps {
        let re = s.r + grow;
        minx = minx.min(s.cx - re);
        miny = miny.min(s.cy - re);
        maxx = maxx.max(s.cx + re);
        maxy = maxy.max(s.cy + re);
    }
    // Early-exit when the entire union of dabs lies outside the image.
    if maxx < 0.0 || maxy < 0.0 || minx >= img_w as f32 || miny >= img_h as f32 {
        return empty;
    }
    let x0 = (minx.floor() as isize - pad as isize).max(0) as usize;
    let y0 = (miny.floor() as isize - pad as isize).max(0) as usize;
    // +1 makes x1/y1 exclusive so the right/bottom edge keeps an unmasked source border.
    let x1 = ((maxx.ceil() as isize + pad as isize + 1).max(0) as usize).min(img_w);
    let y1 = ((maxy.ceil() as isize + pad as isize + 1).max(0) as usize).min(img_h);
    // defensive: unreachable after the off-screen early-exit above
    if x1 <= x0 || y1 <= y0 {
        return empty;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut bits = vec![false; w * h];
    for s in stamps {
        let re2 = (s.r + grow) * (s.r + grow);
        for yy in 0..h {
            for xx in 0..w {
                let px = (x0 + xx) as f32 + 0.5;
                let py = (y0 + yy) as f32 + 0.5;
                let d2 = (px - s.cx) * (px - s.cx) + (py - s.cy) * (py - s.cy);
                if d2 <= re2 {
                    bits[yy * w + xx] = true;
                }
            }
        }
    }
    Mask { x0, y0, w, h, bits }
}

/// Inpaint the masked pixels of `img` using Telea / Fast Marching, operating only
/// on the mask's window. `radius` is the Telea neighborhood size (px). No-op on an
/// empty mask.
pub fn inpaint_masked(img: &mut Image, mask: &Mask, radius: u32) {
    if mask.w == 0 || mask.h == 0 {
        return;
    }
    let (w, h) = (mask.w, mask.h);
    // Copy the window into (h, w, 3) and the mask into (h, w).
    let mut region = Array3::<f32>::zeros((h, w, 3));
    let mut m = Array2::<f32>::zeros((h, w));
    for yy in 0..h {
        for xx in 0..w {
            let gi = (mask.y0 + yy) * img.width + (mask.x0 + xx);
            let p = img.pixels[gi];
            region[[yy, xx, 0]] = p[0];
            region[[yy, xx, 1]] = p[1];
            region[[yy, xx, 2]] = p[2];
            if mask.bits[yy * w + xx] {
                m[[yy, xx]] = 1.0;
            }
        }
    }
    // Isolated third-party call. Real signature: telea_inpaint(image, mask, radius: i32) -> Result<()>
    let _ = inpaint::telea_inpaint(&mut region.view_mut(), &m.view(), radius as i32);
    // Write back only the masked pixels.
    for yy in 0..h {
        for xx in 0..w {
            if mask.bits[yy * w + xx] {
                let gi = (mask.y0 + yy) * img.width + (mask.x0 + xx);
                img.pixels[gi] = [region[[yy, xx, 0]], region[[yy, xx, 1]], region[[yy, xx, 2]]];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterize_marks_a_disc_and_leaves_a_known_border() {
        let m = rasterize(100, 100, &[Stamp { cx: 50.0, cy: 50.0, r: 3.0 }], 1.0, 4);
        // Center is masked.
        let lx = 50 - m.x0;
        let ly = 50 - m.y0;
        assert!(m.bits[ly * m.w + lx], "disc center must be masked");
        // The window has an unmasked border (source pixels for inpaint).
        assert!(!m.bits[0], "top-left of window must be unmasked");
        // Disc radius ~ r+grow=4 → corners of the window are outside the disc.
        assert!(m.w >= 9 && m.h >= 9, "window covers disc + pad");
    }

    #[test]
    fn rasterize_clamps_to_image_edge() {
        // Stamp centred 1px outside the left edge; radius=3 → some columns land on-image.
        let m = rasterize(100, 100, &[Stamp { cx: -1.0, cy: 50.0, r: 3.0 }], 0.0, 0);
        assert_eq!(m.x0, 0, "window clamped to left edge");
        assert!(m.w > 0, "partial dab produces a non-empty mask");
        // pixel (0,50): center (0.5, 50.5), distance ≈1.58 < 3 → masked.
        let bit = m.bits[(50 - m.y0) * m.w + 0];
        assert!(bit, "on-image pixel at col 0 should be masked");
    }

    #[test]
    fn rasterize_empty_when_no_stamps_or_offscreen() {
        assert_eq!(rasterize(100, 100, &[], 1.0, 4).w, 0);
        let off = rasterize(100, 100, &[Stamp { cx: -50.0, cy: -50.0, r: 2.0 }], 1.0, 1);
        assert_eq!(off.w, 0, "fully off-image dab → empty mask");
    }

    #[test]
    fn inpaint_removes_a_speck_against_a_solid_field() {
        // Solid gray 21x21 with one white "dust" pixel in the middle.
        let n = 21usize;
        let mut img = Image {
            width: n,
            height: n,
            pixels: vec![[0.4, 0.4, 0.4]; n * n],
            ir: None,
        };
        let mid = (n / 2) * n + (n / 2);
        img.pixels[mid] = [1.0, 1.0, 1.0];
        let mask = rasterize(n, n, &[Stamp { cx: 10.0, cy: 10.0, r: 1.0 }], 1.0, 4);
        inpaint_masked(&mut img, &mask, 3);
        // The speck is now close to the surrounding gray, not white.
        let p = img.pixels[mid];
        assert!(p[0] < 0.6, "speck should be filled toward gray, got {:?}", p);
        // A far-away pixel is untouched.
        assert_eq!(img.pixels[0], [0.4, 0.4, 0.4]);
    }
}
