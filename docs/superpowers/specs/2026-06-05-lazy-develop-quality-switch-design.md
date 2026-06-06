# Lazy, cache-aware develop on quality switch

**Date:** 2026-06-05
**Status:** Approved (design)

## Problem

Switching the preview quality (`Performance` ⇄ `Quality`) re-decodes **every RAW
in the folder**. `QualityMenu.pick` calls `markAllUndeveloped()` then
`developAll()`, which loops `api.developImage(id)` over the whole folder — each a
full RAW decode. This is slow and almost entirely unnecessary.

## Key facts

The quality setting changes **only one thing**: the resolution of the in-memory
`working` buffer used to render the image currently on screen
(`Quality::cap()` — Performance = 4096px long edge, Quality = uncapped).
Everything else is quality-independent:

- Thumbnails / library grid — fixed small sizes (`THUMB_EDGE`, `AUTOWB_EDGE`).
- Base sample + auto-WB thumb — fixed small sizes.
- The `.oecache` sidecar — **always** stored at ≤4096px (`CACHE_WORKING_CAP`),
  regardless of quality. The cache *is* the Performance-mode buffer.

There is already a lazy rehydration path: `ensure_resident`
(`commands.rs:449`) reloads the working buffer from `.oecache` **without
re-decoding the RAW**.

Consequences:

- **Switching to Performance** never needs a decode — the cache already holds a
  ≤4096 buffer, exactly what Performance wants.
- **Switching to Quality** needs a full-res re-decode only for images the user
  actually views.

## Design

Replace the eager whole-folder re-develop with an idempotent, cache-aware
"ensure this image is developed at the current quality" that runs only for the
image on screen.

### 1. Backend — `ensure_developed(id)` (idempotent command)

Extract the heavy decode path out of `develop_image` into shared code, then add
a command that decides whether work is actually needed:

1. `ensure_resident(id)` — loads the ≤4096 buffer from `.oecache` if not already
   resident (no RAW decode).
2. **Adequacy check:** a resident `working` buffer satisfies quality `Q` when

   ```
   working.max_edge >= min(native.max_edge, Q.cap())
   ```

   where `native.max_edge` comes from `img.metadata.{width,height}` (set on first
   develop) and `working.max_edge = working.{width,height}.max()`.
   - Performance (cap 4096): the cache buffer always satisfies it → **no decode**.
   - Quality (uncapped): satisfied only if the resident buffer is already full-res
     (i.e. native ≤ 4096, or it was decoded under Quality already).
3. Adequate → return immediately. Not adequate (Quality wanted, only ≤4096
   resident) → run the heavy decode path and refresh the resident buffer.

`develop_image` keeps its current signature/behavior for the first-time
Library→Develop bulk pass; it shares the extracted heavy-path code.

The cache write stays capped at `CACHE_WORKING_CAP` (≤4096) — unchanged. A
Quality re-decode upgrades the *resident* buffer but the on-disk cache stays the
Performance-tier artifact, which is fine.

### 2. Frontend — cheap quality switch

`QualityMenu.pick` (`viewport/QualityMenu.svelte`):

- Keep `quality.set(q)` + `await api.setQuality(q)`.
- **Remove** `markAllUndeveloped()` and `developAll()`.
- `await api.ensureDeveloped(activeId)` for the **active image only**, then
  trigger a viewport re-render.

Images keep `developed: true`, so the Develop view never flashes the
"not developed yet" hint and the bulk progress overlay does not appear on the
quality-switch path.

Add `ensureDeveloped: (id) => invoke("ensure_developed", { id })` to `api.ts`.

### 3. Frontend — develop-on-navigation

When `activeId` changes within the Develop module, `await api.ensureDeveloped(id)`
before/with the render. Free in Performance mode (backend no-op); pays a decode
in Quality mode only for images actually visited.

**Stale guard:** capture the id, and after the await, only apply the result if
`activeId` still equals the captured id — prevents rapid arrow-key navigation
from piling up heavy decodes / applying out-of-order results.

## Edge cases

- **Never-developed images (no cache):** out of scope — still handled by the
  initial `developAll` on Library→Develop entry. This change only touches the
  quality-switch and navigation paths.
- **Memory in Quality mode:** strictly better than today — `developAll` made
  *all* buffers resident; lazy makes only visited ones resident.
- **In-flight decode while switching away:** discarded via the activeId guard.
- **Bulk progress overlay:** intentionally removed from the quality-switch path
  (switch is now near-instant). Retained only for the first Library→Develop
  bulk develop.

## Testing

- Unit: adequacy check `working_edge >= min(native, cap)` across
  Perf/Quality × small/large native resolution.
- Unit: existing pure helpers in `workflow.ts` stay pure and tested; the stale
  guard logic factored into a testable helper if practical.
- Manual: switch Perf↔Quality with the app running — confirm no full-folder
  re-develop, active image updates, and navigating to another image upgrades it
  on demand in Quality mode.
