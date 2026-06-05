# Eraser / Dust Removal — Develop tab

**Date:** 2026-06-04
**Branch:** `feat/develop-redesign`
**Status:** Design approved, ready for implementation plan

## Goal

Add dust/scratch removal to the Develop tab as a Lightroom-style **Eraser** tool, plus a
global one-pass **IR-based smart removal** for scans that carry an infrared channel. Both are
non-destructive and re-apply at full resolution on export, consistent with how Crop / WB / tone
already work.

The Eraser tool slot already exists (disabled) in `app/src/lib/develop/Toolbar.svelte`.

## Two features, one stage

1. **Global IR smart dust removal** — detects defects automatically from the preserved infrared
   plane and inpaints them in one pass. Only available when the image has an IR plane (V600 /
   SilverFast DNG). Most files (camera-scans, RAF) have no IR and rely on the manual brush.
2. **Manual eraser brush** — content-aware inpainting of whatever region the user brushes. Tap
   and drag are the same operation; only the brushed area (mask size) differs. The brush *is* the
   selection — there is no defect detector inside the brush.

### Background: how dust removal is done (research summary)

- **Infrared cleaning (Digital ICE / iSRD family):** dust, hair, and scratches are opaque to
  infrared while film dyes are transparent to it, so a separate IR scan yields a near-perfect
  defect mask "for free." Only works where an IR channel exists; inherently a global pass.
- **Inpainting (content-aware fill):** works on any image, no IR needed — the right fit for a
  brush. Classic algorithms: Telea / Fast Marching and Navier–Stokes (fast, small-defect
  friendly), Criminisi exemplar / PatchMatch (texture/grain preserving, heavier), deep models
  like LaMa (flawless but heavy). For small dust, Telea looks effectively flawless.

We use **inpainting (Telea)** as the engine for *both* features — the IR pass and the manual
brush both produce a binary defect mask that is fed to the same inpaint routine.

## Architecture — pipeline placement

Dust removal is a **new pipeline stage on the developed buffer**, after invert → finish →
WB/tone, before display/export:

```
decode → invert → finish → WB/tone → [DUST: IR pass + manual strokes] → display / export
```

- **Live commit is incremental:** committing one stroke inpaints only that stroke's bounding box
  on the existing `developed` buffer — no full re-develop.
- **Full re-develop / export re-applies all dust edits** from edit state (strokes in order, then
  the IR pass), at the working resolution (preview) or full resolution (export). This keeps the
  feature non-destructive and ensures full-res output gets full-res healing.

## Edit-state shape (per image)

Stored alongside the other per-image edits (see `app/src/lib/perImage.ts` / `store.ts`):

```
dust: {
  irRemoval: { enabled: bool, sensitivity: f32 },   // global IR pass
  strokes: [ { x, y, radius } ... ]                  // normalized image coords [0,1], image-space radius
}
```

- `strokes` is a **stack**: ⌘Z pops the last entry.
- **Reset** clears `strokes` and turns off `irRemoval`.
- Radius is stored in **image-space** (normalized), so a stroke covers the same real area of the
  photo regardless of zoom and survives re-develop / quality change / export.

## Inpainting engine (backend, Rust)

- Use the **`inpaint` crate** (native Rust Telea / Fast Marching port; works on `ndarray`, any
  channel count). License **EUPL-1.2** — weak/reciprocal copyleft, commercial-use friendly. Used
  unmodified through its public API, the only obligation is **attribution in a third-party
  licenses page** (which the app should ship anyway for `image`, `rawler`, etc.). No source
  disclosure of RedRoom; no copyleft on app code as long as the crate is not forked/modified.
  If we ever outgrow it we can vendor our own ~150-line Telea (MIT-clean) without touching the
  brush/stroke/UI plumbing.
- **Manual brush:** rasterize stroke stamps (filled circles) into a binary mask → small dilation
  → run `inpaint` over the mask's bounding box on the developed buffer.
- **IR pass:** threshold the preserved IR plane (`Image.ir` in `crates/film-core/src/image.rs`)
  by `sensitivity` → defect mask → dilate → `inpaint`. Guard: no-op when `ir: None`.
- **Performance:** always operate on the mask's bounding box, not the whole frame.

### Backend commands

- Incremental: apply a single committed stroke to the working developed buffer and return the
  patched region (for live feedback).
- Full pipeline: `render_view` / `export` re-apply all dust edits as part of the develop stage.

## UI — Eraser panel

Replaces the Basic panel when the Eraser tool is active (same pattern as Crop):

- **"Remove dust (IR)"** toggle + **Sensitivity** slider. When `ir: None`: toggle disabled,
  tooltip *"Requires an infrared scan channel."*
- **Brush size** slider — image-space; also driven by the scroll wheel. Range ~5–200 px
  image-space, default ~30.
- **Reset** button (clears strokes + IR toggle).

## Viewport interaction (Eraser mode)

- Cursor becomes a **circle outline** at the current brush radius, rendered in screen px
  (= image-space radius × current zoom), following the pointer.
- **Scroll wheel → brush size** (reassigned from zoom while in Eraser mode).
- **Pinch-zoom and pan-drag still work** — only the scroll wheel is reassigned.
- **Pointer drag** paints stamps along the path; **pointer-up commits** the stroke → inpaint →
  result appears. A **tap** is a single stamp (zero-length drag).
- While dragging, show a live highlight of the painted area before commit.

## Testing

**Rust (`film-core` / `src-tauri`):**
- Stamp → binary mask rasterization and dilation.
- IR threshold → defect mask; `ir: None` guard is a no-op.
- `inpaint` integration on a synthetic speck (defect removed, surroundings preserved).
- Stroke re-apply ordering on full re-develop.

**TS (`app`, vitest):**
- Edit-state reducers: add / undo (⌘Z) / reset strokes, toggle + sensitivity.
- Normalized image-space radius ↔ screen px mapping across zoom levels.
- Scroll → brush-size adjustment.
- Stroke stack semantics.

## Out of scope (v1)

- Individually selectable / re-editable spots (handles, hit-testing) — strokes are a stack only.
- Exemplar / PatchMatch / grain re-synthesis — Telea is sufficient for dust; the inpaint function
  stays swappable for a future upgrade.
- Live-during-drag inpaint — commit is on pointer-up.
