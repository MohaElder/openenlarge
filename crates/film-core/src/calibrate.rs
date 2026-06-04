//! Estimating the film base (orange mask) from a region of the scan.

use crate::spectral::SpectralData;
use crate::Image;
use nalgebra::{DMatrix, Matrix3, Vector3};

/// A rectangular region in pixel coords (inclusive top-left, exclusive bottom-right).
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// Estimate per-channel film base from `rect` as a high percentile (95th) of the
/// region — robust to a few dark specks while tracking the bright clear-base value.
/// If `rect` is None, uses the whole image.
pub fn sample_base(img: &Image, rect: Option<Rect>) -> [f32; 3] {
    let r = rect.unwrap_or(Rect { x: 0, y: 0, w: img.width, h: img.height });
    let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for yy in r.y..(r.y + r.h).min(img.height) {
        for xx in r.x..(r.x + r.w).min(img.width) {
            let px = img.pixels[yy * img.width + xx];
            for c in 0..3 {
                chans[c].push(px[c]);
            }
        }
    }
    let mut base = [0.0f32; 3];
    for c in 0..3 {
        if chans[c].is_empty() {
            base[c] = 0.0;
            continue;
        }
        chans[c].sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((chans[c].len() as f32) * 0.95) as usize;
        let idx = idx.min(chans[c].len().saturating_sub(1));
        base[c] = chans[c][idx];
    }
    base
}

/// Gray-world white-balance gains computed from the brightest ~20% of an
/// (already inverted) image: per-channel multipliers that map the bright region's
/// mean color to neutral. Returns `[1,1,1]` for an empty image. Apply as
/// `InversionParams.wb` on a subsequent inversion to neutralize a global cast.
pub fn auto_wb_gains(img: &Image) -> [f32; 3] {
    if img.pixels.is_empty() {
        return [1.0, 1.0, 1.0];
    }
    let mut lums: Vec<f32> = img.pixels.iter().map(|p| (p[0] + p[1] + p[2]) / 3.0).collect();
    lums.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let thresh = lums[((lums.len() as f32 * 0.8) as usize).min(lums.len() - 1)];
    let (mut sum, mut n) = ([0.0f64; 3], 0u64);
    for p in &img.pixels {
        if (p[0] + p[1] + p[2]) / 3.0 >= thresh {
            for c in 0..3 {
                sum[c] += p[c] as f64;
            }
            n += 1;
        }
    }
    if n == 0 {
        return [1.0, 1.0, 1.0];
    }
    let mean = [sum[0] / n as f64, sum[1] / n as f64, sum[2] / n as f64];
    let gray = (mean[0] + mean[1] + mean[2]) / 3.0;
    [
        (gray / mean[0].max(1e-6)) as f32,
        (gray / mean[1].max(1e-6)) as f32,
        (gray / mean[2].max(1e-6)) as f32,
    ]
}

/// Concentration levels used to build the fitting patch grid (6³ = 216 patches).
const FIT_LEVELS: [f32; 6] = [0.0, 0.4, 0.8, 1.2, 1.6, 2.0];

/// Per-channel density of a simulated patch relative to the clear-film base.
fn patch_density(data: &SpectralData, base: [f32; 3], c: [f32; 3]) -> Vector3<f32> {
    let i = data.simulate(c);
    Vector3::new(
        -(i[0] / base[0].max(1e-8)).max(1e-8).log10(),
        -(i[1] / base[1].max(1e-8)).max(1e-8).log10(),
        -(i[2] / base[2].max(1e-8)).max(1e-8).log10(),
    )
}

/// Fit the 3×3 density-space unmixing matrix `M_post` so that
/// `c ≈ M_post · density` over a grid of known concentration patches.
///
/// Stacking patches as rows: C(n×3) ≈ D(n×3) · M_postᵀ, solved by normal
/// equations `M_postᵀ = (DᵀD)⁻¹ DᵀC`. Linear, closed-form, deterministic.
pub fn fit_m_post(data: &SpectralData) -> Matrix3<f32> {
    let base = data.base();
    let mut rows: Vec<([f32; 3], Vector3<f32>)> = Vec::new();
    for &cc in &FIT_LEVELS {
        for &mm in &FIT_LEVELS {
            for &yy in &FIT_LEVELS {
                let c = [cc, mm, yy];
                rows.push((c, patch_density(data, base, c)));
            }
        }
    }
    let n = rows.len();
    let dmat = DMatrix::from_fn(n, 3, |r, col| rows[r].1[col]);
    let cmat = DMatrix::from_fn(n, 3, |r, col| rows[r].0[col]);
    let dtd = dmat.transpose() * &dmat; // 3×3
    let dtc = dmat.transpose() * &cmat; // 3×3
    let inv = dtd.try_inverse().expect("DᵀD must be invertible for a non-degenerate patch set");
    let mpost_t = inv * dtc; // = M_postᵀ
    let m = mpost_t.transpose();
    Matrix3::new(
        m[(0, 0)], m[(0, 1)], m[(0, 2)],
        m[(1, 0)], m[(1, 1)], m[(1, 2)],
        m[(2, 0)], m[(2, 1)], m[(2, 2)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_base_returns_high_percentile() {
        let mut img = Image::new(10, 1);
        for i in 0..10 {
            let v = i as f32 / 10.0;
            img.pixels[i] = [v, 0.5, 0.5];
        }
        let base = sample_base(&img, None);
        assert!(base[0] >= 0.8, "got {}", base[0]);
        assert!((base[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sample_base_respects_rect() {
        let mut img = Image::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.pixels[y * 4 + x] = if x < 2 && y < 2 { [0.9, 0.9, 0.9] } else { [0.1, 0.1, 0.1] };
            }
        }
        let base = sample_base(&img, Some(Rect { x: 0, y: 0, w: 2, h: 2 }));
        assert!((base[0] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn sample_base_empty_region_is_zero_no_panic() {
        let img = Image::new(4, 4);
        // zero-area rect must not panic
        let base = sample_base(&img, Some(Rect { x: 0, y: 0, w: 0, h: 0 }));
        assert_eq!(base, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn auto_wb_gains_neutralize_a_global_cast() {
        // A uniformly magenta-cast gray (R,B high vs G) -> gains restore neutral.
        let cast = [0.6f32, 0.5, 0.4];
        let img = Image { width: 4, height: 4, pixels: vec![cast; 16], ir: None };
        let g = auto_wb_gains(&img);
        // green is the reference (smallest gain), red/blue corrected toward it
        assert!(g[2] > g[1] && g[1] > g[0], "expected B>G>R gains, got {g:?}");
        // applying the gains makes the patch neutral
        let corrected = [cast[0] * g[0], cast[1] * g[1], cast[2] * g[2]];
        let mx = corrected.iter().cloned().fold(f32::MIN, f32::max);
        let mn = corrected.iter().cloned().fold(f32::MAX, f32::min);
        assert!(mx - mn < 1e-4, "not neutral after wb: {corrected:?}");
    }

    #[test]
    fn fit_m_post_beats_identity_on_held_out_patches() {
        use crate::spectral::synthetic_overlapping;
        let data = synthetic_overlapping();
        let m = fit_m_post(&data);
        let base = data.base();
        // Held-out grid: disjoint from FIT_LEVELS {0,0.4,...,2.0}.
        let held = [0.2f32, 0.6, 1.0, 1.4, 1.8];
        let (mut sse_fit, mut sse_id, mut count) = (0.0f32, 0.0f32, 0u32);
        for &cc in &held {
            for &mm in &held {
                for &yy in &held {
                    let c = [cc, mm, yy];
                    let i = data.simulate(c);
                    let dens = nalgebra::Vector3::new(
                        -(i[0] / base[0]).max(1e-8).log10(),
                        -(i[1] / base[1]).max(1e-8).log10(),
                        -(i[2] / base[2]).max(1e-8).log10(),
                    );
                    let rec_fit = m * dens;
                    for k in 0..3 {
                        let e_fit = rec_fit[k] - c[k];
                        sse_fit += e_fit * e_fit;
                        let e_id = dens[k] - c[k]; // identity M_post = mode C
                        sse_id += e_id * e_id;
                        count += 1;
                    }
                }
            }
        }
        let rms_fit = (sse_fit / count as f32).sqrt();
        let rms_id = (sse_id / count as f32).sqrt();
        assert!(rms_fit < rms_id * 0.8, "fit RMS ΔC {rms_fit} not < 0.8 × identity {rms_id}");
    }

    #[test]
    fn fit_m_post_has_significant_off_diagonals() {
        use crate::spectral::synthetic_overlapping;
        let m = fit_m_post(&synthetic_overlapping());
        let off = [m[(0, 1)], m[(0, 2)], m[(1, 0)], m[(1, 2)], m[(2, 0)], m[(2, 1)]];
        let max_off = off.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(max_off > 0.1, "expected real crosstalk correction; max off-diagonal = {max_off}");
    }
}
