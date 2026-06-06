//! Creative finishing layer, applied to the gamma-encoded positive produced by
//! the inversion core. All params are 0.0 = identity. Tone/saturation are
//! per-pixel; texture (Task 2) is a spatial unsharp pass.

use crate::curve::{curve_lut, sample_lut, LUT_SIZE};
use crate::Image;
use rayon::prelude::*;

const EPS: f32 = 1e-5;
/// Unsharp-mask gain at texture = ±1 (empirical).
const USM_GAIN: f32 = 1.5;

// --- Tone Curve region (parametric) constants. Shared with shaders.ts / curve.ts. ---
/// Per-slider lift at ±1 in its zone.
const REGION_GAIN: f32 = 0.25;
/// Half-width of each region's parabolic bump.
const REGION_WIDTH: f32 = 0.22;
/// Tone-zone centers: shadows, darks, lights, highlights.
const REGION_CENTERS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];

// --- Color Grading constants. Shared with shaders.ts. ---
/// Saturation → chroma-offset scale.
const CG_COLOR_GAIN: f32 = 0.5;
/// Luminance slider → brightness-offset scale.
const CG_LUM_GAIN: f32 = 0.3;

#[inline]
fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Pure-hue RGB at full saturation/value. `h` in degrees.
fn hsv_hue_rgb(h: f32) -> [f32; 3] {
    let h = (h.rem_euclid(360.0)) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as i32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// RGB (0..1) → HSL. Hue in degrees [0,360); s,l in [0,1].
fn rgb2hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let l = (mx + mn) * 0.5;
    if (mx - mn).abs() < 1e-7 {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 {
        d / (2.0 - mx - mn)
    } else {
        d / (mx + mn)
    };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hue2rgb(p: f32, q: f32, t: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// HSL → RGB (0..1). Inverse of `rgb2hsl`.
fn hsl2rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hk = h / 360.0;
    [
        hue2rgb(p, q, hk + 1.0 / 3.0),
        hue2rgb(p, q, hk),
        hue2rgb(p, q, hk - 1.0 / 3.0),
    ]
}

// --- Color Mixer / Point Color shared constants (mirror shaders.ts + finish.ts). ---
const BAND_CENTERS: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0];
const CM_FALLOFF_DEG: f32 = 50.0;
const CM_HUE_SHIFT_MAX: f32 = 30.0;
const CM_LUM_GAIN: f32 = 0.25;
const CM_SAT_GATE_LO: f32 = 0.05;
const CM_SAT_GATE_HI: f32 = 0.20;
const PC_RANGE_MIN_DEG: f32 = 5.0;
const PC_RANGE_MAX_DEG: f32 = 60.0;
const PC_SAT_TOL: f32 = 0.25;
const PC_LUM_TOL: f32 = 0.25;
const PC_VAR_SPAN: f32 = 2.0;
const PI: f32 = std::f32::consts::PI;

/// Signed hue difference in (−180, 180].
#[inline]
fn wrap180(d: f32) -> f32 {
    let mut x = (d + 180.0).rem_euclid(360.0) - 180.0;
    if x <= -180.0 {
        x += 360.0;
    }
    x
}

#[inline]
fn band_weight(h: f32, center: f32) -> f32 {
    let d = wrap180(h - center).abs();
    if d >= CM_FALLOFF_DEG {
        0.0
    } else {
        0.5 * (1.0 + (PI * d / CM_FALLOFF_DEG).cos())
    }
}

/// Precomputed Mixer state. Slider values pre-divided to unit (−1..1); sats/lums too.
#[derive(Debug, Clone, Default)]
pub struct ColorMix {
    pub cm_hue: [f32; 8],
    pub cm_sat: [f32; 8],
    pub cm_lum: [f32; 8],
    pub samples: Vec<PcSample>,
}

/// One Point Color sample, pre-scaled for the per-pixel loop.
#[derive(Debug, Clone, Copy)]
pub struct PcSample {
    pub hue: f32,       // 0..360
    pub sat: f32,       // 0..1
    pub lum: f32,       // 0..1
    pub hue_shift: f32, // −1..1
    pub sat_shift: f32,
    pub lum_shift: f32,
    pub variance: f32, // −100..100 (raw; used by pc_tol)
    pub range: f32,    // 0..100 (raw)
}

/// Apply the 8-band HSL Mixer to one pixel.
fn color_mix(rgb: [f32; 3], cm: &ColorMix) -> [f32; 3] {
    let (mut h, mut s, mut l) = rgb2hsl(rgb);
    let gate = smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s);
    let mut sat_factor = 1.0_f32;
    let mut hue_delta = 0.0_f32;
    let mut lum_delta = 0.0_f32;
    for (i, &center) in BAND_CENTERS.iter().enumerate() {
        let w = band_weight(h, center);
        if w <= 0.0 {
            continue;
        }
        hue_delta += w * gate * cm.cm_hue[i] * CM_HUE_SHIFT_MAX;
        sat_factor += w * gate * cm.cm_sat[i];
        lum_delta += w * cm.cm_lum[i] * CM_LUM_GAIN;
    }
    h += hue_delta;
    s = (s * sat_factor).clamp(0.0, 1.0);
    l = (l + lum_delta).clamp(0.0, 1.0);
    hsl2rgb(h, s, l)
}

#[inline]
fn pc_tol(base: f32, variance: f32) -> f32 {
    (base * (1.0 + (variance / 100.0) * PC_VAR_SPAN)).max(0.02)
}

#[inline]
fn pc_hue_weight(h: f32, target: f32, range: f32) -> f32 {
    let hw = PC_RANGE_MIN_DEG + (range / 100.0) * (PC_RANGE_MAX_DEG - PC_RANGE_MIN_DEG);
    let d = wrap180(h - target).abs();
    if d >= hw {
        0.0
    } else {
        0.5 * (1.0 + (PI * d / hw).cos())
    }
}

/// Apply all Point Color samples to one pixel. Masks use the input HSL so samples
/// are order-independent; shifts accumulate then apply once.
fn point_color(rgb: [f32; 3], samples: &[PcSample]) -> [f32; 3] {
    if samples.is_empty() {
        return rgb;
    }
    let (h, s, l) = rgb2hsl(rgb);
    let mut hue_delta = 0.0_f32;
    let mut sat_factor = 1.0_f32;
    let mut lum_delta = 0.0_f32;
    for sm in samples {
        let wh = pc_hue_weight(h, sm.hue, sm.range);
        if wh <= 0.0 {
            continue;
        }
        let ws = (1.0 - (s - sm.sat).abs() / pc_tol(PC_SAT_TOL, sm.variance)).clamp(0.0, 1.0);
        let wl = (1.0 - (l - sm.lum).abs() / pc_tol(PC_LUM_TOL, sm.variance)).clamp(0.0, 1.0);
        let w = wh * ws * wl;
        if w <= 0.0 {
            continue;
        }
        hue_delta += w * sm.hue_shift * CM_HUE_SHIFT_MAX;
        sat_factor += w * sm.sat_shift;
        lum_delta += w * sm.lum_shift * CM_LUM_GAIN;
    }
    hsl2rgb(
        h + hue_delta,
        (s * sat_factor).clamp(0.0, 1.0),
        (l + lum_delta).clamp(0.0, 1.0),
    )
}

/// Parabolic region bump centered at `c` (finite support, peak 1.0).
#[inline]
fn region_bump(v: f32, c: f32) -> f32 {
    let t = (v - c) / REGION_WIDTH;
    (1.0 - t * t).max(0.0)
}

/// Apply the four parametric region sliders (−1..1) to a value in [0,1].
fn parametric(v: f32, regions: [f32; 4]) -> f32 {
    let mut v = v.clamp(0.0, 1.0);
    for k in 0..4 {
        v += regions[k] * REGION_GAIN * region_bump(v, REGION_CENTERS[k]);
    }
    v.clamp(0.0, 1.0)
}

/// Build the three composed tone LUTs: per channel, `channel(master(parametric(x)))`.
/// `regions` are the four sliders (highlights, lights, darks, shadows) pre-scaled to
/// −1..1, ordered [shadows, darks, lights, highlights] to match REGION_CENTERS.
pub fn tone_luts(
    regions: [f32; 4],
    master: &[[f32; 2]],
    red: &[[f32; 2]],
    green: &[[f32; 2]],
    blue: &[[f32; 2]],
) -> ([f32; LUT_SIZE], [f32; LUT_SIZE], [f32; LUT_SIZE]) {
    let m = curve_lut(master);
    let r = curve_lut(red);
    let g = curve_lut(green);
    let b = curve_lut(blue);
    let mut lr = [0.0f32; LUT_SIZE];
    let mut lg = [0.0f32; LUT_SIZE];
    let mut lb = [0.0f32; LUT_SIZE];
    for i in 0..LUT_SIZE {
        let x = i as f32 / (LUT_SIZE - 1) as f32;
        let base = sample_lut(&m, parametric(x, regions));
        lr[i] = sample_lut(&r, base);
        lg[i] = sample_lut(&g, base);
        lb[i] = sample_lut(&b, base);
    }
    (lr, lg, lb)
}

fn identity_lut() -> [f32; LUT_SIZE] {
    let mut l = [0.0f32; LUT_SIZE];
    for (i, v) in l.iter_mut().enumerate() {
        *v = i as f32 / (LUT_SIZE - 1) as f32;
    }
    l
}

/// Precomputed color-grading state: per-region chroma offset + luminance lift, and
/// the luma mask edges. 0/identity everywhere = no change.
#[derive(Debug, Clone, Copy)]
pub struct ColorGrade {
    pub sh_off: [f32; 3],
    pub sh_lum: f32,
    pub mid_off: [f32; 3],
    pub mid_lum: f32,
    pub hi_off: [f32; 3],
    pub hi_lum: f32,
    pub glob_off: [f32; 3],
    pub glob_lum: f32,
    pub sh_edge: f32,
    pub hi_edge: f32,
    pub softness: f32,
}

impl Default for ColorGrade {
    fn default() -> Self {
        ColorGrade {
            sh_off: [0.0; 3],
            sh_lum: 0.0,
            mid_off: [0.0; 3],
            mid_lum: 0.0,
            hi_off: [0.0; 3],
            hi_lum: 0.0,
            glob_off: [0.0; 3],
            glob_lum: 0.0,
            sh_edge: 0.33,
            hi_edge: 0.66,
            softness: 0.25,
        }
    }
}

/// Chroma-only offset for one wheel: hue (deg) + sat (0..1) → zero-luma RGB push.
fn wheel_offset(hue: f32, sat: f32) -> [f32; 3] {
    let col = hsv_hue_rgb(hue);
    let y = luma(col);
    std::array::from_fn(|c| (col[c] - y) * sat * CG_COLOR_GAIN)
}

impl ColorGrade {
    /// Build from UI values. Hues in degrees, sats 0..1, lums −1..1; `blending`
    /// 0..1 (mask overlap width), `balance` −1..1 (shadow↔highlight crossover).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sh: ([f32; 2], f32),
        mid: ([f32; 2], f32),
        hi: ([f32; 2], f32),
        glob: ([f32; 2], f32),
        blending: f32,
        balance: f32,
    ) -> Self {
        let mk = |w: ([f32; 2], f32)| (wheel_offset(w.0[0], w.0[1]), w.1 * CG_LUM_GAIN);
        let (sh_off, sh_lum) = mk(sh);
        let (mid_off, mid_lum) = mk(mid);
        let (hi_off, hi_lum) = mk(hi);
        let (glob_off, glob_lum) = mk(glob);
        ColorGrade {
            sh_off,
            sh_lum,
            mid_off,
            mid_lum,
            hi_off,
            hi_lum,
            glob_off,
            glob_lum,
            sh_edge: 0.33 + balance * 0.25,
            hi_edge: 0.66 + balance * 0.25,
            softness: 0.1 + 0.3 * blending,
        }
    }
}

/// Apply color grading to one pixel: weight each region's offset+lum by a luma mask.
fn color_grade(rgb: [f32; 3], cg: &ColorGrade) -> [f32; 3] {
    let l = luma(rgb);
    let w_sh = 1.0 - smoothstep(cg.sh_edge - cg.softness, cg.sh_edge + cg.softness, l);
    let w_hi = smoothstep(cg.hi_edge - cg.softness, cg.hi_edge + cg.softness, l);
    let w_mid = (1.0 - w_sh - w_hi).clamp(0.0, 1.0);
    std::array::from_fn(|c| {
        (rgb[c]
            + w_sh * (cg.sh_off[c] + cg.sh_lum)
            + w_mid * (cg.mid_off[c] + cg.mid_lum)
            + w_hi * (cg.hi_off[c] + cg.hi_lum)
            + (cg.glob_off[c] + cg.glob_lum))
            .clamp(0.0, 1.0)
    })
}

/// Creative controls. UI sends −100..100 (and EV for exposure, handled upstream);
/// these are pre-scaled to −1..1 by the caller. 0.0 everywhere = identity.
#[derive(Debug, Clone)]
pub struct FinishParams {
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub vibrance: f32,
    pub saturation: f32,
    /// Composed tone-curve LUTs (per channel): channel(master(parametric(x))).
    pub lut_r: [f32; LUT_SIZE],
    pub lut_g: [f32; LUT_SIZE],
    pub lut_b: [f32; LUT_SIZE],
    pub cg: ColorGrade,
    pub cm: ColorMix,
}

impl Default for FinishParams {
    fn default() -> Self {
        FinishParams {
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            lut_r: identity_lut(),
            lut_g: identity_lut(),
            lut_b: identity_lut(),
            cg: ColorGrade::default(),
            cm: ColorMix::default(),
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
    v += p.blacks * 0.20 * (1.0 - v).powi(3);
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

/// Per-pixel finishing. Order: Basic tone curve + saturation → Tone Curve LUT →
/// Color Grading. (Texture is a separate spatial pass in `finish_image`.)
pub fn finish_pixel(rgb: [f32; 3], p: &FinishParams) -> [f32; 3] {
    let toned = [
        tone_curve(rgb[0], p),
        tone_curve(rgb[1], p),
        tone_curve(rgb[2], p),
    ];
    let sat = apply_saturation(toned, p);
    let curved = [
        sample_lut(&p.lut_r, sat[0]),
        sample_lut(&p.lut_g, sat[1]),
        sample_lut(&p.lut_b, sat[2]),
    ];
    let graded = color_grade(curved, &p.cg);
    let mixed = color_mix(graded, &p.cm);
    point_color(mixed, &p.cm.samples)
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
            let (a, b, c) = (
                img.pixels[idx(xl, y)],
                img.pixels[idx(x, y)],
                img.pixels[idx(xr, y)],
            );
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
    Image {
        width: w,
        height: h,
        pixels: out,
        ir: None,
    } // scratch image: ir restored by apply_texture
}

/// Unsharp mask: out = v + amount * (v − blur(v)). amount in −1..1.
fn apply_texture(img: &Image, amount: f32) -> Image {
    let b = blur(img);
    let k = USM_GAIN * amount;
    // par_iter().zip() over two equal-length indexed slices preserves order.
    let pixels = img
        .pixels
        .par_iter()
        .zip(b.pixels.par_iter())
        .map(|(&v, &lo)| std::array::from_fn(|c| (v[c] + k * (v[c] - lo[c])).clamp(0.0, 1.0)))
        .collect();
    Image {
        width: img.width,
        height: img.height,
        pixels,
        ir: img.ir.clone(),
    }
}

pub fn finish_image(img: &Image, p: &FinishParams) -> Image {
    let pixels = img
        .pixels
        .par_iter()
        .map(|&px| finish_pixel(px, p))
        .collect();
    let toned = Image {
        width: img.width,
        height: img.height,
        pixels,
        ir: img.ir.clone(),
    };
    if p.texture.abs() > EPS {
        apply_texture(&toned, p.texture)
    } else {
        toned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img_from(pixels: Vec<[f32; 3]>) -> Image {
        Image {
            width: pixels.len(),
            height: 1,
            pixels,
            ir: None,
        }
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
        let p = FinishParams {
            contrast: 0.5,
            ..Default::default()
        };
        let dark = tone_curve(0.25, &p);
        let bright = tone_curve(0.75, &p);
        assert!(dark < 0.25, "dark {dark}");
        assert!(bright > 0.75, "bright {bright}");
    }

    #[test]
    fn positive_whites_raises_highlights_more_than_mids() {
        let p = FinishParams {
            whites: 1.0,
            ..Default::default()
        };
        assert!(tone_curve(0.9, &p) - 0.9 > tone_curve(0.5, &p) - 0.5);
    }

    #[test]
    fn positive_blacks_lifts_shadows() {
        let p = FinishParams {
            blacks: 1.0,
            ..Default::default()
        };
        assert!(tone_curve(0.1, &p) > 0.1);
    }

    #[test]
    fn positive_shadows_raises_shadows_more_than_mids() {
        let p = FinishParams {
            shadows: 1.0,
            ..Default::default()
        };
        assert!(tone_curve(0.25, &p) - 0.25 > tone_curve(0.6, &p) - 0.6);
    }

    #[test]
    fn positive_saturation_increases_chroma() {
        let p = FinishParams {
            saturation: 0.5,
            ..Default::default()
        };
        let px = [0.6, 0.4, 0.3];
        let out = apply_saturation(px, &p);
        let chroma_in = px[0] - px[2];
        let chroma_out = out[0] - out[2];
        assert!(chroma_out > chroma_in, "in {chroma_in} out {chroma_out}");
    }

    #[test]
    fn vibrance_affects_muted_more_than_vivid() {
        let p = FinishParams {
            vibrance: 1.0,
            ..Default::default()
        };
        let muted = [0.52, 0.50, 0.48];
        let vivid = [0.90, 0.10, 0.05];
        let chroma = |px: [f32; 3]| px[0].max(px[1]).max(px[2]) - px[0].min(px[1]).min(px[2]);
        let ratio = |px: [f32; 3]| chroma(apply_saturation(px, &p)) / chroma(px);
        // Vibrance boosts low-saturation (muted) pixels more than already-vivid ones.
        assert!(
            ratio(muted) > ratio(vivid),
            "muted {} vivid {}",
            ratio(muted),
            ratio(vivid)
        );
    }

    #[test]
    fn finish_image_default_returns_equal_image() {
        let src = img_from(vec![[0.2, 0.4, 0.6], [0.7, 0.5, 0.3]]);
        let out = finish_image(&src, &FinishParams::default());
        assert_eq!(out.width, src.width);
        assert_eq!(out.height, src.height);
        for (o, s) in out.pixels.iter().zip(src.pixels.iter()) {
            for c in 0..3 {
                assert!(
                    (o[c] - s[c]).abs() < 1e-4,
                    "c={c} out={} src={}",
                    o[c],
                    s[c]
                );
            }
        }
    }

    #[test]
    fn texture_zero_is_identity() {
        // A 5x5 ramp; texture=0 must return the same pixels (up to f32 round-trip).
        let mut px = Vec::new();
        for i in 0..25 {
            let v = i as f32 / 25.0;
            px.push([v, v, v]);
        }
        let img = Image {
            width: 5,
            height: 5,
            pixels: px.clone(),
            ir: None,
        };
        let out = finish_image(&img, &FinishParams::default());
        for (o, s) in out.pixels.iter().zip(px.iter()) {
            for c in 0..3 {
                assert!(
                    (o[c] - s[c]).abs() < 1e-5,
                    "c={c} out={} src={}",
                    o[c],
                    s[c]
                );
            }
        }
    }

    const ID: [[f32; 2]; 2] = [[0.0, 0.0], [1.0, 1.0]];

    #[test]
    fn region_shadows_lift_shadow_zone_not_mids() {
        // regions ordered [shadows, darks, lights, highlights]
        let (lr, _, _) = tone_luts([1.0, 0.0, 0.0, 0.0], &ID, &ID, &ID, &ID);
        let i_sh = (0.125 * 255.0) as usize;
        let i_mid = (0.5 * 255.0) as usize;
        assert!(lr[i_sh] > 0.125, "shadow zone lifted: {}", lr[i_sh]);
        assert!(
            (lr[i_mid] - 0.5).abs() < 0.02,
            "mid ~unchanged: {}",
            lr[i_mid]
        );
    }

    #[test]
    fn master_curve_lifts_all_channels() {
        let master = [[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]];
        let (lr, lg, lb) = tone_luts([0.0; 4], &master, &ID, &ID, &ID);
        let i = (0.5 * 255.0) as usize;
        assert!(
            lr[i] > 0.55 && lg[i] > 0.55 && lb[i] > 0.55,
            "{} {} {}",
            lr[i],
            lg[i],
            lb[i]
        );
    }

    #[test]
    fn red_curve_only_affects_red_lut() {
        let red = [[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]];
        let (lr, lg, lb) = tone_luts([0.0; 4], &ID, &red, &ID, &ID);
        let i = (0.5 * 255.0) as usize;
        assert!(lr[i] > 0.55, "red lifted: {}", lr[i]);
        assert!(
            (lg[i] - 0.5).abs() < 0.02 && (lb[i] - 0.5).abs() < 0.02,
            "g/b flat"
        );
    }

    #[test]
    fn color_grade_default_is_identity() {
        let cg = ColorGrade::default();
        for v in [0.1, 0.5, 0.9] {
            let out = color_grade([v, v, v], &cg);
            for (c, &oc) in out.iter().enumerate() {
                assert!((oc - v).abs() < 1e-5, "v={v} c={c} out={oc}");
            }
        }
    }

    #[test]
    fn shadow_wheel_tints_darks_more_than_brights() {
        // Red into shadows (hue 0, full sat), nothing elsewhere.
        let cg = ColorGrade::new(
            ([0.0, 1.0], 0.0),
            ([0.0, 0.0], 0.0),
            ([0.0, 0.0], 0.0),
            ([0.0, 0.0], 0.0),
            0.5,
            0.0,
        );
        let dark = color_grade([0.1, 0.1, 0.1], &cg);
        let bright = color_grade([0.9, 0.9, 0.9], &cg);
        assert!(
            dark[0] - 0.1 > (bright[0] - 0.9) + 1e-3,
            "dark reddened more"
        );
        assert!(dark[0] > dark[2], "dark is warmer (R>B)");
    }

    #[test]
    fn global_lum_raises_everything() {
        let cg = ColorGrade::new(
            ([0.0, 0.0], 0.0),
            ([0.0, 0.0], 0.0),
            ([0.0, 0.0], 0.0),
            ([0.0, 0.0], 1.0),
            0.5,
            0.0,
        );
        let out = color_grade([0.5, 0.5, 0.5], &cg);
        assert!(out[0] > 0.5 && out[1] > 0.5 && out[2] > 0.5, "{:?}", out);
    }

    #[test]
    fn finish_image_matches_scalar_per_pixel_no_texture() {
        // With texture == 0, finish_image is a pure per-pixel map; assert it matches
        // finish_pixel elementwise and in order (guards the parallel collect).
        let p = FinishParams {
            contrast: 0.4,
            saturation: 0.3,
            ..Default::default()
        };
        let pixels = vec![
            [0.6, 0.4, 0.3],
            [0.1, 0.7, 0.2],
            [0.9, 0.9, 0.1],
            [0.2, 0.2, 0.8],
        ];
        let img = Image {
            width: 4,
            height: 1,
            pixels: pixels.clone(),
            ir: None,
        };
        let out = finish_image(&img, &p);
        for (i, &px) in pixels.iter().enumerate() {
            let want = finish_pixel(px, &p);
            for (c, (&got, &exp)) in out.pixels[i].iter().zip(want.iter()).enumerate() {
                assert!((got - exp).abs() < 1e-5, "pixel {i} chan {c}");
            }
        }
    }

    #[test]
    fn finish_image_with_texture_is_stable_and_clamped() {
        // A non-flat image so blur differs from the source; texture > 0 exercises the
        // apply_texture zip-map path. Output must stay in [0,1] and be deterministic.
        let p = FinishParams {
            texture: 1.0,
            ..Default::default()
        };
        let pixels = vec![
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.2, 0.5, 0.8],
            [0.9, 0.1, 0.4],
        ];
        let img = Image {
            width: 2,
            height: 2,
            pixels,
            ir: None,
        };
        let a = finish_image(&img, &p);
        let b = finish_image(&img, &p);
        // assert_eq! (bitwise) is intentional: same binary + deterministic arithmetic,
        // so two back-to-back runs must produce identical bits.
        assert_eq!(a.pixels, b.pixels, "must be deterministic across runs");
        for px in &a.pixels {
            for &v in px.iter() {
                assert!((0.0..=1.0).contains(&v), "value {v} out of range");
            }
        }
    }

    #[test]
    fn rgb_hsl_round_trip() {
        let colors = [
            [0.2_f32, 0.4, 0.6],
            [0.9, 0.1, 0.3],
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.7, 0.7, 0.2],
        ];
        for c in colors {
            let (h, s, l) = rgb2hsl(c);
            let back = hsl2rgb(h, s, l);
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-4, "c={c:?} back={back:?}");
            }
        }
    }

    fn mix_with(set: impl Fn(&mut ColorMix)) -> ColorMix {
        let mut cm = ColorMix::default();
        set(&mut cm);
        cm
    }

    #[test]
    fn color_mix_default_is_identity() {
        let cm = ColorMix::default();
        for c in [[0.2_f32, 0.4, 0.6], [0.8, 0.2, 0.5], [0.5, 0.5, 0.5]] {
            let out = color_mix(c, &cm);
            for k in 0..3 {
                assert!((out[k] - c[k]).abs() < 1e-4, "c={c:?} out={out:?}");
            }
        }
    }

    #[test]
    fn mixer_band_isolation() {
        // Push the BLUE band saturation up; a pure-red pixel must be ~unchanged,
        // a blue pixel must gain chroma.
        let cm = mix_with(|m| m.cm_sat[5] = 1.0); // blue = index 5, slider +100 → unit 1.0
        let red = color_mix([0.8, 0.1, 0.1], &cm);
        assert!(
            (red[0] - 0.8).abs() < 0.02 && (red[1] - 0.1).abs() < 0.02,
            "red moved: {red:?}"
        );
        let blue_in = [0.2, 0.3, 0.8];
        let blue = color_mix(blue_in, &cm);
        let chroma = |p: [f32; 3]| {
            p.iter().cloned().fold(0.0_f32, f32::max) - p.iter().cloned().fold(1.0_f32, f32::min)
        };
        assert!(
            chroma(blue) > chroma(blue_in),
            "blue chroma: {} -> {}",
            chroma(blue_in),
            chroma(blue)
        );
    }

    #[test]
    fn mixer_gray_pixel_unaffected_by_hue() {
        let cm = mix_with(|m| {
            m.cm_hue[0] = 1.0;
            m.cm_hue[5] = 1.0;
        });
        let out = color_mix([0.5, 0.5, 0.5], &cm);
        for k in 0..3 {
            assert!((out[k] - 0.5).abs() < 1e-3, "gray moved: {out:?}");
        }
    }

    fn sample(hue: f32) -> PcSample {
        PcSample {
            hue,
            sat: 0.6,
            lum: 0.5,
            hue_shift: 0.0,
            sat_shift: 1.0,
            lum_shift: 0.0,
            variance: 0.0,
            range: 50.0,
        }
    }

    #[test]
    fn point_color_default_no_samples_is_identity() {
        let cm = ColorMix::default(); // no samples
        let c = [0.8, 0.2, 0.2];
        let out = point_color(c, &cm.samples);
        for k in 0..3 {
            assert!((out[k] - c[k]).abs() < 1e-4, "{out:?}");
        }
    }

    #[test]
    fn point_color_sample_isolation() {
        // Sample targets RED (hue 0); a red pixel gains chroma, a green pixel is untouched.
        let samples = vec![sample(0.0)];
        let chroma = |p: [f32; 3]| {
            p.iter().cloned().fold(0.0_f32, f32::max) - p.iter().cloned().fold(1.0_f32, f32::min)
        };
        let red_in = [0.8, 0.25, 0.25];
        let red = point_color(red_in, &samples);
        assert!(
            chroma(red) > chroma(red_in),
            "red chroma {} -> {}",
            chroma(red_in),
            chroma(red)
        );
        let green_in = [0.2, 0.8, 0.25];
        let green = point_color(green_in, &samples);
        for k in 0..3 {
            assert!(
                (green[k] - green_in[k]).abs() < 0.02,
                "green moved {green:?}"
            );
        }
    }

    #[test]
    fn point_color_order_independent() {
        let a = sample(0.0);
        let b = sample(120.0);
        let c = [0.6, 0.5, 0.3];
        let ab = point_color(c, &[a, b]);
        let ba = point_color(c, &[b, a]);
        for k in 0..3 {
            assert!((ab[k] - ba[k]).abs() < 1e-5, "order matters {ab:?} {ba:?}");
        }
    }

    #[test]
    fn finish_pixel_color_mixer_default_is_identity() {
        // Default FinishParams (no mixer, no samples) must leave pixels unchanged.
        let p = FinishParams::default();
        for v in [0.1_f32, 0.35, 0.7, 0.95] {
            let px = [v, v * 0.6, v * 0.3];
            let out = finish_pixel(px, &p);
            for c in 0..3 {
                assert!((out[c] - px[c]).abs() < 1e-4, "v={v} c={c} {out:?}");
            }
        }
    }

    #[test]
    fn positive_texture_increases_edge_contrast() {
        // Vertical step edge: left half 0.4, right half 0.6 (5x5).
        let mut px = Vec::new();
        for _y in 0..5 {
            for x in 0..5 {
                let v = if x < 2 { 0.4 } else { 0.6 };
                px.push([v, v, v]);
            }
        }
        let img = Image {
            width: 5,
            height: 5,
            pixels: px,
            ir: None,
        };
        let p = FinishParams {
            texture: 1.0,
            ..Default::default()
        };
        let out = finish_image(&img, &p);
        // The bright side of the edge (x=2) should be pushed brighter than its
        // flat-region neighbour (x=4).
        let edge = out.pixels[2 * 5 + 2][0];
        let flat = out.pixels[2 * 5 + 4][0];
        assert!(edge > flat, "edge {edge} flat {flat}");
    }
}
