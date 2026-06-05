# Smooth 90° Rotation Animation Design

**Date:** 2026-06-04
**Branch:** `feat/develop-redesign`
**Status:** Approved, ready for implementation

## Goal

Make a 90° rotation (rotate buttons or ⌘/Ctrl+] / [) animate as a smooth quarter
turn instead of a hard cut, in both crop mode and the Edit-view hotkey. Purely
visual — the rotation result is unchanged.

## Problem

On a 90° rotation the view re-fetches/re-orients the image and the old frame is
replaced abruptly (a flash + the crop box jumps). The transition reads as broken.

## Approach — transient "spin snapshot" overlay

When a 90° rotation fires:
1. Snapshot what's currently displayed: CropView already has a data-URL `src`;
   Viewport's WebGL canvas → `canvas.toDataURL()`.
2. Drop the snapshot as an absolutely-positioned overlay (`z-index` above, pointer
   -events none) at the **old** image rect, then animate it
   `transform: rotate(±90deg) scale(k)` over **~260ms ease-in-out**, where
   **k = newFit / oldFit** so the spin lands exactly on the re-oriented image's
   fitted size (no mid-spin squash on non-square images).
3. The real view re-fetches/re-orients underneath, hidden by the snapshot. After
   ~300ms the overlay removes itself — the new oriented frame is already there.

A snapshot decouples the animation from the underlying re-render (the WebGL canvas
content changes when it re-renders, so animating the live element would fight the
swap). The same component handles both the `<img>` and `<canvas>` cases.

## Components

- **`viewport/SpinOverlay.svelte`** (new, shared): imperative
  `spin(src, rect, dir, k)` — shows the snapshot, animates `rotate(dir·90deg)
  scale(k)` via a CSS transition, self-removes after ~300ms. Rendered inside the
  positioned viewport container so its absolute coords match the image.
- **`crop/CropView.svelte`**: track `prevRot90`; when `rot90` changes by a single
  quarter-turn, compute `dir`, the **old** display rect (from the swapped dims,
  centered, since crop mode is always Fit), and `k`, then call `spinOverlay.spin`
  with the current (old) `src`.
- **`viewport/Viewport.svelte`**: same pattern; snapshot via `canvas.toDataURL`
  (GL path) or `src` (fallback). Only spin when **at Fit** (skip if zoomed, to
  avoid a mispositioned snapshot). Old rect computed from swapped dims, centered.

### Geometry at trigger time
At a rotation, the new (oriented) `imgW/imgH` props are already current and `src`
is still the old frame (the new fetch is async). Compute:
- `oldImgW = newImgH`, `oldImgH = newImgW` (90° swap).
- `avW = vpW − 2·PAD`, `avH = vpH − 2·PAD`.
- `oldFit = min(avW/oldImgW, avH/oldImgH)`, `newFit = min(avW/newImgW, avH/newImgH)`.
- old rect = centered `oldImgW·oldFit × oldImgH·oldFit`; `k = newFit/oldFit`;
  `dir = +1` (CW) if `rot90` advanced by 1, `−1` (CCW) if by 3. A 180° jump (rare
  batched double-press) is not animated.

## Out of scope

- Flip and straighten-slider animation (only rotation was flagged).
- Perfect spin centering while zoomed in the Edit view (skipped there).

## Tests

- The geometry helper `spinGeometry(prevRot90, rot90, imgW, imgH, vpW, vpH, pad)`
  → `{ dir, rect, k } | null` is pure and unit-tested (dir sign, k = newFit/oldFit,
  centered rect, null for 180°/no-change). The CSS animation itself is verified by
  manual smoke.
