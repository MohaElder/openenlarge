# Color Mixer (HSL Mixer + Point Color) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Lightroom-style Color Mixer panel to the Develop section — an 8-band HSL Mixer plus a Point Color eyedropper tool — rendering identically on the GPU (live preview) and CPU (thumbnails/export).

**Architecture:** New per-image params (`cm_*` flat fields + a `pc_samples` array) flow through the existing params store and catalog JSON. A shared RGB→HSL→adjust→HSL primitive is implemented in Rust (`finish.rs`, authoritative + unit-tested) and mirrored in GLSL (`shaders.ts`) and TS (`finish.ts`). A new `ColorMixer.svelte` panel mirrors the existing `ColorGrading.svelte`. The Point Color eyedropper reuses the `tool` store + canvas-readback pattern already in the app.

**Tech Stack:** Svelte 5 + TypeScript (SvelteKit), Rust (`film-core` crate, Tauri), WebGL2 GLSL ES 3.00. Tests: `vitest` (TS), `cargo test` (Rust).

**Reference spec:** `docs/superpowers/specs/2026-06-05-color-mixer-design.md`

---

## Shared constants & math (the contract Rust + GLSL + TS must all match)

These appear verbatim in multiple tasks. Band index order is fixed:
`0 red, 1 orange, 2 yellow, 3 green, 4 aqua, 5 blue, 6 purple, 7 magenta`.

```
BAND_CENTERS (deg) = [0, 30, 60, 120, 180, 240, 280, 320]
CM_FALLOFF_DEG     = 50.0     // band hue half-width
CM_HUE_SHIFT_MAX   = 30.0     // deg at slider = ±100 (×/100 → ±1 unit)
CM_LUM_GAIN        = 0.25     // lightness delta at unit weight, unit lum
CM_SAT_GATE_LO     = 0.05     // below this saturation, a pixel is "gray": no hue/sat band effect
CM_SAT_GATE_HI     = 0.20

PC_RANGE_MIN_DEG   = 5.0      // hue half-width at range = 0
PC_RANGE_MAX_DEG   = 60.0     // hue half-width at range = 100
PC_SAT_TOL         = 0.25     // base sat tolerance (variance widens)
PC_LUM_TOL         = 0.25     // base lum tolerance
PC_VAR_SPAN        = 2.0      // tolerance multiplier swing across variance −1..1
```

Weighting formulas (all languages identical):

```
wrap180(d): d in deg, returned in (−180, 180]
bandWeight(h, center): d = |wrap180(h − center)|;
    w = d >= CM_FALLOFF_DEG ? 0 : 0.5*(1 + cos(PI * d / CM_FALLOFF_DEG))
satGate(s): smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s)
pcHueWeight(h, target, range): hw = PC_RANGE_MIN_DEG + (range/100)*(PC_RANGE_MAX_DEG−PC_RANGE_MIN_DEG);
    d = |wrap180(h − target)|; d >= hw ? 0 : 0.5*(1 + cos(PI * d / hw))
pcTol(base, variance): max(0.02, base * (1 + (variance/100) * PC_VAR_SPAN))   // variance in −100..100
pcAxisWeight(diff, tol): clamp(1 − diff/tol, 0, 1)
```

HSL conversion (standard; hue in degrees, s/l in [0,1]):

```
rgb2hsl(r,g,b):
  mx=max, mn=min, l=(mx+mn)/2
  if mx==mn: return (0, 0, l)
  d=mx−mn
  s = l>0.5 ? d/(2−mx−mn) : d/(mx+mn)
  h = mx==r ? (g−b)/d + (g<b?6:0) : mx==g ? (b−r)/d + 2 : (r−g)/d + 4
  return (h*60, s, l)
hue2rgb(p,q,t): t = fract(t); // wrap to [0,1)
  t<1/6 → p+(q−p)*6*t; t<1/2 → q; t<2/3 → p+(q−p)*(2/3−t)*6; else p
hsl2rgb(h,s,l):
  if s==0: return (l,l,l)
  q = l<0.5 ? l*(1+s) : l+s−l*s; p = 2*l−q; hk = h/360
  return (hue2rgb(p,q,hk+1/3), hue2rgb(p,q,hk), hue2rgb(p,q,hk−1/3))
```

Stage order in finishing: `… → color_grade → color_mix (Mixer) → point_color`. Point
Color masks are computed from the **post-Mixer** pixel (its input), all sample
shifts accumulated, then applied once.

---

## Task 1: TS params, sample type, and defaults

**Files:**
- Modify: `app/src/lib/api.ts` (interface `InvertParams` ~28-53; `defaultParams` ~190-208)

- [ ] **Step 1: Add the `PointColorSample` type and Mixer/Point Color fields to `InvertParams`**

In `app/src/lib/api.ts`, immediately above `export interface InvertParams {` add:

```ts
/** One Point Color sample: a picked target color + its per-sample adjustments. */
export interface PointColorSample {
  hue: number;   // target hue, 0..360 (fixed at pick time)
  sat: number;   // target saturation, 0..1
  lum: number;   // target lightness, 0..1
  hue_shift: number;  // −100..100
  sat_shift: number;  // −100..100
  lum_shift: number;  // −100..100
  variance: number;   // −100..100 (widens sat/lum tolerance)
  range: number;      // 0..100 (hue-window half-width), default 50
}
/** Lightroom Color Mixer band order (fixed). */
export const CM_BANDS = ["red","orange","yellow","green","aqua","blue","purple","magenta"] as const;
export type CmBand = (typeof CM_BANDS)[number];
```

Then inside `InvertParams`, after the `cg_*` block (after `cg_balance: number;`) add:

```ts

  // --- Color Mixer (HSL): 8 bands × hue/sat/lum, each −100..100, 0 = identity ---
  cm_red_hue: number; cm_red_sat: number; cm_red_lum: number;
  cm_orange_hue: number; cm_orange_sat: number; cm_orange_lum: number;
  cm_yellow_hue: number; cm_yellow_sat: number; cm_yellow_lum: number;
  cm_green_hue: number; cm_green_sat: number; cm_green_lum: number;
  cm_aqua_hue: number; cm_aqua_sat: number; cm_aqua_lum: number;
  cm_blue_hue: number; cm_blue_sat: number; cm_blue_lum: number;
  cm_purple_hue: number; cm_purple_sat: number; cm_purple_lum: number;
  cm_magenta_hue: number; cm_magenta_sat: number; cm_magenta_lum: number;
  // --- Point Color: up to 8 sampled swatches ---
  pc_samples: PointColorSample[];
```

- [ ] **Step 2: Add defaults in `defaultParams()`**

In `app/src/lib/api.ts`, inside the object returned by `defaultParams()`, after the `cg_blending: 50, cg_balance: 0,` line add:

```ts

  cm_red_hue: 0, cm_red_sat: 0, cm_red_lum: 0,
  cm_orange_hue: 0, cm_orange_sat: 0, cm_orange_lum: 0,
  cm_yellow_hue: 0, cm_yellow_sat: 0, cm_yellow_lum: 0,
  cm_green_hue: 0, cm_green_sat: 0, cm_green_lum: 0,
  cm_aqua_hue: 0, cm_aqua_sat: 0, cm_aqua_lum: 0,
  cm_blue_hue: 0, cm_blue_sat: 0, cm_blue_lum: 0,
  cm_purple_hue: 0, cm_purple_sat: 0, cm_purple_lum: 0,
  cm_magenta_hue: 0, cm_magenta_sat: 0, cm_magenta_lum: 0,
  pc_samples: [],
```

- [ ] **Step 3: Typecheck**

Run: `cd app && npm run check 2>&1 | tail -5`
Expected: No new errors referencing `api.ts`. (Other panels don't yet read these fields, so no breakage.)

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/api.ts
git commit -m "feat(color-mixer): add Mixer + Point Color params and defaults (TS)"
```

---

## Task 2: Rust params mirror + defaults

**Files:**
- Modify: `app/src-tauri/src/session.rs` (`struct InvertParams` 37-90)
- Modify: `app/src-tauri/src/commands.rs` (`default_invert_params` 81-98)

- [ ] **Step 1: Add the `PointColorSample` struct and fields to the Rust `InvertParams`**

In `app/src-tauri/src/session.rs`, directly above `pub struct InvertParams {` add:

```rust
/// One Point Color sample: a picked target color + per-sample adjustments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointColorSample {
    pub hue: f32,        // 0..360
    pub sat: f32,        // 0..1
    pub lum: f32,        // 0..1
    pub hue_shift: f32,  // −100..100
    pub sat_shift: f32,
    pub lum_shift: f32,
    pub variance: f32,   // −100..100
    pub range: f32,      // 0..100
}
```

Then inside `pub struct InvertParams`, after the `cg_balance` field (line ~89) add:

```rust

    // Color Mixer (HSL): 8 bands × hue/sat/lum, each −100..100.
    #[serde(default)] pub cm_red_hue: f32, #[serde(default)] pub cm_red_sat: f32, #[serde(default)] pub cm_red_lum: f32,
    #[serde(default)] pub cm_orange_hue: f32, #[serde(default)] pub cm_orange_sat: f32, #[serde(default)] pub cm_orange_lum: f32,
    #[serde(default)] pub cm_yellow_hue: f32, #[serde(default)] pub cm_yellow_sat: f32, #[serde(default)] pub cm_yellow_lum: f32,
    #[serde(default)] pub cm_green_hue: f32, #[serde(default)] pub cm_green_sat: f32, #[serde(default)] pub cm_green_lum: f32,
    #[serde(default)] pub cm_aqua_hue: f32, #[serde(default)] pub cm_aqua_sat: f32, #[serde(default)] pub cm_aqua_lum: f32,
    #[serde(default)] pub cm_blue_hue: f32, #[serde(default)] pub cm_blue_sat: f32, #[serde(default)] pub cm_blue_lum: f32,
    #[serde(default)] pub cm_purple_hue: f32, #[serde(default)] pub cm_purple_sat: f32, #[serde(default)] pub cm_purple_lum: f32,
    #[serde(default)] pub cm_magenta_hue: f32, #[serde(default)] pub cm_magenta_sat: f32, #[serde(default)] pub cm_magenta_lum: f32,
    // Point Color: up to 8 samples.
    #[serde(default)] pub pc_samples: Vec<PointColorSample>,
```

Confirm `Deserialize` is imported in `session.rs` (the existing struct already derives `Serialize, Deserialize`; reuse the same `use serde::...` import).

- [ ] **Step 2: Add defaults in `default_invert_params()`**

In `app/src-tauri/src/commands.rs`, inside `default_invert_params()` after `cg_blending: 50.0, cg_balance: 0.0,` add:

```rust
        cm_red_hue: 0.0, cm_red_sat: 0.0, cm_red_lum: 0.0,
        cm_orange_hue: 0.0, cm_orange_sat: 0.0, cm_orange_lum: 0.0,
        cm_yellow_hue: 0.0, cm_yellow_sat: 0.0, cm_yellow_lum: 0.0,
        cm_green_hue: 0.0, cm_green_sat: 0.0, cm_green_lum: 0.0,
        cm_aqua_hue: 0.0, cm_aqua_sat: 0.0, cm_aqua_lum: 0.0,
        cm_blue_hue: 0.0, cm_blue_sat: 0.0, cm_blue_lum: 0.0,
        cm_purple_hue: 0.0, cm_purple_sat: 0.0, cm_purple_lum: 0.0,
        cm_magenta_hue: 0.0, cm_magenta_sat: 0.0, cm_magenta_lum: 0.0,
        pc_samples: Vec::new(),
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -15` (from repo root — builds the whole workspace including the Tauri crate `app`).
Expected: compiles. Any "missing field" error in another `InvertParams { … }` literal means there is a second constructor — grep `InvertParams {` and add the same defaults there.

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/session.rs app/src-tauri/src/commands.rs
git commit -m "feat(color-mixer): mirror Mixer + Point Color params in Rust"
```

---

## Task 3: Rust HSL primitive + round-trip test

**Files:**
- Modify: `crates/film-core/src/finish.rs` (add functions near `hsv_hue_rgb`, tests in the `tests` module)

- [ ] **Step 1: Write the failing round-trip test**

In `crates/film-core/src/finish.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn rgb_hsl_round_trip() {
        let colors = [
            [0.2_f32, 0.4, 0.6], [0.9, 0.1, 0.3], [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.7, 0.7, 0.2],
        ];
        for c in colors {
            let (h, s, l) = rgb2hsl(c);
            let back = hsl2rgb(h, s, l);
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-4, "c={c:?} back={back:?}");
            }
        }
    }
```

- [ ] **Step 2: Run — expect failure (functions undefined)**

Run: `cargo test -p film-core rgb_hsl_round_trip 2>&1 | tail -15`
Expected: compile error `cannot find function rgb2hsl`.

- [ ] **Step 3: Implement the HSL primitive**

In `crates/film-core/src/finish.rs`, after `hsv_hue_rgb` (line ~50) add:

```rust
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
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
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
    if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t }
    else if t < 0.5 { q }
    else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 }
    else { p }
}

/// HSL → RGB (0..1). Inverse of `rgb2hsl`.
fn hsl2rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hk = h / 360.0;
    [
        hue2rgb(p, q, hk + 1.0 / 3.0),
        hue2rgb(p, q, hk),
        hue2rgb(p, q, hk - 1.0 / 3.0),
    ]
}
```

- [ ] **Step 4: Run — expect pass**

Run: `cargo test -p film-core rgb_hsl_round_trip 2>&1 | tail -8`
Expected: `test ... rgb_hsl_round_trip ... ok`.

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(color-mixer): RGB<->HSL primitive in film-core"
```

---

## Task 4: Rust `ColorMix` struct + Mixer (`color_mix`) + tests

**Files:**
- Modify: `crates/film-core/src/finish.rs`

- [ ] **Step 1: Write failing tests for the Mixer**

In `mod tests`, add:

```rust
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
            for k in 0..3 { assert!((out[k] - c[k]).abs() < 1e-4, "c={c:?} out={out:?}"); }
        }
    }

    #[test]
    fn mixer_band_isolation() {
        // Push the BLUE band saturation up; a pure-red pixel must be ~unchanged,
        // a blue pixel must gain chroma.
        let cm = mix_with(|m| m.cm_sat[5] = 1.0); // blue = index 5, slider +100 → unit 1.0
        let red = color_mix([0.8, 0.1, 0.1], &cm);
        assert!((red[0] - 0.8).abs() < 0.02 && (red[1] - 0.1).abs() < 0.02, "red moved: {red:?}");
        let blue_in = [0.2, 0.3, 0.8];
        let blue = color_mix(blue_in, &cm);
        let chroma = |p: [f32; 3]| p.iter().cloned().fold(0.0_f32, f32::max)
            - p.iter().cloned().fold(1.0_f32, f32::min);
        assert!(chroma(blue) > chroma(blue_in), "blue chroma: {} -> {}", chroma(blue_in), chroma(blue));
    }

    #[test]
    fn mixer_gray_pixel_unaffected_by_hue() {
        let cm = mix_with(|m| { m.cm_hue[0] = 1.0; m.cm_hue[5] = 1.0; });
        let out = color_mix([0.5, 0.5, 0.5], &cm);
        for k in 0..3 { assert!((out[k] - 0.5).abs() < 1e-3, "gray moved: {out:?}"); }
    }
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p film-core color_mix 2>&1 | tail -15`
Expected: compile errors (`ColorMix` / `color_mix` undefined).

- [ ] **Step 3: Implement `ColorMix` and `color_mix`**

In `crates/film-core/src/finish.rs`, after the `hsl2rgb` block add the shared constants and the Mixer:

```rust
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
    if x <= -180.0 { x += 360.0; }
    x
}

#[inline]
fn band_weight(h: f32, center: f32) -> f32 {
    let d = wrap180(h - center).abs();
    if d >= CM_FALLOFF_DEG { 0.0 } else { 0.5 * (1.0 + (PI * d / CM_FALLOFF_DEG).cos()) }
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
    pub hue: f32,        // 0..360
    pub sat: f32,        // 0..1
    pub lum: f32,        // 0..1
    pub hue_shift: f32,  // −1..1
    pub sat_shift: f32,
    pub lum_shift: f32,
    pub variance: f32,   // −100..100 (raw; used by pc_tol)
    pub range: f32,      // 0..100 (raw)
}

/// Apply the 8-band HSL Mixer to one pixel.
fn color_mix(rgb: [f32; 3], cm: &ColorMix) -> [f32; 3] {
    let (mut h, mut s, mut l) = rgb2hsl(rgb);
    let gate = smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s);
    let mut sat_factor = 1.0_f32;
    let mut hue_delta = 0.0_f32;
    let mut lum_delta = 0.0_f32;
    for i in 0..8 {
        let w = band_weight(h, BAND_CENTERS[i]);
        if w <= 0.0 { continue; }
        hue_delta += w * gate * cm.cm_hue[i] * CM_HUE_SHIFT_MAX;
        sat_factor += w * gate * cm.cm_sat[i];
        lum_delta += w * cm.cm_lum[i] * CM_LUM_GAIN;
    }
    h += hue_delta;
    s = (s * sat_factor).clamp(0.0, 1.0);
    l = (l + lum_delta).clamp(0.0, 1.0);
    hsl2rgb(h, s, l)
}
```

(`smoothstep` already exists in `finish.rs:33`.)

- [ ] **Step 4: Run — expect pass**

Run: `cargo test -p film-core color_mix 2>&1 | tail -12`
Expected: `color_mix_default_is_identity`, `mixer_band_isolation`, `mixer_gray_pixel_unaffected_by_hue` all `ok`.

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(color-mixer): 8-band HSL mixer (color_mix) in film-core"
```

---

## Task 5: Rust Point Color (`point_color`) + tests

**Files:**
- Modify: `crates/film-core/src/finish.rs`

- [ ] **Step 1: Write failing tests**

In `mod tests`, add:

```rust
    fn sample(hue: f32) -> PcSample {
        PcSample { hue, sat: 0.6, lum: 0.5, hue_shift: 0.0, sat_shift: 1.0,
                   lum_shift: 0.0, variance: 0.0, range: 50.0 }
    }

    #[test]
    fn point_color_default_no_samples_is_identity() {
        let cm = ColorMix::default(); // no samples
        let c = [0.8, 0.2, 0.2];
        let out = point_color(c, &cm.samples);
        for k in 0..3 { assert!((out[k] - c[k]).abs() < 1e-4, "{out:?}"); }
    }

    #[test]
    fn point_color_sample_isolation() {
        // Sample targets RED (hue 0); a red pixel gains chroma, a green pixel is untouched.
        let samples = vec![sample(0.0)];
        let chroma = |p: [f32; 3]| p.iter().cloned().fold(0.0_f32, f32::max)
            - p.iter().cloned().fold(1.0_f32, f32::min);
        let red_in = [0.8, 0.25, 0.25];
        let red = point_color(red_in, &samples);
        assert!(chroma(red) > chroma(red_in), "red chroma {} -> {}", chroma(red_in), chroma(red));
        let green_in = [0.2, 0.8, 0.25];
        let green = point_color(green_in, &samples);
        for k in 0..3 { assert!((green[k] - green_in[k]).abs() < 0.02, "green moved {green:?}"); }
    }

    #[test]
    fn point_color_order_independent() {
        let a = sample(0.0);
        let b = sample(120.0);
        let c = [0.6, 0.5, 0.3];
        let ab = point_color(c, &vec![a, b]);
        let ba = point_color(c, &vec![b, a]);
        for k in 0..3 { assert!((ab[k] - ba[k]).abs() < 1e-5, "order matters {ab:?} {ba:?}"); }
    }
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p film-core point_color 2>&1 | tail -15`
Expected: `cannot find function point_color`.

- [ ] **Step 3: Implement `point_color`**

In `crates/film-core/src/finish.rs`, after `color_mix`, add:

```rust
#[inline]
fn pc_tol(base: f32, variance: f32) -> f32 {
    (base * (1.0 + (variance / 100.0) * PC_VAR_SPAN)).max(0.02)
}

#[inline]
fn pc_hue_weight(h: f32, target: f32, range: f32) -> f32 {
    let hw = PC_RANGE_MIN_DEG + (range / 100.0) * (PC_RANGE_MAX_DEG - PC_RANGE_MIN_DEG);
    let d = wrap180(h - target).abs();
    if d >= hw { 0.0 } else { 0.5 * (1.0 + (PI * d / hw).cos()) }
}

/// Apply all Point Color samples to one pixel. Masks use the input HSL so samples
/// are order-independent; shifts accumulate then apply once.
fn point_color(rgb: [f32; 3], samples: &[PcSample]) -> [f32; 3] {
    if samples.is_empty() { return rgb; }
    let (h, s, l) = rgb2hsl(rgb);
    let mut hue_delta = 0.0_f32;
    let mut sat_factor = 1.0_f32;
    let mut lum_delta = 0.0_f32;
    for sm in samples {
        let wh = pc_hue_weight(h, sm.hue, sm.range);
        if wh <= 0.0 { continue; }
        let ws = (1.0 - (s - sm.sat).abs() / pc_tol(PC_SAT_TOL, sm.variance)).clamp(0.0, 1.0);
        let wl = (1.0 - (l - sm.lum).abs() / pc_tol(PC_LUM_TOL, sm.variance)).clamp(0.0, 1.0);
        let w = wh * ws * wl;
        if w <= 0.0 { continue; }
        hue_delta += w * sm.hue_shift * CM_HUE_SHIFT_MAX;
        sat_factor += w * sm.sat_shift;
        lum_delta += w * sm.lum_shift * CM_LUM_GAIN;
    }
    hsl2rgb(h + hue_delta, (s * sat_factor).clamp(0.0, 1.0), (l + lum_delta).clamp(0.0, 1.0))
}
```

- [ ] **Step 4: Run — expect pass**

Run: `cargo test -p film-core point_color 2>&1 | tail -10`
Expected: the three `point_color_*` tests `ok`.

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(color-mixer): Point Color sample application in film-core"
```

---

## Task 6: Wire Mixer + Point Color into `finish_pixel` and `finish_from`

**Files:**
- Modify: `crates/film-core/src/finish.rs` (`FinishParams` 179-205; `finish_pixel` 236-245)
- Modify: `app/src-tauri/src/commands.rs` (`finish_from` ~204-...)

- [ ] **Step 1: Write the failing identity test for the full chain**

In `mod tests`, add:

```rust
    #[test]
    fn finish_pixel_color_mixer_default_is_identity() {
        // Default FinishParams (no mixer, no samples) must leave pixels unchanged.
        let p = FinishParams::default();
        for v in [0.1_f32, 0.35, 0.7, 0.95] {
            let px = [v, v * 0.6, v * 0.3];
            let out = finish_pixel(px, &p);
            for c in 0..3 { assert!((out[c] - px[c]).abs() < 1e-4, "v={v} c={c} {out:?}"); }
        }
    }
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p film-core finish_pixel_color_mixer_default_is_identity 2>&1 | tail -12`
Expected: compile error — `FinishParams` has no field `cm` yet (you will reference it in step 3). If it compiles and passes already, you still must complete step 3 to actually invoke the mixer; proceed.

- [ ] **Step 3: Add `cm` to `FinishParams` and call it in `finish_pixel`**

In `crates/film-core/src/finish.rs`, add to `pub struct FinishParams` after `pub cg: ColorGrade,`:

```rust
    pub cm: ColorMix,
```

In `impl Default for FinishParams`, after `cg: ColorGrade::default(),` add:

```rust
            cm: ColorMix::default(),
```

In `finish_pixel`, change the final line from:

```rust
    color_grade(curved, &p.cg)
```

to:

```rust
    let graded = color_grade(curved, &p.cg);
    let mixed = color_mix(graded, &p.cm);
    point_color(mixed, &p.cm.samples)
```

- [ ] **Step 4: Run — expect pass**

Run: `cargo test -p film-core finish_pixel_color_mixer_default_is_identity 2>&1 | tail -8`
Expected: `ok`. Also run the whole module: `cargo test -p film-core finish 2>&1 | tail -20` — all green.

- [ ] **Step 5: Build the `ColorMix` from `InvertParams` in `finish_from`**

In `app/src-tauri/src/commands.rs`, add this helper above `finish_from`:

```rust
fn color_mix_from(p: &crate::session::InvertParams) -> film_core::finish::ColorMix {
    use film_core::finish::{ColorMix, PcSample};
    let cm_hue = [
        p.cm_red_hue, p.cm_orange_hue, p.cm_yellow_hue, p.cm_green_hue,
        p.cm_aqua_hue, p.cm_blue_hue, p.cm_purple_hue, p.cm_magenta_hue,
    ];
    let cm_sat = [
        p.cm_red_sat, p.cm_orange_sat, p.cm_yellow_sat, p.cm_green_sat,
        p.cm_aqua_sat, p.cm_blue_sat, p.cm_purple_sat, p.cm_magenta_sat,
    ];
    let cm_lum = [
        p.cm_red_lum, p.cm_orange_lum, p.cm_yellow_lum, p.cm_green_lum,
        p.cm_aqua_lum, p.cm_blue_lum, p.cm_purple_lum, p.cm_magenta_lum,
    ];
    let samples = p.pc_samples.iter().map(|s| PcSample {
        hue: s.hue, sat: s.sat, lum: s.lum,
        hue_shift: s.hue_shift / 100.0, sat_shift: s.sat_shift / 100.0, lum_shift: s.lum_shift / 100.0,
        variance: s.variance, range: s.range,
    }).collect();
    ColorMix {
        cm_hue: cm_hue.map(|v| v / 100.0),
        cm_sat: cm_sat.map(|v| v / 100.0),
        cm_lum: cm_lum.map(|v| v / 100.0),
        samples,
    }
}
```

Then in `finish_from`, in the returned `FinishParams { … }` literal, after `lut_r, lut_g, lut_b, cg,` add:

```rust
        cm: color_mix_from(p),
```

Make sure `ColorMix`, `PcSample` are `pub` and re-exported: in `crates/film-core/src/finish.rs` they are declared `pub struct` — confirm `finish` module is public (it is, since `ColorGrade`/`FinishParams` are used the same way in `commands.rs`).

- [ ] **Step 6: Build + full test**

Run: `cargo build 2>&1 | tail -8 && cargo test -p film-core 2>&1 | tail -15`
Expected: builds; all film-core tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/film-core/src/finish.rs app/src-tauri/src/commands.rs
git commit -m "feat(color-mixer): apply Mixer + Point Color in CPU finish pipeline"
```

---

## Task 7: TS mirror — `colorMix()` uniforms + identity test

**Files:**
- Modify: `app/src/lib/develop/finish.ts`
- Test: `app/src/lib/develop/finish.test.ts` (existing file — add cases)

- [ ] **Step 1: Write the failing identity test**

In `app/src/lib/develop/finish.test.ts`, add (import `colorMix` alongside existing imports):

```ts
import { colorMix } from "./finish";
import { defaultParams } from "../api";

describe("colorMix", () => {
  it("default params yield identity uniforms", () => {
    const u = colorMix(defaultParams());
    expect(Array.from(u.cm_hue)).toEqual(new Array(8).fill(0));
    expect(Array.from(u.cm_sat)).toEqual(new Array(8).fill(0));
    expect(Array.from(u.cm_lum)).toEqual(new Array(8).fill(0));
    expect(u.pc_count).toBe(0);
  });

  it("packs a single sample and divides shifts by 100", () => {
    const p = defaultParams();
    p.pc_samples = [{ hue: 200, sat: 0.5, lum: 0.4, hue_shift: 50, sat_shift: -100,
      lum_shift: 0, variance: 0, range: 50 }];
    const u = colorMix(p);
    expect(u.pc_count).toBe(1);
    expect(u.pc_hue[0]).toBeCloseTo(200);
    expect(u.pc_hue_shift[0]).toBeCloseTo(0.5);
    expect(u.pc_sat_shift[0]).toBeCloseTo(-1);
  });
});
```

- [ ] **Step 2: Run — expect failure**

Run: `cd app && npx vitest run src/lib/develop/finish.test.ts 2>&1 | tail -15`
Expected: FAIL — `colorMix` is not exported.

- [ ] **Step 3: Implement `colorMix` + `ColorMixUniforms`**

In `app/src/lib/develop/finish.ts`, append:

```ts
import { CM_BANDS, type PointColorSample } from "../api";

/** Packed Color Mixer uniforms for the GPU (mirror finish.rs::ColorMix). Mixer
 *  slider values are pre-divided by 100; sample shifts too. Arrays are length 8;
 *  Point Color slots beyond pc_count are zero-filled. */
export interface ColorMixUniforms {
  cm_hue: Float32Array; cm_sat: Float32Array; cm_lum: Float32Array;
  pc_count: number;
  pc_hue: Float32Array; pc_sat: Float32Array; pc_lum: Float32Array;
  pc_hue_shift: Float32Array; pc_sat_shift: Float32Array; pc_lum_shift: Float32Array;
  pc_variance: Float32Array; pc_range: Float32Array;
}

export function colorMix(p: InvertParams): ColorMixUniforms {
  const cm_hue = new Float32Array(8);
  const cm_sat = new Float32Array(8);
  const cm_lum = new Float32Array(8);
  CM_BANDS.forEach((b, i) => {
    cm_hue[i] = num((p as Record<string, number>)[`cm_${b}_hue`]) / 100;
    cm_sat[i] = num((p as Record<string, number>)[`cm_${b}_sat`]) / 100;
    cm_lum[i] = num((p as Record<string, number>)[`cm_${b}_lum`]) / 100;
  });
  const mk = () => new Float32Array(8);
  const pc_hue = mk(), pc_sat = mk(), pc_lum = mk();
  const pc_hue_shift = mk(), pc_sat_shift = mk(), pc_lum_shift = mk();
  const pc_variance = mk(), pc_range = mk();
  const samples: PointColorSample[] = Array.isArray(p.pc_samples) ? p.pc_samples.slice(0, 8) : [];
  samples.forEach((s, i) => {
    pc_hue[i] = num(s.hue); pc_sat[i] = num(s.sat); pc_lum[i] = num(s.lum);
    pc_hue_shift[i] = num(s.hue_shift) / 100;
    pc_sat_shift[i] = num(s.sat_shift) / 100;
    pc_lum_shift[i] = num(s.lum_shift) / 100;
    pc_variance[i] = num(s.variance);
    pc_range[i] = num(s.range);
  });
  return { cm_hue, cm_sat, cm_lum, pc_count: samples.length,
    pc_hue, pc_sat, pc_lum, pc_hue_shift, pc_sat_shift, pc_lum_shift, pc_variance, pc_range };
}
```

(`num` and `InvertParams` are already in scope in `finish.ts`.)

- [ ] **Step 4: Run — expect pass**

Run: `cd app && npx vitest run src/lib/develop/finish.test.ts 2>&1 | tail -12`
Expected: all `colorMix` tests PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/develop/finish.ts app/src/lib/develop/finish.test.ts
git commit -m "feat(color-mixer): TS uniform mirror (colorMix)"
```

---

## Task 8: GPU shader — HSL + `colorMixer` + `pointColor`

**Files:**
- Modify: `app/src/lib/viewport/gl/shaders.ts` (`FRAG`)

- [ ] **Step 1: Declare the new uniforms**

In `app/src/lib/viewport/gl/shaders.ts`, in `FRAG`, after the color-grading uniform block (the `uniform float u_cg_sh_edge, u_cg_hi_edge, u_cg_soft;` line) add:

```glsl
// Color Mixer (HSL): per-band sliders pre-divided to unit. Band centers are const.
uniform float u_cm_hue[8];
uniform float u_cm_sat[8];
uniform float u_cm_lum[8];
// Point Color: up to 8 samples.
uniform int u_pc_count;
uniform float u_pc_hue[8];
uniform float u_pc_sat[8];
uniform float u_pc_lum[8];
uniform float u_pc_hue_shift[8];
uniform float u_pc_sat_shift[8];
uniform float u_pc_lum_shift[8];
uniform float u_pc_variance[8];
uniform float u_pc_range[8];
```

- [ ] **Step 2: Add the HSL + mixer + point-color GLSL functions**

In `FRAG`, immediately before `vec3 finishAt(vec2 uv) {`, add:

```glsl
const float PI_F = 3.14159265358979;
const float BAND_CENTERS[8] = float[8](0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0);
const float CM_FALLOFF_DEG = 50.0;
const float CM_HUE_SHIFT_MAX = 30.0;
const float CM_LUM_GAIN = 0.25;
const float CM_SAT_GATE_LO = 0.05;
const float CM_SAT_GATE_HI = 0.20;
const float PC_RANGE_MIN_DEG = 5.0;
const float PC_RANGE_MAX_DEG = 60.0;
const float PC_SAT_TOL = 0.25;
const float PC_LUM_TOL = 0.25;
const float PC_VAR_SPAN = 2.0;

vec3 rgb2hsl(vec3 c) {
  float mx = max(max(c.r, c.g), c.b);
  float mn = min(min(c.r, c.g), c.b);
  float l = (mx + mn) * 0.5;
  if (mx - mn < 1e-7) return vec3(0.0, 0.0, l);
  float d = mx - mn;
  float s = l > 0.5 ? d / (2.0 - mx - mn) : d / (mx + mn);
  float h;
  if (mx == c.r) h = (c.g - c.b) / d + (c.g < c.b ? 6.0 : 0.0);
  else if (mx == c.g) h = (c.b - c.r) / d + 2.0;
  else h = (c.r - c.g) / d + 4.0;
  return vec3(h * 60.0, s, l);
}
float hue2rgb(float p, float q, float t) {
  t = fract(t);
  if (t < 1.0/6.0) return p + (q - p) * 6.0 * t;
  if (t < 0.5) return q;
  if (t < 2.0/3.0) return p + (q - p) * (2.0/3.0 - t) * 6.0;
  return p;
}
vec3 hsl2rgb(float h, float s, float l) {
  if (s <= 0.0) return vec3(l);
  float q = l < 0.5 ? l * (1.0 + s) : l + s - l * s;
  float p = 2.0 * l - q;
  float hk = h / 360.0;
  return vec3(hue2rgb(p, q, hk + 1.0/3.0), hue2rgb(p, q, hk), hue2rgb(p, q, hk - 1.0/3.0));
}
float wrap180(float d) {
  float x = mod(d + 180.0, 360.0) - 180.0;
  return x <= -180.0 ? x + 360.0 : x;
}
float bandWeight(float h, float center) {
  float d = abs(wrap180(h - center));
  return d >= CM_FALLOFF_DEG ? 0.0 : 0.5 * (1.0 + cos(PI_F * d / CM_FALLOFF_DEG));
}
vec3 colorMixer(vec3 rgb) {
  vec3 hsl = rgb2hsl(rgb);
  float h = hsl.x, s = hsl.y, l = hsl.z;
  float gate = smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s);
  float hueDelta = 0.0, satFactor = 1.0, lumDelta = 0.0;
  for (int i = 0; i < 8; i++) {
    float w = bandWeight(h, BAND_CENTERS[i]);
    hueDelta += w * gate * u_cm_hue[i] * CM_HUE_SHIFT_MAX;
    satFactor += w * gate * u_cm_sat[i];
    lumDelta += w * u_cm_lum[i] * CM_LUM_GAIN;
  }
  return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}
float pcTol(float base, float variance) {
  return max(0.02, base * (1.0 + (variance / 100.0) * PC_VAR_SPAN));
}
float pcHueWeight(float h, float target, float range) {
  float hw = PC_RANGE_MIN_DEG + (range / 100.0) * (PC_RANGE_MAX_DEG - PC_RANGE_MIN_DEG);
  float d = abs(wrap180(h - target));
  return d >= hw ? 0.0 : 0.5 * (1.0 + cos(PI_F * d / hw));
}
vec3 pointColor(vec3 rgb) {
  if (u_pc_count <= 0) return rgb;
  vec3 hsl = rgb2hsl(rgb);
  float h = hsl.x, s = hsl.y, l = hsl.z;
  float hueDelta = 0.0, satFactor = 1.0, lumDelta = 0.0;
  for (int k = 0; k < 8; k++) {
    if (k >= u_pc_count) break;
    float wh = pcHueWeight(h, u_pc_hue[k], u_pc_range[k]);
    if (wh <= 0.0) continue;
    float ws = clamp(1.0 - abs(s - u_pc_sat[k]) / pcTol(PC_SAT_TOL, u_pc_variance[k]), 0.0, 1.0);
    float wl = clamp(1.0 - abs(l - u_pc_lum[k]) / pcTol(PC_LUM_TOL, u_pc_variance[k]), 0.0, 1.0);
    float w = wh * ws * wl;
    hueDelta += w * u_pc_hue_shift[k] * CM_HUE_SHIFT_MAX;
    satFactor += w * u_pc_sat_shift[k];
    lumDelta += w * u_pc_lum_shift[k] * CM_LUM_GAIN;
  }
  return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}
```

- [ ] **Step 3: Call them after color grading**

In `FRAG`, in `finishAt`, change the last line from:

```glsl
  return colorGrade(cu);
```

to:

```glsl
  return pointColor(colorMixer(colorGrade(cu)));
```

- [ ] **Step 4: Typecheck (shader is a string; just ensure the module compiles)**

Run: `cd app && npm run check 2>&1 | grep -i shaders || echo "no shaders.ts type errors"`
Expected: `no shaders.ts type errors`. (Runtime GLSL compile is verified visually in Task 13.)

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/viewport/gl/shaders.ts
git commit -m "feat(color-mixer): GPU shader HSL mixer + point color"
```

---

## Task 9: Renderer plumbing — upload Color Mixer uniforms

**Files:**
- Modify: `app/src/lib/viewport/gl/renderer.ts`

- [ ] **Step 1: Import the uniform type and add a field + setter**

In `app/src/lib/viewport/gl/renderer.ts`:

At the top, extend the existing import from `../../develop/finish`:

```ts
import type { ColorGradeUniforms, ColorMixUniforms } from "../../develop/finish";
```

After `private cg: ColorGradeUniforms | null = null;` (line ~82) add:

```ts
  private cm: ColorMixUniforms | null = null;
```

After `setColorGrade(cg: ColorGradeUniforms) { this.cg = cg; }` (line ~178) add:

```ts
  setColorMix(cm: ColorMixUniforms) { this.cm = cm; }
```

- [ ] **Step 2: Look up the new uniform locations**

In the constructor, after the loop `for (const [u] of CG_FLOAT) this.loc[u] = gl.getUniformLocation(prog, u);` (line ~129) add:

```ts
    for (const u of [
      "u_cm_hue","u_cm_sat","u_cm_lum","u_pc_count","u_pc_hue","u_pc_sat","u_pc_lum",
      "u_pc_hue_shift","u_pc_sat_shift","u_pc_lum_shift","u_pc_variance","u_pc_range",
    ]) this.loc[u] = gl.getUniformLocation(prog, u);
```

- [ ] **Step 3: Upload them in `drawFinishPass`**

In `drawFinishPass`, after the color-grade upload block (after the `if (cg) { … }` block, ~line 124) add:

```ts
    const cm = this.cm;
    if (cm) {
      gl.uniform1fv(this.loc.u_cm_hue, cm.cm_hue);
      gl.uniform1fv(this.loc.u_cm_sat, cm.cm_sat);
      gl.uniform1fv(this.loc.u_cm_lum, cm.cm_lum);
      gl.uniform1i(this.loc.u_pc_count, cm.pc_count);
      gl.uniform1fv(this.loc.u_pc_hue, cm.pc_hue);
      gl.uniform1fv(this.loc.u_pc_sat, cm.pc_sat);
      gl.uniform1fv(this.loc.u_pc_lum, cm.pc_lum);
      gl.uniform1fv(this.loc.u_pc_hue_shift, cm.pc_hue_shift);
      gl.uniform1fv(this.loc.u_pc_sat_shift, cm.pc_sat_shift);
      gl.uniform1fv(this.loc.u_pc_lum_shift, cm.pc_lum_shift);
      gl.uniform1fv(this.loc.u_pc_variance, cm.pc_variance);
      gl.uniform1fv(this.loc.u_pc_range, cm.pc_range);
    }
```

- [ ] **Step 4: Thread through GPU export**

In `renderExport(...)` (signature ~line 141), add a `cm: ColorMixUniforms` parameter after the `cg: ColorGradeUniforms` parameter, and after the existing `this.setColorGrade(cg);` call add `this.setColorMix(cm);`.

Then update the one caller of `renderExport` — grep for it:

Run: `cd app && rg -n "renderExport\(" src`

For the caller (in the GPU export path, likely `src/lib/viewport/Viewport.svelte` or an export module), pass `colorMix(params)` as the new argument, importing `colorMix` from `../develop/finish` if not already imported there.

- [ ] **Step 5: Typecheck**

Run: `cd app && npm run check 2>&1 | tail -8`
Expected: no errors in `renderer.ts` or the export caller. Fix any signature mismatch the checker reports.

- [ ] **Step 6: Commit**

```bash
git add app/src/lib/viewport/gl/renderer.ts app/src/lib/viewport/Viewport.svelte
git commit -m "feat(color-mixer): upload Color Mixer uniforms in renderer + export"
```

---

## Task 10: Viewport live-preview wiring

**Files:**
- Modify: `app/src/lib/viewport/Viewport.svelte`

- [ ] **Step 1: Import + set the uniforms on draw**

In `app/src/lib/viewport/Viewport.svelte`, extend the existing finish import:

```ts
import { toneLutBytes, colorGrade, colorMix } from "../develop/finish";
```

After `renderer.setColorGrade(colorGrade(params));` (line ~143) add:

```ts
    renderer.setColorMix(colorMix(params));
```

- [ ] **Step 2: Make the live preview re-render on Color Mixer changes**

In the `finishKey` reactive string (the `[ ... ].join("|")` near line 270-279), after the `params.cg_blending, params.cg_balance,` entry add:

```ts
    params.cm_red_hue, params.cm_red_sat, params.cm_red_lum,
    params.cm_orange_hue, params.cm_orange_sat, params.cm_orange_lum,
    params.cm_yellow_hue, params.cm_yellow_sat, params.cm_yellow_lum,
    params.cm_green_hue, params.cm_green_sat, params.cm_green_lum,
    params.cm_aqua_hue, params.cm_aqua_sat, params.cm_aqua_lum,
    params.cm_blue_hue, params.cm_blue_sat, params.cm_blue_lum,
    params.cm_purple_hue, params.cm_purple_sat, params.cm_purple_lum,
    params.cm_magenta_hue, params.cm_magenta_sat, params.cm_magenta_lum,
    JSON.stringify(params.pc_samples),
```

- [ ] **Step 3: Typecheck**

Run: `cd app && npm run check 2>&1 | tail -8`
Expected: no new errors.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(color-mixer): wire Color Mixer into live GPU preview"
```

---

## Task 11: Slider gradients + i18n strings

**Files:**
- Modify: `app/src/lib/develop/gradients.ts`
- Modify: `app/src/lib/i18n/dict.ts`

- [ ] **Step 1: Add per-band Hue gradients and a shared Luminance gradient**

In `app/src/lib/develop/gradients.ts`, append:

```ts
/** Per-band hue-slider tracks (dark→band color→light is overkill; show the band hue
 *  shifting to its neighbors, Lightroom-style). Keyed by CM band name. */
export const CM_HUE_GRADIENTS: Record<string, string> = {
  red:     "linear-gradient(90deg,#ff00d4 0%,#ff0000 50%,#ff7a00 100%)",
  orange:  "linear-gradient(90deg,#ff0000 0%,#ff7a00 50%,#ffe000 100%)",
  yellow:  "linear-gradient(90deg,#ff7a00 0%,#ffe000 50%,#9dff00 100%)",
  green:   "linear-gradient(90deg,#ffe000 0%,#1fdf3f 50%,#00d9c0 100%)",
  aqua:    "linear-gradient(90deg,#1fdf3f 0%,#00d9c0 50%,#2a7bff 100%)",
  blue:    "linear-gradient(90deg,#00d9c0 0%,#2a7bff 50%,#7a3cff 100%)",
  purple:  "linear-gradient(90deg,#2a7bff 0%,#7a3cff 50%,#ff00d4 100%)",
  magenta: "linear-gradient(90deg,#7a3cff 0%,#ff00d4 50%,#ff0000 100%)",
};
/** Per-band saturation track: gray → the band's pure color. */
export const CM_SAT_GRADIENTS: Record<string, string> = {
  red:     "linear-gradient(90deg,#808080 0%,#ff2b2b 100%)",
  orange:  "linear-gradient(90deg,#808080 0%,#ff8c1a 100%)",
  yellow:  "linear-gradient(90deg,#808080 0%,#ffe000 100%)",
  green:   "linear-gradient(90deg,#808080 0%,#1fdf3f 100%)",
  aqua:    "linear-gradient(90deg,#808080 0%,#00d9c0 100%)",
  blue:    "linear-gradient(90deg,#808080 0%,#2a7bff 100%)",
  purple:  "linear-gradient(90deg,#808080 0%,#7a3cff 100%)",
  magenta: "linear-gradient(90deg,#808080 0%,#ff00d4 100%)",
};
/** Luminance track: dark → light. */
export const CM_LUM_GRADIENT = "linear-gradient(90deg,#1a1a1a 0%,#808080 50%,#f0f0f0 100%)";
```

- [ ] **Step 2: Add i18n strings**

In `app/src/lib/i18n/dict.ts`, find the `colorGrading.*` block (line ~160) and add a parallel `colorMixer.*` block within the same dictionary object (match the existing quoting/comma style):

```ts
    "colorMixer.title": "Color Mixer",
    "colorMixer.reset": "Reset",
    "colorMixer.tab.mixer": "Mixer",
    "colorMixer.tab.point": "Point Color",
    "colorMixer.adjust.hue": "Hue",
    "colorMixer.adjust.saturation": "Saturation",
    "colorMixer.adjust.luminance": "Luminance",
    "colorMixer.adjust.all": "All",
    "colorMixer.band.red": "Red",
    "colorMixer.band.orange": "Orange",
    "colorMixer.band.yellow": "Yellow",
    "colorMixer.band.green": "Green",
    "colorMixer.band.aqua": "Aqua",
    "colorMixer.band.blue": "Blue",
    "colorMixer.band.purple": "Purple",
    "colorMixer.band.magenta": "Magenta",
    "colorMixer.point.hint": "Use the Point Color dropper to add samples.",
    "colorMixer.point.hueShift": "Hue Shift",
    "colorMixer.point.satShift": "Sat. Shift",
    "colorMixer.point.lumShift": "Lum. Shift",
    "colorMixer.point.variance": "Variance",
    "colorMixer.point.range": "Range",
    "colorMixer.point.dropper": "Pick color",
    "colorMixer.point.delete": "Remove sample",
```

If `dict.ts` has more than one locale object (e.g. a `ja` map), add the same keys there too with translated or duplicated values so lookups never miss.

- [ ] **Step 3: Typecheck**

Run: `cd app && npm run check 2>&1 | tail -6`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/develop/gradients.ts app/src/lib/i18n/dict.ts
git commit -m "feat(color-mixer): band gradients + i18n strings"
```

---

## Task 12: `ColorMixer.svelte` panel (Mixer + Point Color tabs)

**Files:**
- Create: `app/src/lib/develop/ColorMixer.svelte`
- Modify: `app/src/lib/tabs/Develop.svelte` (import + place after `<ColorGrading />`)

This task builds the panel UI **without** the live eyedropper (that's Task 13). The
eyedropper button is present but calls a `startPick()` stub that's wired in Task 13.

- [ ] **Step 1: Create the panel component**

Create `app/src/lib/develop/ColorMixer.svelte`:

```svelte
<script lang="ts">
  import { t } from "$lib/i18n";
  import { params } from "../store";
  import { defaultParams, CM_BANDS, type PointColorSample } from "../api";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import { signed } from "./gradients";
  import { CM_HUE_GRADIENTS, CM_SAT_GRADIENTS, CM_LUM_GRADIENT } from "./gradients";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  // Set by Task 13 (Develop.svelte passes a callback to arm the viewport dropper).
  export let onPick: (() => void) | null = null;
  export let picking = false;

  let open = true;
  type Tab = "mixer" | "point";
  let tab: Tab = "mixer";
  type Adjust = "hue" | "saturation" | "luminance" | "all";
  let adjust: Adjust = "hue";

  const ADJ: { id: Adjust; key: string }[] = [
    { id: "hue", key: "colorMixer.adjust.hue" },
    { id: "saturation", key: "colorMixer.adjust.saturation" },
    { id: "luminance", key: "colorMixer.adjust.luminance" },
    { id: "all", key: "colorMixer.adjust.all" },
  ];

  // Map adjust → field suffix.
  const suffix = (a: Exclude<Adjust, "all">) =>
    a === "hue" ? "hue" : a === "saturation" ? "sat" : "lum";

  function resetMixer() {
    const d = defaultParams() as Record<string, number>;
    params.update((p) => {
      const next = { ...p } as Record<string, unknown>;
      for (const b of CM_BANDS) for (const s of ["hue", "sat", "lum"])
        next[`cm_${b}_${s}`] = d[`cm_${b}_${s}`];
      return next as typeof p;
    });
  }
  function resetPoint() {
    params.update((p) => ({ ...p, pc_samples: [] }));
  }

  // --- Point Color sample editing ---
  let selected = 0;
  $: samples = $params.pc_samples ?? [];
  $: sel = samples[selected] as PointColorSample | undefined;

  function updateSample(patch: Partial<PointColorSample>) {
    params.update((p) => {
      const arr = (p.pc_samples ?? []).slice();
      if (!arr[selected]) return p;
      arr[selected] = { ...arr[selected], ...patch };
      return { ...p, pc_samples: arr };
    });
  }
  function removeSample(i: number) {
    params.update((p) => {
      const arr = (p.pc_samples ?? []).slice();
      arr.splice(i, 1);
      return { ...p, pc_samples: arr };
    });
    if (selected >= samples.length - 1) selected = Math.max(0, samples.length - 2);
  }
  // CSS color for a sample swatch (its sampled HSL).
  const swatch = (s: PointColorSample) => `hsl(${s.hue} ${Math.round(s.sat * 100)}% ${Math.round(s.lum * 100)}%)`;
</script>

<div class="section">
  <div class="head">
    <button class="toggle" on:click={() => (open = !open)}>
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
      <span>{$t('colorMixer.title')}</span>
    </button>
    <button class="reset" on:click={() => (tab === "mixer" ? resetMixer() : resetPoint())}>
      {$t('colorMixer.reset')}
    </button>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <div class="tabs">
        <button class:on={tab === "mixer"} on:click={() => (tab = "mixer")}>{$t('colorMixer.tab.mixer')}</button>
        <button class:on={tab === "point"} on:click={() => (tab = "point")}>{$t('colorMixer.tab.point')}</button>
      </div>

      {#if tab === "mixer"}
        <div class="modes">
          {#each ADJ as a}
            <button class:on={adjust === a.id} on:click={() => (adjust = a.id)}>{$t(a.key)}</button>
          {/each}
        </div>

        {#if adjust === "all"}
          {#each CM_BANDS as b}
            <div class="bandgroup">
              <div class="bandname">{$t(`colorMixer.band.${b}`)}</div>
              <Slider label={$t('colorMixer.adjust.hue')} min={-100} max={100}
                bind:value={$params[`cm_${b}_hue`]} def={0} format={signed} gradient={CM_HUE_GRADIENTS[b]} />
              <Slider label={$t('colorMixer.adjust.saturation')} min={-100} max={100}
                bind:value={$params[`cm_${b}_sat`]} def={0} format={signed} gradient={CM_SAT_GRADIENTS[b]} />
              <Slider label={$t('colorMixer.adjust.luminance')} min={-100} max={100}
                bind:value={$params[`cm_${b}_lum`]} def={0} format={signed} gradient={CM_LUM_GRADIENT} />
            </div>
          {/each}
        {:else}
          {#each CM_BANDS as b}
            <Slider label={$t(`colorMixer.band.${b}`)} min={-100} max={100}
              bind:value={$params[`cm_${b}_${suffix(adjust)}`]} def={0} format={signed}
              gradient={adjust === "hue" ? CM_HUE_GRADIENTS[b] : adjust === "saturation" ? CM_SAT_GRADIENTS[b] : CM_LUM_GRADIENT} />
          {/each}
        {/if}
      {:else}
        <div class="point">
          <button class="dropper" class:on={picking} on:click={() => onPick?.()}>
            <Icon name="eyedropper" size={14} />
            <span>{$t('colorMixer.point.dropper')}</span>
          </button>

          {#if samples.length === 0}
            <p class="hint">{$t('colorMixer.point.hint')}</p>
          {:else}
            <div class="swatches">
              {#each samples as s, i}
                <button class="sw" class:sel={i === selected} style="background:{swatch(s)}"
                  on:click={() => (selected = i)} title={`${Math.round(s.hue)}°`}>
                  {#if i === selected}
                    <span class="rm" on:click|stopPropagation={() => removeSample(i)}
                      title={$t('colorMixer.point.delete')}>×</span>
                  {/if}
                </button>
              {/each}
            </div>

            {#if sel}
              <Slider label={$t('colorMixer.point.hueShift')} min={-100} max={100}
                value={sel.hue_shift} def={0} format={signed}
                on:input={(e) => updateSample({ hue_shift: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.satShift')} min={-100} max={100}
                value={sel.sat_shift} def={0} format={signed}
                on:input={(e) => updateSample({ sat_shift: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.lumShift')} min={-100} max={100}
                value={sel.lum_shift} def={0} format={signed}
                on:input={(e) => updateSample({ lum_shift: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.variance')} min={-100} max={100}
                value={sel.variance} def={0} format={signed}
                on:input={(e) => updateSample({ variance: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.range')} min={0} max={100}
                value={sel.range} def={50}
                on:input={(e) => updateSample({ range: +(e.target as HTMLInputElement).value })} />
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { display: flex; align-items: center; justify-content: space-between; width: 100%; padding: 4px 0; }
  .toggle { display: flex; align-items: center; gap: 6px; background: transparent; border: 0;
    color: var(--text); font-weight: 600; padding: 0; cursor: pointer; }
  .reset { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  .tabs { display: flex; gap: 4px; margin: 6px 0 8px; }
  .tabs button { flex: 1; background: var(--bg-1); border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 5px 2px; font-size: 11px; cursor: pointer; }
  .tabs button.on { color: var(--text); border-color: var(--accent); }
  .modes { display: flex; gap: 4px; margin: 4px 0 10px; }
  .modes button { flex: 1; background: var(--bg-1); border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 4px 2px; font-size: 10px; cursor: pointer; }
  .modes button.on { color: var(--text); border-color: var(--accent); }
  .bandgroup { margin-bottom: 10px; }
  .bandname { font-size: 11px; color: var(--text); margin: 6px 0 2px; }
  .point { margin-top: 6px; }
  .dropper { display: inline-flex; align-items: center; gap: 6px; background: var(--bg-1);
    border: 1px solid var(--glass-brd); color: var(--text-dim); border-radius: 6px;
    padding: 5px 10px; font-size: 11px; cursor: pointer; }
  .dropper.on { color: var(--text); border-color: var(--accent); }
  .hint { color: var(--text-dim); font-size: 11px; margin: 10px 2px; }
  .swatches { display: flex; flex-wrap: wrap; gap: 6px; margin: 10px 0; }
  .sw { width: 26px; height: 26px; border-radius: 6px; border: 1px solid var(--glass-brd);
    position: relative; cursor: pointer; padding: 0; }
  .sw.sel { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  .rm { position: absolute; top: -6px; right: -6px; width: 14px; height: 14px; line-height: 13px;
    text-align: center; font-size: 11px; border-radius: 50%; background: #222; color: #fff;
    border: 1px solid var(--glass-brd); }
</style>
```

Note on the Mixer sliders: they use `bind:value={$params[...]}` for two-way
binding (same pattern as Color Grading). The Point Color sliders use
`value` + `on:input` because they edit an array element, not a flat field.

If `svelte-check` flags `$params[\`cm_${b}_hue\`]` index typing, change those
bindings to a small helper: add `const F = (k: string) => k;` is not enough for
binding — instead bind via a getter/setter is awkward in Svelte. Simpler proven
fallback: cast once at the top, `$: P = $params as unknown as Record<string, number>;`
and bind `bind:value={P[...]}`. Svelte 5 supports binding to a member expression.
Verify in Step 3; if binding to an index expression isn't allowed, replace the
Mixer `{#each}` with an explicit per-band block using literal field names
(`bind:value={$params.cm_red_hue}` etc.) — verbose but guaranteed.

- [ ] **Step 2: Place the panel in Develop.svelte**

In `app/src/lib/tabs/Develop.svelte`, add the import next to the other develop panels (~line 14):

```ts
  import ColorMixer from "../develop/ColorMixer.svelte";
```

And in the edit-tool panel list (after `<ColorGrading />`, ~line 247) add:

```svelte
        <ColorMixer />
```

- [ ] **Step 3: Typecheck + run dev build**

Run: `cd app && npm run check 2>&1 | tail -12`
Expected: no errors. If index-binding errors appear on the Mixer sliders, apply the
fallback described in Step 1.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/develop/ColorMixer.svelte app/src/lib/tabs/Develop.svelte
git commit -m "feat(color-mixer): ColorMixer panel (Mixer + Point Color tabs)"
```

---

## Task 13: Point Color eyedropper (viewport pick → add sample)

**Files:**
- Modify: `app/src/lib/store.ts` (`Tool` union)
- Create: `app/src/lib/develop/colorPick.ts` (RGB→HSL helper for the picked pixel + canvas readback)
- Test: `app/src/lib/develop/colorPick.test.ts`
- Modify: `app/src/lib/viewport/Viewport.svelte` (sample the rendered canvas on click in pick mode)
- Modify: `app/src/lib/tabs/Develop.svelte` (arm/disarm + receive picked color → append sample)

- [ ] **Step 1: Add the tool mode**

In `app/src/lib/store.ts`, change:

```ts
export type Tool = "edit" | "crop" | "eraser" | "base_picker";
```

to:

```ts
export type Tool = "edit" | "crop" | "eraser" | "base_picker" | "point_picker";
```

- [ ] **Step 2: Write a failing test for the RGB→HSL pick helper**

Create `app/src/lib/develop/colorPick.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { rgbToHslSample } from "./colorPick";

describe("rgbToHslSample", () => {
  it("converts a mid red byte pixel to HSL fields", () => {
    const s = rgbToHslSample(204, 51, 51); // ~ [0.8,0.2,0.2]
    expect(s.hue).toBeCloseTo(0, 0);
    expect(s.sat).toBeGreaterThan(0.5);
    expect(s.lum).toBeCloseTo(0.5, 1);
    expect(s.hue_shift).toBe(0);
    expect(s.range).toBe(50);
  });
  it("gray maps to zero saturation", () => {
    const s = rgbToHslSample(128, 128, 128);
    expect(s.sat).toBeCloseTo(0, 2);
  });
});
```

- [ ] **Step 3: Run — expect failure**

Run: `cd app && npx vitest run src/lib/develop/colorPick.test.ts 2>&1 | tail -10`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement `colorPick.ts`**

Create `app/src/lib/develop/colorPick.ts`:

```ts
import type { PointColorSample } from "../api";

/** Convert an sRGB byte pixel to a fresh Point Color sample (zeroed shifts). */
export function rgbToHslSample(r8: number, g8: number, b8: number): PointColorSample {
  const r = r8 / 255, g = g8 / 255, b = b8 / 255;
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b);
  const l = (mx + mn) / 2;
  let h = 0, s = 0;
  if (mx - mn > 1e-7) {
    const d = mx - mn;
    s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn);
    if (mx === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (mx === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
  }
  return { hue: h, sat: s, lum: l,
    hue_shift: 0, sat_shift: 0, lum_shift: 0, variance: 0, range: 50 };
}

/** Read one pixel (CSS-pixel coords within the canvas) from a WebGL canvas that
 *  was created with preserveDrawingBuffer:true. Returns [r,g,b] bytes or null. */
export function readCanvasPixel(canvas: HTMLCanvasElement, cssX: number, cssY: number): [number, number, number] | null {
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true });
  if (!gl) return null;
  // Map CSS coords → drawing-buffer coords (account for devicePixelRatio scaling).
  const sx = Math.round(cssX * (canvas.width / canvas.clientWidth));
  const syTop = Math.round(cssY * (canvas.height / canvas.clientHeight));
  const sy = canvas.height - 1 - syTop; // GL origin is bottom-left
  const px = new Uint8Array(4);
  gl.readPixels(sx, sy, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
  return [px[0], px[1], px[2]];
}
```

- [ ] **Step 5: Run — expect pass**

Run: `cd app && npx vitest run src/lib/develop/colorPick.test.ts 2>&1 | tail -8`
Expected: both tests PASS. (`readCanvasPixel` is not unit-tested — it needs a real GL canvas; it's exercised manually in Step 9.)

- [ ] **Step 6: Emit a pick from the Viewport on click in `point_picker` mode**

In `app/src/lib/viewport/Viewport.svelte`:

Ensure the component imports the tool store and exposes the canvas element (it
already has `el`/canvas refs and uses `tool` indirectly via props; check how
`eraser`/`base_picker` are passed in). Add a prop:

```ts
  export let pointPick = false; // true when tool === "point_picker"
```

In `onDown(e)` (the pointerdown handler), at the very top after the
`if (e.button !== 0) return;` guard, add:

```ts
    if (pointPick) {
      const rect = el.getBoundingClientRect();
      dispatch("pointpick", { x: e.clientX - rect.left, y: e.clientY - rect.top });
      return;
    }
```

(The canvas used for GL is the same element the renderer draws into; if the GL
canvas is a separate element from `el`, dispatch the raw client coords and let
Develop.svelte resolve the canvas. Confirm which element is the WebGL canvas —
grep `new FinishRenderer(` to see what element is passed, and read the pixel from
that same element in Step 7.)

- [ ] **Step 7: Arm the dropper + handle the pick in Develop.svelte**

In `app/src/lib/tabs/Develop.svelte`:

Import the helpers:

```ts
  import { rgbToHslSample, readCanvasPixel } from "../develop/colorPick";
```

Add state + handlers in the script:

```ts
  function startPointPick() {
    tool.set($tool === "point_picker" ? "edit" : "point_picker");
  }
  function onPointPick(e: CustomEvent<{ x: number; y: number }>) {
    // Resolve the WebGL canvas element (the one FinishRenderer draws into).
    const canvas = document.querySelector<HTMLCanvasElement>("canvas.gl") // adjust selector to the GL canvas
      ?? document.querySelector<HTMLCanvasElement>("canvas");
    if (!canvas) return;
    const rgb = readCanvasPixel(canvas, e.detail.x, e.detail.y);
    if (!rgb) return;
    params.update((p) => {
      const arr = (p.pc_samples ?? []).slice();
      if (arr.length >= 8) return p; // cap at 8
      arr.push(rgbToHslSample(rgb[0], rgb[1], rgb[2]));
      return { ...p, pc_samples: arr };
    });
    tool.set("edit");
  }
```

Pass the arm callback + picking flag into the panel, and the prop + listener into
the Viewport. Update the `<ColorMixer />` usage to:

```svelte
        <ColorMixer onPick={startPointPick} picking={$tool === "point_picker"} />
```

And the `<Viewport ... />` usage: add `pointPick={$tool === "point_picker"}` and
`on:pointpick={onPointPick}`. If the viewport is wrapped (e.g. only rendered in
the `{#if $tool === "edit"}` branch), make sure `point_picker` also keeps the
viewport mounted and interactive — i.e. treat `point_picker` like `edit` for the
purpose of showing the main image (it shows the same finished image, just with a
crosshair cursor). Add a crosshair cursor when `pointPick` via a class on the
viewport container.

- [ ] **Step 8: Typecheck**

Run: `cd app && npm run check 2>&1 | tail -12`
Expected: no errors. Resolve any prop-name mismatches the checker reports.

- [ ] **Step 9: Manual verification (the real proof)**

Run the app:

Run: `cd app && npm run tauri dev` (or the project's usual dev command — check `app/package.json` scripts; if `tauri` isn't present, `npm run dev` for the web UI is enough to see the panel, but the GL canvas readback needs the real viewport).

Verify:
1. Develop a film frame; the **Color Mixer** panel appears under Color Grading.
2. Mixer tab: switch Hue/Saturation/Luminance/All; drag the **Blue** Hue slider —
   blue regions of the image shift hue in real time; **All** shows 3 sliders/band.
3. Point Color tab: click **Pick color**, click a colored area in the image — a
   swatch appears; select it and drag **Hue Shift / Sat Shift** — only that color
   range changes. Add up to 8; the 9th is ignored. Delete via the × on the
   selected swatch.
4. Export the image (TIFF) and confirm the exported file matches the preview
   (CPU/GPU parity) — strong primaries shifted the same way.

- [ ] **Step 10: Commit**

```bash
git add app/src/lib/store.ts app/src/lib/develop/colorPick.ts app/src/lib/develop/colorPick.test.ts \
  app/src/lib/viewport/Viewport.svelte app/src/lib/tabs/Develop.svelte
git commit -m "feat(color-mixer): Point Color eyedropper (pick → sample)"
```

---

## Final verification

- [ ] **Rust tests:** `cargo test -p film-core 2>&1 | tail -20` — all pass.
- [ ] **TS tests:** `cd app && npx vitest run 2>&1 | tail -20` — all pass.
- [ ] **Typecheck:** `cd app && npm run check 2>&1 | tail -6` — clean.
- [ ] **Manual:** Task 13 Step 9 checklist all confirmed in the running app.
- [ ] **Parity:** a Mixer + Point Color edit looks identical in live preview and in an exported TIFF.

---

## Notes for the implementer

- **GPU/CPU parity is the #1 risk.** The HSL math, the constants block, and the
  weighting formulas are duplicated in `finish.rs` (Task 4/5), `shaders.ts`
  (Task 8), and partially `colorPick.ts` (Task 13). If you tune any constant,
  change it in **all** copies. The Rust version is authoritative.
- **Tuning is expected.** `CM_HUE_SHIFT_MAX`, `CM_LUM_GAIN`, `CM_FALLOFF_DEG`, the
  range/variance spans — adjust to taste during Task 13 manual review. Correctness
  tests assert direction and isolation, not magnitude, so they won't fight you.
- **The GL canvas selector** in Task 13 Step 7 (`querySelector`) is a placeholder —
  replace it with the actual element/ref the `FinishRenderer` draws into. Prefer a
  bound `this`/ref over a DOM query if the Viewport exposes one.
- **Index-typed bindings** in the Mixer sliders (Task 12) are the one Svelte
  gotcha; the fallback (explicit per-band literal fields) is guaranteed to work if
  the terse `{#each}` form fights the type checker.
