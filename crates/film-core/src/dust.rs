//! Manual dust removal: rasterize brush stamps to a windowed mask, then Telea-inpaint.

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
    let mut minx = f32::MAX;
    let mut miny = f32::MAX;
    let mut maxx = f32::MIN;
    let mut maxy = f32::MIN;
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
    let x1 = ((maxx.ceil() as isize + pad as isize + 1).max(0) as usize).min(img_w);
    let y1 = ((maxy.ceil() as isize + pad as isize + 1).max(0) as usize).min(img_h);
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
    fn rasterize_empty_when_no_stamps_or_offscreen() {
        assert_eq!(rasterize(100, 100, &[], 1.0, 4).w, 0);
        let off = rasterize(100, 100, &[Stamp { cx: -50.0, cy: -50.0, r: 2.0 }], 1.0, 1);
        assert_eq!(off.w, 0, "fully off-image dab → empty mask");
    }
}
