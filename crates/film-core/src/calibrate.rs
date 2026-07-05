//! Estimating the film base (orange mask) from a region of the scan.

use crate::Image;

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
    let r = rect.unwrap_or(Rect {
        x: 0,
        y: 0,
        w: img.width,
        h: img.height,
    });
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

/// Estimate the film base on a frame whose brightest pixels are NOT the film.
///
/// A clear leader shot on a lightbox (a dedicated d_min frame) has a blown-out
/// surround that clips near 1.0 — brighter than the orange mask itself. A plain
/// high-percentile sampler ([`sample_base`]) latches onto that blown surround and
/// returns ~`[1,1,1]`, giving the inversion no mask compensation (a heavy blue
/// cast). This rejects pixels whose max channel exceeds `reject` (clipped /
/// non-film) and returns the `pct` percentile per channel of what remains — the
/// brightest *non-clipped* value, i.e. the clear film base. If rejection leaves
/// too little (<1% of the frame, e.g. a frame with no usable film), it falls back
/// to [`sample_base`] so the result is never empty.
pub fn sample_base_clearfilm(img: &Image, reject: f32, pct: f32) -> [f32; 3] {
    let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for px in &img.pixels {
        if px[0].max(px[1]).max(px[2]) <= reject {
            for c in 0..3 {
                chans[c].push(px[c]);
            }
        }
    }
    let min_keep = (img.pixels.len() / 100).max(1);
    if chans[0].len() < min_keep {
        return sample_base(img, None);
    }
    let mut base = [0.0f32; 3];
    for c in 0..3 {
        chans[c].sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = (((chans[c].len() as f32) * pct) as usize).min(chans[c].len() - 1);
        base[c] = chans[c][idx];
    }
    base
}

/// Luma band for sampling the base from a deliberately-drawn CLEAR-FILM rect:
/// a central trimmed mean rejects dark specks and specular hot pixels.
pub const BASE_BAND_REBATE: (f32, f32) = (0.1, 0.9);
/// Luma band for the whole-frame FALLBACK: the brightest cluster is the clear
/// film / lightbox. Trims the very top to avoid clipped specular highlights.
pub const BASE_BAND_AUTO: (f32, f32) = (0.90, 0.99);

/// Long-edge cap for whole-region statistics (base color / density percentile).
/// Sampling a downscaled proxy is ~constant-time and plenty accurate for a mean
/// color or a robust percentile; sorting a full-res frame (hundreds of ms at
/// 11–24 MP) on every develop / image-switch / export was the perf regression.
const SAMPLE_CAP: usize = 512;

/// A pixel is a spoke "candidate" (negative) when its luma is at least the base
/// luma scaled by `(1 - SPOKE_MARGIN)` — i.e. as clear as the rebate or clearer.
/// Catches the orange rebate band AND the clear sprocket holes.
const SPOKE_MARGIN: f32 = 0.25;
/// Strictness of the masked-region uniformity term in the confidence score.
const MASK_UNIF_K: f32 = 3.0;
/// A pixel is a spoke "candidate" (positive) when its luma is within this of either
/// rail — near-black (slide rebate) or near-white (clear sprocket).
const POS_RAIL_MARGIN: f32 = 0.06;
/// Minimum detection confidence for `auto` to apply the mask.
const CONF_THRESH: f32 = 0.45;
/// Plausible spoke coverage band; outside this `auto` treats the frame as spoke-free.
const FRAC_MIN: f32 = 0.02;
const FRAC_MAX: f32 = 0.60;

/// Map a source-coord `rect` onto a downscaled image. `None` → the whole small
/// image. Widths/heights floor at 1px so a tiny crop never yields an empty band.
fn scaled_rect(rect: Option<Rect>, src: &Image, small: &Image) -> Rect {
    match rect {
        None => Rect {
            x: 0,
            y: 0,
            w: small.width,
            h: small.height,
        },
        Some(r) => {
            let sx = small.width as f32 / src.width.max(1) as f32;
            let sy = small.height as f32 / src.height.max(1) as f32;
            Rect {
                x: (r.x as f32 * sx) as usize,
                y: (r.y as f32 * sy) as usize,
                w: ((r.w as f32 * sx) as usize).max(1),
                h: ((r.h as f32 * sy) as usize).max(1),
            }
        }
    }
}

/// Sort a region's pixels by luma, keep the central [lo, hi] luma-rank band, and
/// average RGB over that one pixel set — a coherent clear-film color whose trim
/// rejects dark specks and specular hot pixels. Returns `[0,0,0]` for empty input.
fn coherent_band_avg(mut px: Vec<[f32; 3]>, lo: f32, hi: f32) -> [f32; 3] {
    if px.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    px.sort_by(|a, b| {
        let la = (a[0] + a[1] + a[2]) / 3.0;
        let lb = (b[0] + b[1] + b[2]) / 3.0;
        la.partial_cmp(&lb).unwrap()
    });
    let lo = lo.clamp(0.0, 1.0);
    let hi = hi.clamp(0.0, 1.0);
    let n = px.len();
    let i0 = ((n as f32 * lo) as usize).min(n - 1);
    let i1 = ((n as f32 * hi) as usize).clamp(i0 + 1, n);
    let band = &px[i0..i1];
    let mut sum = [0.0f64; 3];
    for p in band {
        for c in 0..3 {
            sum[c] += p[c] as f64;
        }
    }
    let k = band.len() as f64;
    [
        (sum[0] / k) as f32,
        (sum[1] / k) as f32,
        (sum[2] / k) as f32,
    ]
}

/// Sample the film base as a single COHERENT color: collect the region's pixels,
/// sort by luma, keep the [lo, hi] luma-rank band, and average RGB over that one
/// pixel set. Unlike [`sample_base`] (three independent per-channel percentiles)
/// the result is a real clear-film color, so it removes the orange mask without
/// injecting a per-channel cast. Returns `[0,0,0]` for an empty region. Runs on a
/// downscaled proxy (see [`SAMPLE_CAP`]) so cost is independent of source size.
pub fn sample_base_coherent(img: &Image, rect: Option<Rect>, lo: f32, hi: f32) -> [f32; 3] {
    let small = downscale_for_detect(img, SAMPLE_CAP);
    let r = scaled_rect(rect, img, &small);
    let mut px: Vec<[f32; 3]> = Vec::new();
    for yy in r.y..(r.y + r.h).min(small.height) {
        for xx in r.x..(r.x + r.w).min(small.width) {
            px.push(small.pixels[yy * small.width + xx]);
        }
    }
    coherent_band_avg(px, lo, hi)
}

/// Like [`sample_base_coherent`] but reads source pixels DIRECTLY at full
/// resolution (no whole-image downscale). For the manual base picker, where the
/// rect is a small, deliberately-aimed patch: the 512-proxy point-samples only a
/// handful of source pixels for a small rect (nearest-neighbor at an ~`src/512`
/// stride), so the result is aliased and grain/dust-sensitive. Reading every true
/// pixel in the rect lets the [lo,hi] trim reject specks over a full population,
/// giving a stable color even on a thin rebate. Cheap because the rect is small —
/// do NOT call with a full-frame rect (sorts the whole image; use the proxy).
pub fn sample_base_coherent_fullres(img: &Image, rect: Rect, lo: f32, hi: f32) -> [f32; 3] {
    let mut px: Vec<[f32; 3]> = Vec::new();
    for yy in rect.y..(rect.y + rect.h).min(img.height) {
        for xx in rect.x..(rect.x + rect.w).min(img.width) {
            px.push(img.pixels[yy * img.width + xx]);
        }
    }
    coherent_band_avg(px, lo, hi)
}

/// Default damping for [`auto_wb_gains`]: gains are shrunk toward neutral by this
/// factor. Gray-world is aggressive; applying it at full strength overshoots,
/// especially on film where highlights run warm. 0.7 corrects most of the cast
/// while leaving headroom for the user's manual temp/tint.
pub const AUTO_WB_STRENGTH: f32 = 0.7;

/// Robust gray-world white-balance gains from an (already inverted) image:
/// per-channel multipliers that map a neutral-pixel estimate to gray. Returns
/// `[1,1,1]` for an empty image. Apply as `InversionParams.wb` on a subsequent
/// inversion to neutralize a global cast. Uses [`AUTO_WB_STRENGTH`] damping.
///
/// Unlike a naive average of the brightest pixels — which neutralizes warm
/// highlights (sun/tungsten/skin) and so reads the scene as warm, over-cooling
/// the result into a blue cast — this rejects strongly chromatic pixels (sky,
/// foliage, skin, warm highlights), near-clipped highlights, and shadow noise
/// before averaging, so only genuinely near-neutral pixels drive the estimate.
pub fn auto_wb_gains(img: &Image) -> [f32; 3] {
    auto_wb_gains_strength(img, AUTO_WB_STRENGTH)
}

/// [`auto_wb_gains`] with an explicit damping `strength` in 0..=1 (1 = full
/// gray-world correction, 0 = no correction / `[1,1,1]`).
pub fn auto_wb_gains_strength(img: &Image, strength: f32) -> [f32; 3] {
    auto_wb_gains_strength_masked(img, strength, None)
}

/// Like [`auto_wb_gains`] but restricts the gray-world estimate to pixels
/// whose mask entry is `true`. `None` = include all (identical to the
/// unmasked variant). Pixel indices align to `img.pixels` order.
pub fn auto_wb_gains_masked(img: &Image, mask: Option<&[bool]>) -> [f32; 3] {
    auto_wb_gains_strength_masked(img, AUTO_WB_STRENGTH, mask)
}

/// Like [`auto_wb_gains_strength`] but restricts the gray-world estimate to
/// pixels whose mask entry is `true`. `None` = include all (identical to
/// [`auto_wb_gains_strength`]).
pub fn auto_wb_gains_strength_masked(
    img: &Image,
    strength: f32,
    mask: Option<&[bool]>,
) -> [f32; 3] {
    if img.pixels.is_empty() {
        return [1.0, 1.0, 1.0];
    }
    let mask_ok = |idx: usize| mask.is_none_or(|m| m.get(idx).copied().unwrap_or(true));
    // Exposure-invariant brightness gates. The gray-world gains are channel
    // ratios, so a uniform exposure nudge cancels out — *except* through which
    // pixels survive the bright/dark gate. Deriving the gate from the image's own
    // distribution (percentiles of max-channel / luma, both of which scale with
    // exposure) keeps the selected set — and the gains — stable under exposure,
    // which is what made auto-WB jump on small exposure changes (B4). Computed
    // once, deterministically (a total sort), so re-runs are bit-identical.
    let percentile = |key: &dyn Fn(&[f32; 3]) -> f32, q: f32| -> f32 {
        let mut v: Vec<f32> = img
            .pixels
            .iter()
            .enumerate()
            .filter(|(idx, _)| mask_ok(*idx))
            .map(|(_, p)| key(p))
            .collect();
        if v.is_empty() {
            // No masked pixels: return sentinel values that let collect fall through.
            return if q > 0.5 { f32::INFINITY } else { 0.0 };
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = (((v.len() - 1) as f32) * q).round() as usize;
        v[idx.min(v.len() - 1)]
    };
    let hi = percentile(&|p| p[0].max(p[1]).max(p[2]), 0.95); // drop brightest 5% (often warm-clipped)
    let lo = percentile(&|p| (p[0] + p[1] + p[2]) / 3.0, 0.05); // drop darkest 5% (shadow noise)

    // Collect near-neutral, well-exposed pixels per channel. `sat_max` rejects
    // chromatic content that violates the gray assumption; `hi`/`lo` drop clipped
    // highlights and shadow noise.
    let collect = |sat_max: f32, hi: f32, lo: f32| -> [Vec<f32>; 3] {
        let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (idx, p) in img.pixels.iter().enumerate() {
            if !mask_ok(idx) {
                continue;
            }
            let mx = p[0].max(p[1]).max(p[2]);
            let mn = p[0].min(p[1]).min(p[2]);
            let luma = (p[0] + p[1] + p[2]) / 3.0;
            if luma < lo || mx > hi {
                continue;
            }
            let sat = if mx > 1e-6 { (mx - mn) / mx } else { 0.0 };
            if sat > sat_max {
                continue;
            }
            for c in 0..3 {
                chans[c].push(p[c]);
            }
        }
        chans
    };

    // Need a meaningful sample. If a strong global cast desaturates too few pixels,
    // relax saturation; if still empty (e.g. all clipped), fall back to everything.
    let kept_total = (0..img.pixels.len()).filter(|i| mask_ok(*i)).count();
    let min_keep = (kept_total / 20).max(1);
    let mut chans = collect(0.25, hi, lo);
    if chans[0].len() < min_keep {
        chans = collect(0.6, hi, lo);
    }
    if chans[0].is_empty() {
        chans = collect(1.0, f32::INFINITY, 0.0);
    }
    if chans[0].is_empty() {
        return [1.0, 1.0, 1.0];
    }

    // Per-channel trimmed mean (drop the top/bottom 10%): a robust central
    // estimate that ignores the few chromatic outliers that slip past the gate,
    // so the result doesn't lurch when one such pixel enters/leaves the crop.
    let trimmed_mean = |v: &mut Vec<f32>| -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let cut = v.len() / 10;
        let slice = &v[cut..(v.len() - cut).max(cut + 1)];
        slice.iter().map(|&x| x as f64).sum::<f64>() / slice.len() as f64
    };
    let mean = [
        trimmed_mean(&mut chans[0]),
        trimmed_mean(&mut chans[1]),
        trimmed_mean(&mut chans[2]),
    ];
    let gray = (mean[0] + mean[1] + mean[2]) / 3.0;
    let raw = [
        (gray / mean[0].max(1e-6)) as f32,
        (gray / mean[1].max(1e-6)) as f32,
        (gray / mean[2].max(1e-6)) as f32,
    ];
    // Damp toward neutral so the auto seed corrects most of the cast without
    // overshooting; the user can push further with the manual temp/tint sliders.
    let k = strength.clamp(0.0, 1.0);
    [
        1.0 + k * (raw[0] - 1.0),
        1.0 + k * (raw[1] - 1.0),
        1.0 + k * (raw[2] - 1.0),
    ]
}

/// Per-zone damping (analogous to AUTO_WB_STRENGTH): corrects most of a zone's
/// residual cast while leaving the look faithful. Tunable on real problem rolls.
pub const PER_ZONE_STRENGTH: f32 = 0.7;
/// Hard clamp on any per-zone per-channel gain — keeps the correction gentle so it
/// never reads as "AI-processed". ±25%.
pub const PER_ZONE_MAX_GAIN: f32 = 1.25;

/// Zone edges + softness reused from the color-grade split-toning (finish.rs).
/// Cross-app consistency: HI_EDGE=0.66 must match the color-grade highlight edge.
const PZ_SH_EDGE: f32 = 0.33;
const PZ_HI_EDGE: f32 = 0.66;
const PZ_SOFT: f32 = 0.25;

#[inline]
fn pz_luma(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}
#[inline]
fn pz_smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Per-zone gray-world WB gains [shadows, mids, highlights] from an already-inverted
/// positive. Bins near-neutral pixels into 3 luma zones (the color-grade edges),
/// computes a damped, clamped gray-world correction per zone, and returns identity
/// for any zone with too few neutral pixels. With a uniform cast all zones agree, so
/// the result collapses to a global correction (faithfulness invariant).
pub fn per_zone_wb_gains(img: &Image, strength: f32, mask: Option<&[bool]>) -> [[f32; 3]; 3] {
    let identity = [1.0f32, 1.0, 1.0];
    if img.pixels.is_empty() {
        return [identity; 3];
    }
    let k = strength.clamp(0.0, 1.0);
    let mask_ok = |idx: usize| mask.is_none_or(|m| m.get(idx).copied().unwrap_or(true));
    // Accumulate a saturation-gated, weighted channel sum per zone.
    // weights: [shadow, mid, highlight] from the luma smoothstep masks.
    let mut sum = [[0.0f64; 3]; 3];
    let mut wsum = [0.0f64; 3];
    for (idx, &p) in img.pixels.iter().enumerate() {
        if !mask_ok(idx) {
            continue;
        }
        let mx = p[0].max(p[1]).max(p[2]);
        let mn = p[0].min(p[1]).min(p[2]);
        let sat = if mx > 1e-6 { (mx - mn) / mx } else { 0.0 };
        if sat > 0.25 {
            continue; // reject chromatic content (same gate spirit as auto_wb_gains)
        }
        let l = pz_luma(p);
        let w_sh = 1.0 - pz_smoothstep(PZ_SH_EDGE - PZ_SOFT, PZ_SH_EDGE + PZ_SOFT, l);
        let w_hi = pz_smoothstep(PZ_HI_EDGE - PZ_SOFT, PZ_HI_EDGE + PZ_SOFT, l);
        let w_mid = (1.0 - w_sh - w_hi).clamp(0.0, 1.0);
        let w = [w_sh, w_mid, w_hi];
        for z in 0..3 {
            wsum[z] += w[z] as f64;
            for c in 0..3 {
                sum[z][c] += (w[z] * p[c]) as f64;
            }
        }
    }
    // Min effective pixel weight for a zone to be trusted.
    // Floor of 8.0: a zone with fewer than ~8 effective weighted pixels must yield identity.
    let min_w = (img.pixels.len() as f64 / 50.0).max(8.0);
    let mut out = [identity; 3];
    for z in 0..3 {
        if wsum[z] < min_w {
            continue; // too few neutral pixels → identity, never invent a cast
        }
        let mean = [
            (sum[z][0] / wsum[z]) as f32,
            (sum[z][1] / wsum[z]) as f32,
            (sum[z][2] / wsum[z]) as f32,
        ];
        let gray = (mean[0] + mean[1] + mean[2]) / 3.0;
        for c in 0..3 {
            let raw = gray / mean[c].max(1e-6);
            let damped = 1.0 + k * (raw - 1.0);
            out[z][c] = damped.clamp(1.0 / PER_ZONE_MAX_GAIN, PER_ZONE_MAX_GAIN);
        }
    }
    out
}

/// Auto-derive the Cineon `D_max` (negative density range) over a region: per
/// channel take a low transmission percentile (the densest neg / brightest scene),
/// convert to density `log10(base_c / I_low_c)`, and take the max across channels
/// so no channel clips past print white. Clamped to a sane `[1.0, 4.0]`. Sampling
/// within `rect` lets the caller exclude borders (the image-area crop).
pub fn sample_dmax(img: &Image, base: [f32; 3], rect: Option<Rect>) -> f32 {
    sample_dmax_spread(img, base, rect, None).0
}

/// Like [`sample_dmax`] but also returns the crop's **density spread**: the max
/// across channels of `log10(i_high / i_low)` (99th vs 1st percentile transmission).
/// A flat crop (no real blacks — e.g. cropping into sky/highlights, the B3 trigger)
/// yields a tiny spread; callers use it to reject a range-destroying d_max estimate.
pub fn sample_dmax_spread(
    img: &Image,
    base: [f32; 3],
    rect: Option<Rect>,
    mask: Option<&[bool]>,
) -> (f32, f32) {
    const LOW_PCT: f32 = 0.01; // 1st percentile transmission (densest neg)
    const HIGH_PCT: f32 = 0.99; // 99th percentile (brightest transmission = base-ish)
    let small = downscale_for_detect(img, SAMPLE_CAP);
    let r = scaled_rect(rect, img, &small);
    let mask_ok = |idx: usize| mask.is_none_or(|m| m.get(idx).copied().unwrap_or(true));
    let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for yy in r.y..(r.y + r.h).min(small.height) {
        for xx in r.x..(r.x + r.w).min(small.width) {
            let idx = yy * small.width + xx;
            if !mask_ok(idx) {
                continue;
            }
            let px = small.pixels[idx];
            for c in 0..3 {
                chans[c].push(px[c]);
            }
        }
    }
    let mut d_max = 1.0f32;
    let mut spread = 0.0f32;
    for c in 0..3 {
        if chans[c].is_empty() || base[c] <= 1e-6 {
            continue;
        }
        chans[c].sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = chans[c].len();
        let lo = chans[c][((n as f32 * LOW_PCT) as usize).min(n - 1)].max(1e-5);
        let hi = chans[c][((n as f32 * HIGH_PCT) as usize).min(n - 1)].max(1e-5);
        d_max = d_max.max((base[c] / lo).log10());
        spread = spread.max((hi / lo).log10());
    }
    (d_max.clamp(1.0, 4.0), spread)
}

/// Per-channel density-neutralisation factors for camera-matrix mode. Equalises each
/// channel's MEAN optical density (over a subsampled frame) so a colour cast — the orange
/// mask plus the camera-matrix decode — self-corrects WITHOUT a manual white balance. This
/// is FreeCCR's optical-density mean-equalisation, adapted to the engine's density domain:
/// `d_c = log10(base_c/scan_c)`, factor `= target/mean(d_c)` with `target = mean of the
/// three channel means`. `[1,1,1]` when already balanced (identity → no effect). The
/// Faithful arm multiplies the raw density `d` by these per channel (see engine::invert_d).
/// Gray-world assumption (scene average is neutral); clamped so a strongly-tinted scene
/// can't drive an extreme correction.
pub fn sample_channel_balance(img: &Image, base: [f32; 3]) -> [f32; 3] {
    let small = downscale_for_detect(img, SAMPLE_CAP);
    let mut sum = [0.0f64; 3];
    let mut n = 0u64;
    for px in &small.pixels {
        for c in 0..3 {
            let dmin = base[c].max(1e-5);
            let scan = px[c].max(1e-5);
            sum[c] += (dmin / scan).log10().max(0.0) as f64;
        }
        n += 1;
    }
    if n == 0 {
        return [1.0, 1.0, 1.0];
    }
    let mean_d = [
        (sum[0] / n as f64) as f32,
        (sum[1] / n as f64) as f32,
        (sum[2] / n as f64) as f32,
    ];
    let target = (mean_d[0] + mean_d[1] + mean_d[2]) / 3.0;
    if target <= 1e-4 {
        return [1.0, 1.0, 1.0];
    }
    std::array::from_fn(|c| (target / mean_d[c].max(1e-4)).clamp(0.6, 1.7))
}

/// Cineon `D_max` from a **measured white-point**: the fully-exposed leader,
/// sampled in `rect`. The leader is the densest film (max light recorded), hence
/// the darkest region of the scan, so per channel we take a robust low value (5th
/// percentile, rejecting dust specks) as the white-point and convert to density
/// `log10(base_c / white_c)`, then take the max across channels (keeps `D_max`
/// scalar so white balance stays a separate print-side gain, not baked into the
/// inversion). Clamped to `[1.0, 4.0]`, mirroring `sample_dmax`. Unlike
/// `sample_dmax`, the anchor comes from the user's leader rect, not scene content —
/// giving a roll-constant highlight anchor instead of a per-frame estimate.
pub fn dmax_from_white_point(img: &Image, base: [f32; 3], rect: Option<Rect>) -> f32 {
    const WHITE_PCT: f32 = 0.05; // 5th percentile of the leader patch
    let small = downscale_for_detect(img, SAMPLE_CAP);
    let r = scaled_rect(rect, img, &small);
    let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for yy in r.y..(r.y + r.h).min(small.height) {
        for xx in r.x..(r.x + r.w).min(small.width) {
            let px = small.pixels[yy * small.width + xx];
            for c in 0..3 {
                chans[c].push(px[c]);
            }
        }
    }
    let mut d_max = 1.0f32;
    for c in 0..3 {
        if chans[c].is_empty() || base[c] <= 1e-6 {
            continue;
        }
        chans[c].sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((chans[c].len() as f32) * WHITE_PCT) as usize;
        let white = chans[c][idx.min(chans[c].len() - 1)].max(1e-5);
        let density = (base[c] / white).log10();
        d_max = d_max.max(density);
    }
    d_max.clamp(1.0, 4.0)
}

/// Result of rebate detection: the sampled clear-film base and a 0..1 confidence.
#[derive(Debug, Clone, Copy)]
pub struct RebateBase {
    pub base: [f32; 3],
    pub confidence: f32,
}

/// Outer fraction of each edge searched for the rebate.
const REBATE_BAND_FRAC: f32 = 0.12;
/// Small square patch (downscaled px) slid in 2D within each edge band, so it can
/// sit entirely inside a thin rebate sliver instead of straddling rebate+scene.
const REBATE_PATCH: usize = 14;
/// Uniformity penalty: higher = stricter about flatness.
const REBATE_UNIF_K: f32 = 4.0;
/// "Confident rebate" score: at/above this we trust the detected base outright and
/// show no UI hint. Tuned on real scans — a clean rebate scores ~0.16–0.18 (its
/// orange mask caps luma ~0.24); a dim/partial rebate (e.g. the underexposed
/// "Phoenix" roll ~0.07) falls below and is used only via the anti-blue rule in
/// `auto_base`, flagged low-confidence for an optional repoint.
pub const REBATE_CONFIDENCE: f32 = 0.12;

/// Nearest-neighbour downscale so detection stats are cheap/stable.
fn downscale_for_detect(img: &Image, target_long: usize) -> Image {
    let long = img.width.max(img.height);
    if long <= target_long {
        return img.clone();
    }
    let scale = target_long as f32 / long as f32;
    let w = ((img.width as f32 * scale) as usize).max(1);
    let h = ((img.height as f32 * scale) as usize).max(1);
    let mut pixels = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        let sy = ((y as f32 / scale) as usize).min(img.height - 1);
        for x in 0..w {
            let sx = ((x as f32 / scale) as usize).min(img.width - 1);
            pixels[y * w + x] = img.pixels[sy * img.width + sx];
        }
    }
    Image {
        width: w,
        height: h,
        pixels,
        ir: None,
    }
}

/// Mean RGB and mean per-channel coefficient-of-variation over a window.
fn patch_stats(img: &Image, x0: usize, y0: usize, pw: usize, ph: usize) -> ([f32; 3], f32) {
    let (mut sum, mut sumsq, mut n) = ([0.0f64; 3], [0.0f64; 3], 0u64);
    for y in y0..(y0 + ph).min(img.height) {
        for x in x0..(x0 + pw).min(img.width) {
            let p = img.pixels[y * img.width + x];
            for c in 0..3 {
                sum[c] += p[c] as f64;
                sumsq[c] += (p[c] as f64) * (p[c] as f64);
            }
            n += 1;
        }
    }
    if n == 0 {
        return ([0.0; 3], 1.0);
    }
    let nf = n as f64;
    let mean = [
        (sum[0] / nf) as f32,
        (sum[1] / nf) as f32,
        (sum[2] / nf) as f32,
    ];
    let mut cv_sum = 0.0f32;
    for c in 0..3 {
        let m = sum[c] / nf;
        let var = (sumsq[c] / nf - m * m).max(0.0);
        cv_sum += (var.sqrt() as f32) / (m as f32).max(1e-4);
    }
    (mean, cv_sum / 3.0)
}

/// bright × uniform × orange (each clamped 0..1). Orange requires the C-41 mask
/// ordering R≥G≥B; a blue/neutral patch scores 0 even if bright and uniform.
fn rebate_score(mean: [f32; 3], cv: f32) -> f32 {
    let bright = ((mean[0] + mean[1] + mean[2]) / 3.0).clamp(0.0, 1.0);
    let uniform = (1.0 - REBATE_UNIF_K * cv).clamp(0.0, 1.0);
    let orange = if mean[0] >= mean[1] && mean[1] >= mean[2] {
        ((mean[0] - mean[2]) / mean[0].max(1e-5)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    bright * uniform * orange
}

/// Slide a small `REBATE_PATCH` square in 2D across the rect region `[rx,ry,rw,rh)`,
/// scoring each placement and keeping the best. A small square (vs a full-band-tall
/// strip) can land entirely inside a thin rebate, avoiding rebate+scene mixing.
fn scan_region(img: &Image, rx: usize, ry: usize, rw: usize, rh: usize, best: &mut RebateBase) {
    let step = (REBATE_PATCH / 2).max(1);
    let mut y = ry;
    while y < ry + rh {
        let mut x = rx;
        while x < rx + rw {
            let (mean, cv) = patch_stats(img, x, y, REBATE_PATCH, REBATE_PATCH);
            let s = rebate_score(mean, cv);
            if s > best.confidence {
                *best = RebateBase {
                    base: mean,
                    confidence: s,
                };
            }
            x += step;
        }
        y += step;
    }
}

/// Detect the C-41 orange-mask film base from the frame's edge bands. Slides a
/// small square across the outer `REBATE_BAND_FRAC` band of each edge, scoring by
/// `rebate_score`, and returns the best patch's mean as `base` with its score as
/// `confidence`.
pub fn detect_rebate_base(img: &Image) -> RebateBase {
    let small = downscale_for_detect(img, 512);
    let (w, h) = (small.width, small.height);
    if w == 0 || h == 0 {
        return RebateBase {
            base: [0.0; 3],
            confidence: 0.0,
        };
    }
    let bw = ((w as f32 * REBATE_BAND_FRAC) as usize).max(REBATE_PATCH);
    let bh = ((h as f32 * REBATE_BAND_FRAC) as usize).max(REBATE_PATCH);
    let mut best = RebateBase {
        base: [0.0; 3],
        confidence: 0.0,
    };
    scan_region(&small, 0, 0, w, bh, &mut best); // top
    scan_region(&small, 0, h.saturating_sub(bh), w, bh, &mut best); // bottom
    scan_region(&small, 0, 0, bw, h, &mut best); // left
    scan_region(&small, w.saturating_sub(bw), 0, bw, h, &mut best); // right
    best
}

/// A per-pixel mask flagging the film "spokes" (sprocket holes / rebate / frame
/// lines) so metering can exclude them. `mask[i] == true` means pixel `i` is image
/// (keep); `false` means spoke/gap (exclude). Aligned to a `SAMPLE_CAP`-downscaled
/// copy of the input — the same grid the masked samplers reduce on.
#[derive(Debug, Clone)]
pub struct PhotoMask {
    pub mask: Vec<bool>,
    pub excluded_fraction: f32,
    pub confidence: f32,
}

fn luma3(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Border-connected flood fill over a candidate grid: returns the keep-mask
/// (true = image) and the count of excluded (spoke) pixels. Only candidates
/// reachable from the image edge through other candidates are excluded, so
/// interior speckle (real shadows/speculars) is kept.
fn border_connected_exclude(cand: &[bool], w: usize, h: usize) -> (Vec<bool>, usize) {
    debug_assert_eq!(cand.len(), w * h, "cand length must equal w*h");
    let mut excluded = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let push = |i: usize, stack: &mut Vec<usize>, excluded: &mut Vec<bool>| {
        if cand[i] && !excluded[i] {
            excluded[i] = true;
            stack.push(i);
        }
    };
    for x in 0..w {
        push(x, &mut stack, &mut excluded); // top row
        push((h - 1) * w + x, &mut stack, &mut excluded); // bottom row
    }
    for y in 0..h {
        push(y * w, &mut stack, &mut excluded); // left col
        push(y * w + (w - 1), &mut stack, &mut excluded); // right col
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        if x > 0 {
            push(i - 1, &mut stack, &mut excluded);
        }
        if x + 1 < w {
            push(i + 1, &mut stack, &mut excluded);
        }
        if y > 0 {
            push(i - w, &mut stack, &mut excluded);
        }
        if y + 1 < h {
            push(i + w, &mut stack, &mut excluded);
        }
    }
    let n_excl = excluded.iter().filter(|&&e| e).count();
    let keep: Vec<bool> = excluded.iter().map(|&e| !e).collect();
    (keep, n_excl)
}

/// Coefficient-of-variation of luma over the excluded (spoke) pixels — spokes are
/// flat, so low CV → high uniformity. Returns 1.0 (max uniform) for an empty set.
fn excluded_uniformity(small: &Image, keep: &[bool]) -> f32 {
    let (mut sum, mut sumsq, mut n) = (0.0f64, 0.0f64, 0u64);
    for (i, p) in small.pixels.iter().enumerate() {
        if !keep[i] {
            let l = luma3(*p) as f64;
            sum += l;
            sumsq += l * l;
            n += 1;
        }
    }
    if n == 0 {
        return 1.0;
    }
    let mean = sum / n as f64;
    let var = (sumsq / n as f64 - mean * mean).max(0.0);
    let cv = (var.sqrt() / mean.max(1e-4)) as f32;
    (1.0 - MASK_UNIF_K * cv).clamp(0.0, 1.0)
}

fn positive_candidates(small: &Image) -> Vec<bool> {
    small
        .pixels
        .iter()
        .map(|p| {
            let l = luma3(*p);
            l <= POS_RAIL_MARGIN || l >= 1.0 - POS_RAIL_MARGIN
        })
        .collect()
}

/// Detect the spoke/gap region. `positive` selects the value predicate; the spatial
/// confirmation (border flood-fill + uniformity) is shared. The negative predicate
/// flags pixels at-or-clearer-than the film base; the positive predicate
/// flags pixels near either rail.
pub fn detect_photo_mask(scan: &Image, base: [f32; 3], positive: bool) -> PhotoMask {
    let small = downscale_for_detect(scan, SAMPLE_CAP);
    let n = small.pixels.len();
    if n == 0 {
        return PhotoMask {
            mask: Vec::new(),
            excluded_fraction: 0.0,
            confidence: 0.0,
        };
    }
    let cand: Vec<bool> = if positive {
        positive_candidates(&small)
    } else {
        let base_l = luma3(base).max(1e-4);
        let thresh = base_l * (1.0 - SPOKE_MARGIN);
        small.pixels.iter().map(|p| luma3(*p) >= thresh).collect()
    };
    let cand_count = cand.iter().filter(|&&c| c).count();
    let (keep, n_excl) = border_connected_exclude(&cand, small.width, small.height);
    let excluded_fraction = n_excl as f32 / n as f32;
    let border_ratio = if cand_count == 0 {
        0.0
    } else {
        n_excl as f32 / cand_count as f32
    };
    let uniformity = excluded_uniformity(&small, &keep);
    let confidence = if n_excl == 0 {
        0.0
    } else {
        border_ratio * uniformity
    };
    PhotoMask {
        mask: keep,
        excluded_fraction,
        confidence,
    }
}

/// How the user's `meter_border` choice maps onto masking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterBorder {
    Auto,
    Exclude,
    Include,
}

impl MeterBorder {
    /// Parse the wire string; unknown values default to `Auto`.
    pub fn from_str_lenient(s: &str) -> MeterBorder {
        match s {
            "exclude" => MeterBorder::Exclude,
            "include" => MeterBorder::Include,
            _ => MeterBorder::Auto,
        }
    }
}

/// Decide whether to apply `pm`'s keep-mask given the user's mode. Returns the
/// keep-mask (`true` = image pixel) to hand the samplers, or `None` to meter the
/// full region. Never returns an all-excluded mask (degenerate guard).
pub fn gate_photo_mask(pm: PhotoMask, mode: MeterBorder) -> Option<Vec<bool>> {
    let kept = pm.mask.iter().filter(|&&m| m).count();
    if kept == 0 {
        return None; // never meter zero pixels
    }
    let apply = match mode {
        MeterBorder::Include => false,
        MeterBorder::Exclude => pm.excluded_fraction > 0.0,
        MeterBorder::Auto => {
            pm.confidence >= CONF_THRESH
                && pm.excluded_fraction >= FRAC_MIN
                && pm.excluded_fraction <= FRAC_MAX
        }
    };
    if apply {
        Some(pm.mask)
    } else {
        None
    }
}

/// The `SAMPLE_CAP`-downscaled grid `detect_photo_mask` runs on, exposed so callers
/// can run masked samplers on the same pixel grid the mask is aligned to.
pub fn detect_grid(img: &Image) -> Image {
    downscale_for_detect(img, SAMPLE_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearfilm_base_rejects_blown_surround() {
        // 70% orange film + 30% blown white. A plain high-percentile sampler
        // latches onto the white; clearfilm rejects it and returns the orange.
        let orange = [0.40, 0.50, 0.25];
        let white = [0.99, 0.99, 0.99];
        let mut img = Image::new(10, 10); // 100 px
        for i in 0..100 {
            img.pixels[i] = if i < 30 { white } else { orange };
        }
        // sample_base (95th pct) is dominated by the blown surround.
        let naive = sample_base(&img, None);
        assert!(
            naive[0] > 0.9,
            "naive base should latch onto white: {naive:?}"
        );
        // clearfilm rejects the >0.92 pixels and recovers the orange base.
        let cf = sample_base_clearfilm(&img, 0.92, 0.95);
        for c in 0..3 {
            assert!(
                (cf[c] - orange[c]).abs() < 1e-3,
                "clearfilm channel {c}: got {} want {}",
                cf[c],
                orange[c]
            );
        }
        // Degenerate: an all-clipped frame leaves nothing → falls back to sample_base.
        let allwhite = Image {
            width: 10,
            height: 10,
            pixels: vec![white; 100],
            ir: None,
        };
        let fb = sample_base_clearfilm(&allwhite, 0.92, 0.95);
        assert!(fb[0] > 0.9, "all-clipped frame must fall back, got {fb:?}");
    }

    #[test]
    fn sample_dmax_recovers_density_range_and_clamps() {
        // base = 1.0; log-spaced transmission from 1.0 (i=0) down to 0.01 (i=99),
        // so the density range spans log10(1/0.01) = 2.0. The 1st-percentile pick
        // lands at the second-densest pixel (~0.0105 transmission → density ~1.98).
        let mut img = Image::new(100, 1);
        for i in 0..100 {
            let t = 10f32.powf(-2.0 * i as f32 / 99.0);
            img.pixels[i] = [t, t, t];
        }
        let d = sample_dmax(&img, [1.0, 1.0, 1.0], None);
        assert!(
            (d - 2.0).abs() < 0.2,
            "expected ~2.0 density range, got {d}"
        );

        // A near-clear region (all bright) → tiny range, clamped up to the floor 1.0.
        let flat = Image {
            width: 4,
            height: 1,
            pixels: vec![[0.9, 0.9, 0.9]; 4],
            ir: None,
        };
        assert!(
            (sample_dmax(&flat, [1.0, 1.0, 1.0], None) - 1.0).abs() < 1e-4,
            "floor 1.0"
        );

        // A pitch-black region (transmission ~0) must not blow up — clamped to 4.0.
        let dark = Image {
            width: 4,
            height: 1,
            pixels: vec![[0.0001, 0.0001, 0.0001]; 4],
            ir: None,
        };
        assert!(
            (sample_dmax(&dark, [1.0, 1.0, 1.0], None) - 4.0).abs() < 1e-4,
            "ceil 4.0"
        );
    }

    #[test]
    fn sample_dmax_downscale_preserves_range_on_large_image() {
        // A >SAMPLE_CAP image must still recover the density range via the 512px
        // proxy (perf fix). Column-wise transmission 1.0 → 0.01 → range ~2.0.
        let (w, h) = (1024usize, 1024usize);
        let mut img = Image::new(w, h);
        for x in 0..w {
            let t = 10f32.powf(-2.0 * x as f32 / (w - 1) as f32);
            for y in 0..h {
                img.pixels[y * w + x] = [t, t, t];
            }
        }
        let d = sample_dmax(&img, [1.0, 1.0, 1.0], None);
        assert!((d - 2.0).abs() < 0.25, "downscaled d_max ~2.0, got {d}");
    }

    #[test]
    fn sample_base_coherent_downscale_keeps_color_and_rect() {
        // >SAMPLE_CAP image: uniform orange border (outer 10%) over a noisy center;
        // the AUTO bright band over a rect covering a border edge must still read
        // the orange via the 512px proxy (rect scaled correctly).
        let (w, h) = (1024usize, 1024usize);
        let orange = [0.42, 0.19, 0.10];
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let edge = y < h / 10;
                img.pixels[y * w + x] = if edge {
                    orange
                } else if (x + y) % 2 == 0 {
                    [0.30, 0.30, 0.30]
                } else {
                    [0.20, 0.20, 0.20]
                };
            }
        }
        // Sample within the top border strip (full width, top 10%).
        let rect = Some(Rect {
            x: 0,
            y: 0,
            w,
            h: h / 10,
        });
        let (blo, bhi) = BASE_BAND_REBATE;
        let b = sample_base_coherent(&img, rect, blo, bhi);
        for c in 0..3 {
            assert!(
                (b[c] - orange[c]).abs() < 0.02,
                "ch {c} = {} (want {})",
                b[c],
                orange[c]
            );
        }
    }

    #[test]
    fn sample_base_coherent_fullres_trims_grain_and_dust_on_thin_rebate() {
        // A THIN orange rebate (12px) on a wide (>>SAMPLE_CAP) image, peppered with
        // grain and dark dust specks. Reading every full-res pixel lets the [lo,hi]
        // trim reject the specks and recover the true mean orange.
        let (w, h) = (4096usize, 64usize);
        let orange = [0.42f32, 0.19, 0.10];
        let band = 12usize; // ~0.3% of width — a narrow rephotographed rebate
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // Deterministic ± grain, plus a sprinkling of near-black dust.
                let grain = (((x * 7 + y * 13) % 5) as f32 - 2.0) * 0.01;
                let dust = (x * 31 + y * 17) % 23 == 0;
                let p = if x < band {
                    if dust {
                        [0.02, 0.02, 0.02]
                    } else {
                        [orange[0] + grain, orange[1] + grain, orange[2] + grain]
                    }
                } else {
                    [0.85, 0.85, 0.80]
                };
                img.pixels[y * w + x] = p;
            }
        }
        // Rect parked inside the rebate (cols 1..11).
        let r = Rect {
            x: 1,
            y: 0,
            w: band - 2,
            h,
        };
        let (lo, hi) = BASE_BAND_REBATE;
        let b = sample_base_coherent_fullres(&img, r, lo, hi);
        for c in 0..3 {
            assert!(
                (b[c] - orange[c]).abs() < 0.02,
                "ch {c} = {} (want {})",
                b[c],
                orange[c]
            );
        }
    }

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
                img.pixels[y * 4 + x] = if x < 2 && y < 2 {
                    [0.9, 0.9, 0.9]
                } else {
                    [0.1, 0.1, 0.1]
                };
            }
        }
        let base = sample_base(
            &img,
            Some(Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            }),
        );
        assert!((base[0] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn sample_base_empty_region_is_zero_no_panic() {
        let img = Image::new(4, 4);
        // zero-area rect must not panic
        let base = sample_base(
            &img,
            Some(Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }),
        );
        assert_eq!(base, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn warm_highlights_do_not_trigger_blue_cast() {
        use crate::wb::gains_to_cct;
        // A neutral mid-gray scene (the real subject) with a bright WARM highlight
        // region — the common case (sun/tungsten/skin highlights). The estimator
        // must NOT read this as a warm illuminant and over-cool the image (the
        // white-patch-on-warm-highlights bug that made photos come out too blue).
        let mut pixels = vec![[0.45f32, 0.45, 0.45]; 800];
        pixels.extend(std::iter::repeat_n([0.90f32, 0.78, 0.62], 200));
        let img = Image {
            width: 1000,
            height: 1,
            pixels,
            ir: None,
        };

        let g = auto_wb_gains(&img);
        // Blue must not be strongly boosted over red (that is the cast).
        assert!(
            g[2] - g[0] < 0.12,
            "warm highlights over-cooled the image: gains {g:?}"
        );
        // And the seeded temperature must stay near daylight, not swing warm.
        let (temp, _tint) = gains_to_cct(g);
        assert!(temp >= 4900.0, "estimate read scene as warm: {temp}K");
    }

    #[test]
    fn auto_wb_gains_neutralize_a_global_cast() {
        // A uniformly magenta-cast gray (R,B high vs G) -> gains restore neutral.
        let cast = [0.6f32, 0.5, 0.4];
        let img = Image {
            width: 4,
            height: 4,
            pixels: vec![cast; 16],
            ir: None,
        };
        let spread = |p: [f32; 3]| {
            p.iter().cloned().fold(f32::MIN, f32::max) - p.iter().cloned().fold(f32::MAX, f32::min)
        };

        // Default (damped) gains: green reference, red/blue corrected toward it,
        // and the residual cast is reduced (but not fully removed — see below).
        let g = auto_wb_gains(&img);
        assert!(
            g[2] > g[1] && g[1] > g[0],
            "expected B>G>R gains, got {g:?}"
        );
        let corrected = [cast[0] * g[0], cast[1] * g[1], cast[2] * g[2]];
        assert!(
            spread(corrected) < spread(cast),
            "cast not reduced: {corrected:?}"
        );

        // At full strength the gray-world correction fully neutralizes the patch.
        let gf = auto_wb_gains_strength(&img, 1.0);
        let full = [cast[0] * gf[0], cast[1] * gf[1], cast[2] * gf[2]];
        assert!(
            spread(full) < 1e-4,
            "not neutral at full strength: {full:?}"
        );
    }

    #[test]
    fn auto_wb_gains_invariant_to_exposure_scaling() {
        // A scene whose chroma is correlated with brightness: shadows lean red,
        // highlights lean blue (a tame, sub-saturation cast). A uniform exposure
        // nudge scales every pixel equally, which must NOT change the white
        // balance — the gray-world gains are channel ratios, so the scale cancels.
        // The only way it can wobble is if an *absolute* luma gate drops a
        // brightness-correlated slice of pixels; the estimator must avoid that.
        let n = 200usize;
        let pixels: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                let frac = i as f32 / (n - 1) as f32;
                let l = 0.10 + 0.80 * frac; // luma 0.10 .. 0.90
                let c = frac - 0.5; // -0.5 (shadow, red) .. +0.5 (highlight, blue)
                [l * (1.0 - 0.10 * c), l, l * (1.0 + 0.10 * c)]
            })
            .collect();
        let img = Image {
            width: n,
            height: 1,
            pixels: pixels.clone(),
            ir: None,
        };
        let scaled = Image {
            width: n,
            height: 1,
            pixels: pixels
                .iter()
                .map(|p| [p[0] * 1.5, p[1] * 1.5, p[2] * 1.5])
                .collect(),
            ir: None,
        };

        let g = auto_wb_gains(&img);
        let gs = auto_wb_gains(&scaled);
        for c in 0..3 {
            assert!(
                (g[c] - gs[c]).abs() < 0.01,
                "exposure changed WB on channel {c}: {g:?} vs {gs:?}"
            );
        }
    }

    #[test]
    fn auto_wb_gains_deterministic_on_repeat() {
        // Same pixels in → bit-identical gains out (the floor for "same image →
        // same temperature on repeated auto-WB").
        let pixels: Vec<[f32; 3]> = (0..256)
            .map(|i| {
                let l = 0.2 + 0.6 * (i as f32 / 255.0);
                [l * 1.04, l, l * 0.97]
            })
            .collect();
        let img = Image {
            width: 256,
            height: 1,
            pixels,
            ir: None,
        };
        assert_eq!(auto_wb_gains(&img), auto_wb_gains(&img));
    }

    #[test]
    fn auto_wb_gains_masked_excludes_region() {
        // A neutral center surrounded by a subtle blue-cast border that passes the
        // saturation gate (sat≈0.20 < 0.25). With mask=None the border pixels
        // contribute and the gains correct toward warm (R>B). With mask=Some
        // excluding the border pixels the estimate sees only the neutral center
        // and returns gains close to [1,1,1].
        use super::{auto_wb_gains, auto_wb_gains_masked};
        let (w, h) = (20usize, 20usize);
        let neutral = [0.50f32, 0.50, 0.50];
        // blue_cast: sat = (0.40-0.32)/0.40 = 0.20 < 0.25 → passes gate; R<B so
        // the full image mean has R<B, which drives a warm (R-boost) correction.
        let blue_cast = [0.32f32, 0.32, 0.40];
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let border = x < 4 || y < 4 || x >= 16 || y >= 16;
                img.pixels[y * w + x] = if border { blue_cast } else { neutral };
            }
        }
        // Mask: exclude the border (border pixels → false, interior → true)
        let keep: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                !(x < 4 || y < 4 || x >= 16 || y >= 16)
            })
            .collect();
        let gains_full = auto_wb_gains(&img);
        let gains_masked = auto_wb_gains_masked(&img, Some(&keep));
        // Full estimate is skewed by the blue border: R gain > B gain (warm correction).
        assert!(
            gains_full[0] > gains_full[2],
            "full: expected R>B gains, got {gains_full:?}"
        );
        // Masked estimate sees only neutral pixels → gains near [1,1,1], R ≈ B.
        let diff = (gains_masked[0] - gains_masked[2]).abs();
        assert!(
            diff < 0.05,
            "masked: expected near-equal R/B, got {gains_masked:?}"
        );
    }

    #[test]
    fn auto_wb_gains_masked_none_equals_unmasked() {
        // Delegate contract: passing None must be byte-identical to auto_wb_gains.
        let mut img = Image::new(8, 8);
        for (i, px) in img.pixels.iter_mut().enumerate() {
            let v = 0.2 + 0.01 * (i % 7) as f32;
            *px = [v * 1.1, v, v * 0.9]; // mild non-neutral content
        }
        assert_eq!(auto_wb_gains_masked(&img, None), auto_wb_gains(&img));
    }

    #[test]
    fn coherent_base_is_a_trimmed_mean_of_one_pixel_set() {
        // A clear-film rect: mostly uniform orange, plus a dark speck and a bright
        // specular speck that a central luma band must trim away.
        let mut img = Image::new(10, 1);
        for i in 0..10 {
            img.pixels[i] = [0.43, 0.19, 0.11];
        }
        img.pixels[0] = [0.02, 0.01, 0.01]; // dark speck (trimmed by lo)
        img.pixels[9] = [0.99, 0.98, 0.97]; // specular speck (trimmed by hi)
        let b = sample_base_coherent(&img, None, 0.1, 0.9);
        for c in 0..3 {
            assert!(
                (b[c] - [0.43, 0.19, 0.11][c]).abs() < 1e-3,
                "ch {c} = {}",
                b[c]
            );
        }
    }

    /// Build a HxW image: uniform `border` color in the outer 10% band on all
    /// edges, `center` (optionally noisy) inside.
    fn bordered(w: usize, h: usize, border: [f32; 3], center: [f32; 3], noisy: bool) -> Image {
        let mut img = Image::new(w, h);
        let bw = (w as f32 * 0.10) as usize;
        let bh = (h as f32 * 0.10) as usize;
        for y in 0..h {
            for x in 0..w {
                let edge = x < bw || x >= w - bw || y < bh || y >= h - bh;
                let mut px = if edge { border } else { center };
                if !edge && noisy {
                    let j = if (x + y) % 2 == 0 { 0.18 } else { -0.18 };
                    px = [
                        (px[0] + j).clamp(0.0, 1.0),
                        (px[1] + j).clamp(0.0, 1.0),
                        (px[2] + j).clamp(0.0, 1.0),
                    ];
                }
                img.pixels[y * w + x] = px;
            }
        }
        img
    }

    #[test]
    fn detect_rebate_finds_orange_border_over_textured_center() {
        let orange = [0.42, 0.19, 0.10];
        let img = bordered(200, 150, orange, [0.5, 0.5, 0.5], true);
        let r = detect_rebate_base(&img);
        for (c, &o) in orange.iter().enumerate() {
            assert!((r.base[c] - o).abs() < 0.03, "ch {c}={}", r.base[c]);
        }
        assert!(r.confidence > 0.1, "confidence {}", r.confidence);
    }

    #[test]
    fn detect_rebate_ignores_bright_blue_center_phoenix() {
        let orange = [0.42, 0.19, 0.10];
        let img = bordered(200, 150, orange, [0.30, 0.21, 0.55], false);
        let r = detect_rebate_base(&img);
        assert!(
            r.base[0] > r.base[2],
            "must pick orange (R>B), got {:?}",
            r.base
        );
        assert!((r.base[0] - orange[0]).abs() < 0.05, "base {:?}", r.base);
    }

    #[test]
    fn detect_rebate_low_confidence_when_no_orange_border() {
        let img = Image {
            width: 200,
            height: 150,
            pixels: vec![[0.5, 0.5, 0.5]; 200 * 150],
            ir: None,
        };
        let r = detect_rebate_base(&img);
        assert!(
            r.confidence < 0.05,
            "expected low confidence, got {}",
            r.confidence
        );
    }

    #[test]
    fn coherent_base_bright_band_picks_clear_film_cluster() {
        // A luma gradient (dark scene → bright clear film). The bright band must
        // return ~the bright end, not the per-channel max or the global mean.
        let mut img = Image::new(100, 1);
        for i in 0..100 {
            let v = i as f32 / 99.0;
            img.pixels[i] = [v, v * 0.45, v * 0.26]; // orange-tinted ramp
        }
        let b = sample_base_coherent(&img, None, 0.90, 0.99);
        assert!(b[0] > 0.88, "bright R cluster, got {}", b[0]);
        assert!(
            (b[1] / b[0] - 0.45).abs() < 0.03,
            "G/R ratio {}",
            b[1] / b[0]
        );
        assert!(
            (b[2] / b[0] - 0.26).abs() < 0.03,
            "B/R ratio {}",
            b[2] / b[0]
        );
    }

    #[test]
    fn dmax_from_white_point_uses_channel_max_density() {
        // 4x4 uniform "leader" patch: dense (dark) scan values.
        // base/white per channel: R 0.8/0.08 → 1.0, G 0.8/0.008 → 2.0, B 0.8/0.08 → 1.0.
        // max across channels → 2.0.
        let white = [0.08f32, 0.008, 0.08];
        let pixels = vec![white; 16];
        let img = Image {
            width: 4,
            height: 4,
            pixels,
            ir: None,
        };
        let d = dmax_from_white_point(&img, [0.8, 0.8, 0.8], None);
        assert!((d - 2.0).abs() < 1e-3, "expected ~2.0, got {d}");
    }

    #[test]
    fn dmax_from_white_point_clamps_to_range() {
        // Extremely dense leader would exceed 4.0 → clamp; near-clear would underflow → 1.0.
        let dense = vec![[1e-5f32, 1e-5, 1e-5]; 16];
        let img = Image {
            width: 4,
            height: 4,
            pixels: dense,
            ir: None,
        };
        assert_eq!(dmax_from_white_point(&img, [0.8, 0.8, 0.8], None), 4.0);

        let clearish = vec![[0.79f32, 0.79, 0.79]; 16];
        let img2 = Image {
            width: 4,
            height: 4,
            pixels: clearish,
            ir: None,
        };
        assert_eq!(dmax_from_white_point(&img2, [0.8, 0.8, 0.8], None), 1.0);
    }

    #[test]
    fn flat_crop_has_low_density_spread_ranged_has_high() {
        let base = [1.0, 1.0, 1.0];
        // No real blacks (the B3 trigger: crop into sky/highlights) → tiny spread.
        let flat = Image {
            width: 8,
            height: 8,
            pixels: vec![[0.85, 0.85, 0.85]; 64],
            ir: None,
        };
        let (_d, spread) = sample_dmax_spread(&flat, base, None, None);
        assert!(spread < 0.1, "flat crop spread should be tiny: {spread}");
        // Spans blacks → brights → substantial density range.
        let mut px = vec![[0.9, 0.9, 0.9]; 32];
        px.extend(vec![[0.02, 0.02, 0.02]; 32]);
        let ranged = Image {
            width: 8,
            height: 8,
            pixels: px,
            ir: None,
        };
        let (_d2, spread2) = sample_dmax_spread(&ranged, base, None, None);
        assert!(
            spread2 > 0.5,
            "ranged crop spread should be substantial: {spread2}"
        );
    }

    #[test]
    fn dmax_mask_excludes_clear_border() {
        // 30x30 negative thumb (<= SAMPLE_CAP so no internal downscale): dark/dense
        // border pixels (low transmission) inflate d_max; masking them out must lower
        // the estimate. The interior is "clear" (high transmission = low density).
        let base = [0.5, 0.4, 0.3];
        let clear = [0.95, 0.95, 0.95]; // high transmission → low/negative density
        let dark = [0.02, 0.02, 0.02]; // very low transmission → high density → inflates d_max
        let mut img = Image::new(30, 30);
        for y in 0..30 {
            for x in 0..30 {
                let border = x < 4 || y < 4 || x >= 26 || y >= 26;
                img.pixels[y * 30 + x] = if border { dark } else { clear };
            }
        }
        let keep: Vec<bool> = (0..900)
            .map(|i| {
                let (x, y) = (i % 30, i / 30);
                !(x < 4 || y < 4 || x >= 26 || y >= 26)
            })
            .collect();
        let (d_full, _) = sample_dmax_spread(&img, base, None, None);
        let (d_masked, _) = sample_dmax_spread(&img, base, None, Some(&keep));
        assert!(d_masked < d_full, "masked={d_masked} full={d_full}");
    }

    fn high_conf_mask() -> PhotoMask {
        // 100 px, 30 excluded, confident.
        let mut mask = vec![true; 100];
        mask[..30].fill(false);
        PhotoMask {
            mask,
            excluded_fraction: 0.30,
            confidence: 0.9,
        }
    }

    #[test]
    fn gate_include_never_masks() {
        assert!(gate_photo_mask(high_conf_mask(), MeterBorder::Include).is_none());
    }

    #[test]
    fn gate_auto_applies_when_confident() {
        assert!(gate_photo_mask(high_conf_mask(), MeterBorder::Auto).is_some());
    }

    #[test]
    fn gate_auto_rejects_low_confidence() {
        let mut pm = high_conf_mask();
        pm.confidence = 0.1;
        assert!(gate_photo_mask(pm, MeterBorder::Auto).is_none());
    }

    #[test]
    fn gate_auto_rejects_implausible_fraction() {
        let mut pm = high_conf_mask();
        pm.excluded_fraction = 0.85; // > FRAC_MAX
        assert!(gate_photo_mask(pm, MeterBorder::Auto).is_none());
    }

    #[test]
    fn gate_exclude_forces_even_low_confidence() {
        let mut pm = high_conf_mask();
        pm.confidence = 0.0;
        assert!(gate_photo_mask(pm, MeterBorder::Exclude).is_some());
    }

    #[test]
    fn gate_rejects_all_masked() {
        let pm = PhotoMask {
            mask: vec![false; 100],
            excluded_fraction: 1.0,
            confidence: 1.0,
        };
        assert!(gate_photo_mask(pm, MeterBorder::Exclude).is_none());
    }

    #[test]
    fn gate_auto_rejects_tiny_fraction() {
        // excluded_fraction = 0.005 is below FRAC_MIN=0.02 → Auto gates out even when confident.
        let mut pm = high_conf_mask();
        pm.excluded_fraction = 0.005;
        assert!(gate_photo_mask(pm, MeterBorder::Auto).is_none());
    }

    #[test]
    fn gate_exclude_no_border_returns_none() {
        // Exclude applies only when excluded_fraction > 0.0; a no-border mask returns None.
        let pm = PhotoMask {
            mask: vec![true; 100],
            excluded_fraction: 0.0,
            confidence: 0.9,
        };
        assert!(gate_photo_mask(pm, MeterBorder::Exclude).is_none());
    }

    #[test]
    fn meter_border_parse() {
        assert!(matches!(
            MeterBorder::from_str_lenient("exclude"),
            MeterBorder::Exclude
        ));
        assert!(matches!(
            MeterBorder::from_str_lenient("include"),
            MeterBorder::Include
        ));
        assert!(matches!(
            MeterBorder::from_str_lenient("auto"),
            MeterBorder::Auto
        ));
        assert!(matches!(
            MeterBorder::from_str_lenient("garbage"),
            MeterBorder::Auto
        ));
    }
}

#[cfg(test)]
mod per_zone_tests {
    use super::*;

    #[test]
    fn uniform_cast_gives_equal_zone_gains() {
        // 300 pixels spanning a wide luma range with one consistent mild cast ratio.
        // sat ≈ 0.18 (< 0.25 gate), spans shadow→highlight — all three zones see the
        // same cast ratio, so they must report equal gains (faithfulness invariant:
        // per-zone collapses to a global correction). Wide span ensures every zone
        // clears min_w=8.
        let mut pixels = Vec::new();
        for i in 0..300 {
            let l = 0.12 + 0.76 * (i as f32 / 299.0); // ~0.12..0.88
            pixels.push([l * 1.12, l, l * 0.92]); // sat ≈ 0.18 (< 0.25 gate), spans shadow→highlight
        }
        let img = Image {
            width: 300,
            height: 1,
            pixels,
            ir: None,
        };
        let z = per_zone_wb_gains(&img, 1.0, None);
        for c in 0..3 {
            assert!((z[0][c] - z[1][c]).abs() < 0.06, "sh vs mid ch{c}: {z:?}");
            assert!((z[1][c] - z[2][c]).abs() < 0.06, "mid vs hi ch{c}: {z:?}");
        }
    }

    #[test]
    fn pink_highlights_neutral_mids_corrects_only_highlights() {
        // Mids neutral gray, highlights pushed pink (R,B up vs G). The highlight zone
        // must get a correction (G boosted relative to R/B), the mid zone ≈ identity.
        // Pink pixels have luma ≈ 0.83, which is above HI_EDGE=0.66 — they fall
        // squarely in the highlight zone with w_hi > 0.
        let mut pixels = Vec::new();
        for _ in 0..400 {
            pixels.push([0.45f32, 0.45, 0.45]);
        } // neutral mids (luma 0.45)
        for _ in 0..200 {
            pixels.push([0.92f32, 0.80, 0.90]);
        } // pink highlights (luma ~0.83, sat ~0.13)
        let img = Image {
            width: 600,
            height: 1,
            pixels,
            ir: None,
        };
        let z = per_zone_wb_gains(&img, 1.0, None);
        for c in 0..3 {
            assert!(
                (z[1][c] - 1.0).abs() < 0.08,
                "mid not identity ch{c}: {z:?}"
            );
        }
        assert!(
            z[2][1] > z[2][0] && z[2][1] > z[2][2],
            "highlights not de-pinked: {z:?}"
        );
    }

    #[test]
    fn empty_zone_is_identity() {
        // All-bright neutral → SHADOW zone gets ~0 weight (luma 0.85 > 0.58 ⇒ w_sh=0) → identity.
        let bright = Image {
            width: 200,
            height: 1,
            pixels: vec![[0.85, 0.85, 0.85]; 200],
            ir: None,
        };
        assert_eq!(
            per_zone_wb_gains(&bright, 1.0, None)[0],
            [1.0, 1.0, 1.0],
            "empty shadow zone must be identity"
        );
        // All-dark neutral → HIGHLIGHT zone gets ~0 weight (luma 0.10 < 0.41 ⇒ w_hi=0) → identity.
        let dark = Image {
            width: 200,
            height: 1,
            pixels: vec![[0.10, 0.10, 0.10]; 200],
            ir: None,
        };
        assert_eq!(
            per_zone_wb_gains(&dark, 1.0, None)[2],
            [1.0, 1.0, 1.0],
            "empty highlight zone must be identity"
        );
    }

    #[test]
    fn gains_stay_within_clamp_bounds() {
        // Reuse a de-pinking case; assert EVERY zone/channel gain is within the clamp bounds.
        // The sat<0.25 gate keeps gains well within the clamp in practice; the clamp is a safety bound.
        let mut pixels = Vec::new();
        for _ in 0..300 {
            pixels.push([0.45f32, 0.45, 0.45]);
        }
        for _ in 0..300 {
            pixels.push([0.92f32, 0.78, 0.90]);
        } // stronger pink, still sat<0.25
        let img = Image {
            width: 600,
            height: 1,
            pixels,
            ir: None,
        };
        let z = per_zone_wb_gains(&img, 1.0, None);
        for zone in &z {
            for &g in zone {
                assert!(
                    (1.0 / PER_ZONE_MAX_GAIN - 1e-4..=PER_ZONE_MAX_GAIN + 1e-4).contains(&g),
                    "gain out of clamp: {g} in {z:?}"
                );
            }
        }
    }

    #[test]
    fn pos_black_rebate_border_is_masked() {
        // Positive slide: outer ring near-black (dense rebate), interior mid-tone scene.
        let black = [0.01, 0.01, 0.01];
        let scene = [0.5, 0.45, 0.4];
        let mut img = Image::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let border = x < 3 || y < 3 || x >= 17 || y >= 17;
                img.pixels[y * 20 + x] = if border { black } else { scene };
            }
        }
        let pm = detect_photo_mask(&img, [0.0; 3], true);
        assert!(pm.excluded_fraction > 0.30, "frac={}", pm.excluded_fraction);
        assert!(pm.confidence > 0.5, "conf={}", pm.confidence);
    }

    #[test]
    fn pos_white_sprocket_border_is_masked() {
        let white = [0.99, 0.99, 0.99];
        let scene = [0.5, 0.45, 0.4];
        let mut img = Image::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let border = x < 3 || y < 3 || x >= 17 || y >= 17;
                img.pixels[y * 20 + x] = if border { white } else { scene };
            }
        }
        let pm = detect_photo_mask(&img, [0.0; 3], true);
        assert!(pm.excluded_fraction > 0.30, "frac={}", pm.excluded_fraction);
    }

    #[test]
    fn pos_interior_shadow_not_masked() {
        // A spoke-free slide with a deep-shadow BLOB in the interior (not border-connected).
        let scene = [0.5, 0.45, 0.4];
        let shadow = [0.01, 0.01, 0.01];
        let mut img = Image::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let interior_blob = (8..12).contains(&x) && (8..12).contains(&y);
                img.pixels[y * 20 + x] = if interior_blob { shadow } else { scene };
            }
        }
        let pm = detect_photo_mask(&img, [0.0; 3], true);
        // The blob doesn't touch the border, so nothing is excluded.
        assert!(pm.excluded_fraction < 0.02, "frac={}", pm.excluded_fraction);
    }

    #[test]
    fn neg_clear_border_is_masked() {
        // 20x20: outer 2px ring = clear sprocket/rebate (brighter than base),
        // interior 16x16 = darker scene content. Negative.
        let base = [0.40, 0.30, 0.20]; // orange mask, luma ~0.30
        let clear = [0.95, 0.95, 0.95];
        let scene = [0.10, 0.10, 0.10];
        let mut img = Image::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let border = x < 2 || y < 2 || x >= 18 || y >= 18;
                img.pixels[y * 20 + x] = if border { clear } else { scene };
            }
        }
        let pm = detect_photo_mask(&img, base, false);
        // border band excluded ≈ 0.36; interior (16x16=256 of 400) stays kept.
        assert!(pm.excluded_fraction > 0.30, "frac={}", pm.excluded_fraction);
        assert!(pm.confidence > 0.5, "conf={}", pm.confidence);
        let kept = pm.mask.iter().filter(|&&m| m).count();
        assert!(kept * 2 > pm.mask.len(), "kept={kept} of {}", pm.mask.len());
    }

    #[test]
    fn neg_spoke_free_frame_low_confidence() {
        // A gradient scene with NO clear border: nothing should mask out.
        let base = [0.40, 0.30, 0.20];
        let mut img = Image::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                // A gentle dark gradient that stays well below the base-clear threshold
                // (base luma ~0.31, threshold ~0.236) — no pixel is a spoke candidate.
                let v = 0.05 + 0.006 * x as f32; // luma 0.05..0.164, all < threshold
                img.pixels[y * 20 + x] = [v, v, v];
            }
        }
        let pm = detect_photo_mask(&img, base, false);
        assert!(
            pm.confidence < 0.5 || pm.excluded_fraction < 0.02,
            "frac={} conf={}",
            pm.excluded_fraction,
            pm.confidence
        );
    }

    #[test]
    fn per_zone_mask_changes_estimate() {
        // A developed positive with a strongly-tinted border; masking it should change
        // the per-zone gains vs. including it.
        let tint = [0.6f32, 0.5, 0.5]; // subtle red border (sat≈0.17 < 0.25 gate → accumulates)
        let neutral = [0.5, 0.5, 0.5];
        let mut img = Image::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let border = x < 3 || y < 3 || x >= 17 || y >= 17;
                img.pixels[y * 20 + x] = if border { tint } else { neutral };
            }
        }
        let keep: Vec<bool> = (0..400)
            .map(|i| {
                let (x, y) = (i % 20, i / 20);
                !(x < 3 || y < 3 || x >= 17 || y >= 17)
            })
            .collect();
        let full = per_zone_wb_gains(&img, 0.7, None);
        let masked = per_zone_wb_gains(&img, 0.7, Some(&keep));
        assert!(full != masked, "mask had no effect");
    }
}
