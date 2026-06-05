# Per-Roll Base Calibration + Base-Picker Tool Design

**Date:** 2026-06-05
**Branch:** `main`
**Status:** Approved, ready for implementation planning

## Goal

Give the user control over the film-base (orange-mask) value used for inversion.
Today the base is auto-sampled per image as the whole-frame 95th percentile
(`sample_base(&working, None)`), which is content-dependent and produces a
drastic, image-varying tint. A roll generally shares one base (same film, same
scan settings), so the user should be able to **calibrate the base once on one
frame and apply it to the whole folder**, with a per-image override escape hatch
for oddball frames. Calibration is a manual picker tool in the Develop section:
drag a box over clear film, sample it, apply.

This is "fix #2" from the color-quality work that began with the magenta fix
(`ab272f6`) and removing the C develop mode (`6f4ca0c`).

## Resolved decisions

- **Share the base VALUE, not the rect.** Sampling the clear-film patch yields one
  RGB base value; that value is applied to every frame in the folder. The orange
  mask is constant across a roll, and the clear-film patch sits at different
  coordinates on each frame, so re-sampling a shared rect would be fragile.
- **Folder default + per-image override.** "Apply to roll" sets the folder default
  (all frames use it). "This image only" sets a per-image override that wins over
  the folder default for the active frame. The override target is the active
  Develop image (no grid multi-select for MVP).
- **Precedence:** per-image override → folder default → whole-frame auto (today's
  behaviour, the fallback when nothing is calibrated).
- **No DB migration.** Folder defaults reuse the existing `app_state` KV table.
- **Picker shows the uncropped raw scan**, so the user can reach the clear-film
  rebate/border that a crop would hide.

## Current state (relevant facts)

- Base is computed in `develop_image` (`app/src-tauri/src/commands.rs:272`) as
  `sample_base(&working, None)` and stored in the in-memory `Developed { working,
  thumb, base }` (`session.rs`) plus the `.oecache` sidecar (`cache.rs`). It is
  **not** in the catalog DB.
- `sample_base(img, rect: Option<Rect>)` (`crates/film-core/src/calibrate.rs:19`)
  already accepts a pixel `Rect` and samples the 95th percentile per channel.
- `base_rect: Option<[usize;4]>` exists in `InvertParams` (`session.rs`,
  `#[allow(dead_code)]`) and `base_rect: [..]|null` in `api.ts`, but is **not
  wired** to anything. This spec does not reuse it; it introduces `base_override`
  (a value, not a rect) instead. `base_rect` may be removed as dead code during
  implementation.
- Render/resolve paths take a `base: [f32;3]` (= `dev.base`):
  `build_params(p, base)` (`commands.rs:124`), `resolve_params` (CPU),
  `resolve_to_uniforms` (GPU, `gpu_upload.rs`). Both CPU and GPU build params via
  `build_params`, so injecting an effective base there covers both.
- Tools follow a pattern: `tool` store (`store.ts`, `"edit"|"crop"|"eraser"`),
  a `Toolbar.svelte` button, a panel under `app/src/lib/develop/`, an overlay
  mounted in `Viewport.svelte`, and a draft/commit lifecycle in `Develop.svelte`
  keyed on `$tool` transitions (see `CropOverlay.svelte` / `EraserPanel.svelte`).
- Folder scope: `imageDir(img)`, `scopeToFolder`, `selectedFolder`, `folderImages`
  (`library/folderScope.ts`, `store.ts`). `imageDir(img)` gives the folder path.
- Per-image edits live in the `edits` table (`params_json` etc.), written through
  debounced savers in `catalog.ts`. Global/per-folder-ish data lives in `prefs`
  and `app_state` KV tables (`catalog.rs` `save_app_state`/`load_app_state`).

## Architecture

### Base resolution

A single helper resolves the effective base for a frame:

```
effectiveBase(params, dir) =
    params.base_override            // per-image, persisted, normally null
    ?? folderBaseByPath[dir]        // folder default, persisted in app_state
    ?? null                         // -> backend falls back to dev.base (auto)
```

Backend stays simple: every resolve site computes
`let base = params.base_override.unwrap_or(dev.base)` (a small `effective_base`
helper). The frontend owns override-vs-folder precedence and injects the resolved
value into a throwaway params object per render call, so the persisted per-image
`base_override` is never clobbered by the folder default.

### Data & storage

- **`base_override: [number,number,number] | null`** added to `InvertParams`
  (TS `api.ts` + Rust `session.rs`). Per-image override. Persisted via the
  existing `save_params` write-through. Default `null`.
- **`folderBaseByPath: Record<string, [r,g,b]>`** store (`store.ts`). Persisted
  through `app_state` as key `folder_base:{dir}` → `[r,g,b]` JSON, loaded on
  catalog hydration like the other `app_state` values.
- **`withEffectiveBase(params, dir)`** helper (frontend): returns
  `{...params, base_override: params.base_override ?? folderBaseByPath[dir] ?? null}`.
  Applied at every render/export/thumbnail/`resolved_inversion` call.

### Sampling command

```
sample_base_at(id: String, rect: [f64;4]) -> [f32;3]
```

`rect` is normalized [x,y,w,h] over the working image. The command maps it to
pixel coords (reusing `crop_px`-style mapping), calls `sample_base(&working,
Some(rect))`, and returns the value. No re-decode, no re-develop — base only
feeds the inversion, so the preview updates live once the value is stored.

### Picker tool

- Add `"base_picker"` to the `Tool` union and a `Toolbar.svelte` button
  (eyedropper icon).
- `Develop.svelte` gains a draft/commit lifecycle on `$tool === "base_picker"`,
  mirroring crop: on enter, switch the viewport to the **full uncropped raw**
  scan and start with a default box; on a different tool, tear down.
- `BasePickerOverlay.svelte` — a stripped-down `CropOverlay`: one draggable/
  resizable box (no thirds grid). Maps pointer coords to normalized image coords
  via the existing `imgPoint`/`normPoint` pattern. As the box moves it calls
  `sample_base_at` (debounced) and shows a **live RGB swatch**.
- `BasePanel.svelte` — shows the sampled swatch and three actions:
  **[Apply to roll]** (set `folderBaseByPath[dir]`), **[This image only]** (set
  `params.base_override`), **[Reset]** (clear the per-image override; a second
  reset clears the folder default → auto). Buttons are disabled until a box has
  been sampled.

### Live update

`Viewport.svelte` computes its effective base reactively from BOTH
`params.base_override` and `folderBaseByPath[dir]`, so changing either re-fetches
`resolved_inversion` (GPU) / re-renders and the preview updates immediately.

## Components & boundaries

| Unit | Responsibility | Depends on |
|------|----------------|------------|
| `sample_base_at` (Tauri cmd) | Sample working image at a normalized rect → RGB | `sample_base`, resident working image |
| `effective_base` (Rust helper) | `base_override ?? dev.base` at resolve sites | `InvertParams`, `Developed` |
| `base_override` field | Persisted per-image override | `save_params` |
| `folderBaseByPath` store + `app_state` | Persisted folder default | `save_app_state`/`load_app_state` |
| `withEffectiveBase` (TS helper) | Inject resolved base per render call | `folderBaseByPath`, `imageDir` |
| `BasePickerOverlay.svelte` | Drag box, live swatch | `imgPoint`, `sample_base_at` |
| `BasePanel.svelte` | Apply-to-roll / this-image / reset | `folderBaseByPath`, `params` |

## Error handling & edge cases

- **No rebate in the scan.** If the scan is cropped tight with no clear film, the
  user samples a highlight instead — same quality as auto; the tool still works.
- **Empty/degenerate rect.** `sample_base_at` clamps to ≥1px (as `crop_px` does)
  and returns the existing whole-frame value if the rect is empty.
- **Image not resident.** `sample_base_at` runs `ensure_resident` first.
- **Folder default for an image with no folder / loose file.** `imageDir` returns
  the file's directory; a one-off file just gets its own single-image "folder".
- **Reset semantics.** Reset clears the per-image override first; if there is no
  override, Reset clears the folder default. Surface which scope is active so the
  user knows what Reset will do.

## Testing

- **Rust:** `sample_base_at` maps a normalized rect to the right pixel region and
  returns the rect's 95th-percentile value (vs whole-frame). `effective_base`
  precedence: override wins over `dev.base`; `None` falls back. Reuse the existing
  `sample_base` tests as the sampling-correctness baseline.
- **Frontend:** `withEffectiveBase` precedence (override → folder → null);
  `folderBaseByPath` hydrates from and persists to `app_state`. Catalog
  round-trip test for `folder_base:{dir}` keys.
- **Manual:** calibrate on one frame → roll updates; override one frame → only it
  changes; Reset reverts correctly; live preview updates without re-develop.

## Out of scope (YAGNI)

- Grid multi-select batch calibration (deferred; active-image override covers it).
- Auto-detecting the rebate/border (manual pick only).
- A `folder_bases` SQL table (the `app_state` KV is sufficient).
- Reworking `base_rect` into a live per-image rect (we share values, not rects).
