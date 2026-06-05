# Color Mixer (HSL Mixer + Point Color) — Design

Date: 2026-06-05
Status: Approved (design), pending implementation plan

## Goal

Add a **Color Mixer** panel to the Develop section, replicating Lightroom's two
sibling tools that share one panel:

1. **Mixer (HSL)** — 8 fixed hue bands (Red, Orange, Yellow, Green, Aqua, Blue,
   Purple, Magenta), each with **Hue / Saturation / Luminance** sliders.
   Sub-tabs **Hue / Saturation / Luminance / All** switch which column is shown.
2. **Point Color** — an eyedropper that samples colors from the image into a
   list (up to 8 swatches); each sample is tuned with **Hue Shift, Sat Shift,
   Lum Shift, Variance, Range**.

Both tools are creative, per-image adjustments that live in the finishing layer,
participate in the existing per-image params store, persist through the catalog
JSON path, and render identically on the GPU (live preview) and CPU
(thumbnails/export).

Out of scope for this spec (explicitly dropped per brainstorming): the
"Adjust: HSL" dropdown (redundant with H/S/L/All tabs), the targeted-adjustment
tool (drag-on-image), the range "visualize" overlay, and before/after swatch
chips.

## Background / existing architecture

The app has a **dual rendering path** that must stay in lock-step:

- **GPU** (`app/src/lib/viewport/gl/shaders.ts` `FRAG`, driven by
  `app/src/lib/viewport/gl/renderer.ts`) — live develop preview.
- **CPU** (`crates/film-core/src/finish.rs` `finish_pixel`) — thumbnails + export.
- **TS uniform mirror** (`app/src/lib/develop/finish.ts`) precomputes per-pixel
  constants from `InvertParams` and hands them to the renderer.

The existing **Color Grading** feature is the closest precedent and the template
for this work. Its slices, for reference:

- Params: flat `cg_*` fields on `InvertParams`
  (`app/src/lib/api.ts:46-52`, Rust mirror `app/src-tauri/src/session.rs:75-89`).
- Defaults: `defaultParams()` (`api.ts:203-207`) +
  `default_invert_params()` (`commands.rs:92-96`).
- UI panel: `app/src/lib/develop/ColorGrading.svelte` (tabs, reset, sliders).
- CPU math: `ColorGrade` struct + `color_grade()` (`finish.rs:103-175`),
  composed in `finish_from()` (`commands.rs:208-219`), applied last in
  `finish_pixel()` (`finish.rs:236-245`).
- GPU math: `colorGrade()` in `shaders.ts:37-48`, uniforms declared at
  `shaders.ts:22-25`.
- TS mirror: `colorGrade()` + `ColorGradeUniforms` (`finish.ts:82-103`).
- Renderer plumbing: `CG_VEC3` / `CG_FLOAT` tables (`renderer.ts:62-70`),
  uniform-location lookup (`renderer.ts:128-129`), upload in `drawFinishPass`
  (`renderer.ts:120-124`), `setColorGrade` (`renderer.ts:178`).
- Viewport wiring: `Viewport.svelte:141-143`.
- Panel assembly: `app/src/lib/tabs/Develop.svelte:245-247`.
- i18n: `app/src/lib/i18n/dict.ts:160+`.

The eyedropper precedent is the film-base picker
(`app/src/lib/develop/BasePickerOverlay.svelte` + `api.sampleBaseAt`), and the
viewport already runs WebGL with `preserveDrawingBuffer: true`, so the rendered
canvas can be read back pixel-by-pixel.

## Data model

### Mixer (flat fields, mirroring `cg_*`)

8 bands × 3 controls = 24 new `InvertParams` fields, all `−100..100`, default 0:

```
cm_red_hue, cm_red_sat, cm_red_lum
cm_orange_hue, cm_orange_sat, cm_orange_lum
cm_yellow_hue, cm_yellow_sat, cm_yellow_lum
cm_green_hue, cm_green_sat, cm_green_lum
cm_aqua_hue, cm_aqua_sat, cm_aqua_lum
cm_blue_hue, cm_blue_sat, cm_blue_lum
cm_purple_hue, cm_purple_sat, cm_purple_lum
cm_magenta_hue, cm_magenta_sat, cm_magenta_lum
```

All three controls use the `−100..100` UI convention (matching the screenshots,
which show values like `−2`, `+3`). Internal scaling to physical units happens in
the TS/Rust mirrors, never in the stored params.

### Point Color (array of samples)

```ts
export interface PointColorSample {
  // Sampled target color (HSL of the picked pixel), fixed at pick time.
  hue: number;   // 0..360
  sat: number;   // 0..1
  lum: number;   // 0..1
  // User adjustments (all −100..100, default 0).
  hue_shift: number;
  sat_shift: number;
  lum_shift: number;
  variance: number;  // widens the sat/lum tolerance
  range: number;     // hue-window half-width; default 50
}
```

`InvertParams.pc_samples: PointColorSample[]` — default `[]`, max length 8.

Rust mirror: a serde struct `PointColorSample` and
`#[serde(default)] pub pc_samples: Vec<PointColorSample>` on the Rust
`InvertParams`. JSON array round-trips through the existing
`save_edits`/`load_catalog` path with no command-signature changes.

### Defaults / reset

- `defaultParams()` (`api.ts`) and `default_invert_params()` (`commands.rs`):
  all 24 `cm_*` = 0, `pc_samples` = `[]`.
- Panel reset (mirroring `resetColorGrading`): one `resetMixer()` that zeros the
  24 `cm_*` fields, and one `resetPointColor()` that clears `pc_samples`. The
  active sub-tab (local UI state) is left untouched, matching Color Grading.

## Rendering algorithm

The existing `wheel_offset` (zero-luma chroma push) cannot target *input* hue, so
both new tools need a real **RGB→HSL → adjust → HSL→RGB** round trip. This is the
one genuinely new primitive. It is implemented three times and MUST stay
bit-comparable across them: GLSL (`shaders.ts`), Rust (`finish.rs`), and — only
if needed by tests — TS (`finish.ts`). The Rust and GLSL are the authorities.

### Shared HSL primitive

`rgb2hsl(vec3) -> (h_deg, s, l)` and `hsl2rgb(h_deg, s, l) -> vec3`, standard
formulas, hue in degrees `[0,360)`, `s`/`l` in `[0,1]`. Defined once per language
with identical constants and branch order. Identity property: `hsl2rgb(rgb2hsl(c))
== c` within `1e-4`.

### Pipeline position

Both run in the finishing stage, **after** `color_grade`, operating on the
developed positive RGB. Order within the new stage: **Mixer first, then Point
Color** (Point Color is the finer, user-targeted correction and should see the
band-mixed result).

- CPU: extend `finish_pixel` (`finish.rs:236-245`) →
  `color_mix(point_color(...))` chained after `color_grade`.
- GPU: extend `finishAt` (`shaders.ts:50-65`) → call `colorMixer()` then
  `pointColor()` after `colorGrade(cu)`.

### Mixer math

Band centers (degrees), tunable constants shared by both languages:

```
Red 0, Orange 30, Yellow 60, Green 120, Aqua 180, Blue 240, Purple 280, Magenta 320
```

Per pixel:

1. `(h, s, l) = rgb2hsl(rgb)`.
2. For each band `i`, compute a weight `w_i` from circular hue distance
   `d = wrap180(h − center_i)` via a smooth falloff that overlaps neighbors so
   weights blend continuously across the wheel (no banding). Falloff half-width
   is a shared constant (~the spacing to the nearest neighbor). Weights are not
   forced to sum to 1; overlap is intentional and bounded.
3. Accumulate, scaled by the band's three sliders (each `−100..100` → unit):
   - **Hue:** `h += w_i * (cm_*_hue/100) * HUE_SHIFT_MAX_DEG`
   - **Saturation:** `s *= (1 + w_i * (cm_*_sat/100))` (clamped ≥ 0)
   - **Luminance:** `l += w_i * (cm_*_lum/100) * LUM_GAIN` (toward 0/1)
4. Saturation weighting also scales by `s` itself lightly so fully-desaturated
   pixels are not hue-shifted arbitrarily (a near-gray pixel has no meaningful
   band); i.e. effective weight `w_i * smoothstep(low, high, s)`.
5. `rgb = hsl2rgb(h, clamp(s), clamp(l))`.

Constants (`HUE_SHIFT_MAX_DEG`, `LUM_GAIN`, falloff width, sat gate thresholds)
are defined once and shared; exact values tuned during implementation to feel
like Lightroom but are not load-bearing for correctness tests (which assert
*direction* and *isolation*, not absolute magnitude).

### Point Color math

Masks are computed from the **stage-input** HSL (the value entering the Point
Color stage, i.e. post-Mixer), so samples are order-independent and stable: all
sample weights are computed first, then all shifts accumulate, then applied once.

Per pixel, for each active sample `k`:

1. `dh = wrap180(h − sample.hue)`; hue weight `wh = smoothstep` window whose
   half-width grows with `range` (`0..100`) — small range = tight hue selection.
2. `ds = |s − sample.sat|`, `dl = |l − sample.lum|`; sat/lum tolerance widens
   with `variance` (`−100..100`; higher = looser). Combined
   `w_k = wh * sat_lum_tolerance(ds, dl, variance)`.
3. Accumulate (same unit scaling as Mixer):
   `h += w_k * (hue_shift/100) * HUE_SHIFT_MAX_DEG`,
   `s *= (1 + w_k * (sat_shift/100))`,
   `l += w_k * (lum_shift/100) * LUM_GAIN`.
4. After all samples: `rgb = hsl2rgb(h, clamp(s), clamp(l))`.

### GPU uniform layout

- Mixer: three `float[8]` arrays `u_cm_hue[8]`, `u_cm_sat[8]`, `u_cm_lum[8]`
  (pre-divided by 100 in the TS mirror). Band centers are GLSL `const`.
- Point Color: `u_pc_count` (int) + fixed-size arrays of length 8 for the per-
  sample fields (`u_pc_hue[8]`, `u_pc_sat[8]`, `u_pc_lum[8]`, `u_pc_hue_shift[8]`,
  `u_pc_sat_shift[8]`, `u_pc_lum_shift[8]`, `u_pc_variance[8]`, `u_pc_range[8]`),
  iterated `for (int k=0;k<u_pc_count;k++)`. The TS mirror packs the
  `pc_samples` array into these `Float32Array`s, zero-filling unused slots.

### Renderer / TS mirror plumbing

Following the `ColorGrade` pattern in `finish.ts` + `renderer.ts`:

- `finish.ts`: add `ColorMixUniforms` (the packed Mixer + Point Color arrays) and
  a `colorMix(p: InvertParams): ColorMixUniforms` builder.
- `renderer.ts`: add a `setColorMix(...)` setter, location lookups for the new
  uniforms, and an upload block in `drawFinishPass` (array uniforms via
  `gl.uniform1fv` / `gl.uniform1iv`). Extend `renderExport(...)` to thread the
  same uniforms (so GPU export matches preview).
- `Viewport.svelte`: `renderer.setColorMix(colorMix(params))` alongside the
  existing `setColorGrade`.

### CPU plumbing

- `finish.rs`: `ColorMix` struct (the 24 band values pre-scaled + the parsed
  sample list) with a `new(...)` builder and a `color_mix()` / `point_color()`
  applied in `finish_pixel`.
- `commands.rs` `finish_from()`: build `ColorMix` from `InvertParams` and store
  it on `FinishParams` (new field), mirroring how `cg` is built and attached.

## Eyedropper interaction

A new viewport tool mode "point-color pick" (armed by the eyedropper button in
the Point Color tab):

1. Armed state shows a crosshair cursor over the viewport (reuse the overlay
   approach from `BasePickerOverlay`).
2. On click, map the click to a normalized image coordinate, read the displayed
   pixel color from the rendered canvas (canvas is `preserveDrawingBuffer:true`;
   read via a 1×1 `readPixels` / 2D-canvas draw at that point).
3. Convert the sampled sRGB pixel to HSL, push a new `PointColorSample`
   `{ hue, sat, lum, hue_shift:0, sat_shift:0, lum_shift:0, variance:0,
   range:50 }`, select it, and disarm.
4. Enforce the 8-sample cap (ignore/notify on the 9th).

Sampling the *displayed* (fully-finished) pixel means a freshly-added sample
starts with zeroed shifts and therefore no immediate change — no feedback loop.
The stored target is only ever used for masking.

## UI / panel structure

New `app/src/lib/develop/ColorMixer.svelte`, inserted in
`Develop.svelte` after `<ColorGrading />`. Structure mirrors `ColorGrading.svelte`
(section header with collapse + reset, `slide` transition, local tab state).

Top-level tabs: **Mixer** | **Point Color**.

### Mixer tab

- Sub-tabs: **Hue / Saturation / Luminance / All** (same `.modes` button-row
  styling as Color Grading).
- Hue / Saturation / Luminance views: one column of 8 `Slider`s (one per band,
  `−100..100`, def 0, `signed` format), each labeled by band name and with a
  band-appropriate colored gradient track (extend `gradients.ts`).
- All view: 8 rows, each row showing the band's three sliders.
- Reset button → `resetMixer()`.

### Point Color tab

- An eyedropper toggle button + helper text ("Use the Point Color Dropper to add
  samples.") shown when `pc_samples` is empty.
- A swatch strip of added samples (color chip per sample); clicking selects,
  with a delete affordance per sample.
- For the selected sample: **Hue Shift, Sat Shift, Lum Shift, Variance** sliders
  (`−100..100`, def 0, `signed`) + **Range** slider (`0..100`, def 50).
- Reset button → `resetPointColor()` (clears all samples).

i18n: add a `colorMixer.*` block to `dict.ts` for all labels (tabs, band names,
slider labels, reset, helper text).

## Testing

### Rust (`finish.rs` unit tests, mirroring the existing `color_grade` tests)

- `color_mix_default_is_identity`: all `cm_*` = 0 and no samples → pixel unchanged
  (within `1e-4`) across a value/hue sweep.
- `rgb_hsl_round_trip`: `hsl2rgb(rgb2hsl(c)) ≈ c` for a sweep of colors.
- `mixer_band_isolation`: pushing the Blue band's hue/sat leaves a pure-red pixel
  unchanged and visibly shifts a pure-blue pixel.
- `mixer_saturation_direction`: positive band saturation increases chroma of an
  in-band pixel; negative decreases it.
- `mixer_gray_pixel_unaffected_by_hue`: a near-gray pixel is not hue-shifted
  (sat gate).
- `point_color_sample_isolation`: a sample targeting red shifts a red pixel and
  leaves a far-hue (green) pixel unchanged; widening `range` extends the affected
  window.
- `point_color_order_independent`: two overlapping samples produce the same
  result regardless of array order (masks from stage input).
- `finish_pixel_default_unchanged`: extend the existing `default_is_identity`
  coverage so the new stage is a no-op at defaults.

### TS / component (if the project's vitest setup supports component mounting;
otherwise unit-test the mirror)

- `finish.test.ts`: `colorMix(defaultParams())` produces identity uniforms
  (zeroed arrays, `pc_count = 0`).
- Mirror-vs-Rust spot check: a handful of sample pixels through the TS HSL
  primitive match the Rust output within tolerance (guards GPU/CPU parity at the
  math level; the shader itself is validated by eye + the identity uniform case).
- Component (if available): Mixer sub-tab switching renders the right slider
  column; eyedropper add appends a sample and respects the 8-cap; delete removes
  the selected sample.

### Manual / parity

- Visual check that live GPU preview matches the exported (CPU or GPU-export)
  image for a chart with strong primaries.

## Files touched (summary)

| Area | File | Change |
|---|---|---|
| Params (TS) | `app/src/lib/api.ts` | `cm_*` fields, `PointColorSample`, `pc_samples`, defaults |
| Params (Rust) | `app/src-tauri/src/session.rs` | mirror fields + `PointColorSample` struct |
| Defaults (Rust) | `app/src-tauri/src/commands.rs` | `default_invert_params`, `finish_from` builds `ColorMix` |
| CPU math | `crates/film-core/src/finish.rs` | HSL primitive, `ColorMix`, `color_mix`/`point_color`, tests |
| GPU shader | `app/src/lib/viewport/gl/shaders.ts` | HSL primitive, `colorMixer`/`pointColor`, uniforms |
| TS mirror | `app/src/lib/develop/finish.ts` | `ColorMixUniforms`, `colorMix()` |
| Renderer | `app/src/lib/viewport/gl/renderer.ts` | `setColorMix`, locations, upload, export thread |
| Viewport | `app/src/lib/viewport/Viewport.svelte` | `setColorMix(colorMix(params))` + eyedropper mode |
| Panel | `app/src/lib/develop/ColorMixer.svelte` | new panel (Mixer + Point Color tabs) |
| Assembly | `app/src/lib/tabs/Develop.svelte` | insert `<ColorMixer />` |
| Gradients | `app/src/lib/develop/gradients.ts` | per-band slider gradients |
| i18n | `app/src/lib/i18n/dict.ts` | `colorMixer.*` strings |

## Open questions / tuning (resolved during implementation, not blocking)

- Exact band centers and falloff width (start from the values above).
- `HUE_SHIFT_MAX_DEG`, `LUM_GAIN`, sat gate thresholds — tuned to taste.
- Range/Variance display formatting (the LR screenshot shows `5.0` for Range —
  cosmetic; internal scale is `0..100`).
