# Smooth 90° Rotation Animation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Animate 90° rotations as a smooth quarter-turn (snapshot spin overlay) in crop mode and the Edit-view hotkey.

**Architecture:** A pure `spinGeometry()` computes the spin's direction/scale/old-rect; a shared `SpinOverlay.svelte` spins a snapshot via CSS; CropView and Viewport trigger it on a single-quarter-turn `rot90` change.

**Tech Stack:** Svelte 5, TS, vitest. Frontend cwd: `/Users/mohaelder/Repos/filmrev/app`.
**Spec:** `docs/superpowers/specs/2026-06-04-rotate-spin-animation-design.md`. **Branch:** `feat/develop-redesign`.

---

## Task 1: `spinGeometry` (pure, tested) + `SpinOverlay.svelte`

**Files:** Create `app/src/lib/viewport/spin.ts` (+ `.test.ts`), `app/src/lib/viewport/SpinOverlay.svelte`

- [ ] **Step 1: Failing test** — `app/src/lib/viewport/spin.test.ts`
```ts
import { describe, it, expect } from "vitest";
import { spinGeometry } from "./spin";

describe("spinGeometry", () => {
  it("returns null for no change or a 180° jump", () => {
    expect(spinGeometry(0, 0, 200, 300, 1000, 800, 60)).toBeNull();
    expect(spinGeometry(0, 2, 200, 300, 1000, 800, 60)).toBeNull();
  });
  it("CW (+1) is dir 1, CCW (+3) is dir -1", () => {
    expect(spinGeometry(0, 1, 200, 300, 1000, 800, 60)!.dir).toBe(1);
    expect(spinGeometry(1, 0, 200, 300, 1000, 800, 60)!.dir).toBe(-1);
  });
  it("k = newFit/oldFit and rect is centered at the old fitted size", () => {
    // new oriented dims 200x300 (portrait). old (pre-turn) = 300x200 (landscape).
    const g = spinGeometry(0, 1, 200, 300, 1000, 800, 60)!;
    const avW = 1000 - 120, avH = 800 - 120;
    const oldFit = Math.min(avW / 300, avH / 200);
    const newFit = Math.min(avW / 200, avH / 300);
    expect(g.k).toBeCloseTo(newFit / oldFit, 5);
    expect(g.rect.width).toBeCloseTo(300 * oldFit, 3);
    expect(g.rect.left).toBeCloseTo((1000 - 300 * oldFit) / 2, 3);
    expect(g.rect.top).toBeCloseTo((800 - 200 * oldFit) / 2, 3);
  });
});
```

- [ ] **Step 2: Run → FAIL** `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/viewport/spin.test.ts`

- [ ] **Step 3: Implement `app/src/lib/viewport/spin.ts`**
```ts
export interface SpinRect { left: number; top: number; width: number; height: number }
export interface Spin { dir: number; k: number; rect: SpinRect }

/** Geometry for animating a single 90° turn. `imgW/imgH` are the NEW oriented
 *  dims (the props after the turn). Returns null unless it's a single quarter-turn. */
export function spinGeometry(
  prevRot90: number, rot90: number, imgW: number, imgH: number,
  vpW: number, vpH: number, pad: number,
): Spin | null {
  const d = (((rot90 - prevRot90) % 4) + 4) % 4;
  if (d !== 1 && d !== 3) return null;
  const dir = d === 1 ? 1 : -1;
  const oldImgW = imgH, oldImgH = imgW; // 90° swap
  const avW = Math.max(1, vpW - 2 * pad), avH = Math.max(1, vpH - 2 * pad);
  const oldFit = Math.min(avW / oldImgW, avH / oldImgH);
  const newFit = Math.min(avW / imgW, avH / imgH);
  if (!(oldFit > 0) || !(newFit > 0)) return null;
  const w = oldImgW * oldFit, h = oldImgH * oldFit;
  return { dir, k: newFit / oldFit, rect: { left: (vpW - w) / 2, top: (vpH - h) / 2, width: w, height: h } };
}
```

- [ ] **Step 4: Implement `app/src/lib/viewport/SpinOverlay.svelte`**
```svelte
<script lang="ts">
  import type { SpinRect } from "./spin";
  let active = false;
  let src = "";
  let style = "";
  let timer: ReturnType<typeof setTimeout> | null = null;

  /** Spin a snapshot from `rect` through `dir·90°` + `scale(k)`, then remove. */
  export function spin(snapshot: string, rect: SpinRect, dir: number, k: number) {
    if (timer) clearTimeout(timer);
    src = snapshot;
    active = true;
    const base = `left:${rect.left}px; top:${rect.top}px; width:${rect.width}px; height:${rect.height}px;`;
    style = `${base} transform:none; transition:none;`;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      style = `${base} transform:rotate(${dir * 90}deg) scale(${k}); transition:transform 260ms cubic-bezier(0.4,0,0.2,1);`;
    }));
    timer = setTimeout(() => { active = false; src = ""; }, 300);
  }
</script>

{#if active}<img class="spin" {src} alt="" style={style} />{/if}

<style>
  .spin { position: absolute; z-index: 5; pointer-events: none; transform-origin: center center; display: block; }
</style>
```

- [ ] **Step 5: Run tests + typecheck**
`cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/viewport/spin.test.ts && npm run check 2>&1 | tail -8`
Expected: pass; only the pre-existing `workflow.test.ts` error.

- [ ] **Step 6: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/viewport/spin.ts app/src/lib/viewport/spin.test.ts app/src/lib/viewport/SpinOverlay.svelte
git commit -m "feat(app): spinGeometry + SpinOverlay for 90° rotation animation"
```

---

## Task 2: Wire CropView

**Files:** Modify `app/src/lib/crop/CropView.svelte`

- [ ] **Step 1: Add the spin trigger**
In the `<script>`, add imports + state (near the other imports/lets):
```ts
  import SpinOverlay from "../viewport/SpinOverlay.svelte";
  import { spinGeometry } from "../viewport/spin";
  let spinOverlay: SpinOverlay;
  let prevRot90 = rot90;
```
Add a reactive AFTER the existing reactives (so `imgW/imgH/vpW/vpH/src` are current):
```ts
  // Animate a single 90° turn: snapshot the current (old) frame and spin it.
  $: if (rot90 !== prevRot90) {
    const g = src ? spinGeometry(prevRot90, rot90, imgW, imgH, vpW, vpH, PAD) : null;
    if (g && spinOverlay) spinOverlay.spin(src, g.rect, g.dir, g.k);
    prevRot90 = rot90;
  }
```
In the markup, add the overlay inside `.cropvp` (after the `<img>`/overlay, before closing `</div>`):
```svelte
  <SpinOverlay bind:this={spinOverlay} />
```

- [ ] **Step 2: Typecheck + commit**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -8` (only pre-existing error).
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/crop/CropView.svelte
git commit -m "feat(app): spin animation on 90° rotation in CropView"
```

---

## Task 3: Wire Viewport (Edit-view hotkey)

**Files:** Modify `app/src/lib/viewport/Viewport.svelte`

- [ ] **Step 1: Add the spin trigger (Fit-only)**
Add imports + state:
```ts
  import SpinOverlay from "./SpinOverlay.svelte";
  import { spinGeometry } from "./spin";
  let spinOverlay: SpinOverlay;
  let prevRot90 = rot90;
```
Add a reactive (after the existing reactives that compute `eff`, `fit`, etc.):
```ts
  // Animate a single 90° turn at Fit (skip while zoomed). Snapshot = GL canvas or img src.
  $: if (rot90 !== prevRot90) {
    const atFit = eff <= fit + 1e-6;
    const snap = useGL && canvas ? canvas.toDataURL("image/jpeg", 0.9) : src;
    const g = atFit && snap ? spinGeometry(prevRot90, rot90, imgW, imgH, vpW, vpH, PAD) : null;
    if (g && spinOverlay) spinOverlay.spin(snap, g.rect, g.dir, g.k);
    prevRot90 = rot90;
  }
```
In the markup, add the overlay inside the `.vp` container (after the `{#if src}`/canvas block, before the `{#if id && interactive}` zoom label):
```svelte
  <SpinOverlay bind:this={spinOverlay} />
```

- [ ] **Step 2: Typecheck + vitest + commit**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -8 && npx vitest run 2>&1 | tail -4` (only pre-existing error; all tests pass).
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(app): spin animation on 90° rotation in Viewport (Edit hotkey, Fit-only)"
```

---

## Task 4: Verify + manual smoke

- [ ] **Step 1:** `cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -1 && npx vitest run 2>&1 | tail -4` — only pre-existing error; all vitest pass.
- [ ] **Step 2: Manual smoke (user):** In crop mode, Rotate CW/CCW buttons and ⌘/Ctrl+]/[ produce a smooth ~260ms quarter-turn that lands cleanly on the re-oriented image (no flash, no mid-spin squash on non-square photos); the crop box appears correctly after. In the Edit view at Fit, ⌘/Ctrl+]/[ also spins smoothly. Zoomed-in Edit rotation just snaps (no spin) — acceptable.

---

## Self-Review notes
- **Spec coverage:** spinGeometry + SpinOverlay (T1), CropView trigger (T2), Viewport Fit-only trigger (T3), verify (T4).
- **Types:** `Spin`/`SpinRect` from `spin.ts` used by `SpinOverlay.spin()` and both triggers; `spinGeometry` signature identical across call sites.
- **No loop:** the trigger reactive guards on `rot90 !== prevRot90` and sets `prevRot90` immediately, firing once per change.
- **Known carry-over:** pre-existing `workflow.test.ts` error is unrelated.
