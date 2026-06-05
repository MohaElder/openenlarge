# Color Grading Reset + Collapsible Section Animation

## Goal

1. Add a "Reset" button to the **Color Grading** section header, mirroring the
   existing Basic-section reset.
2. Animate the collapse/expand of all three develop edit-panel sections (Basic,
   Tonal Curve, Color Grading).

## Scope

`app/src/lib/develop/Basic.svelte`, `ColorGrading.svelte`, `TonalCurve.svelte`.
No store, API, type, or backend changes.

These three sections render in the edit panel when `$tool === "edit"` (see
`app/src/lib/tabs/Develop.svelte`) and share an identical collapsible pattern:
`.section` → `.head` `<button>` → `{#if open}` → `.body`.

## 1. Color Grading reset

A "Reset" button at the right of the "Color Grading" header. The header becomes a
flex row: chevron+label toggle button on the left, Reset button on the right
(styled like Basic's reset). Clicking Reset must not toggle the section.

`resetColorGrading()` restores every Color Grading field from `defaultParams()`,
acting on the active image via `params.update(...)`:

- `cg_sh_hue/sat/lum`, `cg_mid_*`, `cg_hi_*`, `cg_glob_*` → `0`
- `cg_blending` → `50`
- `cg_balance` → `0`

All fields outside Color Grading are preserved (spread `...p`). The local
view-mode selector (3-way / Shadows / Midtones / Highlights / Global) is **not**
changed by reset — it only selects which wheel is visible, not an edit value.

## 2. Collapse/expand animation

Use Svelte's built-in `slide` transition on each section's `.body` block:

```svelte
import { slide } from "svelte/transition";
import { cubicInOut } from "svelte/easing";
...
{#if open}
  <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
```

Applied uniformly to Basic, Tonal Curve, and Color Grading. Duration 280ms with
`cubicInOut` easing — a smooth, deliberate feel on both open and close. The
chevron keeps its existing down/right icon swap.

## Testing

Manual verification in the running app:
1. Color Grading: adjust wheels + blending/balance, click Reset → all cg fields
   return to default; sliders/wheels outside Color Grading unchanged; view-mode
   stays put; clicking Reset does not collapse the section.
2. Each of the three sections animates smoothly when toggled open/closed.
3. `npm run check` reports no new errors.
