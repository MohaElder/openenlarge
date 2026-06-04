# Develop Interactivity — Per-Image Edits + GPU Live Preview Design

**Date:** 2026-06-04
**Branch:** `feat/develop-redesign` (continues from the Plan-1 Develop redesign)
**Status:** Approved, ready for implementation planning

## Goal

Two improvements to the Develop editing experience:

1. **Per-image edits** — slider edits currently apply to *every* imported image
   because `params` is a single global store. Make edits per-image.
2. **GPU live preview** — slider drags round-trip to the Rust backend
   (re-invert + re-finish + JPEG + IPC, ~5–10fps with latency). Move all slider
   math to a WebGL2 fragment shader so every slider updates per-frame on the GPU.
   The authoritative full-res render (export) stays in Rust.

## Decomposition — three shippable, independently testable plans

- **Plan 1 — Per-image edits** (correctness fix; small; no backend change). Do first.
- **Plan 2A — Finishing on GPU** (8-bit; the 8 finishing sliders live).
- **Plan 2B — Exposure + WB on GPU** (16-bit neutral; every slider live).

Each plan gets its own implementation plan and is shippable on its own.

## Resolved decisions

- All sliders should ultimately be live on the GPU (not just the finishing 8).
- The GPU work is phased 2A → 2B for an earlier verifiable checkpoint.
- Plan 2B's neutral preview is **capped (~2048px long edge)**; at deeper zoom the
  preview is slightly soft. Export remains full-res exact (matches today's CAP
  philosophy). If image-switch transfer feels hitchy, the follow-up is binary IPC
  / Tauri asset protocol instead of base64 (not in scope here).
- Per-image edits are **session-scoped, in-memory** (file persistence remains
  deferred project-wide). New images start at `defaultParams()`; WB auto-seeds via
  the existing `Basic.svelte` seed.

## Current state (relevant facts)

- `app/src/lib/store.ts`: `export const params = writable<InvertParams>(defaultParams())`
  — a single global, shared by all images. Consumed via `$params` in
  `tabs/Develop.svelte` (refreshThumb, Viewport, export), `develop/Basic.svelte`
  (sliders + WB seed). `activeId` is a separate writable.
- Engine pipeline (`film-core/src/engine.rs`): `invert_b`/`invert_c` compute a
  per-channel density value, then `tone(v, gain, p) = pow((v*exposure*gain −
  black).max(0), gamma)`. So **exposure, WB gain, black, gamma are applied inside
  the inversion**. The **8 finishing controls** (contrast, highlights, shadows,
  whites, blacks, texture, vibrance, saturation) are applied afterward by
  `finish::finish_image`.
- `commands.rs::render_view` (non-raw): `invert_image` → `finish_image` →
  `to_jpeg_b64`. WB gains come from `wb_from_kelvin(temp, tint/150)`.
- Preview transport: a gamma-encoded JPEG data-URL over Tauri IPC; the develop
  `Viewport.svelte` shows it in an `<img>` positioned via CSS (whole image at zoom
  resolution; pan is pure CSS). `Histogram.svelte` reads the `previewSrc` data-URL.

---

## Plan 1 — Per-image edits

### Store (`app/src/lib/store.ts`)
Replace the global `params` writable with per-image storage plus a custom store
that proxies to the active image:

```ts
import { writable, derived } from "svelte/store";
import { defaultParams } from "./api";

const editsById = writable<Record<string, InvertParams>>({});
let activeIdVal: string | null = null;
activeId.subscribe((v) => (activeIdVal = v));

function entry(map: Record<string, InvertParams>, id: string | null): InvertParams {
  return (id && map[id]) || defaultParams();
}

// $params reads the ACTIVE image's params; set/update write to the active image.
export const params = {
  subscribe: derived([editsById, activeId], ([m, id]) => entry(m, id)).subscribe,
  set: (p: InvertParams) => {
    if (activeIdVal) editsById.update((m) => ({ ...m, [activeIdVal!]: p }));
  },
  update: (fn: (p: InvertParams) => InvertParams) => {
    if (activeIdVal) editsById.update((m) => ({ ...m, [activeIdVal!]: fn(entry(m, activeIdVal)) }));
  },
};
```

- `$params` is reactive to both edits and the active image; switching images shows
  that image's edits.
- `params.set`/`params.update` (used by sliders and the WB seed) write only to the
  active image.
- New images lazily resolve to `defaultParams()`. WB seeding (`Basic.svelte`)
  writes temp/tint via `params.update` → the active image only.
- All existing `$params` consumers keep working unchanged.

### Edge cases
- Editing with no active image is a no-op (guarded by `activeIdVal`).
- `refreshThumb` and `export` already use `$params` (active image) — correct.
- Removing/re-importing an image: stale `editsById` entries are harmless (keyed by
  id; a new import gets a new id).

### Tests
- A small pure helper `entry(map, id)` is unit-tested (vitest): returns defaults
  for unknown id, the stored entry otherwise.
- Behavioral: setting params for image A then switching to B yields B's defaults
  (not A's edits); switching back to A restores A's edits.

---

## Plan 2A — Finishing on GPU (8-bit)

### Backend (`commands.rs`)
Add a way to render the preview **without** the finishing layer (inverted +
exposure + WB + gamma baked, pre-`finish_image`). Implemented as a `ViewSpec` flag
(e.g. `finish: bool`, default true) or a sibling field; when false, `render_view`
returns the JPEG right after `invert_image`, skipping `finish_image`. No other
backend change. (Export and the legacy path keep finishing on.)

### Frontend — WebGL preview
- New `app/src/lib/viewport/gl/` module: a small WebGL2 renderer.
  - `app/src/lib/viewport/gl/finish.frag` (or an inline template string): a
    fragment shader porting `finish.rs` — `tone_curve` (whites/blacks → shadows/
    highlights → contrast) + vibrance/saturation. Texture (unsharp) is a **2-pass**
    render: pass A writes tone+sat to an offscreen framebuffer texture; pass B
    samples it for a 3-tap-separable blur and outputs `v + k·(v − blur)`.
  - `app/src/lib/viewport/gl/renderer.ts`: compiles the program, manages the source
    texture + FBO, exposes `setSource(imageBitmap)`, `setParams(finishUniforms)`,
    and `draw()` into a `<canvas>`. Uniforms: the 8 finishing values (−1..1).
- `Viewport.svelte` (develop, interactive) renders into a `<canvas>` instead of an
  `<img>`. The backend "pre-finish" preview is loaded as the source texture
  (`createImageBitmap` from the data-URL). On any finishing-slider change, update
  uniforms and `draw()` (rAF-coalesced) — no backend call. On image/params that
  affect inversion (mode, stock, exposure, temp, tint) or zoom/viewport change, the
  pre-finish preview is re-fetched (debounced, as today) and re-uploaded.
  Pan/zoom of the canvas stays CSS-positioned exactly as the `<img>` is today.
- **WebGL2 fallback:** if WebGL2 is unavailable, keep the current `<img>` +
  backend-finished preview path (no live finishing, but functional).

### Histogram
`Histogram.svelte` reads pixels from the WebGL canvas after each draw
(`gl.readPixels` into a small offscreen, or `canvas.toDataURL`) rather than the old
`previewSrc` data-URL. Update `Viewport`/store to publish the post-shader result
(e.g. a `previewSrc` set from `canvas.toDataURL()` after draw, debounced) so the
histogram stays accurate to what's on screen.

### Shader/engine parity
The GLSL `tone_curve`, saturation/vibrance, and texture must match `finish.rs`
numerically. Co-locate a parity note and keep export Rust-authoritative. Manual
parity spot-check at a few slider settings during verification.

---

## Plan 2B — Exposure + WB on GPU (16-bit neutral)

### Engine (`film-core`)
Add a function that returns the **pre-tone** value per pixel — the density-space
value passed into `tone()` (for mode B: `m_post · dens` = "unmixed"; for mode C:
`density`), i.e. inversion output before exposure/WB/black/gamma. Refactor
`invert_b`/`invert_c` to share that core, or add `invert_image_neutral`.

### Backend (`commands.rs`)
New command (or `ViewSpec` mode) emitting the neutral preview as a **16-bit RGBA
texture buffer** at a capped resolution (≤ ~2048px long edge): encode each density
`v` as `u16 = clamp(v / NEUTRAL_MAX, 0, 1) · 65535` with a fixed `NEUTRAL_MAX`
(headroom, e.g. 8.0). Returns base64 bytes + width/height. Depends only on
image/mode/stock/crop — NOT on exposure/WB/finishing.

### Frontend — extended shader
- Upload the 16-bit buffer to a WebGL2 normalized texture (`RGBA16` /
  `UNSIGNED_SHORT`), sampled as 0..1 → multiply by `NEUTRAL_MAX` to recover `v`.
- Shader pipeline (per pixel), mirroring the engine back half:
  `x = v · exp2(ev) · wbGain[c]; x = max(x, 0); x = pow(x, gamma=0.4545);` then the
  Plan-2A finishing (`tone_curve` → sat/vibrance → texture).
- **WB gains in TS:** port `wb_from_kelvin(temp, tint/150)` (the small Tanner-
  Helland math) to TypeScript so temp/tint update uniforms with no backend call.
  Unit-test the TS port against known points (neutral≈[1,1,1], warm cuts red, cool
  cuts blue) to keep it in parity with the Rust version.
- The neutral texture is re-fetched only on image/mode/stock/zoom change; every
  slider (exposure, temp, tint, and the finishing 8) is now pure-GPU per-frame.

### Precision note
The 16-bit density encoding preserves highlight headroom so exposure-down recovers
highlights in the live preview. At deep zoom the preview is capped/soft; export is
full-res exact via Rust.

### Tests
- TS `wbFromKelvin` parity unit tests (vitest).
- Manual parity check: a developed image at several (ev, temp, tint, finishing)
  settings should look the same in the GPU preview and in the exported TIFF.

---

## Out of scope

- File/edit persistence (deferred project-wide).
- Binary-IPC / Tauri asset-protocol transport for the neutral texture (a perf
  follow-up if base64 transfer on image-switch feels slow).
- Crop tool (still the separate Develop-redesign Plan 2).
- Moving the *inversion* (mode B/C matrices) to the GPU — it stays in Rust; only
  the post-inversion back half (exposure/WB/gamma/finishing) is shaded.
