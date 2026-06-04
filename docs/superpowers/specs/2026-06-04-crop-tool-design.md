# Crop Tool Design

**Date:** 2026-06-04
**Branch:** `feat/develop-redesign`
**Status:** Approved, ready for implementation planning

## Goal

An interactive crop tool for the Develop tab: a draggable crop box with brackets
and rule-of-thirds guides, aspect-ratio presets, orientation toggle, and (later)
Lightroom-style straightening. Crop is per-image and applies to the develop
preview and the export.

## Decomposition — two plans

- **Plan A — Crop (no rotation).** The crop box, brackets, move/resize, Shift
  aspect-lock, aspect presets + orientation/`x` key, default 80%, commit-on-leave
  /Enter, Esc-discard, and backend crop applied to preview + export.
- **Plan B — Straighten.** Adds rotate-on-hover-outside-corner + a rotate slider,
  live CSS image rotation under a screen-axis-aligned box, constrain-to-rotated-
  image, and backend rotate-then-crop. Captured here at architecture level.

## Resolved decisions

- Rotation model is **Lightroom straighten**: the crop box stays axis-aligned to
  the screen; the image rotates beneath it; output is an upright axis-aligned crop.
- Built in **two plans** (crop first, straighten second).
- Crop is **per-image, session-scoped, in-memory** (file persistence deferred).
- In crop mode the user sees only the crop view (full image + box); switching to
  another tool commits and shows the cropped result.

## Current state (relevant facts)

- `Develop.svelte` switches the right panel on the `tool` store (`edit`/`crop`/…);
  `tool === "crop"` currently shows a "Crop — coming in Plan 2" placeholder, and
  the center always shows `Viewport`.
- `Viewport.svelte` shows the whole developed image (GPU canvas or `<img>`), with
  zoom/pan; it fetches via `api.renderView(id, params, view)` where `view`
  (`ViewSpec`) has `crop` (zoom region, full-res px), `out_w/out_h`, `raw`,
  `finish`.
- Backend `commands.rs::render_view` maps `view.crop` to working px via
  `s_scale = working.width / metadata.width`, crops, resizes, inverts, finishes.
  `export_image` re-decodes full-res, inverts, finishes. `convert::crop(img, x, y,
  w, h)` exists (pixel crop, clamped). No rotation exists.
- Per-image params already use a `cropById`-style pattern via `perImage.ts`
  (`createPerImageParams`); crop will reuse the same shape.

---

## Plan A — Crop (no rotation)

### Data model

```ts
// app/src/lib/crop/types.ts
export interface CropRect {
  rect: [number, number, number, number]; // x,y,w,h normalized 0..1 on the original image
  aspect: string;        // preset id: "original" | "1:1" | "4:5" | ... | "custom"
  orientation: "landscape" | "portrait";
}
```

- **Per-image committed crop:** `cropById: Writable<Record<string, CropRect | null>>`
  in `store.ts` (null = no crop). A helper resolves the active image's crop.
- **Draft** (live editing) is local to the crop tool component; committed only on
  commit.

### Backend — `ViewSpec.image_crop`

Add `image_crop: Option<[f64; 4]>` (normalized x,y,w,h on the original image) to
`ViewSpec` (`#[serde(default)]`, default `None`). In `render_view`, when present,
crop the working image to that normalized rect FIRST (px = normalized × working
dims), then proceed with the existing `view.crop`/resize/invert/finish on the
cropped image (the `s_scale` is unchanged, so `view.crop` stays in cropped-full-res
coords). `export_image` gains an `image_crop` parameter applied to the full image
before inversion. Reuses `convert::crop`; no new backend geometry fn.

- Tests: `render_view`/`crop` math is covered by a small pure helper
  `crop_px(norm, w, h) -> (x,y,w,h)` (rounding/clamping), unit-tested in Rust.

### Viewport modes (`Develop.svelte`)

- **Not cropping:** render `Viewport` with `imageCrop = committed.rect` (or null)
  and `imgW/imgH = round(rect.w·origW) × round(rect.h·origH)` (full dims if no
  crop). `Viewport` forwards `imageCrop` into the `ViewSpec`. The preview shows the
  cropped result; zoom/pan operate in the cropped frame.
- **Cropping** (`tool === "crop"`): render `CropView` instead of `Viewport`, and
  `CropPanel` in the right panel.

`Viewport.svelte` change: add an `imageCrop: [number,number,number,number] | null`
prop, include it in the `renderView` `ViewSpec`, and add it to `srcKey` so a
committed-crop change re-fetches.

### `CropView` (`crop/CropView.svelte`)

- Fetches the **finished full image** (`api.renderView(id, params, { crop:
  [0,0,W,H], out_w, out_h, raw:false, finish:true, image_crop:null })`) and shows
  it in an `<img>` at **Fit** (centered, 60px pad — same fit math as Viewport,
  but no zoom/pan). Measures its container with a `ResizeObserver`.
- Overlays `CropOverlay` positioned over the displayed image rect.
- No GPU needed here (finishing isn't being adjusted during crop).

### `CropOverlay` (`crop/CropOverlay.svelte`)

Given the displayed image rect (screen px) and the draft `CropRect`:
- Draws the crop box (rect → screen), **8 brackets** (4 corners + 4 edge
  midpoints) as grey/white L-shapes, **rule-of-thirds** guides (2 v + 2 h lines
  splitting the box into 9), and a dimmed scrim over the area outside the box.
- **Pointer interactions** (in normalized image coords):
  - Inside the box → **hand cursor**, drag **moves** the rect (clamped to [0,1]).
  - On a bracket → **resize cursor**, drag resizes from that corner/edge. Holding
    **Shift** locks the current aspect ratio; without Shift it is freeform
    (`aspect` becomes `"custom"`).
  - Minimum box size enforced (e.g. ≥ 5% of each dimension) and clamped to image.
- The mapping (normalized ↔ screen) and the hit-testing/resize math live in a pure,
  unit-tested helper `crop/cropMath.ts` (e.g. `handleAt(point, rect, imgScreenRect)`,
  `applyResize(handle, rect, dx, dy, aspect|null)`, `clampRect(rect)`).

### `CropPanel` (`crop/CropPanel.svelte`)

- **Aspect dropdown** with presets (id → ratio w:h):
  Original (native image ratio), 1:1, 4:5 (8:10), 8.5:11, 5:7, 2:3 (4:6), 4:4,
  16:9, 16:10. Shows the active ratio name; selecting a preset conforms the draft
  box to that ratio (centered on the current box center, clamped).
- **Orientation toggle** button **and the `x` key** swap the active ratio's w:h
  (landscape ↔ portrait) and re-conform the box.
- **Reset** button → default 80% centered, aspect Original.
- (Rotate slider is Plan B.)

### Commit / discard

- **Enter crop mode:** draft = committed crop for the active image, or a centered
  **80%** rect (aspect Original) if none.
- **Commit:** leaving crop mode (the `tool` store changes away from `"crop"`) OR
  pressing **Enter** → write the draft to `cropById[activeId]`. On a tool change,
  `Develop.svelte` commits the pending draft before switching the panel.
- **Discard (Esc):** revert the draft to the last committed crop (or the 80%
  default if none); stay in crop mode.
- Key handlers (`Enter`, `Esc`, `x`) are active only while `tool === "crop"`.

### Aspect semantics

- A preset id maps to a ratio `w/h`. **Original** resolves to the image's native
  `origW/origH`. Selecting a preset, or Shift-resize, conforms the box to that
  ratio. Freeform (no-Shift) resize sets `aspect = "custom"` and the label shows
  "Custom".
- `x`/orientation swaps the ratio to its reciprocal and re-conforms.

### Files (Plan A)

**Create:** `crop/types.ts`, `crop/cropMath.ts` (+ `.test.ts`), `crop/CropView.svelte`,
`crop/CropOverlay.svelte`, `crop/CropPanel.svelte`; aspect-preset table in
`crop/presets.ts` (+ `.test.ts`).
**Modify:** `store.ts` (`cropById` + active-crop helper), `api.ts`
(`ViewSpec.image_crop`, `exportImage` crop arg), `commands.rs` (`image_crop` in
`render_view`/`export_image` + `crop_px` helper + test), `Viewport.svelte`
(`imageCrop` prop → ViewSpec + srcKey), `Develop.svelte` (crop-mode switch, commit
-on-tool-change, effective dims).

### Tests (Plan A)

- Rust `crop_px(norm, w, h)` rounding/clamping.
- TS `cropMath` (handle hit-testing, resize with/without aspect lock, clamp) and
  `presets` (ratio lookup, orientation swap) via vitest.
- Manual smoke: draw a crop, commit (switch to Edit) → preview shows the crop;
  export reflects the crop; Esc discards; aspect presets + `x` work; per-image
  (crop on A doesn't affect B).

---

## Plan B — Straighten (architecture only)

- `CropRect` gains `angle: number` (deg, −45..45).
- `CropView` rotates the displayed image **live via CSS** (`transform: rotate`)
  under the screen-axis-aligned crop box; the box auto-constrains to the largest
  rect that stays inside the rotated image quad. Rotate-on-hover-outside-corner
  (cursor = `rotate-cw`) + a rotate slider in `CropPanel` both drive `angle`.
- Backend: a new `convert::rotate(img, deg)` (bilinear, expanded canvas) applied
  before the crop in `render_view`/`export_image`; the committed crop rect is
  expressed in the rotated frame; cropped output dims account for the angle.
- The normalized↔screen and constrain math extends `cropMath.ts`.

---

## Out of scope

- File/edit persistence (deferred project-wide).
- Cropping while zoomed (crop mode is Fit-only).
- Aspect "lens corrections" / perspective (not requested).
- 2B GPU exposure/WB (separate deferred work).
