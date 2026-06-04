# Develop Redesign — Plan 1: Panels & Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the Develop tab's right panel (histogram, toolbar, compact Basic edit panel) and expand the engine with a creative finishing layer (Tone + Presence) and real Kelvin white balance.

**Architecture:** The Rust inversion core stays untouched; creative controls become a new per-pixel + spatial finishing layer (`film-core/src/finish.rs`) called after `invert_image`. White balance becomes a Kelvin→RGB-gain mapping (`film-core/src/wb.rs`). The Svelte right panel is recomposed from focused components (`Histogram`, `Toolbar`, `Basic`, reusable `Slider`) driven by an extended `InvertParams` contract.

**Tech Stack:** Rust (`film-core`, Tauri commands), Svelte 5, TypeScript, vitest, cargo test. Run cargo with `source "$HOME/.cargo/env" && cargo ...` (cargo is not on PATH in this env).

**Spec:** `docs/superpowers/specs/2026-06-04-develop-redesign-design.md` (sections A–F).

**Scope note:** Crop (spec section G) is Plan 2 and is NOT in this plan.

---

## File Structure

**Create:**
- `crates/film-core/src/finish.rs` — `FinishParams`, `finish_image`, tone curve, vibrance/saturation, texture (Gaussian unsharp).
- `crates/film-core/src/wb.rs` — `wb_from_kelvin`, `gains_to_cct`.
- `app/src/lib/develop/Slider.svelte` — reusable compact slider (gradient track, double-click reset).
- `app/src/lib/develop/Toolbar.svelte` — Edit/Crop/Eraser/Brush icon row.
- `app/src/lib/develop/Basic.svelte` — collapsible Basic section (WB + Tone + Presence).
- `app/src/lib/develop/gradients.ts` — gradient CSS strings + value formatters (pure, testable).
- `app/src/lib/develop/gradients.test.ts` — vitest for the above.
- `app/src/lib/viewport/histogram.ts` — pixel binning (pure, testable).
- `app/src/lib/viewport/histogram.test.ts` — vitest for binning.
- `app/src/lib/viewport/Histogram.svelte` — canvas histogram.

**Modify:**
- `crates/film-core/src/lib.rs` — register `finish`, `wb` modules.
- `app/src-tauri/src/session.rs` — extend `InvertParams`.
- `app/src-tauri/src/commands.rs` — EV conversion, wire `finish_image`, replace `wb_from_temp_tint`, add `as_shot_wb` command.
- `app/src-tauri/src/lib.rs` — register `as_shot_wb` in `invoke_handler`.
- `app/src/lib/api.ts` — extend `InvertParams`, `defaultParams`, add `asShotWb`.
- `app/src/lib/store.ts` — add `tool` and `previewSrc` stores.
- `app/src/lib/icons/Icon.svelte` — add glyphs.
- `app/src/lib/viewport/Viewport.svelte` — publish `previewSrc`.
- `app/src/lib/tabs/Develop.svelte` — layout rewrite, wire new panel.

**Delete (after migration):**
- `app/src/lib/panels/Adjustments.svelte` — replaced by `Basic.svelte` + Export footer.

---

## Task 1: Finishing layer — tone curve + vibrance/saturation

**Files:**
- Create: `crates/film-core/src/finish.rs`
- Modify: `crates/film-core/src/lib.rs`

- [ ] **Step 1: Register the module**

In `crates/film-core/src/lib.rs`, add after `pub mod export;`:

```rust
pub mod finish;
pub mod wb;
```

(`wb` is used in Task 3; declaring it now avoids a second edit. Create an empty `wb.rs` so it compiles: `touch crates/film-core/src/wb.rs`.)

- [ ] **Step 2: Write `finish.rs` with the params struct and per-pixel finishing, plus failing tests**

Create `crates/film-core/src/finish.rs`:

```rust
//! Creative finishing layer, applied to the gamma-encoded positive produced by
//! the inversion core. All params are 0.0 = identity. Tone/saturation are
//! per-pixel; texture (Task 2) is a spatial unsharp pass.

use crate::Image;

const EPS: f32 = 1e-5;

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

/// Apply finishing to a whole image. Texture (spatial) is added in Task 2.
pub fn finish_image(img: &Image, p: &FinishParams) -> Image {
    let pixels = img.pixels.iter().map(|&px| finish_pixel(px, p)).collect();
    Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() }
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
    fn negative_blacks_lowers_shadows() {
        let p = FinishParams { blacks: 1.0, ..Default::default() };
        assert!(tone_curve(0.1, &p) < 0.1);
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
        // Muted pixel (low current sat) vs vivid pixel (high current sat).
        let muted = [0.52, 0.50, 0.48];
        let vivid = [0.90, 0.10, 0.05];
        let dm = apply_saturation(muted, &p)[0] - muted[0];
        let dv = apply_saturation(vivid, &p)[0] - vivid[0];
        assert!(dm.abs() > 0.0);
        // Vivid pixel's per-unit chroma change is suppressed by (1 − cur_sat).
        let muted_sat = (muted[0] - muted[2]) / muted[0];
        let vivid_sat = (vivid[0] - vivid[2]) / vivid[0];
        assert!(muted_sat < vivid_sat);
    }

    #[test]
    fn finish_image_default_returns_equal_image() {
        let src = img_from(vec![[0.2, 0.4, 0.6], [0.7, 0.5, 0.3]]);
        let out = finish_image(&src, &FinishParams::default());
        assert_eq!(out.pixels, src.pixels);
    }
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core finish`
Expected: all `finish::tests::*` PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/film-core/src/finish.rs crates/film-core/src/lib.rs crates/film-core/src/wb.rs
git commit -m "feat(film-core): finishing layer — tone curve + vibrance/saturation"
```

---

## Task 2: Finishing layer — texture (Gaussian unsharp)

**Files:**
- Modify: `crates/film-core/src/finish.rs`

- [ ] **Step 1: Add the failing texture test**

Append to the `tests` module in `finish.rs`:

```rust
    #[test]
    fn texture_zero_is_identity() {
        // A 5x5 ramp; texture=0 must return the same pixels.
        let mut px = Vec::new();
        for i in 0..25 { let v = i as f32 / 25.0; px.push([v, v, v]); }
        let img = Image { width: 5, height: 5, pixels: px.clone(), ir: None };
        let out = finish_image(&img, &FinishParams::default());
        assert_eq!(out.pixels, px);
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
```

- [ ] **Step 2: Run to verify `positive_texture_increases_edge_contrast` fails**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core finish::tests::positive_texture`
Expected: FAIL (texture not yet applied — `edge == flat`).

- [ ] **Step 3: Implement Gaussian blur + texture, integrate into `finish_image`**

In `finish.rs`, add a separable 1-D Gaussian blur and the texture pass, and call it from `finish_image`. Replace the existing `finish_image` body:

```rust
/// Separable 3-tap Gaussian (radius 1, weights 1/4,1/2,1/4) on the luma-neutral
/// channels. Edges clamp. Small radius keeps it cheap; texture is a local effect.
fn blur(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let idx = |x: usize, y: usize| y * w + x;
    let mut tmp = vec![[0.0_f32; 3]; w * h];
    // Horizontal
    for y in 0..h {
        for x in 0..w {
            let xl = x.saturating_sub(1);
            let xr = (x + 1).min(w - 1);
            for c in 0..3 {
                tmp[idx(x, y)][c] =
                    0.25 * img.pixels[idx(xl, y)][c]
                    + 0.5 * img.pixels[idx(x, y)][c]
                    + 0.25 * img.pixels[idx(xr, y)][c];
            }
        }
    }
    // Vertical
    let mut out = vec![[0.0_f32; 3]; w * h];
    for y in 0..h {
        let yu = y.saturating_sub(1);
        let yd = (y + 1).min(h - 1);
        for x in 0..w {
            for c in 0..3 {
                out[idx(x, y)][c] =
                    0.25 * tmp[idx(x, yu)][c]
                    + 0.5 * tmp[idx(x, y)][c]
                    + 0.25 * tmp[idx(x, yd)][c];
            }
        }
    }
    Image { width: w, height: h, pixels: out, ir: None }
}

/// Unsharp mask: out = v + amount * (v − blur(v)). amount in −1..1.
fn apply_texture(img: &Image, amount: f32) -> Image {
    let b = blur(img);
    let k = 1.5 * amount; // perceptual gain
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
```

- [ ] **Step 4: Run all finish tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core finish`
Expected: all PASS (including the two texture tests and `texture_zero_is_identity`).

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(film-core): texture via Gaussian unsharp mask in finishing layer"
```

---

## Task 3: Kelvin white balance — `wb_from_kelvin` + `gains_to_cct`

**Files:**
- Modify: `crates/film-core/src/wb.rs` (created empty in Task 1)

- [ ] **Step 1: Write `wb.rs` with the mapping and failing tests**

Replace the contents of `crates/film-core/src/wb.rs`:

```rust
//! Correlated-colour-temperature ↔ per-channel white-balance gains.
//!
//! `wb_from_kelvin` uses the Tanner-Helland blackbody approximation to get an
//! RGB white point, then returns gains that neutralise it — normalised so the
//! reference (NEUTRAL_K, tint 0) yields ≈ [1,1,1]. `tint` shifts green↔magenta.

/// White point that maps to neutral [1,1,1] gains.
pub const NEUTRAL_K: f32 = 5500.0;

/// Tanner-Helland blackbody RGB (each channel 0..1) for a temperature in Kelvin.
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
    // Tint: + → magenta (cut green), − → green (boost green).
    g[1] *= 1.0 - 0.5 * tint;
    // Normalise so green gain stays 1 (keeps overall exposure stable).
    let gn = g[1].max(1e-4);
    [g[0] / gn, 1.0, g[2] / gn]
}

/// Estimate (temp_k, tint) from a set of WB gains (inverse of wb_from_kelvin).
/// Coarse search over the Planckian range minimising the red/blue gain ratio
/// error; tint from the residual green deviation.
pub fn gains_to_cct(gains: [f32; 3]) -> (f32, f32) {
    let target_rb = (gains[0] / gains[2].max(1e-4)).max(1e-4);
    let mut best_k = NEUTRAL_K;
    let mut best_err = f32::INFINITY;
    let mut k = 2000.0_f32;
    while k <= 15000.0 {
        let g = wb_from_kelvin(k, 0.0);
        let rb = g[0] / g[2].max(1e-4);
        let err = (rb.ln() - target_rb.ln()).abs();
        if err < best_err { best_err = err; best_k = k; }
        k += 50.0;
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
        for c in 0..3 { assert!((g[c] - 1.0).abs() < 0.05, "c{c}={}", g[c]); }
    }

    #[test]
    fn warm_scene_cuts_red_boosts_blue() {
        // Low K (warm) → gains should reduce red, raise blue vs neutral.
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
```

- [ ] **Step 2: Run the tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core wb`
Expected: all `wb::tests::*` PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/film-core/src/wb.rs
git commit -m "feat(film-core): Kelvin white balance — wb_from_kelvin + gains_to_cct"
```

---

## Task 4: Backend plumbing — extend params, EV, wire finishing, Kelvin WB, as_shot_wb

**Files:**
- Modify: `app/src-tauri/src/session.rs:35-50`
- Modify: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs:28-35`

- [ ] **Step 1: Extend `InvertParams` in `session.rs`**

In `app/src-tauri/src/session.rs`, replace the `InvertParams` struct (lines ~36-50) with:

```rust
/// Knobs the UI sends for an inversion (mirrors the engine's exposed controls).
#[derive(Debug, Clone, Deserialize)]
pub struct InvertParams {
    pub mode: String,
    pub stock: String,
    #[allow(dead_code)]
    pub base_rect: Option<[usize; 4]>,
    /// Exposure in EV stops (−5..5); converted to a multiplier (2^ev) downstream.
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    pub auto_wb: bool,
    /// Kelvin (e.g. 5500) and green↔magenta tint (−150..150).
    pub temp: f32,
    pub tint: f32,
    // Creative finishing (UI −100..100; 0 = identity).
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub vibrance: f32,
    pub saturation: f32,
}
```

- [ ] **Step 2: Update `commands.rs` helpers — EV, FinishParams, Kelvin, finishing**

In `app/src-tauri/src/commands.rs`:

1. Update imports:

```rust
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::finish::{finish_image, FinishParams};
use film_core::wb::{gains_to_cct, wb_from_kelvin};
```

2. Update `default_invert_params` (exposure → EV 0.0, temp → NEUTRAL 5500, new fields 0):

```rust
fn default_invert_params() -> InvertParams {
    InvertParams {
        mode: "b".into(), stock: "none".into(), base_rect: None,
        exposure: 0.0, black: 0.0, gamma: 0.4545, auto_wb: true,
        temp: 5500.0, tint: 0.0,
        contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
        texture: 0.0, vibrance: 0.0, saturation: 0.0,
    }
}
```

3. In `build_params`, convert EV → multiplier. Replace `build_params`:

```rust
fn build_params(p: &InvertParams, base: [f32; 3]) -> InversionParams {
    let exposure = 2f32.powf(p.exposure); // EV stops → linear multiplier
    match stock_from(&p.stock) {
        Some(s) if p.mode == "b" => params_for_stock(s, base, exposure, p.black, p.gamma),
        _ => InversionParams { base, exposure, black: p.black, gamma: p.gamma, ..Default::default() },
    }
}
```

4. Replace `wb_from_temp_tint` with the Kelvin mapping (UI tint −150..150 → −1..1):

```rust
fn wb_from_params(temp: f32, tint: f32) -> [f32; 3] {
    wb_from_kelvin(temp, tint / 150.0)
}
```

5. Update `resolve_params`. White balance is now absolute Kelvin (no manual×auto stacking — the UI seeds temp/tint from `as_shot_wb`):

```rust
fn resolve_params(p: &InvertParams, _autowb_src: &film_core::Image, base: [f32; 3]) -> InversionParams {
    let mut ip = build_params(p, base);
    ip.wb = wb_from_params(p.temp, p.tint);
    ip
}
```

6. Add a `FinishParams` builder (UI −100..100 → −1..1):

```rust
fn finish_from(p: &InvertParams) -> FinishParams {
    FinishParams {
        contrast: p.contrast / 100.0,
        highlights: p.highlights / 100.0,
        shadows: p.shadows / 100.0,
        whites: p.whites / 100.0,
        blacks: p.blacks / 100.0,
        texture: p.texture / 100.0,
        vibrance: p.vibrance / 100.0,
        saturation: p.saturation / 100.0,
    }
}
```

- [ ] **Step 3: Wire `finish_image` into the three render paths**

In `render_view` (after the inversion, the non-raw branch):

```rust
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    let fin = finish_image(&inv, &finish_from(&params));
    to_jpeg_b64(&fin, false, PREVIEW_JPEG_QUALITY)
```

In `thumbnail`:

```rust
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&small, &ip, mode_from(&params.mode));
    let fin = finish_image(&inv, &finish_from(&params));
    to_jpeg_b64(&fin, false, 82)
```

In `export_image`:

```rust
    let ip = resolve_params(&params, &thumb, base);
    let inv = invert_image(&full, &ip, mode_from(&params.mode));
    let fin = finish_image(&inv, &finish_from(&params));
    film_core::export::write_tiff16(&fin, Path::new(&out_path)).map_err(|e| format!("{e}"))
```

In `develop_image`, the thumbnail render also goes through finishing (defaults = identity, so behaviour unchanged but consistent):

```rust
    let ip = resolve_params(&default_invert_params(), &thumb, base);
    let inv_thumb = invert_image(&small, &ip, Mode::B);
    let inv_thumb = finish_image(&inv_thumb, &finish_from(&default_invert_params()));
    let thumbnail = to_jpeg_b64(&inv_thumb, false, 82)?;
```

- [ ] **Step 4: Add the `as_shot_wb` command**

Add to `commands.rs`:

```rust
/// Estimated as-shot white point for the developed image, as (Kelvin, tint).
/// The UI seeds the Temp/Tint sliders with this when an image becomes active.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AsShotWb { pub temp: f32, pub tint: f32 }

#[tauri::command]
pub fn as_shot_wb(id: String, session: State<Session>) -> Result<AsShotWb, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    // Gray-world estimate on a neutral first inversion, then gains → CCT.
    let neutral = default_invert_params();
    let ip = build_params(&neutral, dev.base);
    let first = invert_image(&dev.thumb, &ip, mode_from(&neutral.mode));
    let gains = auto_wb_gains(&first);
    let (temp, tint) = gains_to_cct(gains);
    Ok(AsShotWb { temp, tint: tint * 150.0 }) // back to UI −150..150
}
```

- [ ] **Step 5: Register the command in `lib.rs`**

In `app/src-tauri/src/lib.rs`, add to the `generate_handler!` list:

```rust
            commands::as_shot_wb,
```

- [ ] **Step 6: Update the existing backend test that references `wb_from_temp_tint`**

The test `wb_temp_tint_directions` (commands.rs `tests`) calls the removed `wb_from_temp_tint`. Replace it with a Kelvin-based directional test:

```rust
    #[test]
    fn wb_from_params_directions() {
        let warm = wb_from_params(3000.0, 0.0);
        let cool = wb_from_params(9000.0, 0.0);
        assert!(warm[0] < cool[0], "warm should cut red vs cool");
        let green = wb_from_params(5500.0, -150.0);
        assert!(green[1] > 1.0, "negative tint boosts green");
    }
```

- [ ] **Step 7: Build and run backend tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core && cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: film-core (now 27 + new finish/wb tests) PASS; app backend tests PASS including `wb_from_params_directions`.

- [ ] **Step 8: Commit**

```bash
git add app/src-tauri/src/session.rs app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(backend): EV exposure, Kelvin WB, finishing layer wiring, as_shot_wb command"
```

---

## Task 5: TS contract — extend `InvertParams`, `defaultParams`, `asShotWb`

**Files:**
- Modify: `app/src/lib/api.ts`

- [ ] **Step 1: Extend the `InvertParams` interface and `api`**

In `app/src/lib/api.ts`, replace the `InvertParams` interface and add the new fields + command:

```ts
export interface InvertParams {
  mode: "b" | "c";
  stock: "none" | "portra400" | "fujic200";
  base_rect: [number, number, number, number] | null;
  exposure: number; // EV stops (−5..5)
  black: number; gamma: number;
  auto_wb: boolean;
  temp: number; // Kelvin
  tint: number; // −150..150
  contrast: number; highlights: number; shadows: number;
  whites: number; blacks: number;
  texture: number; vibrance: number; saturation: number;
}

export interface AsShotWb { temp: number; tint: number }
```

Add to the `api` object:

```ts
  asShotWb: (id: string) => invoke<AsShotWb>("as_shot_wb", { id }),
```

Replace `defaultParams`:

```ts
export const defaultParams = (): InvertParams => ({
  mode: "b", stock: "none", base_rect: null,
  exposure: 0, black: 0, gamma: 0.4545,
  auto_wb: true, temp: 5500, tint: 0,
  contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
  texture: 0, vibrance: 0, saturation: 0,
});
```

- [ ] **Step 2: Typecheck**

Run: `cd app && npm run check`
Expected: no NEW errors from `api.ts`. (The pre-existing `workflow.test.ts` `path` error and a11y warnings remain; do not introduce new errors. `Adjustments.svelte` will now error on missing fields — that's expected and removed in Task 10.)

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/api.ts
git commit -m "feat(app): extend InvertParams contract (EV, Kelvin, finishing) + asShotWb"
```

---

## Task 6: Reusable `Slider.svelte`

**Files:**
- Create: `app/src/lib/develop/Slider.svelte`

- [ ] **Step 1: Create the component**

Create `app/src/lib/develop/Slider.svelte`:

```svelte
<script lang="ts">
  export let label: string;
  export let min: number;
  export let max: number;
  export let step = 1;
  export let value: number;
  export let def = 0;                 // double-click reset target
  export let gradient = "";           // CSS background for the track
  export let format: (v: number) => string = (v) => `${Math.round(v)}`;
</script>

<div class="slider">
  <div class="row">
    <span class="label" on:dblclick={() => (value = def)}>{label}</span>
    <span class="val">{format(value)}</span>
  </div>
  <input
    type="range" {min} {max} {step} bind:value
    class:grad={!!gradient}
    style={gradient ? `--track:${gradient}` : ""}
    on:dblclick={() => (value = def)}
  />
</div>

<style>
  .slider { margin: 7px 0; }
  .row { display: flex; justify-content: space-between; font-size: 11px;
    color: var(--text-dim); margin-bottom: 2px; }
  .val { color: var(--text); font-variant-numeric: tabular-nums; }
  .label { cursor: default; }
  input[type="range"] { width: 100%; height: 3px; border-radius: 3px;
    -webkit-appearance: none; appearance: none; background: var(--glass-brd);
    accent-color: var(--accent); }
  input.grad { background: var(--track); }
  input[type="range"]::-webkit-slider-thumb { -webkit-appearance: none;
    width: 12px; height: 12px; border-radius: 50%; background: #fff;
    border: 1px solid rgba(0,0,0,0.3); box-shadow: 0 1px 3px rgba(0,0,0,0.4); cursor: grab; }
  input[type="range"]:active::-webkit-slider-thumb { cursor: grabbing; }
</style>
```

- [ ] **Step 2: Typecheck**

Run: `cd app && npm run check`
Expected: no new errors from `Slider.svelte`.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/develop/Slider.svelte
git commit -m "feat(app): reusable compact Slider with gradient track + dbl-click reset"
```

---

## Task 7: Histogram — binning helper + component

**Files:**
- Create: `app/src/lib/viewport/histogram.ts`
- Create: `app/src/lib/viewport/histogram.test.ts`
- Create: `app/src/lib/viewport/Histogram.svelte`
- Modify: `app/src/lib/store.ts`
- Modify: `app/src/lib/viewport/Viewport.svelte`

- [ ] **Step 1: Add the `previewSrc` store**

In `app/src/lib/store.ts`, add:

```ts
/** Data-URL of the latest rendered develop preview; drives the histogram. */
export const previewSrc = writable<string>("");
```

- [ ] **Step 2: Write the binning helper + failing test**

Create `app/src/lib/viewport/histogram.ts`:

```ts
export interface Bins { r: number[]; g: number[]; b: number[] }

/** Bin RGBA bytes (from canvas getImageData) into 256 buckets per channel. */
export function binPixels(data: Uint8ClampedArray): Bins {
  const r = new Array(256).fill(0);
  const g = new Array(256).fill(0);
  const b = new Array(256).fill(0);
  for (let i = 0; i < data.length; i += 4) {
    r[data[i]]++; g[data[i + 1]]++; b[data[i + 2]]++;
  }
  return { r, g, b };
}

/** Build an SVG polyline points string for one channel, normalized to height h. */
export function channelPath(bins: number[], w: number, h: number): string {
  const max = Math.max(1, ...bins);
  return bins.map((v, i) => {
    const x = (i / 255) * w;
    const y = h - (v / max) * h;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(" ");
}
```

Create `app/src/lib/viewport/histogram.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { binPixels, channelPath } from "./histogram";

describe("binPixels", () => {
  it("counts each channel value into its bucket", () => {
    // two pixels: (255,0,0) and (255,128,0)
    const data = new Uint8ClampedArray([255, 0, 0, 255, 255, 128, 0, 255]);
    const bins = binPixels(data);
    expect(bins.r[255]).toBe(2);
    expect(bins.g[0]).toBe(1);
    expect(bins.g[128]).toBe(1);
    expect(bins.b[0]).toBe(2);
  });
});

describe("channelPath", () => {
  it("maps the peak bucket to y=0 (top)", () => {
    const bins = new Array(256).fill(0);
    bins[0] = 10;
    const pts = channelPath(bins, 256, 80);
    expect(pts.startsWith("0.0,0.0")).toBe(true);
  });
});
```

- [ ] **Step 3: Run the test**

Run: `cd app && npx vitest run src/lib/viewport/histogram.test.ts`
Expected: PASS.

- [ ] **Step 4: Create `Histogram.svelte`**

Create `app/src/lib/viewport/Histogram.svelte`:

```svelte
<script lang="ts">
  import { previewSrc } from "../store";
  import { binPixels, channelPath } from "./histogram";

  const W = 256, H = 76;
  let rPath = "", gPath = "", bPath = "";
  let timer: ReturnType<typeof setTimeout> | null = null;
  const cv = typeof document !== "undefined" ? document.createElement("canvas") : null;

  function compute(src: string) {
    if (!src || !cv) { rPath = gPath = bPath = ""; return; }
    const img = new Image();
    img.onload = () => {
      const w = 256, h = Math.max(1, Math.round((img.height / img.width) * 256));
      cv.width = w; cv.height = h;
      const ctx = cv.getContext("2d", { willReadFrequently: true });
      if (!ctx) return;
      ctx.drawImage(img, 0, 0, w, h);
      const { data } = ctx.getImageData(0, 0, w, h);
      const bins = binPixels(data);
      rPath = channelPath(bins.r, W, H);
      gPath = channelPath(bins.g, W, H);
      bPath = channelPath(bins.b, W, H);
    };
    img.src = src;
  }
  $: { const s = $previewSrc; if (timer) clearTimeout(timer); timer = setTimeout(() => compute(s), 120); }
</script>

<div class="hist">
  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none">
    <polyline points={rPath} class="r" />
    <polyline points={gPath} class="g" />
    <polyline points={bPath} class="b" />
  </svg>
</div>

<style>
  .hist { height: 76px; border-radius: 8px; background: rgba(0,0,0,0.35);
    padding: 4px; margin-bottom: 10px; }
  svg { width: 100%; height: 100%; display: block; }
  polyline { fill: none; stroke-width: 1; mix-blend-mode: screen; }
  .r { stroke: #ff5a5a; } .g { stroke: #5aff7a; } .b { stroke: #5a9cff; }
</style>
```

- [ ] **Step 5: Publish `previewSrc` from Viewport**

In `app/src/lib/viewport/Viewport.svelte`, import the store and set it after a successful render. Add to the script imports:

```ts
  import { previewSrc } from "../store";
```

In `render()`, after `src = await api.renderView(...)`, publish it (only for the interactive develop canvas):

```ts
      src = await api.renderView(id, params, { crop: [0, 0, imgW, imgH], out_w, out_h, raw });
      if (interactive && !raw) previewSrc.set(src);
```

- [ ] **Step 6: Typecheck + vitest**

Run: `cd app && npm run check && npx vitest run`
Expected: no new errors; histogram + existing vitest pass.

- [ ] **Step 7: Commit**

```bash
git add app/src/lib/viewport/histogram.ts app/src/lib/viewport/histogram.test.ts app/src/lib/viewport/Histogram.svelte app/src/lib/store.ts app/src/lib/viewport/Viewport.svelte
git commit -m "feat(app): live color histogram from the develop preview"
```

---

## Task 8: Icons + tool store + Toolbar

**Files:**
- Modify: `app/src/lib/icons/Icon.svelte`
- Modify: `app/src/lib/store.ts`
- Create: `app/src/lib/develop/Toolbar.svelte`

- [ ] **Step 1: Add glyphs to `Icon.svelte`**

In the `paths` record of `app/src/lib/icons/Icon.svelte`, add (Lucide path data):

```ts
    sliders: '<line x1="4" x2="4" y1="21" y2="14"/><line x1="4" x2="4" y1="10" y2="3"/><line x1="12" x2="12" y1="21" y2="12"/><line x1="12" x2="12" y1="8" y2="3"/><line x1="20" x2="20" y1="21" y2="16"/><line x1="20" x2="20" y1="12" y2="3"/><line x1="2" x2="6" y1="14" y2="14"/><line x1="10" x2="14" y1="8" y2="8"/><line x1="18" x2="22" y1="16" y2="16"/>',
    crop: '<path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M18 22V8a2 2 0 0 0-2-2H2"/>',
    eraser: '<path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/>',
    brush: '<path d="m9.06 11.9 8.07-8.06a2.85 2.85 0 1 1 4.03 4.03l-8.06 8.08"/><path d="M7.07 14.94c-1.66 0-3 1.35-3 3.02 0 1.33-2.5 1.52-2 2.02 1.08 1.1 2.49 2.02 4 2.02 2.2 0 4-1.8 4-4.04a3.01 3.01 0 0 0-3-3.02z"/>',
    "rotate-cw": '<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>',
```

- [ ] **Step 2: Add the `tool` store**

In `app/src/lib/store.ts`, add:

```ts
export type Tool = "edit" | "crop" | "eraser" | "brush";
export const tool = writable<Tool>("edit");
```

- [ ] **Step 3: Create `Toolbar.svelte`**

Create `app/src/lib/develop/Toolbar.svelte`:

```svelte
<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { tool, type Tool } from "../store";

  const tools: { id: Tool; icon: string; label: string; enabled: boolean }[] = [
    { id: "edit", icon: "sliders", label: "Edit", enabled: true },
    { id: "crop", icon: "crop", label: "Crop", enabled: true },
    { id: "eraser", icon: "eraser", label: "Eraser (soon)", enabled: false },
    { id: "brush", icon: "brush", label: "Brush (soon)", enabled: false },
  ];
</script>

<div class="toolbar">
  {#each tools as t}
    <button
      class:on={$tool === t.id} disabled={!t.enabled} title={t.label}
      on:click={() => t.enabled && tool.set(t.id)}
    >
      <Icon name={t.icon} size={17} />
    </button>
  {/each}
</div>

<style>
  .toolbar { display: flex; gap: 4px; margin-bottom: 12px;
    padding-bottom: 10px; border-bottom: 1px solid var(--glass-brd); }
  button { flex: 1; display: grid; place-items: center; padding: 7px 0;
    border-radius: 8px; border: 1px solid transparent; background: transparent;
    color: var(--text-dim); cursor: pointer; }
  button.on { color: #fff; background: rgba(224,52,52,0.18);
    border-color: rgba(224,52,52,0.5); }
  button:disabled { opacity: 0.35; cursor: default; }
</style>
```

- [ ] **Step 4: Typecheck**

Run: `cd app && npm run check`
Expected: no new errors.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/icons/Icon.svelte app/src/lib/store.ts app/src/lib/develop/Toolbar.svelte
git commit -m "feat(app): develop toolbar (edit/crop + disabled eraser/brush) + tool store"
```

---

## Task 9: Basic panel — WB (Kelvin + gradients) + Tone + Presence

**Files:**
- Create: `app/src/lib/develop/gradients.ts`
- Create: `app/src/lib/develop/gradients.test.ts`
- Create: `app/src/lib/develop/Basic.svelte`

- [ ] **Step 1: Write gradient/format helpers + failing test**

Create `app/src/lib/develop/gradients.ts`:

```ts
// CSS linear-gradient track backgrounds for sliders.
export const TEMP_GRADIENT =
  "linear-gradient(90deg, #4a90ff 0%, #cfd8e6 50%, #ffd24a 100%)";
export const TINT_GRADIENT =
  "linear-gradient(90deg, #4ad24a 0%, #cfcfcf 50%, #ff4af0 100%)";
export const SAT_GRADIENT =
  "linear-gradient(90deg, #808080 0%, #ff0000 17%, #ffff00 33%, " +
  "#00ff00 50%, #00ffff 67%, #0000ff 83%, #ff00ff 100%)";

/** Lightroom-style signed integer (e.g. +24, −13, 0). */
export function signed(v: number): string {
  const r = Math.round(v);
  return r > 0 ? `+${r}` : `${r}`;
}

/** EV display with two decimals and sign (e.g. +1.30, 0.00). */
export function ev(v: number): string {
  return (v > 0 ? "+" : "") + v.toFixed(2);
}

/** Kelvin display (rounded to nearest 10). */
export function kelvin(v: number): string {
  return `${Math.round(v / 10) * 10}`;
}
```

Create `app/src/lib/develop/gradients.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { signed, ev, kelvin } from "./gradients";

describe("formatters", () => {
  it("signed adds + for positives, − stays, 0 is 0", () => {
    expect(signed(24)).toBe("+24");
    expect(signed(-13)).toBe("-13");
    expect(signed(0)).toBe("0");
  });
  it("ev shows two decimals with sign", () => {
    expect(ev(1.3)).toBe("+1.30");
    expect(ev(0)).toBe("0.00");
  });
  it("kelvin rounds to nearest 10", () => {
    expect(kelvin(8437)).toBe("8440");
  });
});
```

- [ ] **Step 2: Run the test**

Run: `cd app && npx vitest run src/lib/develop/gradients.test.ts`
Expected: PASS.

- [ ] **Step 3: Create `Basic.svelte`**

Create `app/src/lib/develop/Basic.svelte`:

```svelte
<script lang="ts">
  import { params, activeId } from "../store";
  import { api } from "../api";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import { TEMP_GRADIENT, TINT_GRADIENT, SAT_GRADIENT, signed, ev, kelvin } from "./gradients";

  let open = true;

  // Seed Temp/Tint from the estimated as-shot white point when the image changes.
  let seededFor: string | null = null;
  async function seed(id: string | null) {
    if (!id || seededFor === id) return;
    seededFor = id;
    try {
      const wb = await api.asShotWb(id);
      params.update((p) => ({ ...p, temp: wb.temp, tint: wb.tint }));
    } catch { /* not developed yet */ }
  }
  $: seed($activeId);

  function autoWb() { seededFor = null; seed($activeId); }
</script>

<div class="section">
  <button class="head" on:click={() => (open = !open)}>
    <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
    <span>Basic</span>
  </button>

  {#if open}
    <div class="body">
      <!-- White Balance -->
      <div class="sub">White Balance</div>
      <div class="seg">
        <button class:on={$params.mode === "b"} on:click={() => params.update((p) => ({ ...p, mode: "b" }))}>B · density</button>
        <button class:on={$params.mode === "c"} on:click={() => params.update((p) => ({ ...p, mode: "c" }))}>C · per-chan</button>
      </div>
      <select bind:value={$params.stock}>
        <option value="none">No film profile</option>
        <option value="portra400">Kodak Portra 400</option>
        <option value="fujic200">Fuji C200</option>
      </select>
      <div class="wbhead">
        <span>Temp / Tint</span>
        <button class="auto" on:click={autoWb}>Auto</button>
      </div>
      <Slider label="Temp" min={2000} max={50000} step={50}
        bind:value={$params.temp} def={5500} gradient={TEMP_GRADIENT} format={kelvin} />
      <Slider label="Tint" min={-150} max={150} step={1}
        bind:value={$params.tint} def={0} gradient={TINT_GRADIENT} format={signed} />

      <!-- Tone -->
      <div class="sub">Tone</div>
      <Slider label="Exposure" min={-5} max={5} step={0.05} bind:value={$params.exposure} def={0} format={ev} />
      <Slider label="Contrast" min={-100} max={100} bind:value={$params.contrast} def={0} format={signed} />
      <Slider label="Highlights" min={-100} max={100} bind:value={$params.highlights} def={0} format={signed} />
      <Slider label="Shadows" min={-100} max={100} bind:value={$params.shadows} def={0} format={signed} />
      <Slider label="Whites" min={-100} max={100} bind:value={$params.whites} def={0} format={signed} />
      <Slider label="Blacks" min={-100} max={100} bind:value={$params.blacks} def={0} format={signed} />

      <!-- Presence -->
      <div class="sub">Presence</div>
      <Slider label="Texture" min={-100} max={100} bind:value={$params.texture} def={0} format={signed} />
      <Slider label="Vibrance" min={-100} max={100} bind:value={$params.vibrance} def={0} gradient={SAT_GRADIENT} format={signed} />
      <Slider label="Saturation" min={-100} max={100} bind:value={$params.saturation} def={0} gradient={SAT_GRADIENT} format={signed} />
    </div>
  {/if}
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { display: flex; align-items: center; gap: 6px; width: 100%;
    background: transparent; border: 0; color: var(--text); font-weight: 600;
    padding: 4px 0; cursor: pointer; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  .seg { display: flex; gap: 6px; margin-bottom: 8px; }
  .seg button { flex: 1; padding: 6px; border-radius: 8px; font-size: 12px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text-dim); }
  .seg button.on { color: #fff; background: rgba(224,52,52,0.18); border-color: rgba(224,52,52,0.5); }
  select { width: 100%; padding: 6px; border-radius: 8px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); margin-bottom: 8px; }
  .wbhead { display: flex; justify-content: space-between; align-items: center;
    font-size: 11px; color: var(--text-dim); margin: 4px 0; }
  .auto { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
</style>
```

- [ ] **Step 4: Typecheck + vitest**

Run: `cd app && npm run check && npx vitest run`
Expected: no new errors; gradient tests pass.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/develop/gradients.ts app/src/lib/develop/gradients.test.ts app/src/lib/develop/Basic.svelte
git commit -m "feat(app): Basic edit panel — compact Kelvin WB, Tone, Presence with gradients"
```

---

## Task 10: Develop layout rewrite — wire panel, Export footer, remove left

**Files:**
- Modify: `app/src/lib/tabs/Develop.svelte`
- Delete: `app/src/lib/panels/Adjustments.svelte`

- [ ] **Step 1: Rewrite `Develop.svelte`**

Replace `app/src/lib/tabs/Develop.svelte` entirely:

```svelte
<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { activeId, params, images, tool } from "../store";
  import { api } from "../api";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";
  import QualityMenu from "../viewport/QualityMenu.svelte";
  import Histogram from "../viewport/Histogram.svelte";
  import Toolbar from "../develop/Toolbar.svelte";
  import Basic from "../develop/Basic.svelte";
  import GlassPanel from "../glass/GlassPanel.svelte";

  $: active = $images.find((i) => i.id === $activeId);

  let thumbTimer: ReturnType<typeof setTimeout> | null = null;
  function refreshThumb() {
    if (thumbTimer) clearTimeout(thumbTimer);
    const id = $activeId;
    if (!id) return;
    thumbTimer = setTimeout(async () => {
      try {
        const t = await api.thumbnail(id, $params);
        images.update((xs) => xs.map((i) => (i.id === id ? { ...i, thumbnail: t } : i)));
      } catch { /* ignore */ }
    }, 400);
  }
  $: $params, $activeId, refreshThumb();

  let menu: { x: number; y: number } | null = null;
  function onContext(e: MouseEvent) { e.preventDefault(); menu = { x: e.clientX, y: e.clientY }; }

  let exporting = false, msg = "";
  async function exportTiff() {
    if (!$activeId) return;
    const out = await save({ defaultPath: "redroom-export.tiff", filters: [{ name: "TIFF", extensions: ["tiff"] }] });
    if (!out) return;
    exporting = true; msg = "";
    try { await api.exportImage($activeId, $params, out); msg = "Exported ✓"; }
    catch (e) { msg = "Error: " + e; }
    exporting = false;
  }
</script>

<div class="layout" on:contextmenu={onContext}>
  <section class="center">
    {#if active?.developed}
      <Viewport id={$activeId} params={$params}
                imgW={active.metadata.width} imgH={active.metadata.height} />
    {:else}<div class="hint">Not developed yet</div>{/if}
  </section>

  <aside class="right">
    <GlassPanel>
      <Histogram />
      <Toolbar />
      {#if $tool === "edit"}
        <Basic />
      {:else if $tool === "crop"}
        <div class="placeholder">Crop — coming in Plan 2</div>
      {/if}
      <button class="export" on:click={exportTiff} disabled={exporting || !$activeId}>
        {exporting ? "Exporting…" : "Export 16-bit TIFF"}
      </button>
      {#if msg}<div class="msg">{msg}</div>{/if}
    </GlassPanel>
  </aside>

  <footer class="bottom"><Filmstrip /></footer>
</div>
{#if menu}<QualityMenu x={menu.x} y={menu.y} on:close={() => (menu = null)} />{/if}

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 1fr 300px; grid-template-rows: 1fr 88px;
    grid-template-areas: "center right" "bottom bottom"; }
  .right { grid-area: right; min-height: 0; overflow-y: auto; }
  .center { grid-area: center; min-height: 0; display: grid; place-items: center; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
  .placeholder { color: var(--text-dim); font-size: 12px; padding: 20px 0; text-align: center; }
  .export { width: 100%; margin-top: 12px; padding: 10px; border: 0; border-radius: 10px;
    background: var(--accent); color: white; font-weight: 600; cursor: pointer; }
  .export:disabled { opacity: 0.5; }
  .msg { margin-top: 8px; color: var(--text-dim); font-size: 12px; }
</style>
```

- [ ] **Step 2: Delete the obsolete panel**

```bash
git rm app/src/lib/panels/Adjustments.svelte
```

- [ ] **Step 3: Typecheck**

Run: `cd app && npm run check`
Expected: no new errors (the `Adjustments.svelte` errors from Task 5 are gone; the only remaining ERROR is the pre-existing `workflow.test.ts` `path` fixture).

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/tabs/Develop.svelte
git commit -m "feat(app): Develop layout — remove left pane, wire histogram/toolbar/Basic + Export footer"
```

---

## Task 11: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Rust tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core && cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: all PASS (film-core core 27 + new finish/wb; app backend incl. `wb_from_params_directions`).

- [ ] **Step 2: Frontend typecheck + unit tests**

Run: `cd app && npm run check && npx vitest run`
Expected: no NEW typecheck errors (pre-existing `workflow.test.ts` `path` error + a11y warnings remain); all vitest pass including `histogram` and `gradients`.

- [ ] **Step 3: Manual smoke (user, in the running app)**

Verify in the Develop tab:
- Left pane gone; histogram visible at top of the right panel and updates as sliders move.
- Toolbar shows Edit (active) + Crop, with Eraser/Brush greyed out.
- Basic section collapses/expands; Temp shows a Kelvin number with blue→yellow track, Tint signed with green→magenta track; Vibrance/Saturation show the spectrum track. Double-click a slider label/handle resets it.
- Exposure/Contrast/Highlights/Shadows/Whites/Blacks/Texture/Vibrance/Saturation visibly change the image.
- "Auto" reseeds Temp/Tint.
- Export still produces a TIFF.

- [ ] **Step 4: Final commit (if any verification fixups were needed)**

```bash
git add -A && git commit -m "test: verify Develop redesign Plan 1 (panels + engine)"
```

---

## Self-Review notes

- **Spec coverage:** A (Task 10) · B (Task 7) · C (Task 8) · D (Tasks 6, 9) · E (Tasks 1, 2, 4, 5) · F (Tasks 3, 4, 9). Crop (G) is Plan 2, intentionally excluded.
- **Type consistency:** `InvertParams` field names match across `api.ts` (Task 5), `session.rs` (Task 4), and `Basic.svelte` bindings (Task 9). `FinishParams` field names match across `finish.rs` (Task 1) and `finish_from` (Task 4). `AsShotWb { temp, tint }` matches between `commands.rs` and `api.ts`.
- **Known carry-over:** the pre-existing `app/src/lib/workflow.test.ts` `path` fixture error is unrelated and out of scope.
