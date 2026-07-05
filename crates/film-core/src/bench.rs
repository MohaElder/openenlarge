//! Benchmark metrics: ColorChecker ΔE2000 (neutralized + as-shipped) and the
//! tone-transfer curve from the step wedge. Pure functions over a decoded
//! negative `Image`; no IO. The bench binary owns files and manifests.

use crate::calibrate::auto_wb_gains;
use crate::chart::{sample_grid, GridSpec};
use crate::color::{delta_e_2000, srgbf_to_lab};
use crate::colorchecker::{classic24_lab, NEUTRAL_INDICES};
use crate::engine::{invert_image, InversionParams, Mode};
use crate::Image;

pub struct ColorScore {
    pub mean: f32,
    pub max: f32,
    pub p95: f32,
    pub per_patch: Vec<f32>,
}

pub struct ColorReport {
    /// ΔE over all 24 patches, after WB-neutralizing on the grays.
    pub neutralized: ColorScore,
    /// ΔE over all 24 patches, using the engine's default auto-WB.
    pub as_shipped: ColorScore,
    /// ΔE over the 18 chromatic patches only (neutralized).
    pub neutralized_chroma_only: ColorScore,
}

fn score_from_deltas(deltas: Vec<f32>) -> ColorScore {
    let mut sorted = deltas.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Greater));
    let n = sorted.len().max(1);
    let mean = sorted.iter().sum::<f32>() / n as f32;
    let max = sorted.last().copied().unwrap_or(0.0);
    let p95 = sorted[((0.95 * (n as f32 - 1.0)).round() as usize).min(n - 1)];
    ColorScore {
        mean,
        max,
        p95,
        per_patch: deltas,
    }
}

/// Invert with neutral WB (wb=[1,1,1]) for a clean per-patch positive sample.
fn invert_neutral(neg: &Image, base: [f32; 3]) -> Image {
    let params = InversionParams {
        base,
        ..Default::default()
    };
    invert_image(neg, &params, Mode::D)
}

pub fn score_color(neg: &Image, base: [f32; 3], corners: &[[f32; 2]; 4]) -> ColorReport {
    let positive = invert_neutral(neg, base);
    let spec = GridSpec {
        cols: 6,
        rows: 4,
        inset: 0.5,
    };
    let patches = sample_grid(&positive, corners, &spec, 0.2);
    let reference = classic24_lab();

    // Neutralized gains: make the mean of the 6 grays equal across channels (anchor to G).
    let mut mean_g = [0.0f32; 3];
    for &i in NEUTRAL_INDICES.iter() {
        for c in 0..3 {
            mean_g[c] += patches[i][c];
        }
    }
    for g in &mut mean_g {
        *g /= NEUTRAL_INDICES.len() as f32;
    }
    let eps = 1e-5;
    let neut_gain = [
        mean_g[1] / mean_g[0].max(eps),
        1.0,
        mean_g[1] / mean_g[2].max(eps),
    ];

    // As-shipped gains: the engine's own auto-WB on the full inverted frame.
    // We model WB as a per-channel multiply on the (display-referred) filmic
    // positive — this is exactly the engine's `WbMode::Gain` at EV 0
    // (`engine.rs`: `filmic_s_raw(t) * wb[c]`), so for Gain mode it is engine-exact
    // bar minor divergence at near-white from the engine's filmic round-trip.
    // It does NOT reproduce `WbMode::Subtractive` (a pre-curve density multiply);
    // if the app ships Subtractive WB by default, treat as-shipped as an
    // approximation of, not an exact match to, the develop-view render.
    let ship_gain = auto_wb_gains(&positive);

    let apply = |p: [f32; 3], g: [f32; 3]| [p[0] * g[0], p[1] * g[1], p[2] * g[2]];

    let mut neut_d = Vec::with_capacity(24);
    let mut ship_d = Vec::with_capacity(24);
    let mut chroma_d = Vec::new();
    for (i, &p) in patches.iter().enumerate() {
        let dn = delta_e_2000(srgbf_to_lab(apply(p, neut_gain)), reference[i]);
        let ds = delta_e_2000(srgbf_to_lab(apply(p, ship_gain)), reference[i]);
        neut_d.push(dn);
        ship_d.push(ds);
        if !NEUTRAL_INDICES.contains(&i) {
            chroma_d.push(dn);
        }
    }

    ColorReport {
        neutralized: score_from_deltas(neut_d),
        as_shipped: score_from_deltas(ship_d),
        neutralized_chroma_only: score_from_deltas(chroma_d),
    }
}

pub struct ToneReport {
    pub ev: Vec<f32>,
    pub lstar: Vec<f32>,
    pub mid_gray_l: f32,
    pub shadow_latitude_ev: f32,
    pub highlight_latitude_ev: f32,
    pub mid_slope: f32,
    pub monotonic: bool,
}

/// Sample an N-step wedge (sampled as an n_steps×1 grid along the corner span) and
/// build the output-L* vs scene-EV transfer curve plus scalar metrics.
pub fn score_tone(
    neg: &Image,
    base: [f32; 3],
    corners: &[[f32; 2]; 4],
    n_steps: usize,
    ev_per_step: f32,
    mid_step: usize,
    drop_last: usize,
) -> ToneReport {
    let positive = invert_neutral(neg, base);
    let spec = GridSpec {
        cols: n_steps,
        rows: 1,
        inset: 0.5,
    };
    let cells = sample_grid(&positive, corners, &spec, 0.25);
    let keep = n_steps.saturating_sub(drop_last);

    let mut ev = Vec::with_capacity(keep);
    let mut lstar = Vec::with_capacity(keep);
    for (i, c) in cells.iter().take(keep).enumerate() {
        ev.push((i as f32 - mid_step as f32) * ev_per_step);
        lstar.push(srgbf_to_lab(*c)[0]);
    }

    let mid_gray_l = *lstar.get(mid_step).unwrap_or(&f32::NAN);
    // Monotonic if L* is non-decreasing with EV.
    let monotonic = lstar.windows(2).all(|w| w[1] >= w[0] - 1.0);
    // Mid slope: dL*/dEV around the mid step (central difference where possible).
    let mid_slope = if mid_step > 0 && mid_step + 1 < lstar.len() {
        (lstar[mid_step + 1] - lstar[mid_step - 1]) / (2.0 * ev_per_step)
    } else {
        f32::NAN
    };
    // Shadow latitude: most-negative EV whose L* still exceeds a near-black floor.
    let shadow_latitude_ev = ev
        .iter()
        .zip(lstar.iter())
        .filter(|(_, &l)| l > 2.0)
        .map(|(&e, _)| e)
        .fold(f32::INFINITY, f32::min);
    // Highlight latitude: most-positive EV whose L* is still below a near-white ceiling.
    let highlight_latitude_ev = ev
        .iter()
        .zip(lstar.iter())
        .filter(|(_, &l)| l < 98.0)
        .map(|(&e, _)| e)
        .fold(f32::NEG_INFINITY, f32::max);

    ToneReport {
        ev,
        lstar,
        mid_gray_l,
        shadow_latitude_ev,
        highlight_latitude_ev,
        mid_slope,
        monotonic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Image;

    // A trivial "negative": uniform mid value so inversion yields a flat gray frame.
    fn flat_neg(w: usize, h: usize, v: f32) -> Image {
        Image {
            width: w,
            height: h,
            pixels: vec![[v, v, v]; w * h],
            ir: None,
        }
    }

    #[test]
    fn color_report_populates_and_is_finite() {
        let neg = flat_neg(60, 40, 0.25);
        let corners = [[2.0, 2.0], [58.0, 2.0], [58.0, 38.0], [2.0, 38.0]];
        let rep = score_color(&neg, [0.5, 0.5, 0.5], &corners);
        assert_eq!(rep.neutralized.per_patch.len(), 24);
        assert!(rep.neutralized.mean.is_finite() && rep.neutralized.mean >= 0.0);
        assert!(rep.as_shipped.mean.is_finite());
        // A flat frame has neutral grays, so neutralized gains ≈ 1 and the score is finite.
        assert!(rep.neutralized.max >= rep.neutralized.mean);
        assert_eq!(
            rep.neutralized_chroma_only.per_patch.len(),
            24 - crate::colorchecker::NEUTRAL_INDICES.len(),
            "chroma-only score must exclude the 6 neutral patches"
        );
    }

    #[test]
    fn tone_report_monotone_on_ramp() {
        // Build a horizontal ramp negative: denser (smaller value) on the left.
        let (w, h) = (100usize, 20usize);
        let mut px = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                let t = 0.05 + 0.9 * (x as f32 / (w as f32 - 1.0)); // transmission rises L→R
                px[y * w + x] = [t, t, t];
            }
        }
        let neg = Image {
            width: w,
            height: h,
            pixels: px,
            ir: None,
        };
        // 5 steps across the width; brighter scene (more density) is on the LEFT (low transmission).
        let corners = [[100.0, 0.0], [0.0, 0.0], [0.0, 20.0], [100.0, 20.0]];
        let rep = score_tone(&neg, [1.0, 1.0, 1.0], &corners, 5, 1.0, 2, 0);
        assert_eq!(rep.lstar.len(), 5);
        assert!(rep.lstar.iter().all(|v| v.is_finite()));
        assert!(rep.mid_gray_l.is_finite());
        assert!(
            rep.monotonic,
            "expected a monotone ramp to report monotonic=true"
        );
    }
}
