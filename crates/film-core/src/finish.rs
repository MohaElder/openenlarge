//! Creative finishing layer, applied to the gamma-encoded positive produced by
//! the inversion core. All params are 0.0 = identity. Tone/saturation are
//! per-pixel; texture (Task 2) is a spatial unsharp pass.

use crate::curve::{curve_lut, sample_lut, LUT_SIZE};
use crate::engine::{HDR_HEADROOM, HDR_KNEE};
use crate::Image;
use rayon::prelude::*;

const EPS: f32 = 1e-5;
// --- Texture (unsharp / clarity) constants. Shared with shaders.ts USM_FRAG. ---
/// Blur sigma as a fraction of the image's *smaller* dimension. Defining the
/// radius in image-fraction (not pixels) keeps the spatial span identical between
/// the CPU full-res export and the GPU (often proxy-resolution) preview, which is
/// the parity contract for I3. ~0.0025 → ~6px sigma on a 2560px proxy, wide
/// enough that the high-pass survives the proxy downscale and the slider visibly
/// bites at both ends.
const TEXTURE_SIGMA_FRAC: f32 = 0.0025;
/// Cap the blur radius (in pixels) so huge exports stay bounded; mirrored by
/// MAXR in shaders.ts. Both paths clamp identically so the kernels match.
const TEXTURE_MAX_RADIUS: usize = 64;
/// Unsharp gain at texture = +1 (sharpen). Raised from the old 1.5 so the slider
/// reaches a clearly sharper / higher-local-contrast result.
const USM_POS_GAIN: f32 = 2.5;
/// Unsharp gain at texture = -1 (soften). 1.0 makes the output *exactly* the
/// blurred image at the extreme (out = v + (-1)·(v - blur) = blur), i.e. clearly
/// soft instead of the old barely-perceptible negative high-pass.
const USM_NEG_GAIN: f32 = 1.0;

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

// --- Per-zone white-balance correction. ---
// Corrective (not creative) gains measured on the inverted positive BEFORE the
// creative color grade. Zone edges mirror ColorGrade so the neutraliser and the
// artistic toning always divide the tonal range the same way. Mirrored in
// shaders.ts `applyPerZoneWb` (Task 10).
const PZ_SH_EDGE: f32 = 0.33;
const PZ_HI_EDGE: f32 = 0.66;
const PZ_SOFT: f32 = 0.25;

/// Corrective per-zone white-balance gains (shadows/mids/highlights), applied to
/// the inverted positive BEFORE the creative color grade. Separate from
/// `ColorGrade` so it never overwrites the user's artistic toning. Identity gains
/// + the `enabled = false` flag both render as a no-op (back-compat for old edits).
#[derive(Debug, Clone, Copy)]
pub struct PerZoneWb {
    pub enabled: bool,
    pub sh: [f32; 3],
    pub mid: [f32; 3],
    pub hi: [f32; 3],
}

impl Default for PerZoneWb {
    fn default() -> Self {
        PerZoneWb {
            enabled: true,
            sh: [1.0; 3],
            mid: [1.0; 3],
            hi: [1.0; 3],
        }
    }
}

/// Apply the per-zone WB correction: luma-keyed blend of the three zone gains,
/// then a per-channel multiply. Returns `rgb` unchanged when disabled or when all
/// gains are identity. Mirror in shaders.ts `applyPerZoneWb`.
pub fn apply_per_zone_wb(rgb: [f32; 3], pz: &PerZoneWb) -> [f32; 3] {
    if !pz.enabled {
        return rgb;
    }
    let l = luma(rgb);
    let w_sh = 1.0 - smoothstep(PZ_SH_EDGE - PZ_SOFT, PZ_SH_EDGE + PZ_SOFT, l);
    let w_hi = smoothstep(PZ_HI_EDGE - PZ_SOFT, PZ_HI_EDGE + PZ_SOFT, l);
    let w_mid = (1.0 - w_sh - w_hi).clamp(0.0, 1.0);
    std::array::from_fn(|c| {
        let gain = w_sh * pz.sh[c] + w_mid * pz.mid[c] + w_hi * pz.hi[c];
        // No [0,1] clamp: per-zone WB runs before tone_curve on the super-white body;
        // clamping here would re-clip the highlight headroom the tone tools recover.
        (rgb[c] * gain).max(0.0)
    })
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
    /// Brightness/density (−1..1; 0 = identity). A log-curve gain on the finished
    /// image applied BEFORE the tone curve, so equal steps = equal density. See
    /// [`brightness_gain`].
    pub brightness: f32,
    /// Composed tone-curve LUTs (per channel): channel(master(parametric(x))).
    pub lut_r: [f32; LUT_SIZE],
    pub lut_g: [f32; LUT_SIZE],
    pub lut_b: [f32; LUT_SIZE],
    pub cg: ColorGrade,
    pub cm: ColorMix,
    pub per_zone: PerZoneWb,
    /// When `true` (default), input to `tone_curve` is the Faithful gamma BODY
    /// (may exceed 1.0 = super-white); the function applies `display_finalize` at
    /// the end. When `false`, input is ALREADY display-referred (positive passthrough
    /// or HDR rendition) — the function reproduces the pre-headroom behavior (leading
    /// clamp + sliders + trailing clamp, NO display_finalize) so those paths are
    /// byte-identical to before the headroom-recovery feature.
    pub finalize_body: bool,
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
            brightness: 0.0,
            lut_r: identity_lut(),
            lut_g: identity_lut(),
            lut_b: identity_lut(),
            cg: ColorGrade::default(),
            cm: ColorMix::default(),
            per_zone: PerZoneWb::default(),
            finalize_body: true,
        }
    }
}

/// Brightness/density slider span. At the extremes (b = ±1) the gain is
/// `10^(±BRIGHTNESS_DENSITY_RANGE)` ≈ ×3.16 / ×0.32 (~±1.66 stops). MUST equal
/// `BRIGHTNESS_DENSITY_RANGE` in shaders.ts (FRAG) so the GPU proxy preview and
/// the CPU full-res export brighten identically.
pub const BRIGHTNESS_DENSITY_RANGE: f32 = 0.5;

/// Map the normalised brightness/density slider (−1..1) to a multiplicative gain
/// through a log (density) curve: equal slider steps → equal density steps.
fn brightness_gain(b: f32) -> f32 {
    10f32.powf(b * BRIGHTNESS_DENSITY_RANGE)
}

/// Per-channel parametric tone curve. Behavior controlled by `p.finalize_body`:
///
/// * `true` (Faithful-SDR-negative default): input is the Faithful gamma BODY (may
///   exceed 1.0 = super-white). NO leading clamp — the tone tools must see headroom
///   so negative Whites/Contrast can pull blown highlights back below the shoulder
///   and recover detail. `display_finalize` is applied at the end.
///
/// * `false` (positive passthrough / HDR rendition): input is ALREADY display-referred.
///   Reproduces the pre-headroom behavior exactly: leading clamp(0,1) + sliders +
///   trailing clamp(0,1), NO `display_finalize`. Those paths stay byte-identical to
///   before the headroom-recovery feature.
fn tone_curve(v: f32, p: &FinishParams) -> f32 {
    // finalize_body=true: input is the Faithful gamma BODY (may exceed 1.0); the tone
    // tools get headroom and we display-finalize at the end (highlight recovery).
    // finalize_body=false: input is ALREADY a display value (positive passthrough / HDR
    // rendition / non-Faithful) — reproduce the pre-headroom tone_curve EXACTLY (leading
    // clamp + trailing clamp, no display_finalize) so those paths are byte-identical to before.
    let mut v = if p.finalize_body { v } else { v.clamp(0.0, 1.0) };
    v += p.whites * 0.20 * v.powi(3);
    v += p.blacks * 0.20 * (1.0 - v).powi(3);
    v += p.highlights * 0.18 * smoothstep(0.5, 1.0, v);
    v += p.shadows * 0.18 * (1.0 - smoothstep(0.0, 0.5, v));
    v = 0.5 + (v - 0.5) * (1.0 + p.contrast);
    if p.finalize_body {
        crate::engine::display_finalize(v)
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// HDR finalize: a color-managed soft tone-shoulder that maps the finished
/// super-white body into `[HDR_KNEE, HDR_HEADROOM]` instead of the SDR hard clamp
/// to `[0,1]` or the Faithful `display_finalize` roll-off to `1.0`. This is the
/// TESTED REFERENCE the MSL (Metal) finalize shader mirrors, and lets the
/// CPU/gain-map path use this principled curve instead of the old
/// split-body+excess workaround.
///
/// Below `HDR_KNEE`: identity, so SDR-range tones are unchanged (matches the
/// existing SDR/Faithful body exactly up to the knee). Above `HDR_KNEE`: a
/// `tanh` roll-off compresses the (unbounded) remaining super-white range into
/// `[HDR_KNEE, HDR_HEADROOM)` — monotonic, continuous AND C1-smooth at the knee
/// (tanh'(0) = 1, matching the identity segment's slope, so there is no visible
/// kink), and asymptotically approaches but never reaches `HDR_HEADROOM`.
///
/// This is a *scalar* building block operating on one channel (or a shared
/// luminance/max-channel factor — see [`hdr_finalize_rgb`]). Curving each RGB
/// channel independently with this function would skew hue in the highlights
/// (each channel compresses by a different amount once channels diverge above
/// the knee); callers that need hue-preserving highlights on real pixels MUST
/// go through `hdr_finalize_rgb`, which applies this curve once to a shared
/// ratio and scales all three channels by it together.
///
/// Not yet called from the runtime finalize path (that wiring — replacing the
/// old split-body+excess CPU/gain-map workaround and mirroring in MSL — is a
/// separate follow-up task); this is the tested reference for that work, so
/// it is only exercised by tests today.
#[inline]
#[allow(dead_code)]
pub(crate) fn hdr_finalize(v: f32) -> f32 {
    if v <= HDR_KNEE {
        v
    } else {
        let span = HDR_HEADROOM - HDR_KNEE;
        HDR_KNEE + span * ((v - HDR_KNEE) / span).tanh()
    }
}

/// Hue-preserving HDR finalize for a full RGB pixel: computes the compression
/// ratio from the max channel (the highlight driver) via [`hdr_finalize`], then
/// scales all three channels by that SAME ratio — so a super-white highlight
/// compresses in brightness without any channel drifting relative to the
/// others (no hue skew), unlike applying `hdr_finalize` independently per
/// channel. Below the knee the ratio is exactly 1.0 (identity), matching
/// `hdr_finalize`'s own identity segment. `max(rgb) <= 0` (black or negative)
/// passes through unchanged — there is no highlight to compress.
///
/// Not yet called from the runtime finalize path; see [`hdr_finalize`]'s note.
#[inline]
#[allow(dead_code)]
pub(crate) fn hdr_finalize_rgb(rgb: [f32; 3]) -> [f32; 3] {
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m <= EPS {
        return rgb;
    }
    let ratio = hdr_finalize(m) / m;
    std::array::from_fn(|c| rgb[c] * ratio)
}

// --- OKLab perceptual saturation (replaces the display-space cube stretch). ---
// Chroma is scaled in OKLab so pushes enrich colour without per-channel clipping
// (which twists hue → neon). Luma (L) is held fixed; near-neutrals and skin hues are
// protected; out-of-gamut chroma is compressed along the (gray→colour) line back to
// the boundary. MUST be mirrored in shaders.ts (FRAG/finishAt).
const SAT_C_REF: f32 = 0.20; // OKLab chroma treated as "fully saturated" (vibrance weight)
const SAT_C_NEUTRAL: f32 = 0.025; // boost ramps from 0 below this chroma (protect neutrals)
const SKIN_HUE: f32 = 0.70; // OKLab hue (rad) at skin/orange
const SKIN_WIDTH: f32 = 0.55; // half-window (rad) of the skin damp
const SKIN_DAMP: f32 = 0.5; // max boost reduction inside the skin window

#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
#[inline]
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}
#[inline]
fn linear_to_oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    ]
}
#[inline]
fn oklab_to_linear(lab: [f32; 3]) -> [f32; 3] {
    let l_ = lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2];
    let m_ = lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2];
    let s_ = lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2];
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}
/// Angular distance on the hue circle, in radians [0, π].
#[inline]
fn hue_dist(a: f32, b: f32) -> f32 {
    let tau = 2.0 * std::f32::consts::PI;
    let mut d = (a - b).rem_euclid(tau);
    if d > std::f32::consts::PI {
        d = tau - d;
    }
    d
}

/// Perceptual saturation in OKLab: scale chroma (luma fixed), protect neutrals + skin,
/// and compress any out-of-gamut result along the hue line instead of clipping per
/// channel. `saturation` is uniform; `vibrance` is weighted toward muted pixels.
/// Identity when both are 0. Mirrored in shaders.ts::finishAt.
fn apply_saturation(rgb: [f32; 3], p: &FinishParams) -> [f32; 3] {
    if p.saturation.abs() < EPS && p.vibrance.abs() < EPS {
        return rgb;
    }
    let lin = [
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
    ];
    let lab = linear_to_oklab(lin[0], lin[1], lin[2]);
    let (l, a, b) = (lab[0], lab[1], lab[2]);
    let c = (a * a + b * b).sqrt();
    if c < EPS {
        return rgb; // pure neutral: nothing to saturate
    }
    let h = b.atan2(a);
    // Boost: saturation uniform; vibrance weighted toward muted (low-chroma) pixels.
    let vib_w = 1.0 - (c / SAT_C_REF).clamp(0.0, 1.0);
    let mut gain = p.saturation + p.vibrance * vib_w;
    // Protect near-neutrals and skin hues from the boost.
    let neutral = smoothstep(0.0, SAT_C_NEUTRAL, c);
    let skin = 1.0 - SKIN_DAMP * smoothstep(SKIN_WIDTH, 0.0, hue_dist(h, SKIN_HUE));
    gain *= neutral * skin;
    let scale = (1.0 + gain).max(0.0);
    let lab2 = [l, a * scale, b * scale];
    // Hue-preserving gamut compression: find the largest fraction `tg` of the
    // (gray → boosted colour) segment that stays in [0,1] on every channel. The
    // achromatic point (L,0,0) is in gamut, so tg ≥ 0 always exists.
    let gray = oklab_to_linear([l, 0.0, 0.0]);
    let col = oklab_to_linear(lab2);
    let mut tg = 1.0_f32;
    for ch in 0..3 {
        let (g0, c0) = (gray[ch], col[ch]);
        if c0 > 1.0 {
            tg = tg.min((1.0 - g0) / (c0 - g0));
        } else if c0 < 0.0 {
            tg = tg.min(g0 / (g0 - c0));
        }
    }
    let tg = tg.clamp(0.0, 1.0);
    let out_lin: [f32; 3] =
        std::array::from_fn(|ch| (gray[ch] + (col[ch] - gray[ch]) * tg).clamp(0.0, 1.0));
    [
        linear_to_srgb(out_lin[0]),
        linear_to_srgb(out_lin[1]),
        linear_to_srgb(out_lin[2]),
    ]
}

/// Per-pixel finishing. Order: Per-zone WB correction → Basic tone curve +
/// saturation → Tone Curve LUT → Color Grading. (Texture is a separate spatial
/// pass in `finish_image`.)
pub fn finish_pixel(rgb: [f32; 3], p: &FinishParams) -> [f32; 3] {
    // Per-zone WB runs on the body before tone_curve; the no-[0,1]-clamp keeps super-white
    // intact (the estimator's display-domain gains are applied here as a small residual
    // correction — see headroom-tone-recovery known-limitation). Identity gains = no-op.
    let rgb = apply_per_zone_wb(rgb, &p.per_zone);
    // Brightness/density: a log-curve gain applied before everything else, so it
    // reads as an overall lift/density move that Contrast then pivots about.
    let g = brightness_gain(p.brightness);
    let toned = [
        tone_curve(rgb[0] * g, p),
        tone_curve(rgb[1] * g, p),
        tone_curve(rgb[2] * g, p),
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

/// Blur sigma (px) and radius (px) for an image of these dims. Sigma scales with
/// the smaller dimension (image-fraction, see `TEXTURE_SIGMA_FRAC`); radius is
/// 3σ, clamped so it never collapses to 0 or exceeds `TEXTURE_MAX_RADIUS`. The
/// GPU (shaders.ts) derives the same values from its viewport, so a full-res CPU
/// export and a proxy GPU preview blur the same fraction of the frame.
fn texture_blur(w: usize, h: usize) -> (f32, usize) {
    let sigma = (TEXTURE_SIGMA_FRAC * (w.min(h) as f32)).max(0.5);
    let radius = ((3.0 * sigma).ceil() as usize).clamp(1, TEXTURE_MAX_RADIUS);
    (sigma, radius)
}

/// Normalised 1-D Gaussian over [-radius, radius]. Matches the in-shader weights.
fn gaussian_kernel(sigma: f32, radius: usize) -> Vec<f32> {
    let inv = 1.0 / (2.0 * sigma * sigma);
    let r = radius as i32;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 * inv).exp()).collect();
    let sum: f32 = k.iter().sum();
    for w in &mut k {
        *w /= sum;
    }
    k
}

/// Separable Gaussian blur. Edges clamp (CLAMP_TO_EDGE on the GPU). Sigma/radius
/// come from `texture_blur` so the span is resolution-independent.
fn blur(img: &Image, sigma: f32, radius: usize) -> Image {
    let (w, h) = (img.width, img.height);
    let r = radius as i32;
    let kernel = gaussian_kernel(sigma, radius);
    let idx = |x: usize, y: usize| y * w + x;
    // Horizontal pass.
    let mut tmp = vec![[0.0_f32; 3]; w * h];
    tmp.par_iter_mut().enumerate().for_each(|(p, out)| {
        let (x, y) = (p % w, p / w);
        let mut acc = [0.0_f32; 3];
        for (j, &kw) in kernel.iter().enumerate() {
            let xx = (x as i32 + j as i32 - r).clamp(0, w as i32 - 1) as usize;
            let s = img.pixels[idx(xx, y)];
            for c in 0..3 {
                acc[c] += kw * s[c];
            }
        }
        *out = acc;
    });
    // Vertical pass.
    let mut out = vec![[0.0_f32; 3]; w * h];
    out.par_iter_mut().enumerate().for_each(|(p, o)| {
        let (x, y) = (p % w, p / w);
        let mut acc = [0.0_f32; 3];
        for (j, &kw) in kernel.iter().enumerate() {
            let yy = (y as i32 + j as i32 - r).clamp(0, h as i32 - 1) as usize;
            let s = tmp[idx(x, yy)];
            for c in 0..3 {
                acc[c] += kw * s[c];
            }
        }
        *o = acc;
    });
    Image {
        width: w,
        height: h,
        pixels: out,
        ir: None,
    } // scratch image: ir restored by apply_texture
}

/// Unsharp mask: out = v + k·(v − blur(v)). `amount` in −1..1; k is asymmetric so
/// +1 strongly sharpens and −1 lands exactly on the blur (clearly soft).
fn apply_texture(img: &Image, amount: f32) -> Image {
    let (sigma, radius) = texture_blur(img.width, img.height);
    let b = blur(img, sigma, radius);
    let k = if amount >= 0.0 {
        amount * USM_POS_GAIN
    } else {
        amount * USM_NEG_GAIN
    };
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

    fn chroma_of(rgb: [f32; 3]) -> f32 {
        let lin = [
            srgb_to_linear(rgb[0]),
            srgb_to_linear(rgb[1]),
            srgb_to_linear(rgb[2]),
        ];
        let lab = linear_to_oklab(lin[0], lin[1], lin[2]);
        (lab[1] * lab[1] + lab[2] * lab[2]).sqrt()
    }
    /// Build a display-space pixel from an OKLab (L, C, hue) triple.
    fn px_from_lch(l: f32, c: f32, h: f32) -> [f32; 3] {
        let lab = [l, c * h.cos(), c * h.sin()];
        let lin = oklab_to_linear(lab);
        std::array::from_fn(|i| linear_to_srgb(lin[i].clamp(0.0, 1.0)))
    }

    #[test]
    fn oklab_saturation_identity_at_zero() {
        let px = [0.42_f32, 0.55, 0.30];
        let p = FinishParams {
            saturation: 0.0,
            vibrance: 0.0,
            ..Default::default()
        };
        assert_eq!(apply_saturation(px, &p), px);
    }

    #[test]
    fn oklab_saturation_raises_chroma() {
        let px = [0.55_f32, 0.40, 0.30];
        let p = FinishParams {
            saturation: 0.5,
            ..Default::default()
        };
        let out = apply_saturation(px, &p);
        assert!(
            chroma_of(out) > chroma_of(px) + 1e-3,
            "{} !> {}",
            chroma_of(out),
            chroma_of(px)
        );
    }

    #[test]
    fn oklab_saturation_stays_in_gamut_under_heavy_push() {
        let px = [0.60_f32, 0.35, 0.25];
        let p = FinishParams {
            saturation: 3.0,
            ..Default::default()
        };
        let out = apply_saturation(px, &p);
        for c in 0..3 {
            assert!(
                (0.0..=1.0).contains(&out[c]),
                "channel {c} out of gamut: {out:?}"
            );
        }
    }

    #[test]
    fn oklab_saturation_preserves_neutral() {
        let px = [0.5_f32, 0.5, 0.5];
        let p = FinishParams {
            saturation: 1.0,
            ..Default::default()
        };
        let out = apply_saturation(px, &p);
        for c in 0..3 {
            assert!((out[c] - 0.5).abs() < 0.02, "neutral drifted: {out:?}");
        }
    }

    #[test]
    fn oklab_saturation_damps_skin() {
        // Two pixels with equal lightness + chroma, one at the skin hue, one opposite.
        // The skin pixel must gain LESS chroma than the non-skin pixel.
        let l = 0.7_f32;
        let c0 = 0.06_f32;
        let skin = px_from_lch(l, c0, SKIN_HUE);
        let other = px_from_lch(l, c0, SKIN_HUE + std::f32::consts::PI);
        let p = FinishParams {
            saturation: 1.0,
            ..Default::default()
        };
        let skin_ratio = chroma_of(apply_saturation(skin, &p)) / chroma_of(skin);
        let other_ratio = chroma_of(apply_saturation(other, &p)) / chroma_of(other);
        assert!(
            skin_ratio < other_ratio - 1e-3,
            "skin {skin_ratio} not damped vs {other_ratio}"
        );
    }

    fn img_from(pixels: Vec<[f32; 3]>) -> Image {
        Image {
            width: pixels.len(),
            height: 1,
            pixels,
            ir: None,
        }
    }

    #[test]
    fn default_is_display_finalize() {
        // Input domain changed: finish now operates on the Faithful body (not display space).
        // Default finish is no longer an identity passthrough — it applies display_finalize
        // per channel (tone tools are identity at 0, saturation/grade/LUT/mix are all no-ops).
        let p = FinishParams::default();
        for v in [0.0_f32, 0.2, 0.5, 0.8, 1.0] {
            let px = [v, v * 0.5, v * 0.25];
            let out = finish_pixel(px, &p);
            for c in 0..3 {
                let want = crate::engine::display_finalize(px[c]);
                assert!((out[c] - want).abs() < 1e-4, "v={v} c={c} out={} want={want}", out[c]);
            }
        }
    }

    #[test]
    fn brightness_is_a_density_gain_about_zero() {
        // 0 = identity; >0 lifts a mid-gray, <0 lowers it; the move is a log/density
        // gain (10^(b·RANGE)) applied before the tone curve. Default finish now applies
        // display_finalize, so the reference is display_finalize of the brightness-scaled body.
        let mid = [0.3_f32, 0.3, 0.3];
        let id = finish_pixel(mid, &FinishParams::default());
        // brightness=0 → gain=1 → body is 0.3 → display_finalize(0.3)
        assert!((id[0] - crate::engine::display_finalize(0.3)).abs() < 1e-4, "0 = display_finalize(0.3)");
        let up = finish_pixel(
            mid,
            &FinishParams {
                brightness: 0.5,
                ..Default::default()
            },
        );
        let down = finish_pixel(
            mid,
            &FinishParams {
                brightness: -0.5,
                ..Default::default()
            },
        );
        assert!(up[0] > 0.3, "+brightness lifts: {}", up[0]);
        assert!(down[0] < 0.3, "-brightness lowers: {}", down[0]);
        // Density curve: gain at b=0.5 is 10^(0.5·RANGE); check the lifted mid matches
        // display_finalize of the brightness-scaled body (the finalizer is now part of tone_curve).
        let g = 10f32.powf(0.5 * BRIGHTNESS_DENSITY_RANGE);
        assert!((up[0] - crate::engine::display_finalize(0.3 * g)).abs() < 1e-4, "gain {g}: {}", up[0]);
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
    fn negative_highlights_pull_down_near_white() {
        // Highlights must actually reach the brightest (near-clipped) tones — the
        // old v²(1−v) weight vanished at v→1, so the slider couldn't recover blown
        // highlights. The shelf weight peaks at white.
        let p = FinishParams {
            highlights: -1.0,
            ..Default::default()
        };
        assert!(
            tone_curve(0.97, &p) < 0.90,
            "near-white pulled down: {}",
            tone_curve(0.97, &p)
        );
    }

    #[test]
    fn positive_shadows_lift_near_black() {
        // Symmetric: shadows must reach near-black tones (old (1−v)²v vanished at v→0).
        let p = FinishParams {
            shadows: 1.0,
            ..Default::default()
        };
        assert!(
            tone_curve(0.03, &p) > 0.10,
            "near-black lifted: {}",
            tone_curve(0.03, &p)
        );
    }

    #[test]
    fn tone_curve_monotonic_under_extreme_sliders() {
        // Even with opposing endpoint+region sliders maxed, the curve must not fold
        // (a non-monotonic tone curve inverts tones — a visible artifact).
        for combo in [
            FinishParams {
                shadows: 1.0,
                blacks: 1.0,
                ..Default::default()
            },
            FinishParams {
                highlights: -1.0,
                whites: -1.0,
                ..Default::default()
            },
            FinishParams {
                highlights: 1.0,
                shadows: 1.0,
                whites: 1.0,
                blacks: 1.0,
                contrast: 1.0,
                ..Default::default()
            },
        ] {
            let mut prev = tone_curve(0.0, &combo);
            let mut v = 0.0;
            while v <= 1.0 {
                let cur = tone_curve(v, &combo);
                assert!(cur >= prev - 1e-4, "fold at v={v}: {cur} < {prev}");
                prev = cur;
                v += 1.0 / 256.0;
            }
        }
    }

    #[test]
    fn tone_curve_default_is_display_finalize() {
        // At sliders 0, tone_curve must equal the moved display finalize (identity invariant).
        let p = FinishParams::default();
        for &v in &[0.0_f32, 0.3, 0.8, 1.0, 1.4, 2.0] {
            let got = tone_curve(v, &p);
            let want = crate::engine::display_finalize(v);
            assert!((got - want).abs() < 1e-6, "v={v}: {got} vs {want}");
        }
    }

    #[test]
    fn lowering_whites_recovers_super_white_detail() {
        // Two distinct super-white bodies collapse to ~white at sliders 0, but lowering
        // Whites pulls them apart below the shoulder → recovered, distinct, lower values.
        let mut lo = FinishParams::default();
        lo.whites = -1.0;                          // wire range is /100; -1.0 == slider -100
        let (a, b) = (1.25_f32, 1.45_f32);
        let da0 = (tone_curve(a, &FinishParams::default()) - tone_curve(b, &FinishParams::default())).abs();
        let da1 = (tone_curve(a, &lo) - tone_curve(b, &lo)).abs();
        assert!(tone_curve(a, &lo) < tone_curve(a, &FinishParams::default()), "whites<0 must darken highlights");
        assert!(da1 > da0 + 1e-4, "lower whites must re-separate detail: {da0} -> {da1}");
    }

    #[test]
    fn positive_saturation_increases_chroma() {
        let px = [0.6_f32, 0.4, 0.3];
        let p = FinishParams {
            saturation: 0.5,
            ..Default::default()
        };
        assert!(chroma_of(apply_saturation(px, &p)) > chroma_of(px));
    }

    #[test]
    fn vibrance_affects_muted_more_than_vivid() {
        let p = FinishParams {
            vibrance: 1.0,
            ..Default::default()
        };
        let muted = [0.52_f32, 0.50, 0.48];
        let vivid = [0.90_f32, 0.10, 0.05];
        // Vibrance boosts low-chroma (muted) pixels more than already-vivid ones,
        // measured as chroma ratio in OKLab.
        let ratio = |px: [f32; 3]| chroma_of(apply_saturation(px, &p)) / chroma_of(px);
        assert!(
            ratio(muted) > ratio(vivid),
            "muted ratio {} vivid ratio {}",
            ratio(muted),
            ratio(vivid)
        );
    }

    #[test]
    fn finish_image_default_applies_display_finalize() {
        // Default finish is no longer identity: it applies display_finalize per channel
        // (saturation/grade/LUT/mix are all no-ops at defaults; tone tools are zero).
        let src = img_from(vec![[0.2, 0.4, 0.6], [0.7, 0.5, 0.3]]);
        let out = finish_image(&src, &FinishParams::default());
        assert_eq!(out.width, src.width);
        assert_eq!(out.height, src.height);
        for (o, s) in out.pixels.iter().zip(src.pixels.iter()) {
            for c in 0..3 {
                let want = crate::engine::display_finalize(s[c]);
                assert!(
                    (o[c] - want).abs() < 1e-4,
                    "c={c} out={} want={want}",
                    o[c]
                );
            }
        }
    }

    #[test]
    fn texture_zero_skips_usm_pass() {
        // texture=0 must not apply the USM pass; output must equal per-pixel finish_pixel.
        // Default finish now applies display_finalize, so we compare against that (not source).
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
        let p = FinishParams::default();
        let out = finish_image(&img, &p);
        for (o, s) in out.pixels.iter().zip(px.iter()) {
            for c in 0..3 {
                // At default params (neutral input), finish_pixel = display_finalize per channel.
                let want = crate::engine::display_finalize(s[c]);
                assert!(
                    (o[c] - want).abs() < 1e-5,
                    "c={c} out={} want={want}",
                    o[c]
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
    fn texture_minus_one_equals_blur() {
        // At texture = -1 the unsharp collapses to the blurred image (k = -1 →
        // v + -1·(v - blur) = blur), i.e. clearly soft. finish_image applies
        // finish_pixel first (which applies display_finalize at defaults), then blurs.
        // So `want` is blur of the display_finalize-mapped source, not the raw source.
        let p = FinishParams {
            texture: -1.0,
            ..Default::default()
        };
        // 8x8 ramp so the blur genuinely differs from the source.
        let (w, h) = (8usize, 8usize);
        let pixels: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let v = (i % w) as f32 / (w as f32 - 1.0);
                [v, 1.0 - v, 0.5]
            })
            .collect();
        let img = Image {
            width: w,
            height: h,
            pixels,
            ir: None,
        };
        let out = finish_image(&img, &p);
        let (sigma, radius) = texture_blur(w, h);
        // finish_pixel at defaults applies display_finalize per channel; the texture
        // pass then blurs that output. So want = blur(display_finalize(source)).
        let finished = Image {
            width: w,
            height: h,
            pixels: img.pixels.iter().map(|&px| {
                std::array::from_fn(|c| crate::engine::display_finalize(px[c]))
            }).collect(),
            ir: None,
        };
        let want = blur(&finished, sigma, radius);
        for (i, (&got, &exp)) in out.pixels.iter().zip(want.pixels.iter()).enumerate() {
            for c in 0..3 {
                assert!(
                    (got[c] - exp[c]).abs() < 1e-5,
                    "px {i} chan {c}: got {} want {}",
                    got[c],
                    exp[c]
                );
            }
        }
    }

    #[test]
    fn texture_blur_radius_scales_with_image() {
        // Image-fraction sigma: a larger frame gets a proportionally wider blur,
        // clamped to [1, TEXTURE_MAX_RADIUS]. This is what keeps a full-res export
        // and a proxy preview spatially matched.
        let (_, small) = texture_blur(200, 300);
        let (_, big) = texture_blur(4000, 6000);
        assert!(small >= 1);
        assert!(big > small, "big {big} should exceed small {small}");
        assert!(big <= TEXTURE_MAX_RADIUS);
        let (_, huge) = texture_blur(60_000, 60_000);
        assert_eq!(huge, TEXTURE_MAX_RADIUS, "radius must cap");
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
    fn finish_pixel_color_mixer_default_applies_display_finalize() {
        // Default FinishParams (no mixer, no samples) must apply display_finalize per channel
        // (the mixer/saturation/grade/LUT are all identity at defaults; tone tools are zero).
        let p = FinishParams::default();
        for v in [0.1_f32, 0.35, 0.7, 0.95] {
            let px = [v, v * 0.6, v * 0.3];
            let out = finish_pixel(px, &p);
            for c in 0..3 {
                let want = crate::engine::display_finalize(px[c]);
                assert!((out[c] - want).abs() < 1e-4, "v={v} c={c} out={} want={want}", out[c]);
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

    #[test]
    fn tone_curve_unfinalized_matches_legacy_clamped_curve() {
        // finalize_body=false must reproduce the pre-headroom tone_curve exactly:
        // leading clamp(0,1) + same slider ops + trailing clamp(0,1), NO display_finalize.
        let mut p = FinishParams::default();
        p.finalize_body = false;
        p.whites = -0.5; p.contrast = 0.3; p.highlights = 0.4; p.shadows = -0.2; p.blacks = 0.1;
        let legacy = |v: f32, p: &FinishParams| -> f32 {
            let mut v = v.clamp(0.0, 1.0);
            v += p.whites * 0.20 * v.powi(3);
            v += p.blacks * 0.20 * (1.0 - v).powi(3);
            v += p.highlights * 0.18 * smoothstep(0.5, 1.0, v);
            v += p.shadows * 0.18 * (1.0 - smoothstep(0.0, 0.5, v));
            v = 0.5 + (v - 0.5) * (1.0 + p.contrast);
            v.clamp(0.0, 1.0)
        };
        for &v in &[0.0_f32, 0.2, 0.5, 0.8, 1.0, 1.5, 2.5] {
            assert!((tone_curve(v, &p) - legacy(v, &p)).abs() < 1e-7, "v={v}");
        }
    }

    #[test]
    fn tone_curve_finalized_differs_from_unfinalized_in_highlights() {
        // The two modes must genuinely differ (else the flag is vacuous): a super-white
        // input finalizes (rolls off below 1.0) vs. clamps (hard 1.0).
        // At v=1.1 the Faithful shoulder maps to ~0.991 while the clamped path gives 1.0
        // (difference ~0.009 >> 1e-3).
        let mut pf = FinishParams::default(); pf.finalize_body = true;
        let mut pu = FinishParams::default(); pu.finalize_body = false;
        assert!((tone_curve(1.1, &pf) - tone_curve(1.1, &pu)).abs() > 1e-3);
    }

    #[test]
    fn hdr_finalize_preserves_and_ceilings_superwhite() {
        // Below the knee: identity (matches SDR body).
        assert!((hdr_finalize(0.5) - 0.5).abs() < 1e-6);
        // At/above white: stays > 1.0 (super-white preserved), monotonic, and never exceeds the headroom ceiling.
        let a = hdr_finalize(1.2);
        let b = hdr_finalize(4.0);
        assert!(a > 1.0, "1.2 -> {a}");
        assert!(b > a, "monotonic: {a} !< {b}");
        assert!(b <= HDR_HEADROOM + 1e-4, "ceiling: {b}");
        // Continuity at the knee (no visible kink).
        let k = HDR_KNEE;
        assert!((hdr_finalize(k + 1e-4) - hdr_finalize(k - 1e-4)).abs() < 1e-2);
    }

    #[test]
    fn hdr_finalize_rgb_is_identity_below_knee_and_hue_preserving_above() {
        // Below the knee: exact identity (matches the scalar `hdr_finalize`, and
        // matches the SDR/Faithful body for in-range tones).
        let sdr = [0.2_f32, 0.5, 0.79];
        assert_eq!(hdr_finalize_rgb(sdr), sdr);
        // Above the knee: an off-white highlight (max channel drives the
        // compression) must scale ALL channels by the identical ratio, so the
        // per-channel proportions — and therefore hue — are unchanged.
        let hi = [1.6_f32, 1.2, 0.4];
        let m = hi.iter().cloned().fold(f32::MIN, f32::max);
        let out = hdr_finalize_rgb(hi);
        let ratio = hdr_finalize(m) / m;
        for c in 0..3 {
            assert!(
                (out[c] - hi[c] * ratio).abs() < 1e-6,
                "channel {c}: {} != {}",
                out[c],
                hi[c] * ratio
            );
        }
        // The compression genuinely did something (max channel was above the knee).
        assert!(ratio < 1.0, "ratio {ratio}");
        // Black / non-positive-max input passes through unchanged (no highlight to compress).
        assert_eq!(hdr_finalize_rgb([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn per_zone_default_is_identity_in_finish_pixel() {
        // Regression: applying default PerZoneWb must be a true no-op — the output of
        // apply_per_zone_wb with identity gains equals the input exactly, and
        // finish_pixel with default FinishParams (which contains default PerZoneWb)
        // must yield the same result as the pre-feature path.
        let probe = [0.35_f32, 0.55, 0.20];
        let out_default = finish_pixel(probe, &FinishParams::default());
        // Manually bypass per_zone (identity) and run the rest: apply_per_zone_wb
        // with identity gains must return probe unchanged.
        let pz_out = apply_per_zone_wb(probe, &PerZoneWb::default());
        for c in 0..3 {
            assert!(
                (pz_out[c] - probe[c]).abs() < 1e-6,
                "apply_per_zone_wb identity not no-op ch{c}: {pz_out:?}"
            );
        }
        // And finish_pixel with default params uses identity per_zone, so same result.
        let out_identity = finish_pixel(
            pz_out,
            &FinishParams {
                per_zone: PerZoneWb::default(),
                ..Default::default()
            },
        );
        for c in 0..3 {
            assert!(
                (out_default[c] - out_identity[c]).abs() < 1e-6,
                "default FinishParams not identity ch{c}: default={} identity={}",
                out_default[c],
                out_identity[c]
            );
        }
    }
}

#[cfg(test)]
mod per_zone_apply_tests {
    use super::*;

    #[test]
    fn identity_gains_are_noop() {
        let pz = PerZoneWb::default();
        let rgb = [0.2, 0.5, 0.8];
        let out = apply_per_zone_wb(rgb, &pz);
        for c in 0..3 {
            assert!((out[c] - rgb[c]).abs() < 1e-6, "noop ch{c}: {out:?}");
        }
    }

    #[test]
    fn disabled_is_noop_even_with_gains() {
        let pz = PerZoneWb {
            enabled: false,
            sh: [1.2, 1.0, 0.8],
            mid: [1.1, 1.0, 0.9],
            hi: [0.8, 1.2, 0.9],
        };
        let rgb = [0.2, 0.5, 0.8];
        let out = apply_per_zone_wb(rgb, &pz);
        for c in 0..3 {
            assert!((out[c] - rgb[c]).abs() < 1e-6, "disabled ch{c}: {out:?}");
        }
    }

    #[test]
    fn highlight_gain_affects_bright_pixel_not_dark() {
        // A highlight-only green boost must move a bright pixel's green but leave a
        // shadow pixel ~unchanged (luma-keyed weighting).
        let pz = PerZoneWb {
            enabled: true,
            sh: [1.0; 3],
            mid: [1.0; 3],
            hi: [1.0, 1.2, 1.0],
        };
        let bright = apply_per_zone_wb([0.9, 0.9, 0.9], &pz);
        let dark = apply_per_zone_wb([0.05, 0.05, 0.05], &pz);
        assert!(
            bright[1] > 0.9 + 1e-3,
            "highlight green not boosted: {bright:?}"
        );
        assert!(
            (dark[1] - 0.05).abs() < 1e-2,
            "shadow wrongly changed: {dark:?}"
        );
    }
}
