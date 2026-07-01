# Live HDR — Sub-project C: Chroma-preserving HDR finish (film-core, shared)

**Date:** 2026-07-01
**Status:** Design approved — ready for implementation plan
**Parent effort:** Live HDR preview. Sub-projects A (macOS EDR display surface) and B (per-frame live rendering, Metal/MSL) are COMPLETE and shipped on `main`.

---

## Background

HDR highlights currently **desaturate toward gray** as exposure rises. Root cause: the finish's creative color ops (saturation → LUT → grade → mixer → point) run on the **display-finalized** body — the Faithful shoulder (`display_finalize`) rolls super-white highlights toward **white per-channel**, stripping their chroma *before* any color op or highlight extension sees them. Both consumers inherit this:
- The **live MSL shader** (`msl.rs::finish_frag`) computes the finished SDR color `disp` (already desaturated), then applies a hue-preserving **gain** — scaling gray up gives *bright gray*.
- The **CPU gain-map export** (`commands.rs::render_hdr_image`) uses a **split-body+excess** workaround (finish the `[0,1]`-clamped body, add the raw `>1.0` excess back *unprocessed*) — highlights are equally desaturated, and the export doesn't benefit from B's work at all.

Sub-project C introduces one shared, chroma-preserving HDR finalize in `film-core`, used by **both** the live shader and the export, retiring the split-body+excess workaround.

**Decisions locked in brainstorming:**
- **SDR (HDR-off) stays byte-identical.** The current SDR pipeline (`finish_pixel` + `display_finalize`) is untouched. The HDR finish is a pure **extension** applied only to the HDR rendition/mode, affecting only highlights above the knee.
- **Scope: finish + export.** No headroom-tools UI (separate future work).
- **Approach A (luma-tone + preserve chromaticity):** tone the highlight *luminance* into headroom while reconstructing RGB from the pre-shoulder body's chromaticity — the standard HDR tone-mapping technique. (Approach B, a targeted saturation-restore, was the fallback; C, a full HDR-aware pipeline, was rejected as over-scoped.)

---

## Goal

HDR highlights keep their color (a blown warm highlight stays warm and *bright* into headroom, not gray), consistently in the **live surface AND the exported gain-map JPEG**, from one shared `film-core` reference. SDR and all below-knee output are unchanged.

**Non-goals:** SDR look changes; headroom-tools UI; new export formats (PQ/HLG AVIF/HEIC); touching the color ops' internal `[0,1]` clamps (Approach C).

---

## Architecture

### The shared finalize (`crates/film-core/src/finish.rs`)

A single function is the source of truth, mirrored in MSL and used by the CPU export:

```
pub(crate) fn hdr_finish(body: [f32;3], sdr: [f32;3]) -> [f32;3]
```
- `body` = the pre-shoulder tone body (per-channel, super-white; carries the real highlight chroma). This is the value AFTER per-zone WB + brightness + the tone-curve sliders but BEFORE `display_finalize` — i.e. `bodyU` in the MSL.
- `sdr` = the fully-finished SDR color in `[0,1]` (byte-identical to today's `finish_pixel` output).

Logic:
1. `mU = max(body[0], body[1], body[2])` — the highlight driver.
2. **`mU <= HDR_KNEE` → return `sdr`** (parity: below the knee the highlight extension is identity; the pixel is exactly the SDR output).
3. **`mU > HDR_KNEE`:**
   - `L_hdr = hdr_finalize(mU)` — the existing tanh shoulder mapping `mU` into `[HDR_KNEE, HDR_HEADROOM)` (monotonic, C1-continuous at the knee).
   - `chroma = body / mU` — the body's chromaticity (real hue + saturation), in `[0,1]`, luminance-normalized so the shoulder's per-channel compression is bypassed.
   - `highlight = chroma * L_hdr` — the body's color at the HDR-extended luminance.
   - `w = smoothstep(HDR_KNEE, w_hi, mU)` — blend weight, `0` at the knee (seamless join to `sdr`), ramping to `1` by `w_hi` (a tunable upper bound in the shoulder region).
   - return `mix(sdr, highlight, w)`.

Continuity: at `mU = HDR_KNEE`, `L_hdr = HDR_KNEE`, `chroma·L_hdr ≈ body` (which ≈ `sdr` near the knee since `display_finalize` is near-identity there), and `w = 0` → output `= sdr`. No ring/edge at the highlight boundary.

**Tunable parameters (dial on-device):** `w_hi` (how fast the blend reaches full chroma-preserved highlight), and — if the color-ops hue adjustment must be respected in highlights — an optional hue-reconcile that rotates `chroma` toward `sdr`'s hue. Default: use the body chromaticity directly (grade/mixer effect on near-white highlights is minimal). These are the parameters the on-device visual pass adjusts.

Reuses existing `film-core` helpers: `hdr_finalize` (scalar tanh, already present), `luma`, OKLab/HSL helpers if hue-reconcile is needed. `HDR_KNEE=0.8`, `HDR_HEADROOM=2.5` (`engine.rs`).

### The finish path change (SDR untouched)

`finish_pixel` currently discards the pre-shoulder body. Add an HDR-mode finish that keeps `body` and applies `hdr_finish(body, sdr)` at the end:
- Keep `finish_pixel` (SDR) exactly as-is.
- Add a sibling (e.g. `finish_pixel_hdr` or a flag) that computes both the SDR result and retains `body`, then returns `hdr_finish(body, sdr)`. Only the HDR rendition path uses it. This is a bounded addition in the finish, with SDR byte-identical (verified by the existing finish tests staying green).

---

## Consumer wiring

### CPU export (`app/src-tauri/src/commands.rs`)
- **Delete the split-body+excess** in `render_hdr_image` (`commands.rs:1305-1350`). Replace with: render the HDR rendition via the new HDR-mode finish (keeps `body`, applies `hdr_finish`), producing a display-referred image with color-managed super-white highlights.
- `render_and_encode_hdr` → `encode_gain_map_jpeg` are otherwise unchanged (same SDR + HDR renditions, BT.709, `srgb_to_linear_ext` on the HDR rendition). Result: the exported gain-map JPEG carries the proper HDR highlights, matching the live surface.

### Live MSL shader (`app/src-tauri/src/hdr_surface/msl.rs`)
- Replace the `finish_frag` HDR block (the gain-on-`disp`, `msl.rs:597-610`) with the MSL port of `hdr_finish`: it already has `bodyU` (the pre-shoulder body) and `disp` (the SDR color) in hand — compute `mU`, `L_hdr`, `chroma = bodyU/mU`, blended `mix(disp, chroma·L_hdr, w)`, then the existing `srgbToLinearExt3` output. Match the `film-core` reference exactly (parity discipline).

`film-core` is the single reference; MSL mirrors it; export calls it directly.

---

## Testing

- **`film-core` unit tests** (the reference / parity gate):
  - Below-knee parity: for `body` with `max ≤ HDR_KNEE`, `hdr_finish(body, sdr) == sdr` exactly.
  - Chroma preservation above-knee: a saturated warm highlight (`body` e.g. `[2.0, 1.4, 0.9]`) yields an output whose hue matches `body`'s (not gray/white), with luminance extended into `[1, HDR_HEADROOM]`, monotonic in `mU`, continuous at the knee.
  - The SDR path (`finish_pixel`, `display_finalize`) is byte-identical — existing finish/tone tests stay green.
- **On-device visual (the real gate, macOS HDR display):** SDR (HDR-off) unchanged; HDR highlights keep color as exposure rises (gray gone); the knee blend is seamless (no ring at the highlight edge); tune `w_hi`/hue-reconcile.
- **Export A/B:** export a gain-map JPEG; its HDR highlights match the live surface; view it in Apple Photos / Preview to confirm HDR + graceful SDR fallback.

---

## Decomposition (tasks for the implementation plan)

1. **`hdr_finish` in film-core** — the shared function + the below-knee-parity and chroma-preservation unit tests (TDD; the reference).
2. **HDR-mode finish** — the `finish_pixel` sibling that retains `body` and applies `hdr_finish`; SDR path untouched (regression test: SDR byte-identical).
3. **CPU export wiring** — retire split-body+excess in `render_hdr_image`; route the HDR rendition through the HDR-mode finish; verify `render_and_encode_hdr` gain-map export.
4. **MSL port** — mirror `hdr_finish` in `finish_frag` (replace the gain block); code-review-gated (compiles), visual at Task 5.
5. **On-device visual acceptance + tuning** (USER): SDR unchanged, HDR highlight color, knee blend, export A/B; dial `w_hi`/hue-reconcile.

---

## Constraints (Global)

- **SDR / HDR-off byte-identical** — `finish_pixel` + `display_finalize` untouched; existing finish/tone tests stay green. The HDR finish is invoked only for the HDR rendition/mode.
- **Single source of truth** — `film-core::hdr_finish` is the reference; MSL mirrors it exactly; the CPU export calls it. No third divergent copy of the logic.
- **Retire split-body+excess** — removed from `render_hdr_image`; the export uses the proper finish.
- **Match constants** — `HDR_KNEE=0.8`, `HDR_HEADROOM=2.5`, and reuse `hdr_finalize`/`luma`/OKLab helpers.
- **Below-knee = SDR exactly** — the extension is identity below the knee (parity is the correctness bar).
- Build/test: `cargo test -p film-core` (root); Tauri crate `cd app/src-tauri && cargo build`/`cargo test --lib`; MSL can't compile in-env (runtime/visual-verified). Commit discipline: exact-path `git add`, work on `main`.
- On-device visual tuning is expected (the math gets close; the eye finalizes) — the exact `w_hi`/hue-reconcile land during Task 5.
