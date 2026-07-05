//! Correlated-colour-temperature ↔ per-channel white-balance gains.
//!
//! `wb_from_kelvin` adapts along the **CIE Planckian locus**: it converts the
//! target and reference (NEUTRAL_K) colour temperatures to CIE xy white points,
//! then returns the von-Kries gains (in linear sRGB primaries) that map the
//! target white onto the reference white — normalised so the reference yields
//! ≈ [1,1,1]. Because the white points ride the blackbody curve, cooling/warming
//! stays on-locus instead of drifting through magenta/purple (issue #14). The old
//! Tanner-Helland RGB-ratio approximation under-cut red and over-boosted blue at
//! the cool end, leaving a violet cast. `tint` shifts green↔magenta.

/// White point that maps to neutral [1,1,1] gains.
pub const NEUTRAL_K: f32 = 5500.0;

/// CIE XYZ (D65) → linear sRGB. Standard IEC 61966-2-1 matrix; used to express
/// the locus white points in the working (sRGB-primary) RGB space so a von-Kries
/// gain is a plain per-channel divide.
#[allow(clippy::excessive_precision)]
const XYZ_TO_RGB: [[f32; 3]; 3] = [
    [3.2404542, -1.5371385, -0.4985314],
    [-0.9692660, 1.8760108, 0.0415560],
    [0.0556434, -0.2040259, 1.0572252],
];

/// CIE Planckian-locus chromaticity (x, y) for a CCT in Kelvin — Kim et al. (1999)
/// cubic-spline approximation, accurate over ~1667–25000 K (covers the full
/// 2000–25000 K slider range). This is the actual blackbody track, unlike the
/// Tanner-Helland display approximation it replaces.
#[allow(clippy::excessive_precision)]
fn cct_to_xy(temp_k: f32) -> (f32, f32) {
    let t = temp_k.clamp(1667.0, 25000.0);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    (x, y)
}

/// CIE xy chromaticity (Y normalised to 1) → linear sRGB tristimulus.
fn xy_to_rgb(x: f32, y: f32) -> [f32; 3] {
    let yy = y.max(1e-6);
    let xyz = [x / yy, 1.0, (1.0 - x - y) / yy];
    std::array::from_fn(|i| {
        XYZ_TO_RGB[i][0] * xyz[0] + XYZ_TO_RGB[i][1] * xyz[1] + XYZ_TO_RGB[i][2] * xyz[2]
    })
}

/// Per-channel gains for a target white balance. Lower K → warmer scene → boost
/// blue/cut red on output (gains neutralise the warm cast), normalised to neutral
/// at NEUTRAL_K. `tint` (−1..1-ish, UI −150..150 / 150) shifts green vs magenta.
pub fn wb_from_kelvin(temp_k: f32, tint: f32) -> [f32; 3] {
    let (tx, ty) = cct_to_xy(temp_k);
    let (nx, ny) = cct_to_xy(NEUTRAL_K);
    let tgt = xy_to_rgb(tx, ty);
    let neu = xy_to_rgb(nx, ny);
    // von-Kries: gain maps a neutral lit by the target illuminant back to the
    // reference white. Locus RGB stays positive across 2000–15000 K, but guard
    // the divide so an out-of-gamut excursion can't produce NaN/∞.
    let mut g = [
        neu[0] / tgt[0].max(1e-4),
        neu[1] / tgt[1].max(1e-4),
        neu[2] / tgt[2].max(1e-4),
    ];
    // Tint: + → magenta (cut green), − → green (boost green). 0.5 caps full-range tint at ±50% green shift.
    g[1] *= 1.0 - 0.5 * tint;
    // Normalise so green gain stays 1 (keeps overall exposure stable).
    let gn = g[1].max(1e-4);
    [g[0] / gn, 1.0, g[2] / gn]
}

/// Lower/upper bound of the CCT search — covers all realistic film-scan
/// illuminants, including deep blue-hour skies at the top (issue #17: the old
/// 15000 K cap starved the warm-correction range and clamped auto-WB estimates
/// of very blue frames). 25000 K is the Kim et al. locus validity limit.
/// Manual gains outside this range saturate at the bound.
const CCT_LO: f32 = 2000.0;
const CCT_HI: f32 = 25000.0;

/// Estimate (temp_k, tint) from a set of WB gains (inverse of wb_from_kelvin).
///
/// The red/blue gain ratio of `wb_from_kelvin(k, 0)` is **strictly increasing**
/// in `k` across `[CCT_LO, CCT_HI]`, so we recover `k` by bisecting that ratio
/// rather than scanning a coarse grid. Bisection is continuous and monotone: a
/// tiny change in the input gains moves the estimate by a tiny, proportional
/// amount instead of snapping it to the nearest 50 K bin — which is what made
/// auto-WB flip between neighbouring temperatures on shallow minima (B4). It is
/// also fully deterministic (no randomness, fixed iteration count), so re-running
/// on the same image always returns the same temperature.
///
/// Tint comes from the residual green deviation. Intended for auto-WB seeding;
/// gains beyond the bounds clamp to `CCT_LO`/`CCT_HI`.
pub fn gains_to_cct(gains: [f32; 3]) -> (f32, f32) {
    let target = (gains[0] / gains[2].max(1e-4)).max(1e-4).ln();
    let rb_ln = |k: f32| {
        let g = wb_from_kelvin(k, 0.0);
        (g[0] / g[2].max(1e-4)).max(1e-4).ln()
    };
    // Clamp targets outside the monotone bracket to the corresponding bound.
    let best_k = if target <= rb_ln(CCT_LO) {
        CCT_LO
    } else if target >= rb_ln(CCT_HI) {
        CCT_HI
    } else {
        let (mut lo, mut hi) = (CCT_LO, CCT_HI);
        // ~40 halvings of a 13000 K bracket → sub-Kelvin precision.
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if rb_ln(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    // Residual green vs the neutral-tint model at best_k → tint.
    let model = wb_from_kelvin(best_k, 0.0);
    let resid = gains[1] / model[1].max(1e-4); // >1 means more green applied → green tint (−)
    let tint = ((1.0 - resid) / 0.5).clamp(-1.0, 1.0);
    (best_k, tint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_unity() {
        let g = wb_from_kelvin(NEUTRAL_K, 0.0);
        for (c, &gc) in g.iter().enumerate() {
            assert!((gc - 1.0).abs() < 0.05, "c{c}={gc}");
        }
    }

    #[test]
    fn warm_scene_cuts_red_boosts_blue() {
        let g = wb_from_kelvin(3000.0, 0.0);
        assert!(g[0] < 1.0, "r {}", g[0]);
        assert!(g[2] > 1.0, "b {}", g[2]);
    }

    #[test]
    fn cool_scene_boosts_red_cuts_blue() {
        let g = wb_from_kelvin(9000.0, 0.0);
        assert!(g[0] > 1.0, "r {}", g[0]);
        assert!(g[2] < 1.0, "b {}", g[2]);
    }

    #[test]
    fn tracks_planckian_locus() {
        // Issue #14: the Temp slider must adapt along the CIE Planckian locus, not
        // through magenta/purple. These expected gains are computed independently
        // (Kim et al. CCT→xy locus + von-Kries in linear sRGB, green-normalised);
        // the old Tanner-Helland RGB-ratio model under-cut red and over-boosted
        // blue, missing these by 20–200%. Tolerance is a few % of each gain.
        let cases: [(f32, [f32; 3]); 8] = [
            (2000.0, [0.3016, 1.0, 28.725]),
            (3000.0, [0.5632, 1.0, 2.7544]),
            (4000.0, [0.7685, 1.0, 1.5384]),
            (5500.0, [1.0000, 1.0, 1.0000]),
            (6500.0, [1.1105, 1.0, 0.8442]),
            (9000.0, [1.2925, 1.0, 0.6573]),
            (15000.0, [1.4875, 1.0, 0.5166]),
            (25000.0, [1.5980, 1.0, 0.4551]),
        ];
        for (k, want) in cases {
            let g = wb_from_kelvin(k, 0.0);
            for c in 0..3 {
                let tol = 0.03 * want[c].max(0.05);
                assert!(
                    (g[c] - want[c]).abs() <= tol,
                    "k={k} c{c}: got {} want {} (tol {tol})",
                    g[c],
                    want[c]
                );
            }
        }
    }

    #[test]
    fn cooling_cuts_red_hard_enough_to_avoid_purple() {
        // The purple cast came from leaving too much red when cooling. A true
        // locus adaptation at 3000 K cuts red to ~0.56; the old model left 0.75,
        // and that residual red + boosted blue read as violet. Guard the regression.
        assert!(
            wb_from_kelvin(3000.0, 0.0)[0] < 0.65,
            "red under-cut: {}",
            wb_from_kelvin(3000.0, 0.0)[0]
        );
    }

    #[test]
    fn cct_roundtrips() {
        for k in [3200.0_f32, 4500.0, 5500.0, 6500.0, 8000.0] {
            let g = wb_from_kelvin(k, 0.0);
            let (est, tint) = gains_to_cct(g);
            assert!((est - k).abs() < 400.0, "k={k} est={est}");
            assert!(tint.abs() < 0.1, "k={k} tint={tint}");
        }
    }

    #[test]
    fn cct_is_continuous_not_quantized() {
        // A true temperature falling between the old 50 K grid points must be
        // recovered precisely, not snapped to the nearest bin. A coarse grid can
        // only land within ±25 K; a continuous solve lands within a few K.
        for k in [5523.0_f32, 6137.0, 7841.0, 4310.0] {
            let g = wb_from_kelvin(k, 0.0);
            let (est, _) = gains_to_cct(g);
            assert!((est - k).abs() < 5.0, "k={k} est={est}");
        }
    }

    #[test]
    fn cct_small_input_change_small_output_change() {
        // Sweeping the true temperature in 20 K steps must move the estimate in
        // similarly small, monotone steps — no staircase jumps that would flip a
        // re-run between neighbouring temperatures on a tiny content change. The
        // old 50 K grid staircased (≥50 K steps); the continuous solve tracks the
        // 20 K input step. Unlike the old Tanner-Helland model (whose clamped red
        // channel left a ~91 K dead-zone near 6600 K), the Planckian-locus model
        // has a strictly-monotone r/b ratio everywhere, so no range is skipped.
        let mut prev = gains_to_cct(wb_from_kelvin(3000.0, 0.0)).0;
        let mut k = 3020.0_f32;
        while k <= 12000.0 {
            let est = gains_to_cct(wb_from_kelvin(k, 0.0)).0;
            let step = est - prev;
            assert!(step >= -0.5, "non-monotone at {k}: {prev}->{est}");
            assert!(step < 40.0, "jump at {k}: {prev}->{est} (step {step})");
            prev = est;
            k += 20.0;
        }
    }

    #[test]
    fn cct_small_gain_perturbation_small_temp_change() {
        // A crop/content nudge perturbs the estimated gains by a hair. The
        // recovered temperature must move proportionally and continuously, not
        // snap to a distant bin. We bound the shift in mireds (1e6/K), the
        // perceptually-uniform scale — a fixed r/b ratio change is a roughly
        // constant mired shift regardless of temperature (it only looks large in
        // Kelvin at high T). A 1% ratio nudge must stay well under 5 mired.
        let mired = |k: f32| 1.0e6 / k;
        for k in [3500.0_f32, 5000.0, 8000.0, 11000.0] {
            let g = wb_from_kelvin(k, 0.0);
            let base = gains_to_cct(g).0;
            for delta in [-0.01_f32, 0.01] {
                let perturbed = [g[0] * (1.0 + delta), g[1], g[2]];
                let est = gains_to_cct(perturbed).0;
                assert!(
                    (mired(est) - mired(base)).abs() < 5.0,
                    "1% gain nudge at {k}K swung temp {base}->{est} ({} mired)",
                    (mired(est) - mired(base)).abs()
                );
            }
        }
    }

    #[test]
    fn cct_deterministic_on_repeat() {
        // Same gains in → bit-identical estimate out, every time. This is the
        // floor for "same image → same temperature on repeated auto-WB".
        let g = wb_from_kelvin(6234.0, 0.04);
        assert_eq!(gains_to_cct(g), gains_to_cct(g));
    }
}
