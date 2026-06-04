# RedRoom — Library-First Develop Workflow + Preview Quality Design

**Date:** 2026-06-04
**Status:** Approved (design phase)
**Depends on:** RedRoom UI + zoom/perf specs (2026-06-03 / 2026-06-04)

## Problem

`import_image` currently decodes the full RAW and caches `full_res` for **every** image. For
35mm (~4 MP, ~48 MB) that's fine, but for medium-format scans (200–300 MB ≈ 80–100 MP,
~1.2 GB of f32 RGB each) it exhausts a 16 GB machine and stalls on a big batch. We also lack a
clean "develop a batch then edit" flow, and jumping straight to Develop can show an
unready/magnified frame.

## Goals

1. **Instant import** of any number/size of files (browse + cull in Library before committing
   to heavy compute) — the Library-first model.
2. **Memory-bounded by design**, with a user-chosen **Preview Quality** (Performance/Quality)
   that makes the speed↔resolution tradeoff explicit.
3. A clear **Develop-all** flow with progress, then auto-switch to Develop.
4. Develop tab **disabled until images exist**; an **early-jump popup** that triggers the same
   develop-all flow.

## Preview Quality (setting)

A session-global setting, default **Performance**, changed via a context menu (right-click in
the image area / a small menu):

- **Performance** — working image capped at **4096 px** long edge (~147 MB/image; sharp on 4K).
- **Quality** — **full original resolution** (true 1:1; MF ≈ 1.2 GB/image — user's hardware-gated choice).

Export is **always full-resolution** regardless of this setting (it re-decodes from the file).

## Lifecycle / states

Each imported image has a state: **imported** (thumbnail + metadata only) → **developed**
(working image + base cached, previewable in Develop). Develop-all moves all `imported` →
`developed`.

## Backend changes

```
session.rs   CachedImage split into always-present + lazily-filled:
             { path: String, file_name, metadata, thumbnail (b64),
               developed: Option<Developed> }
             Developed { working: Image, base: [f32;3] }   // working = decoded @ quality cap
             Session also holds: quality: Mutex<Quality {Performance|Quality}>
commands.rs  import_image(path)  -> ImageEntry   // LIGHT: thumbnail (embedded preview or a
                                                 // fast small decode) + metadata + path; NO full decode
             develop_image(id)   -> ()           // decode file -> working image at quality cap
                                                 // -> sample base -> store Developed; drop full_res
             set_quality(q)      -> ()           // "performance" | "quality"
             render_view(id, ...) -> String      // requires developed; errors if not yet developed
             export_image(id, ...) -> ()         // re-decode full-res from path, invert, write TIFF
```

- **Light import:** read the file's embedded preview for the thumbnail (DNG: the small preview
  IFD via `decode_tiff`; RAF: rawler's embedded preview/thumbnail) — fast, no full decode.
  Metadata via the existing `extract`. Store `path`. If no embedded preview is available, fall
  back to a fast downscaled decode for the thumbnail only.
- **`develop_image`:** `decode_any(path)` → `proxy(full, cap)` where `cap` = 4096 (Performance)
  or `u32::MAX` (Quality, i.e. no downscale) → `sample_base(&working)` → store
  `Developed { working, base }`. `full_res` is dropped (not cached).
- **`render_view`:** operates on `developed.working` (replaces today's `proxy`); returns
  `Err("not developed")` if the image isn't developed yet (frontend treats this as "needs
  develop").
- **Quality** is read at `develop_image` time. Changing the quality setting **marks all
  already-developed images stale** (frontend sets their `developed=false`); the frontend then
  re-runs the develop-all flow (progress overlay) so every image is rebuilt at the new cap. This
  keeps one consistent rule: the working image always reflects the current quality.
- **`ImageEntry`** gains `developed: bool` so the frontend knows each image's state.

## Frontend changes

```
store.ts          add: quality writable ("performance"|"quality"); developProgress writable
                  ({ active: boolean, done: number, total: number }); a derived `allDeveloped`.
api.ts            add developImage(id), setQuality(q); ImageEntry.developed: boolean.
panels/Source.svelte   add a "Develop all" button pinned at the bottom of the import list;
                       disabled when no images or all developed; triggers the develop-all flow.
develop/DevelopAll.ts  the flow: for each not-developed image, await api.developImage(id),
                       update developProgress; on finish, switch module to "develop".
overlay/ProgressOverlay.svelte  full-screen frosted overlay showing "Developing N of M…" with a
                       bar; shown while developProgress.active.
App.svelte        Develop tab button disabled when images is empty; clicking Develop when not
                  allDeveloped opens ConfirmDevelop popup instead of switching.
overlay/ConfirmDevelop.svelte   "Develop all N images?" modal → confirm runs the flow, cancel
                       stays in Library.
viewport/QualityMenu  right-click context menu in the Viewport area to pick Performance/Quality
                       (calls setQuality, marks developed images stale, re-runs develop-all).
```

### Develop-all flow (the one code path)
1. Set `developProgress = { active: true, done: 0, total: N_undeveloped }`.
2. For each undeveloped image (sequentially, to bound memory/CPU): `await developImage(id)`;
   `developProgress.done++`; mark entry `developed=true`.
3. On completion: `developProgress.active = false`; `module.set("develop")`; set `activeId` to
   the first image if unset.
4. Errors on a single image: record + skip (toast), continue the batch.

## Data flow

```
drop files ─► import_image (thumbnail+meta+path, instant) ─► Library grid
"Develop all" / Develop-tab popup ─► for each: develop_image (decode@cap+base, drop full_res)
                                      progress overlay ─► switch to Develop
Develop edits ─► render_view (on developed.working) ─► JPEG preview
Export ─► export_image (re-decode full-res from path) ─► 16-bit TIFF
```

## Error handling

- Light import on an unreadable/!supported file → `Err` → toast, skip; the rest still import.
- `develop_image` decode failure → flow records it, marks that entry failed, continues; a failed
  image stays non-developed and is skipped in Develop.
- `render_view` before develop → `Err("not developed")`; Viewport shows a neutral "Not developed
  yet" hint rather than erroring.
- Export re-decode failure → `Err` → toast.

## Testing

- **Backend (Rust unit):** `develop_image` produces a working image whose long edge ≤ cap in
  Performance and == full in Quality (use a synthetic in-memory path-less variant or a small
  fixture); `render_view` returns `Err` when not developed and `Ok` after; quality state set/get.
  (Decode-from-real-file paths validated manually — no RAW fixtures in unit tests.)
- **Frontend (vitest):** the develop-all reducer over a list of entries advances
  `done`/`total` correctly and flips `module` to "develop" at the end; `allDeveloped` derivation;
  tab-enabled logic (`images.length>0`).
- **Manual E2E:** drop a batch of 35mm DNGs → instant Library; Develop tab disabled when empty;
  "Develop all" shows progress then lands in Develop with a correct Fit view; jumping to Develop
  early shows the popup; Performance vs Quality changes preview sharpness/feel; export is
  full-res in both. Record in `poc-findings.md`.

## Scope

**In:** light import; `develop_image` + develop-all flow + progress overlay; Preview Quality
(Performance 4096 / Quality full) via context menu; Develop-tab disable-when-empty; early-jump
confirm popup; export always full-res; `render_view` on the developed working image.

**Out (later):** persisting the quality setting; per-image re-develop on quality change without a
full batch; LRU eviction of developed working images for very large batches; true tiled 1:1 for
100MP files beyond the Quality cap; background/parallel develop (v1 is sequential); catalog
persistence.

## Assumptions

1. Embedded previews exist in the user's DNG/RAF files for fast thumbnails; otherwise a fast
   small decode is the fallback (still cheaper than full decode + keeps import responsive).
2. Sequential develop is acceptable (bounded memory, predictable progress); parallelism is a
   later optimization.
3. Session-global quality (not per-image) is sufficient for v1.
4. Re-decoding full-res at export (seconds for MF) is acceptable for an export action.
