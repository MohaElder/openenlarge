//! Creative finishing layer, applied to the gamma-encoded positive produced by
//! the inversion core. All params are 0.0 = identity. Tone/saturation are
//! per-pixel; texture (Task 2) is a spatial unsharp pass.

use crate::Image;

const EPS: f32 = 1e-5;
/// Unsharp-mask gain at texture = ±1 (empirical).
const USM_GAIN: f32 = 1.5;

/// Creative controls. UI sends −100..100 (and EV for exposure, handled upstream);
/// these are pre-scaled to −1..1 by the caller. 0.0 everywhere = identity.
#[derive(Debug, Clone, Copy)]
pub struct FinishParams {
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub vibrance: f32,
    pub saturation: f32,
}

impl Default for FinishParams {
    fn default() -> Self {
        FinishParams {
            contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
            texture: 0.0, vibrance: 0.0, saturation: 0.0,
        }
    }
}

/// Per-channel parametric tone curve in [0,1] display space. Monotone region
/// weights; final clamp to [0,1]. Order: endpoints (whites/blacks) → region
/// (highlights/shadows) → contrast S-gain about mid-gray.
fn tone_curve(v: f32, p: &FinishParams) -> f32 {
    let mut v = v.clamp(0.0, 1.0);
    // Endpoints: strongest at the extremes.
    v += p.whites * 0.20 * v.powi(3);
    v -= p.blacks * 0.20 * (1.0 - v).powi(3);
    // Regions: lift/pull, zero at both ends.
    v += p.shadows * 0.30 * (1.0 - v).powi(2) * v;
    v += p.highlights * 0.30 * v.powi(2) * (1.0 - v);
    // Contrast: linear gain about 0.5.
    v = 0.5 + (v - 0.5) * (1.0 + p.contrast);
    v.clamp(0.0, 1.0)
}

/// Vibrance/saturation: push each channel away from luma. Saturation is uniform;
/// vibrance is weighted by (1 − current saturation) so vivid pixels move less.
fn apply_saturation(rgb: [f32; 3], p: &FinishParams) -> [f32; 3] {
    let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    let mx = rgb[0].max(rgb[1]).max(rgb[2]);
    let mn = rgb[0].min(rgb[1]).min(rgb[2]);
    let cur_sat = if mx > EPS { (mx - mn) / mx } else { 0.0 };
    let factor = 1.0 + p.saturation + p.vibrance * (1.0 - cur_sat);
    std::array::from_fn(|c| (y + (rgb[c] - y) * factor).clamp(0.0, 1.0))
}

/// Per-pixel finishing (tone curve per channel, then saturation across channels).
pub fn finish_pixel(rgb: [f32; 3], p: &FinishParams) -> [f32; 3] {
    let toned = [tone_curve(rgb[0], p), tone_curve(rgb[1], p), tone_curve(rgb[2], p)];
    apply_saturation(toned, p)
}

/// Separable 3-tap Gaussian (radius 1, weights 1/4,1/2,1/4). Edges clamp. Small
/// radius keeps it cheap; texture is a local effect.
fn blur(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let idx = |x: usize, y: usize| y * w + x;
    let mut tmp = vec![[0.0_f32; 3]; w * h];
    // Horizontal
    for y in 0..h {
        for x in 0..w {
            let xl = x.saturating_sub(1);
            let xr = (x + 1).min(w - 1);
            let (a, b, c) = (img.pixels[idx(xl, y)], img.pixels[idx(x, y)], img.pixels[idx(xr, y)]);
            tmp[idx(x, y)] = std::array::from_fn(|i| 0.25 * a[i] + 0.5 * b[i] + 0.25 * c[i]);
        }
    }
    // Vertical
    let mut out = vec![[0.0_f32; 3]; w * h];
    for y in 0..h {
        let yu = y.saturating_sub(1);
        let yd = (y + 1).min(h - 1);
        for x in 0..w {
            let (a, b, c) = (tmp[idx(x, yu)], tmp[idx(x, y)], tmp[idx(x, yd)]);
            out[idx(x, y)] = std::array::from_fn(|i| 0.25 * a[i] + 0.5 * b[i] + 0.25 * c[i]);
        }
    }
    Image { width: w, height: h, pixels: out, ir: None } // scratch image: ir restored by apply_texture
}

/// Unsharp mask: out = v + amount * (v − blur(v)). amount in −1..1.
fn apply_texture(img: &Image, amount: f32) -> Image {
    let b = blur(img);
    let k = USM_GAIN * amount;
    let pixels = img.pixels.iter().zip(b.pixels.iter())
        .map(|(&v, &lo)| std::array::from_fn(|c| (v[c] + k * (v[c] - lo[c])).clamp(0.0, 1.0)))
        .collect();
    Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() }
}

pub fn finish_image(img: &Image, p: &FinishParams) -> Image {
    let pixels = img.pixels.iter().map(|&px| finish_pixel(px, p)).collect();
    let toned = Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() };
    if p.texture.abs() > EPS { apply_texture(&toned, p.texture) } else { toned }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img_from(pixels: Vec<[f32; 3]>) -> Image {
        Image { width: pixels.len(), height: 1, pixels, ir: None }
    }

    #[test]
    fn default_is_identity() {
        let p = FinishParams::default();
        for v in [0.0_f32, 0.2, 0.5, 0.8, 1.0] {
            let px = [v, v * 0.5, v * 0.25];
            let out = finish_pixel(px, &p);
            for c in 0..3 {
                assert!((out[c] - px[c]).abs() < 1e-4, "v={v} c={c} out={}", out[c]);
            }
        }
    }

    #[test]
    fn positive_contrast_widens_spread() {
        let p = FinishParams { contrast: 0.5, ..Default::default() };
        let dark = tone_curve(0.25, &p);
        let bright = tone_curve(0.75, &p);
        assert!(dark < 0.25, "dark {dark}");
        assert!(bright > 0.75, "bright {bright}");
    }

    #[test]
    fn positive_whites_raises_highlights_more_than_mids() {
        let p = FinishParams { whites: 1.0, ..Default::default() };
        assert!(tone_curve(0.9, &p) - 0.9 > tone_curve(0.5, &p) - 0.5);
    }

    #[test]
    fn positive_blacks_darkens_shadows() {
        let p = FinishParams { blacks: 1.0, ..Default::default() };
        assert!(tone_curve(0.1, &p) < 0.1);
    }

    #[test]
    fn positive_shadows_raises_shadows_more_than_mids() {
        let p = FinishParams { shadows: 1.0, ..Default::default() };
        assert!(tone_curve(0.25, &p) - 0.25 > tone_curve(0.6, &p) - 0.6);
    }

    #[test]
    fn positive_saturation_increases_chroma() {
        let p = FinishParams { saturation: 0.5, ..Default::default() };
        let px = [0.6, 0.4, 0.3];
        let out = apply_saturation(px, &p);
        let chroma_in = px[0] - px[2];
        let chroma_out = out[0] - out[2];
        assert!(chroma_out > chroma_in, "in {chroma_in} out {chroma_out}");
    }

    #[test]
    fn vibrance_affects_muted_more_than_vivid() {
        let p = FinishParams { vibrance: 1.0, ..Default::default() };
        let muted = [0.52, 0.50, 0.48];
        let vivid = [0.90, 0.10, 0.05];
        let chroma = |px: [f32; 3]| px[0].max(px[1]).max(px[2]) - px[0].min(px[1]).min(px[2]);
        let ratio = |px: [f32; 3]| chroma(apply_saturation(px, &p)) / chroma(px);
        // Vibrance boosts low-saturation (muted) pixels more than already-vivid ones.
        assert!(ratio(muted) > ratio(vivid), "muted {} vivid {}", ratio(muted), ratio(vivid));
    }

    #[test]
    fn finish_image_default_returns_equal_image() {
        let src = img_from(vec![[0.2, 0.4, 0.6], [0.7, 0.5, 0.3]]);
        let out = finish_image(&src, &FinishParams::default());
        assert_eq!(out.width, src.width);
        assert_eq!(out.height, src.height);
        for (o, s) in out.pixels.iter().zip(src.pixels.iter()) {
            for c in 0..3 {
                assert!((o[c] - s[c]).abs() < 1e-4, "c={c} out={} src={}", o[c], s[c]);
            }
        }
    }

    #[test]
    fn texture_zero_is_identity() {
        // A 5x5 ramp; texture=0 must return the same pixels (up to f32 round-trip).
        let mut px = Vec::new();
        for i in 0..25 { let v = i as f32 / 25.0; px.push([v, v, v]); }
        let img = Image { width: 5, height: 5, pixels: px.clone(), ir: None };
        let out = finish_image(&img, &FinishParams::default());
        for (o, s) in out.pixels.iter().zip(px.iter()) {
            for c in 0..3 {
                assert!((o[c] - s[c]).abs() < 1e-5, "c={c} out={} src={}", o[c], s[c]);
            }
        }
    }

    #[test]
    fn positive_texture_increases_edge_contrast() {
        // Vertical step edge: left half 0.4, right half 0.6 (5x5).
        let mut px = Vec::new();
        for _y in 0..5 {
            for x in 0..5 { let v = if x < 2 { 0.4 } else { 0.6 }; px.push([v, v, v]); }
        }
        let img = Image { width: 5, height: 5, pixels: px, ir: None };
        let p = FinishParams { texture: 1.0, ..Default::default() };
        let out = finish_image(&img, &p);
        // The bright side of the edge (x=2) should be pushed brighter than its
        // flat-region neighbour (x=4).
        let edge = out.pixels[2 * 5 + 2][0];
        let flat = out.pixels[2 * 5 + 4][0];
        assert!(edge > flat, "edge {edge} flat {flat}");
    }
}
