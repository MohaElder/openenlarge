//! Density-domain negative inversion.
//!
//! Mode B (density matrix):  Ĉ = M_post · log10(M_pre · (base / I))  then tone.
//! Mode C (naive per-chan):  per-channel log-density, no matrices.
//! Mode "naive flip":        1 - normalized, the strawman baseline.

use nalgebra::{Matrix3, Vector3};
use rayon::prelude::*;

/// All knobs for one inversion. Defaults give a reasonable neutral result.
#[derive(Debug, Clone)]
pub struct InversionParams {
    /// Per-channel film-base value (orange mask), from calibrate::sample_base.
    pub base: [f32; 3],
    /// Pre-log linear mix (sensor↔dye crosstalk). Default = identity.
    pub m_pre: Matrix3<f32>,
    /// Post-log density-space unmix. Default = identity.
    pub m_post: Matrix3<f32>,
    /// Exposure multiplier applied after unmix.
    pub exposure: f32,
    /// Black point subtracted (post-exposure), in [0,1)-ish density-output units.
    pub black: f32,
    /// Output gamma encoding exponent (sRGB-ish ~ 1/2.2 applied as power).
    pub gamma: f32,
    /// Per-channel white-balance gain applied in linear light before gamma.
    pub wb: [f32; 3],
}

impl Default for InversionParams {
    fn default() -> Self {
        InversionParams {
            base: [1.0, 1.0, 1.0],
            m_pre: Matrix3::identity(),
            m_post: Matrix3::identity(),
            exposure: 1.0,
            black: 0.0,
            gamma: 1.0 / 2.2,
            wb: [1.0, 1.0, 1.0],
        }
    }
}

const EPS: f32 = 1e-5;

/// Naive baseline: normalize against base, then invert by `1 - x`. No log, no
/// matrices. This is the strawman the density engine must beat.
pub fn invert_naive(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    std::array::from_fn(|c| {
        let norm = (rgb[c] / p.base[c].max(EPS)).clamp(0.0, 1.0);
        1.0 - norm
    })
}

/// Apply white-balance gain, exposure, black point, and output gamma to a linear
/// density-output value.
fn tone(v: f32, gain: f32, p: &InversionParams) -> f32 {
    let v = (v * p.exposure * gain - p.black).max(0.0);
    v.powf(p.gamma)
}

/// Mode C: per-channel log-density. density = log10(base / I); higher film
/// density (less transmission) → brighter positive. Normalized by base density.
pub fn invert_c(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    std::array::from_fn(|c| {
        let t = (rgb[c] / p.base[c].max(EPS)).clamp(EPS, 1.0);
        let density = -t.log10(); // 0 at base, grows as pixel darkens
        tone(density, p.wb[c], p)
    })
}

/// Mode B: Ĉ = M_post · log10(M_pre · (base / I)), then per-channel tone.
///
/// Steps mirror the spec:
///  1. normalize r = I / base  (rgb/base; the later -log10 gives log10(base/I), removing the orange mask)
///  2. linear mix  M_pre · r    (sensor↔dye crosstalk; identity by default)
///  3. log10                    (into Beer-Lambert density space)
///  4. density unmix M_post     (identity by default)
///  5. tone (exposure, black, gamma)
pub fn invert_b(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    // clamp to [EPS,1]: matches mode C; avoids negative density leaking via m_post
    let r = Vector3::new(
        (rgb[0] / p.base[0].max(EPS)).clamp(EPS, 1.0),
        (rgb[1] / p.base[1].max(EPS)).clamp(EPS, 1.0),
        (rgb[2] / p.base[2].max(EPS)).clamp(EPS, 1.0),
    );
    let mixed = p.m_pre * r;
    let dens = Vector3::new(
        -(mixed[0].max(EPS)).log10(),
        -(mixed[1].max(EPS)).log10(),
        -(mixed[2].max(EPS)).log10(),
    );
    let unmixed = p.m_post * dens;
    [
        tone(unmixed[0], p.wb[0], p),
        tone(unmixed[1], p.wb[1], p),
        tone(unmixed[2], p.wb[2], p),
    ]
}

/// Which inversion to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Density-matrix (the product engine).
    B,
    /// Per-channel log-density baseline.
    C,
    /// 1 - x strawman.
    Naive,
}

/// Invert a whole image (returns a new Image, same dims).
pub fn invert_image(img: &crate::Image, p: &InversionParams, mode: Mode) -> crate::Image {
    let f = match mode {
        Mode::B => invert_b,
        Mode::C => invert_c,
        Mode::Naive => invert_naive,
    };
    // par_iter + collect into Vec preserves index order, so output is identical
    // to the sequential map; the per-pixel fn `f` is pure (no shared state).
    let pixels = img.pixels.par_iter().map(|&px| f(px, p)).collect();
    crate::Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() }
}

/// Build inversion params whose `m_post` is fitted from the given film stock's
/// physical model (`m_pre` stays identity). Used by Mode B for cross-channel
/// dye unmixing.
pub fn params_for_stock(
    stock: crate::spectral::Stock,
    base: [f32; 3],
    exposure: f32,
    black: f32,
    gamma: f32,
) -> InversionParams {
    let data = crate::spectral::load_stock(stock);
    let m_post = crate::calibrate::fit_m_post(&data);
    InversionParams {
        base,
        m_pre: Matrix3::identity(),
        m_post,
        exposure,
        black,
        gamma,
        wb: [1.0, 1.0, 1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Image;

    #[test]
    fn naive_inverts_white_base_to_black() {
        let p = InversionParams { base: [0.8, 0.6, 0.4], ..Default::default() };
        let out = invert_naive([0.8, 0.6, 0.4], &p);
        for (c, &v) in out.iter().enumerate() {
            assert!(v.abs() < 1e-4, "channel {c} = {v}");
        }
    }

    #[test]
    fn naive_inverts_dark_pixel_to_bright() {
        let p = InversionParams { base: [0.8, 0.8, 0.8], ..Default::default() };
        let out = invert_naive([0.0, 0.0, 0.0], &p);
        for (c, &v) in out.iter().enumerate() {
            assert!((v - 1.0).abs() < 1e-4, "channel {c} = {v}");
        }
    }

    #[test]
    fn mode_c_base_pixel_is_zero_density() {
        let p = InversionParams { base: [0.5, 0.5, 0.5], gamma: 1.0, ..Default::default() };
        let out = invert_c([0.5, 0.5, 0.5], &p);
        for (c, &v) in out.iter().enumerate() {
            assert!(v.abs() < 1e-4, "channel {c} = {v}");
        }
    }

    #[test]
    fn mode_c_darker_pixel_has_higher_output() {
        let p = InversionParams { base: [1.0, 1.0, 1.0], gamma: 1.0, ..Default::default() };
        let bright = invert_c([0.5, 0.5, 0.5], &p);
        let dark = invert_c([0.1, 0.1, 0.1], &p);
        assert!(dark[0] > bright[0]);
    }

    #[test]
    fn mode_b_identity_matrices_match_mode_c() {
        let p = InversionParams { base: [0.7, 0.6, 0.5], gamma: 1.0, ..Default::default() };
        let probe = [0.3, 0.25, 0.2];
        let b = invert_b(probe, &p);
        let c = invert_c(probe, &p);
        for ch in 0..3 {
            assert!((b[ch] - c[ch]).abs() < 1e-4, "ch {ch}: b={} c={}", b[ch], c[ch]);
        }
    }

    #[test]
    fn mode_b_base_pixel_is_black() {
        let p = InversionParams { base: [0.7, 0.6, 0.5], gamma: 1.0, ..Default::default() };
        let out = invert_b([0.7, 0.6, 0.5], &p);
        for (ch, &v) in out.iter().enumerate() {
            assert!(v.abs() < 1e-4, "ch {ch} = {v}");
        }
    }

    /// Forward model: a neutral scene exposure `e` (per channel) recorded on film
    /// becomes a negative pixel = base * 10^(-k*e) — darker where scene was bright.
    fn synth_negative(scene: [f32; 3], base: [f32; 3], k: f32) -> [f32; 3] {
        [
            base[0] * 10f32.powf(-k * scene[0]),
            base[1] * 10f32.powf(-k * scene[1]),
            base[2] * 10f32.powf(-k * scene[2]),
        ]
    }

    #[test]
    fn mode_b_recovers_neutrals_as_neutral() {
        let base = [0.8, 0.55, 0.35];
        let k = 0.6;
        let scene_grays = [[0.2, 0.2, 0.2], [0.5, 0.5, 0.5], [0.8, 0.8, 0.8]];
        let mut img = Image::new(3, 1);
        for (i, g) in scene_grays.iter().enumerate() {
            img.pixels[i] = synth_negative(*g, base, k);
        }
        let p = InversionParams { base, gamma: 1.0, ..Default::default() };
        let out = invert_image(&img, &p, Mode::B);
        for px in &out.pixels {
            let max = px.iter().cloned().fold(f32::MIN, f32::max);
            let min = px.iter().cloned().fold(f32::MAX, f32::min);
            assert!(max - min < 1e-3, "non-neutral recovery: {px:?}");
        }
    }

    #[test]
    fn mode_b_recovers_monotonic_brightness_order() {
        let base = [0.8, 0.55, 0.35];
        let k = 0.6;
        let mut img = Image::new(3, 1);
        img.pixels[0] = synth_negative([0.2; 3], base, k);
        img.pixels[1] = synth_negative([0.5; 3], base, k);
        img.pixels[2] = synth_negative([0.8; 3], base, k);
        let p = InversionParams { base, gamma: 1.0, ..Default::default() };
        let out = invert_image(&img, &p, Mode::B);
        assert!(out.pixels[0][0] < out.pixels[1][0]);
        assert!(out.pixels[1][0] < out.pixels[2][0]);
    }

    #[test]
    fn naive_and_b_differ_on_typical_pixel() {
        // The strawman must actually differ from the density engine.
        let p = InversionParams { base: [0.8, 0.55, 0.35], gamma: 1.0, ..Default::default() };
        let probe = [0.3, 0.22, 0.15];
        let n = invert_naive(probe, &p);
        let b = invert_b(probe, &p);
        let diff: f32 = (0..3).map(|c| (n[c] - b[c]).abs()).sum();
        assert!(diff > 1e-2, "naive and B should differ; diff={diff}");
    }

    #[test]
    fn stock_params_make_b_differ_from_identity() {
        use crate::spectral::Stock;
        let base = [0.5, 0.4, 0.3];
        let plain = InversionParams { base, gamma: 1.0, ..Default::default() };
        let stock = params_for_stock(Stock::Portra400, base, 1.0, 0.0, 1.0);
        let probe = [0.3, 0.22, 0.15];
        let a = invert_b(probe, &plain);
        let b = invert_b(probe, &stock);
        let diff: f32 = (0..3).map(|c| (a[c] - b[c]).abs()).sum();
        assert!(diff > 1e-3, "stock M_post should change B output; diff={diff}");
    }

    #[test]
    fn wb_gain_scales_channels_before_gamma() {
        // A per-channel wb gain must brighten/darken that channel's output.
        let base = [0.7, 0.6, 0.5];
        let probe = [0.3, 0.25, 0.2];
        let neutral = InversionParams { base, gamma: 1.0, ..Default::default() };
        let warmed = InversionParams { base, gamma: 1.0, wb: [1.5, 1.0, 0.5], ..Default::default() };
        let a = invert_b(probe, &neutral);
        let b = invert_b(probe, &warmed);
        assert!(b[0] > a[0], "R gain 1.5 should brighten R: {} vs {}", b[0], a[0]);
        assert!((b[1] - a[1]).abs() < 1e-6, "G gain 1.0 unchanged");
        assert!(b[2] < a[2], "B gain 0.5 should darken B: {} vs {}", b[2], a[2]);
    }

    #[test]
    fn invert_image_is_per_pixel_and_order_preserving() {
        // A multi-pixel image must invert each pixel exactly as the scalar fn does,
        // in the same order — this guards the parallel collect() against reordering.
        let p = InversionParams { base: [0.8, 0.6, 0.4], ..Default::default() };
        let pixels = vec![
            [0.8, 0.6, 0.4],
            [0.1, 0.2, 0.3],
            [0.5, 0.5, 0.5],
            [0.05, 0.9, 0.45],
        ];
        let img = Image { width: 2, height: 2, pixels: pixels.clone(), ir: None };
        let out = invert_image(&img, &p, Mode::B);
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
        for (i, &px) in pixels.iter().enumerate() {
            let want = invert_b(px, &p);
            for c in 0..3 {
                assert!((out.pixels[i][c] - want[c]).abs() < 1e-6, "pixel {i} chan {c}");
            }
        }
    }

    #[test]
    fn invert_image_preserves_ir_plane() {
        let mut img = Image::new(2, 1);
        img.ir = Some(vec![0.5, 0.25]);
        let p = InversionParams::default();
        let out = invert_image(&img, &p, Mode::B);
        assert_eq!(out.ir, Some(vec![0.5, 0.25]));
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 1);
    }
}
