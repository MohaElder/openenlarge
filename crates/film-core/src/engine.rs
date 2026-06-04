//! Density-domain negative inversion.
//!
//! Mode B (density matrix):  Ĉ = M_post · log10(M_pre · (base / I))  then tone.
//! Mode C (naive per-chan):  per-channel log-density, no matrices.
//! Mode "naive flip":        1 - normalized, the strawman baseline.

use nalgebra::{Matrix3, Vector3};

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
        }
    }
}

const EPS: f32 = 1e-5;

/// Naive baseline: normalize against base, then invert by `1 - x`. No log, no
/// matrices. This is the strawman the density engine must beat.
pub fn invert_naive(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        let norm = (rgb[c] / p.base[c].max(EPS)).clamp(0.0, 1.0);
        out[c] = 1.0 - norm;
    }
    out
}

/// Apply exposure, black point, and output gamma to a linear density-output value.
fn tone(mut v: f32, p: &InversionParams) -> f32 {
    v = (v * p.exposure - p.black).max(0.0);
    v.powf(p.gamma)
}

/// Mode C: per-channel log-density. density = log10(base / I); higher film
/// density (less transmission) → brighter positive. Normalized by base density.
pub fn invert_c(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        let t = (rgb[c] / p.base[c].max(EPS)).clamp(EPS, 1.0);
        let density = -t.log10(); // 0 at base, grows as pixel darkens
        out[c] = tone(density, p);
    }
    out
}

/// Mode B: Ĉ = M_post · log10(M_pre · (base / I)), then per-channel tone.
///
/// Steps mirror the spec:
///  1. normalize r = base / I   (removes orange mask)
///  2. linear mix  M_pre · r    (sensor↔dye crosstalk; identity by default)
///  3. log10                    (into Beer-Lambert density space)
///  4. density unmix M_post     (identity by default)
///  5. tone (exposure, black, gamma)
pub fn invert_b(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    let r = Vector3::new(
        (rgb[0] / p.base[0].max(EPS)).max(EPS),
        (rgb[1] / p.base[1].max(EPS)).max(EPS),
        (rgb[2] / p.base[2].max(EPS)).max(EPS),
    );
    let mixed = p.m_pre * r;
    let dens = Vector3::new(
        -(mixed[0].max(EPS)).log10(),
        -(mixed[1].max(EPS)).log10(),
        -(mixed[2].max(EPS)).log10(),
    );
    let unmixed = p.m_post * dens;
    [
        tone(unmixed[0], p),
        tone(unmixed[1], p),
        tone(unmixed[2], p),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naive_inverts_white_base_to_black() {
        let p = InversionParams { base: [0.8, 0.6, 0.4], ..Default::default() };
        let out = invert_naive([0.8, 0.6, 0.4], &p);
        for c in 0..3 {
            assert!(out[c].abs() < 1e-4, "channel {c} = {}", out[c]);
        }
    }

    #[test]
    fn naive_inverts_dark_pixel_to_bright() {
        let p = InversionParams { base: [0.8, 0.8, 0.8], ..Default::default() };
        let out = invert_naive([0.0, 0.0, 0.0], &p);
        for c in 0..3 {
            assert!((out[c] - 1.0).abs() < 1e-4, "channel {c} = {}", out[c]);
        }
    }

    #[test]
    fn mode_c_base_pixel_is_zero_density() {
        let p = InversionParams { base: [0.5, 0.5, 0.5], gamma: 1.0, ..Default::default() };
        let out = invert_c([0.5, 0.5, 0.5], &p);
        for c in 0..3 {
            assert!(out[c].abs() < 1e-4, "channel {c} = {}", out[c]);
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
            assert!((b[ch] - c[ch]).abs() < 1e-5, "ch {ch}: b={} c={}", b[ch], c[ch]);
        }
    }

    #[test]
    fn mode_b_base_pixel_is_black() {
        let p = InversionParams { base: [0.7, 0.6, 0.5], gamma: 1.0, ..Default::default() };
        let out = invert_b([0.7, 0.6, 0.5], &p);
        for ch in 0..3 {
            assert!(out[ch].abs() < 1e-4, "ch {ch} = {}", out[ch]);
        }
    }
}
