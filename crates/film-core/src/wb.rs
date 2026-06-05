//! Correlated-colour-temperature ↔ per-channel white-balance gains.
//!
//! `wb_from_kelvin` uses the Tanner-Helland blackbody approximation to get an
//! RGB white point, then returns gains that neutralise it — normalised so the
//! reference (NEUTRAL_K, tint 0) yields ≈ [1,1,1]. `tint` shifts green↔magenta.

/// White point that maps to neutral [1,1,1] gains.
pub const NEUTRAL_K: f32 = 5500.0;

/// Tanner-Helland blackbody RGB (each channel 0..1) for a temperature in Kelvin.
#[allow(clippy::excessive_precision)]
fn blackbody_rgb(temp_k: f32) -> [f32; 3] {
    let t = (temp_k / 100.0).clamp(10.0, 400.0);
    let r = if t <= 66.0 {
        1.0
    } else {
        (329.698727446 * (t - 60.0).powf(-0.1332047592) / 255.0).clamp(0.0, 1.0)
    };
    let g = if t <= 66.0 {
        ((99.4708025861 * t.ln() - 161.1195681661) / 255.0).clamp(0.0, 1.0)
    } else {
        (288.1221695283 * (t - 60.0).powf(-0.0755148492) / 255.0).clamp(0.0, 1.0)
    };
    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        ((138.5177312231 * (t - 10.0).ln() - 305.0447927307) / 255.0).clamp(0.0, 1.0)
    };
    [r.max(1e-4), g.max(1e-4), b.max(1e-4)]
}

/// Per-channel gains for a target white balance. Lower K → warmer scene → boost
/// blue/cut red on output (gains neutralise the warm cast), normalised to neutral
/// at NEUTRAL_K. `tint` (−1..1-ish, UI −150..150 / 150) shifts green vs magenta.
pub fn wb_from_kelvin(temp_k: f32, tint: f32) -> [f32; 3] {
    let cur = blackbody_rgb(temp_k);
    let neu = blackbody_rgb(NEUTRAL_K);
    // Gain neutralises the current white point relative to neutral.
    let mut g = [neu[0] / cur[0], neu[1] / cur[1], neu[2] / cur[2]];
    // Tint: + → magenta (cut green), − → green (boost green). 0.5 caps full-range tint at ±50% green shift.
    g[1] *= 1.0 - 0.5 * tint;
    // Normalise so green gain stays 1 (keeps overall exposure stable).
    let gn = g[1].max(1e-4);
    [g[0] / gn, 1.0, g[2] / gn]
}

/// Estimate (temp_k, tint) from a set of WB gains (inverse of wb_from_kelvin).
/// Coarse search over 2000–15000 K (covers all realistic film-scan illuminants),
/// minimising the red/blue gain-ratio error; tint from the residual green
/// deviation. Intended for auto-WB seeding only — manual gains above ~15000 K
/// will saturate at the search ceiling.
pub fn gains_to_cct(gains: [f32; 3]) -> (f32, f32) {
    let target_rb = (gains[0] / gains[2].max(1e-4)).max(1e-4);
    let mut best_k = NEUTRAL_K;
    let mut best_err = f32::INFINITY;
    // 2000..=15000 K in 50 K steps (261 samples) — integer stepping avoids float drift.
    for step in 0_u32..=260 {
        let k = 2000.0 + step as f32 * 50.0;
        let g = wb_from_kelvin(k, 0.0);
        let rb = g[0] / g[2].max(1e-4);
        let err = (rb.ln() - target_rb.ln()).abs();
        if err < best_err {
            best_err = err;
            best_k = k;
        }
    }
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
    fn cct_roundtrips() {
        for k in [3200.0_f32, 4500.0, 5500.0, 6500.0, 8000.0] {
            let g = wb_from_kelvin(k, 0.0);
            let (est, tint) = gains_to_cct(g);
            assert!((est - k).abs() < 400.0, "k={k} est={est}");
            assert!(tint.abs() < 0.1, "k={k} tint={tint}");
        }
    }
}
