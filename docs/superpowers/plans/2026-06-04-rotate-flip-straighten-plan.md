# Rotate / Flip / Straighten Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 90° rotation (CW/CCW buttons + ⌘/Ctrl+] / [), horizontal/vertical flip (buttons), and Lightroom straighten (slider + rotate-on-hover-outside-corner), all per-image, applied to the develop preview and export, with the crop box following each transform.

**Architecture:** Backend pipeline becomes orient → straighten → crop → view-crop → invert → finish. `convert::orient` is an exact lossless remap; `convert::rotate` is a bilinear straighten. Frontend stores `rot90/flipH/flipV/angle` on the per-image `CropRect`; 90°/flips re-fetch the oriented image and transform the rect; the straighten angle is a **live CSS rotation** with the crop box constrained to the rotated image (`constrainToRotated`). Export is Rust-authoritative.

**Tech Stack:** Rust (Tauri), TypeScript, Svelte 5, WebGL-unrelated. vitest, cargo test. cargo NOT on PATH — prefix `source "$HOME/.cargo/env" && `. Frontend cwd: `/Users/mohaelder/Repos/filmrev/app`.

**Spec:** `docs/superpowers/specs/2026-06-04-rotate-flip-straighten-design.md`.
**Branch:** `feat/develop-redesign`.

**Convention (must be consistent across backend + constrain):** positive `angle` rotates the image **clockwise** (matches CSS `transform: rotate(+deg)`). For an output pixel offset `(dx,dy)` from center, the source sample is `( cos·dx + sin·dy , −sin·dx + cos·dy )`.

---

## File Structure

**Create:** `app/src/lib/crop/transforms.ts` (+ `.test.ts`) — rect rotate/flip helpers.
**Modify:**
- `crates`/… none. Backend lives in the app crate.
- `app/src-tauri/src/convert.rs` — `orient`, `orient_dims`, `rotate` (+ tests).
- `app/src-tauri/src/commands.rs` — ViewSpec fields, pipeline, export params.
- `app/src/lib/api.ts` — ViewSpec fields + `exportImage` args.
- `app/src/lib/crop/types.ts` — extend `CropRect`.
- `app/src/lib/crop/cropMath.ts` (+ `.test.ts`) — `constrainToRotated`.
- `app/src/lib/icons/Icon.svelte` — rotate/flip glyphs.
- `app/src/lib/crop/CropPanel.svelte` — straighten slider + rotate/flip buttons.
- `app/src/lib/crop/CropOverlay.svelte` — rotate-on-hover-outside-corner.
- `app/src/lib/crop/CropView.svelte` — oriented fetch + CSS rotate + constrain.
- `app/src/lib/tabs/Develop.svelte` — draft fields, box-follows, shortcuts, dims.
- `app/src/lib/viewport/Viewport.svelte` — rot90/flip/angle props → ViewSpec/srcKey.

---

## Task 1: Backend `orient` + `orient_dims`

**Files:** Modify `app/src-tauri/src/convert.rs`

- [ ] **Step 1: Add failing tests** (append to the `tests` module)
```rust
    fn pattern() -> Image {
        // 2 wide × 3 tall, each pixel = [x/10, y/10, 0]
        let mut img = Image { width: 2, height: 3, pixels: vec![[0.0; 3]; 6], ir: None };
        for y in 0..3 { for x in 0..2 { img.pixels[y * 2 + x] = [x as f32 / 10.0, y as f32 / 10.0, 0.0]; } }
        img
    }
    #[test]
    fn orient_identity() {
        let p = pattern();
        assert_eq!(orient(&p, 0, false, false).pixels, p.pixels);
    }
    #[test]
    fn orient_dims_swaps_on_quarter_turns() {
        assert_eq!(orient_dims(2, 3, 0), (2, 3));
        assert_eq!(orient_dims(2, 3, 1), (3, 2));
        assert_eq!(orient_dims(2, 3, 2), (2, 3));
        assert_eq!(orient_dims(2, 3, 3), (3, 2));
    }
    #[test]
    fn orient_flip_h_mirrors_x() {
        let p = pattern();
        let f = orient(&p, 0, true, false);
        assert_eq!(f.pixels[0], p.pixels[1]); // row 0 swapped
        assert_eq!(f.pixels[1], p.pixels[0]);
    }
    #[test]
    fn orient_rot90_cw_maps_topleft_to_topright() {
        // CW: old top-left (0,0) goes to new top-right.
        let p = pattern(); // 2x3
        let r = orient(&p, 1, false, false); // 3x2
        assert_eq!((r.width, r.height), (3, 2));
        // new top-right corner == old top-left
        assert_eq!(r.pixels[0 * 3 + 2], p.pixels[0]);
    }
```

- [ ] **Step 2: Run to confirm FAIL**
`source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml orient`
Expected: compile error (functions undefined).

- [ ] **Step 3: Implement**
```rust
/// Oriented dimensions after `rot90` clockwise quarter-turns.
pub fn orient_dims(w: usize, h: usize, rot90: u8) -> (usize, usize) {
    if rot90 % 2 == 1 { (h, w) } else { (w, h) }
}

fn flip_h(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut px = vec![[0.0_f32; 3]; w * h];
    for y in 0..h { for x in 0..w { px[y * w + x] = img.pixels[y * w + (w - 1 - x)]; } }
    Image { width: w, height: h, pixels: px, ir: None }
}
fn flip_v(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut px = vec![[0.0_f32; 3]; w * h];
    for y in 0..h { for x in 0..w { px[y * w + x] = img.pixels[(h - 1 - y) * w + x]; } }
    Image { width: w, height: h, pixels: px, ir: None }
}
fn rotate_cw(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let (nw, nh) = (h, w);
    let mut px = vec![[0.0_f32; 3]; nw * nh];
    for ny in 0..nh { for nx in 0..nw {
        let ox = ny; let oy = h - 1 - nx;
        px[ny * nw + nx] = img.pixels[oy * w + ox];
    } }
    Image { width: nw, height: nh, pixels: px, ir: None }
}

/// Lossless orientation: flip-H, flip-V, then `rot90` clockwise quarter-turns.
pub fn orient(img: &Image, rot90: u8, flip_horizontal: bool, flip_vertical: bool) -> Image {
    let mut o = img.clone();
    if flip_horizontal { o = flip_h(&o); }
    if flip_vertical { o = flip_v(&o); }
    for _ in 0..(rot90 % 4) { o = rotate_cw(&o); }
    o
}
```

- [ ] **Step 4: Run tests + clippy**
`source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml orient` → pass.
`source "$HOME/.cargo/env" && cargo clippy --manifest-path app/src-tauri/Cargo.toml 2>&1 | tail -6` → no new warnings.

- [ ] **Step 5: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src-tauri/src/convert.rs
git commit -m "feat(backend): orient (lossless 90°/flip) + orient_dims"
```

---

## Task 2: Backend `rotate` (bilinear straighten)

**Files:** Modify `app/src-tauri/src/convert.rs`

- [ ] **Step 1: Add failing tests**
```rust
    #[test]
    fn rotate_zero_is_identity() {
        let p = pattern();
        assert_eq!(rotate(&p, 0.0).pixels, p.pixels);
    }
    #[test]
    fn rotate_90_on_square_matches_orient_interior() {
        // 3x3 asymmetric; rotate(+90) ≈ orient CW on the center pixel and neighbours.
        let mut s = Image { width: 3, height: 3, pixels: vec![[0.0; 3]; 9], ir: None };
        for y in 0..3 { for x in 0..3 { s.pixels[y * 3 + x] = [x as f32 / 10.0, y as f32 / 10.0, 0.0]; } }
        let a = rotate(&s, 90.0);
        let b = orient(&s, 1, false, false); // 3x3 stays 3x3
        // center pixel identical
        assert!((a.pixels[1 * 3 + 1][0] - b.pixels[1 * 3 + 1][0]).abs() < 1e-3);
        assert!((a.pixels[1 * 3 + 1][1] - b.pixels[1 * 3 + 1][1]).abs() < 1e-3);
    }
    #[test]
    fn rotate_blacks_out_of_bounds_corners() {
        let p = pattern();
        let r = rotate(&p, 30.0);
        // a corner now samples outside → black
        assert_eq!(r.pixels[0], [0.0, 0.0, 0.0]);
    }
```

- [ ] **Step 2: Run to confirm FAIL**
`source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml rotate_`
Expected: compile error.

- [ ] **Step 3: Implement**
```rust
fn sample_bilinear(img: &Image, sx: f32, sy: f32) -> [f32; 3] {
    let (w, h) = (img.width as i32, img.height as i32);
    if sx < -0.5 || sy < -0.5 || sx > w as f32 - 0.5 || sy > h as f32 - 0.5 {
        return [0.0, 0.0, 0.0];
    }
    let x0 = sx.floor() as i32; let y0 = sy.floor() as i32;
    let fx = sx - x0 as f32; let fy = sy - y0 as f32;
    let get = |x: i32, y: i32| -> [f32; 3] {
        let xc = x.clamp(0, w - 1) as usize; let yc = y.clamp(0, h - 1) as usize;
        img.pixels[yc * img.width + xc]
    };
    let p00 = get(x0, y0); let p10 = get(x0 + 1, y0);
    let p01 = get(x0, y0 + 1); let p11 = get(x0 + 1, y0 + 1);
    std::array::from_fn(|c| {
        let a = p00[c] * (1.0 - fx) + p10[c] * fx;
        let b = p01[c] * (1.0 - fx) + p11[c] * fx;
        a * (1.0 - fy) + b * fy
    })
}

/// Straighten: rotate clockwise by `deg` about the centre into a same-size canvas.
/// Out-of-bounds samples are black. No-op below 1e-4 deg.
pub fn rotate(img: &Image, deg: f32) -> Image {
    if deg.abs() < 1e-4 { return img.clone(); }
    let (w, h) = (img.width, img.height);
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let cx = w as f32 / 2.0; let cy = h as f32 / 2.0;
    let mut px = vec![[0.0_f32; 3]; w * h];
    for oy in 0..h { for ox in 0..w {
        let dx = ox as f32 + 0.5 - cx;
        let dy = oy as f32 + 0.5 - cy;
        let sx = cos * dx + sin * dy + cx - 0.5;
        let sy = -sin * dx + cos * dy + cy - 0.5;
        px[oy * w + ox] = sample_bilinear(img, sx, sy);
    } }
    Image { width: w, height: h, pixels: px, ir: None }
}
```

- [ ] **Step 4: Run tests + clippy** (as Task 1 Step 4, for `rotate`). All pass.

- [ ] **Step 5: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src-tauri/src/convert.rs
git commit -m "feat(backend): bilinear straighten rotate()"
```

---

## Task 3: Backend pipeline — ViewSpec fields + render/export

**Files:** Modify `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Add ViewSpec fields**
After `image_crop` in `ViewSpec`:
```rust
    #[serde(default)] pub rot90: u8,
    #[serde(default)] pub flip_h: bool,
    #[serde(default)] pub flip_v: bool,
    #[serde(default)] pub angle: f32,
```
Add to the `convert` import: `use crate::convert::{crop, orient, proxy, resize_to, rotate};` (keep existing names).

- [ ] **Step 2: Apply orient+rotate in `render_view`**
Currently the non-raw path computes `s_scale` from `dev.working` and `img.metadata.width`, then builds `base_img` from `image_crop`. Replace the geometry prelude (from `let s_scale = …;` through `let cropped = crop(&base_img, …);`) with:
```rust
    // Geometry: orient (lossless) → straighten → persistent crop. The view crop is
    // applied within the resulting image. s_scale maps full-res view coords to the
    // working image; orientation is lossless so it is preserved by the same ratio.
    let oriented = orient(&dev.working, view.rot90, view.flip_h, view.flip_v);
    let straightened = rotate(&oriented, view.angle);
    let base_img = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, ix, iy, iw, ih)
        }
        None => straightened,
    };
    // Map the view crop (full-res, in the ORIENTED frame) to working px.
    let (ometa_w, ometa_h) = crate::convert::orient_dims(
        img.metadata.width as usize, img.metadata.height as usize, view.rot90);
    let _ = ometa_h;
    let s_scale = base_img.width as f64
        / (img.metadata.width.max(1) as f64) // fallback; corrected below
        ;
    let _ = s_scale;
    // Recompute s_scale against the oriented metadata width so view.crop lines up.
    let s_scale = oriented.width as f64 / (ometa_w.max(1) as f64);
    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;
    let cropped = crop(&base_img, cx, cy, cw, ch);
```
(Clean equivalent — remove the throwaway `let _ =` lines if you prefer; they are only to make the intent explicit. The essential change: orient+rotate before the persistent crop, and `s_scale = oriented.width / orient_dims(metadata).0`.)

Simplify to this final form (use THIS, not the annotated version above):
```rust
    let oriented = orient(&dev.working, view.rot90, view.flip_h, view.flip_v);
    let straightened = rotate(&oriented, view.angle);
    let base_img = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, ix, iy, iw, ih)
        }
        None => straightened,
    };
    let (ometa_w, _) = crate::convert::orient_dims(
        img.metadata.width as usize, img.metadata.height as usize, view.rot90);
    let s_scale = oriented.width as f64 / ometa_w.max(1) as f64;
    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;
    let cropped = crop(&base_img, cx, cy, cw, ch);
```

- [ ] **Step 3: Apply to `export_image`**
Add params `rot90: u8, flip_h: bool, flip_v: bool, angle: f32` (before `session`). After `let full = decode_any(...)?;` apply geometry before the existing `image_crop` match:
```rust
    let full = orient(&full, rot90, flip_h, flip_v);
    let full = rotate(&full, angle);
    let full = match image_crop {
        Some(nc) => { let (x, y, w, h) = crop_px(nc, full.width, full.height); crop(&full, x, y, w, h) }
        None => full,
    };
```

- [ ] **Step 4: Build + tests + clippy**
`source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml && cargo clippy --manifest-path app/src-tauri/Cargo.toml 2>&1 | tail -8`
Expected: all pass; no new warnings. (ViewSpec defaults are exercised by the existing `viewspec_finish_*` test path; serde `#[serde(default)]` makes the new fields optional.)

- [ ] **Step 5: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src-tauri/src/commands.rs
git commit -m "feat(backend): orient+straighten in render_view/export pipeline"
```

---

## Task 4: TS contract + CropRect extension

**Files:** Modify `app/src/lib/api.ts`, `app/src/lib/crop/types.ts`

- [ ] **Step 1: `types.ts` — extend CropRect**
```ts
export interface CropRect {
  rect: Rect;
  aspect: string;
  orientation: "landscape" | "portrait";
  rot90: 0 | 1 | 2 | 3;
  flipH: boolean;
  flipV: boolean;
  angle: number;
}
```

- [ ] **Step 2: `api.ts` — ViewSpec fields + exportImage args**
In `ViewSpec` add:
```ts
  rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number;
```
Replace `exportImage`:
```ts
  exportImage: (
    id: string, params: InvertParams, outPath: string,
    imageCrop: [number, number, number, number] | null = null,
    geom: { rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number } = {},
  ) =>
    invoke<void>("export_image", {
      id, params, outPath, imageCrop,
      rot90: geom.rot90 ?? 0, flipH: geom.flip_h ?? false,
      flipV: geom.flip_v ?? false, angle: geom.angle ?? 0,
    }),
```
(Tauri maps `flipH`→`flip_h`, `flipV`→`flip_v`.)

- [ ] **Step 3: Typecheck**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -15`
Expected: errors will appear in files that build `CropRect` literals (Develop.svelte) because the new required fields are missing — those are fixed in Task 9. Confirm the ONLY errors are: the pre-existing `workflow.test.ts`, and missing-`CropRect`-field errors in `Develop.svelte` (expected, fixed later). No errors in `api.ts`/`types.ts` themselves.

- [ ] **Step 4: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/api.ts app/src/lib/crop/types.ts
git commit -m "feat(app): CropRect rot90/flip/angle + ViewSpec/export geom args"
```

---

## Task 5: Pure rect transforms + constrainToRotated (tested)

**Files:** Create `app/src/lib/crop/transforms.ts` (+ `.test.ts`); Modify `app/src/lib/crop/cropMath.ts` (+ `.test.ts`)

- [ ] **Step 1: Failing tests**
Create `app/src/lib/crop/transforms.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { rotateRectCW, rotateRectCCW, flipRectH, flipRectV } from "./transforms";
import type { Rect } from "./types";
const r = (x: number, y: number, w: number, h: number): Rect => ({ x, y, w, h });
const close = (a: Rect, b: Rect) => {
  for (const k of ["x", "y", "w", "h"] as const) expect(a[k]).toBeCloseTo(b[k], 6);
};

describe("rect transforms", () => {
  it("rotateRectCW four times is identity", () => {
    let c = r(0.1, 0.2, 0.3, 0.4);
    const start = { ...c };
    for (let i = 0; i < 4; i++) c = rotateRectCW(c);
    close(c, start);
  });
  it("CW then CCW is identity", () => {
    const c = r(0.1, 0.2, 0.3, 0.4);
    close(rotateRectCCW(rotateRectCW(c)), c);
  });
  it("flipRectH twice is identity; mirrors x once", () => {
    const c = r(0.1, 0.2, 0.3, 0.4);
    close(flipRectH(flipRectH(c)), c);
    expect(flipRectH(c).x).toBeCloseTo(1 - 0.1 - 0.3, 6);
  });
  it("flipRectV mirrors y", () => {
    expect(flipRectV(r(0.1, 0.2, 0.3, 0.4)).y).toBeCloseTo(1 - 0.2 - 0.4, 6);
  });
  it("rotateRectCW swaps w/h", () => {
    const c = rotateRectCW(r(0.1, 0.2, 0.3, 0.4));
    expect(c.w).toBeCloseTo(0.4, 6);
    expect(c.h).toBeCloseTo(0.3, 6);
  });
});
```
Append to `app/src/lib/crop/cropMath.test.ts`:
```ts
import { constrainToRotated } from "./cropMath";
describe("constrainToRotated", () => {
  it("is identity at 0 deg", () => {
    const c = constrainToRotated({ x: 0.1, y: 0.1, w: 0.8, h: 0.8 }, 0, 100, 80);
    expect(c).toEqual({ x: 0.1, y: 0.1, w: 0.8, h: 0.8 });
  });
  it("shrinks (centered) so all corners stay in the rotated image", () => {
    const full = { x: 0.05, y: 0.05, w: 0.9, h: 0.9 };
    const c = constrainToRotated(full, 12, 100, 80);
    expect(c.w).toBeLessThan(full.w);
    // centered shrink: same centre
    expect(c.x + c.w / 2).toBeCloseTo(0.5, 3);
    expect(c.y + c.h / 2).toBeCloseTo(0.5, 3);
  });
});
```
(`constrainToRotated` must be importable; add the `import` line at the top of the test file if not already importing from `./cropMath`.)

- [ ] **Step 2: Run to confirm FAIL**
`cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/crop/`
Expected: FAIL (modules/exports missing).

- [ ] **Step 3: Implement `transforms.ts`**
```ts
import type { Rect } from "./types";

/** Transform a normalized rect when the IMAGE is rotated 90° clockwise. */
export function rotateRectCW(r: Rect): Rect {
  return { x: 1 - r.y - r.h, y: r.x, w: r.h, h: r.w };
}
export function rotateRectCCW(r: Rect): Rect {
  return { x: r.y, y: 1 - r.x - r.w, w: r.h, h: r.w };
}
export function flipRectH(r: Rect): Rect { return { ...r, x: 1 - r.x - r.w }; }
export function flipRectV(r: Rect): Rect { return { ...r, y: 1 - r.y - r.h }; }
```

- [ ] **Step 4: Implement `constrainToRotated` in `cropMath.ts`**
Append:
```ts
/** Shrink `rect` about its centre to the largest factor where all four corners,
 *  inverse-rotated by `deg` about the oriented-image centre, stay inside the
 *  image — so a straightened crop never includes the blank wedges. ow/oh are the
 *  oriented pixel dims (rotation must be computed in pixel space). */
export function constrainToRotated(rect: Rect, deg: number, ow: number, oh: number): Rect {
  if (Math.abs(deg) < 1e-4) return rect;
  const rad = (deg * Math.PI) / 180;
  const cos = Math.cos(rad), sin = Math.sin(rad);
  const cx = ow / 2, cy = oh / 2;
  const inside = (s: number): boolean => {
    const rw = rect.w * s, rh = rect.h * s;
    const rx = rect.x + (rect.w - rw) / 2, ry = rect.y + (rect.h - rh) / 2;
    const corners: Array<[number, number]> = [
      [rx, ry], [rx + rw, ry], [rx, ry + rh], [rx + rw, ry + rh],
    ];
    for (const [nx, ny] of corners) {
      const dx = nx * ow - cx, dy = ny * oh - cy;
      const sx = cos * dx + sin * dy + cx;
      const sy = -sin * dx + cos * dy + cy;
      if (sx < 0 || sx > ow || sy < 0 || sy > oh) return false;
    }
    return true;
  };
  if (inside(1)) return rect;
  let lo = 0, hi = 1;
  for (let i = 0; i < 24; i++) { const mid = (lo + hi) / 2; if (inside(mid)) lo = mid; else hi = mid; }
  const s = lo, rw = rect.w * s, rh = rect.h * s;
  return { x: rect.x + (rect.w - rw) / 2, y: rect.y + (rect.h - rh) / 2, w: rw, h: rh };
}
```

- [ ] **Step 5: Run tests + typecheck**
`cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/crop/ && npm run check 2>&1 | tail -10`
Expected: crop tests pass; only the known pre-existing + Task-9-pending Develop errors remain.

- [ ] **Step 6: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/crop/transforms.ts app/src/lib/crop/transforms.test.ts app/src/lib/crop/cropMath.ts app/src/lib/crop/cropMath.test.ts
git commit -m "feat(app): rect rotate/flip transforms + constrainToRotated (tested)"
```

---

## Task 6: Icons + CropPanel (straighten slider + rotate/flip buttons)

**Files:** Modify `app/src/lib/icons/Icon.svelte`, `app/src/lib/crop/CropPanel.svelte`

- [ ] **Step 1: Add glyphs to `Icon.svelte`** (`paths` record)
```ts
    "rotate-ccw": '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>',
    "flip-h": '<path d="M12 3v18"/><path d="M16 7l4 5-4 5"/><path d="M8 7l-4 5 4 5"/>',
    "flip-v": '<path d="M3 12h18"/><path d="M7 8l5-4 5 4"/><path d="M7 16l5 4 5-4"/>',
```
(`rotate-cw` already exists from the toolbar task.)

- [ ] **Step 2: Rewrite `CropPanel.svelte`**
Add a straighten slider and a button row. New props/events: bind `aspect`, `orientation`, `angle`; dispatch `preset`, `swap`, `reset`, `rotate` (detail `-1`|`1`), `flip` (detail `"h"`|`"v"`).
```svelte
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { PRESETS, labelFor } from "./presets";
  import Icon from "../icons/Icon.svelte";

  export let aspect: string;
  export let orientation: "landscape" | "portrait";
  export let angle: number;
  const dispatch = createEventDispatcher<{ preset: string; swap: void; reset: void; rotate: number; flip: "h" | "v" }>();
</script>

<div class="section">
  <div class="head"><span>Crop</span></div>

  <div class="sub">Aspect ratio</div>
  <select value={aspect} on:change={(e) => dispatch("preset", (e.target as HTMLSelectElement).value)}>
    {#if aspect === "custom"}<option value="custom">Custom</option>{/if}
    {#each PRESETS as p}<option value={p.id}>{p.label}</option>{/each}
  </select>
  <div class="current">{labelFor(aspect)}</div>

  <button class="row" on:click={() => dispatch("swap")}>
    Orientation: {orientation === "landscape" ? "Landscape" : "Portrait"} <span class="key">X</span>
  </button>

  <div class="sub">Transform</div>
  <div class="btns">
    <button title="Rotate left (⌘/Ctrl + [)" on:click={() => dispatch("rotate", -1)}><Icon name="rotate-ccw" size={16} /></button>
    <button title="Rotate right (⌘/Ctrl + ])" on:click={() => dispatch("rotate", 1)}><Icon name="rotate-cw" size={16} /></button>
    <button title="Flip horizontal" on:click={() => dispatch("flip", "h")}><Icon name="flip-h" size={16} /></button>
    <button title="Flip vertical" on:click={() => dispatch("flip", "v")}><Icon name="flip-v" size={16} /></button>
  </div>

  <div class="sub">Straighten</div>
  <div class="slrow">
    <input type="range" min="-45" max="45" step="0.1" bind:value={angle} on:dblclick={() => (angle = 0)} />
    <span class="val">{angle.toFixed(1)}°</span>
  </div>

  <button class="row" on:click={() => dispatch("reset")}>Reset</button>
  <div class="hint">Enter to apply · Esc to discard · Shift locks the ratio</div>
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { color: var(--text); font-weight: 600; padding: 4px 0; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  select { width: 100%; padding: 6px; border-radius: 8px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); }
  .current { font-size: 12px; color: var(--text-dim); margin: 4px 0 8px; }
  .row { width: 100%; display: flex; justify-content: space-between; align-items: center;
    padding: 7px 10px; margin: 6px 0; border-radius: 8px; border: 1px solid var(--glass-brd);
    background: transparent; color: var(--text); cursor: pointer; }
  .key { font-size: 10px; border: 1px solid var(--glass-brd); border-radius: 4px; padding: 0 5px; color: var(--text-dim); }
  .btns { display: flex; gap: 6px; }
  .btns button { flex: 1; display: grid; place-items: center; padding: 8px 0; border-radius: 8px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text); cursor: pointer; }
  .slrow { display: flex; align-items: center; gap: 8px; }
  .slrow input[type="range"] { flex: 1; accent-color: var(--accent); }
  .val { font-size: 12px; color: var(--text); width: 44px; text-align: right; font-variant-numeric: tabular-nums; }
  .hint { font-size: 11px; color: var(--text-dim); margin-top: 8px; line-height: 1.5; }
</style>
```

- [ ] **Step 3: Typecheck + commit**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -10` (only pre-existing + Task-9 Develop errors).
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/icons/Icon.svelte app/src/lib/crop/CropPanel.svelte
git commit -m "feat(app): CropPanel straighten slider + rotate/flip buttons + glyphs"
```

---

## Task 7: CropOverlay — rotate-on-hover-outside-corner

**Files:** Modify `app/src/lib/crop/CropOverlay.svelte`

- [ ] **Step 1: Add rotate handling**
Add two props and a dispatch event, and a rotate zone just outside corners. Update the script and the down/move handlers. Add near the existing props:
```ts
  export let angle = 0;            // current straighten angle
```
Add to the dispatcher type: `straighten: number`. Add rotate state and helpers:
```ts
  let rotating = false;
  let rotStartAngle = 0, rotStartPointer = 0;
  const center = () => ({ cx: img.left + img.width / 2, cy: img.top + img.height / 2 });

  // True when the point is just OUTSIDE a corner (rotate zone).
  function inRotateZone(px: number, py: number): boolean {
    const corners = [
      [box.left, box.top], [box.left + box.width, box.top],
      [box.left, box.top + box.height], [box.left + box.width, box.top + box.height],
    ];
    const insideBox = px > box.left && px < box.left + box.width && py > box.top && py < box.top + box.height;
    if (insideBox) return false;
    for (const [cxp, cyp] of corners) {
      const d = Math.hypot(px - cxp, py - cyp);
      if (d <= 30) return true;
    }
    return false;
  }
```
Modify `onMove` so hover sets a rotate cursor and a rotate drag updates angle. Replace `onMove` with:
```ts
  function onMove(e: PointerEvent) {
    const [px, py] = localXY(e);
    if (rotating) {
      const { cx, cy } = center();
      const ang = Math.atan2(py - cy, px - cx);
      const deg = rotStartAngle + ((ang - rotStartPointer) * 180) / Math.PI;
      dispatch("straighten", Math.max(-45, Math.min(45, deg)));
      return;
    }
    if (!active) {
      const h = handleAt(px, py, box, 12);
      hover = h;
      hoverRotate = !h && inRotateZone(px, py);
      return;
    }
    const dnx = (px - startX) / Math.max(1, img.width);
    const dny = (py - startY) / Math.max(1, img.height);
    const lock = e.shiftKey ? lockRatio : null;
    rect = applyDrag(active, startRect, dnx, dny, lock);
    if (active !== "move" && lock == null) dispatch("custom");
  }
```
Add `let hoverRotate = false;` near `let hover`. Replace `onDown` with:
```ts
  function onDown(e: PointerEvent) {
    const [px, py] = localXY(e);
    const h = handleAt(px, py, box, 12);
    if (!h && inRotateZone(px, py)) {
      rotating = true; rotStartAngle = angle;
      const { cx, cy } = center();
      rotStartPointer = Math.atan2(py - cy, px - cx);
      host.setPointerCapture(e.pointerId);
      return;
    }
    if (!h) return;
    active = h; startRect = rect; startX = px; startY = py;
    host.setPointerCapture(e.pointerId);
  }
```
Replace `onUp`: `function onUp() { active = null; rotating = false; }`.
Update the cursor reactive to include rotate:
```ts
  $: cursor = active ? CURSOR[active] : rotating ? "grabbing" : hoverRotate ? "grab" : (hover ? CURSOR[hover] : "default");
```

- [ ] **Step 2: Typecheck**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -10`
Expected: no new errors from CropOverlay (only pre-existing + Task-9 Develop). a11y warning on the div is fine.

- [ ] **Step 3: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/crop/CropOverlay.svelte
git commit -m "feat(app): CropOverlay rotate-on-hover-outside-corner → straighten"
```

---

## Task 8: CropView — oriented fetch + live CSS rotate

**Files:** Modify `app/src/lib/crop/CropView.svelte`

- [ ] **Step 1: Add geometry props, CSS rotate, forward straighten**
Add props after `lockRatio`:
```ts
  export let rot90 = 0;
  export let flipH = false;
  export let flipV = false;
  export let angle = 0;
```
In `render()`, pass orientation to the fetch (NOT the angle — angle is CSS-applied live), changing the `renderView` view object to:
```ts
      src = await api.renderView(id, params, {
        crop: [0, 0, imgW, imgH], out_w, out_h, raw: false, finish: true,
        image_crop: null, rot90, flip_h: flipH, flip_v: flipV, angle: 0,
      });
```
Add `rot90, flipH, flipV` to the fetch `key` so the oriented image re-fetches on those discrete changes (angle is NOT in the key — it's live CSS):
```ts
  $: key = `${id}|${vpW}|${vpH}|${imgW}|${imgH}|${rot90}|${flipH}|${flipV}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${params.contrast}|${params.highlights}|${params.shadows}|${params.whites}|${params.blacks}|${params.texture}|${params.vibrance}|${params.saturation}`;
```
Apply the live CSS rotation to the `<img>` and forward the overlay's straighten event. Update the markup:
```svelte
  {#if src}
    <img {src} alt="crop" draggable="false"
      style="position:absolute; left:{imgScreen.left}px; top:{imgScreen.top}px; width:{dispW}px; height:{dispH}px; transform:rotate({angle}deg);" />
    <CropOverlay bind:rect img={imgScreen} {lockRatio} {angle} on:custom on:straighten />
  {:else}<div class="hint">…</div>{/if}
```
(`imgW`/`imgH` passed by the parent are already the ORIENTED dims; the CSS `transform:rotate` tilts the oriented image live, and the overlay box — positioned via `imgScreen` — sits on it. The parent applies `constrainToRotated` so the box stays on real pixels.)

- [ ] **Step 2: Typecheck + commit**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -10` (only pre-existing + Task-9 Develop errors).
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/crop/CropView.svelte
git commit -m "feat(app): CropView oriented fetch + live CSS straighten + forward straighten"
```

---

## Task 9: Develop wiring + Viewport props (box-follows, shortcuts, dims)

**Files:** Modify `app/src/lib/viewport/Viewport.svelte`, `app/src/lib/tabs/Develop.svelte`

- [ ] **Step 1: Viewport geometry props**
Add after `imageCrop`:
```ts
  export let rot90 = 0;
  export let flipH = false;
  export let flipV = false;
  export let angle = 0;
```
In `render()`, the `renderView` view object — add the geometry fields alongside `image_crop`:
```ts
        image_crop: imageCrop, rot90, flip_h: flipH, flip_v: flipV, angle,
```
Append them to `srcKey`:
```ts
  $: srcKey = `${id}|${raw}|${eff}|${vpW}|${vpH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}`;
```

- [ ] **Step 2: Develop.svelte — full wiring**
Update imports:
```ts
  import { default80, conform, constrainToRotated } from "../crop/cropMath";
  import { effectiveRatio... } // keep presetNormAspect import already present
  import { rotateRectCW, rotateRectCCW, flipRectH, flipRectV } from "../crop/transforms";
  import { orientDims } from "../crop/transforms"; // see note
```
NOTE: add a tiny `orientDims` to `transforms.ts` (so the frontend can swap dims):
```ts
export function orientDims(w: number, h: number, rot90: number): [number, number] {
  return rot90 % 2 === 1 ? [h, w] : [w, h];
}
```
(Add this export to `transforms.ts` in Task 5 if doing them together, or here — include it now and re-run that test file; it needs no new test beyond the existing transform tests, but add: `expect(orientDims(2,3,1)).toEqual([3,2])` to transforms.test.ts.)

Replace the relevant parts of `Develop.svelte`'s script. The draft block becomes:
```ts
  let rect: Rect = { x: 0.1, y: 0.1, w: 0.8, h: 0.8 };
  let aspect = "original";
  let orientation: "landscape" | "portrait" = "landscape";
  let rot90 = 0, flipH = false, flipV = false, angle = 0;
  let cropInit = false;

  // Oriented dims drive the overlay aspect math.
  $: [oW, oH] = orientDims(origW, origH, rot90);
  $: orientedRatio = oH > 0 ? oW / oH : 1;

  function startCrop() {
    const c = $activeCrop;
    if (c) {
      rect = { ...c.rect }; aspect = c.aspect; orientation = c.orientation;
      rot90 = c.rot90; flipH = c.flipH; flipV = c.flipV; angle = c.angle;
    } else {
      rect = default80(); aspect = "original"; orientation = origW >= origH ? "landscape" : "portrait";
      rot90 = 0; flipH = false; flipV = false; angle = 0;
    }
    cropInit = true;
  }
  function draftCrop(): CropRect { return { rect, aspect, orientation, rot90: rot90 as 0|1|2|3, flipH, flipV, angle }; }
  function commitCrop() {
    const id = $activeId; if (!id || !cropInit) return;
    cropById.update((m) => ({ ...m, [id]: draftCrop() }));
  }
  function discardCrop() {
    const c = $activeCrop;
    if (c) { rect = { ...c.rect }; aspect = c.aspect; orientation = c.orientation; rot90 = c.rot90; flipH = c.flipH; flipV = c.flipV; angle = c.angle; }
    else { rect = default80(); aspect = "original"; rot90 = 0; flipH = false; flipV = false; angle = 0; }
  }
  function onPreset(id: string) { aspect = id; rect = conform(rect, presetNormAspect(id, orientedRatio, orientation)); }
  function onSwap() { orientation = orientation === "landscape" ? "portrait" : "landscape"; rect = conform(rect, presetNormAspect(aspect, orientedRatio, orientation)); }
  function onReset() { rect = default80(); aspect = "original"; orientation = origW >= origH ? "landscape" : "portrait"; rot90 = 0; flipH = false; flipV = false; angle = 0; }
  function onRotate(dir: number) {
    if (dir > 0) { rot90 = (rot90 + 1) % 4; rect = rotateRectCW(rect); }
    else { rot90 = (rot90 + 3) % 4; rect = rotateRectCCW(rect); }
  }
  function onFlip(axis: "h" | "v") {
    if (axis === "h") { flipH = !flipH; rect = flipRectH(rect); } else { flipV = !flipV; rect = flipRectV(rect); }
    angle = -angle;
  }
  function onStraighten(v: number) { angle = Math.max(-45, Math.min(45, v)); }

  $: lockRatio = presetNormAspect(aspect, orientedRatio, orientation);
  // Keep the crop inside the rotated image (idempotent → no loop).
  $: if (angle !== 0) rect = constrainToRotated(rect, angle, oW, oH);
```
The `prevTool` commit-on-leave reactive stays. Update `onKey` to add the shortcuts (works in crop mode on the draft, otherwise on the committed crop):
```ts
  function rotateCommitted(dir: number) {
    const id = $activeId; if (!id) return;
    const base = $activeCrop ?? { rect: { x: 0, y: 0, w: 1, h: 1 }, aspect: "custom", orientation: "landscape" as const, rot90: 0 as 0|1|2|3, flipH: false, flipV: false, angle: 0 };
    const nr = dir > 0 ? rotateRectCW(base.rect) : rotateRectCCW(base.rect);
    const nrot = ((base.rot90 + (dir > 0 ? 1 : 3)) % 4) as 0|1|2|3;
    cropById.update((m) => ({ ...m, [id]: { ...base, rect: nr, rot90: nrot } }));
  }
  function onKey(e: KeyboardEvent) {
    const meta = e.metaKey || e.ctrlKey;
    if (meta && (e.key === "]" || e.key === "[")) {
      e.preventDefault();
      const dir = e.key === "]" ? 1 : -1;
      if ($tool === "crop") onRotate(dir); else rotateCommitted(dir);
      return;
    }
    if ($tool !== "crop") return;
    if (e.key === "Enter") { commitCrop(); tool.set("edit"); }
    else if (e.key === "Escape") { discardCrop(); }
    else if (e.key === "x" || e.key === "X") { onSwap(); }
  }
```
Committed-crop reactive for the normal Viewport (replace the existing `committed/effW/effH/imageCrop` block):
```ts
  $: committed = $activeCrop;
  $: cRot = committed?.rot90 ?? 0;
  $: [coW, coH] = orientDims(origW, origH, cRot);
  $: effW = committed ? Math.max(1, Math.round(committed.rect.w * coW)) : coW;
  $: effH = committed ? Math.max(1, Math.round(committed.rect.h * coH)) : coH;
  $: imageCrop = committed ? [committed.rect.x, committed.rect.y, committed.rect.w, committed.rect.h] as [number, number, number, number] : null;
```
Export passes geometry:
```ts
    try {
      await api.exportImage($activeId, $params, out, imageCrop, {
        rot90: committed?.rot90 ?? 0, flip_h: committed?.flipH ?? false,
        flip_v: committed?.flipV ?? false, angle: committed?.angle ?? 0,
      });
      msg = "Exported ✓";
    } catch (e) { msg = "Error: " + e; }
```
Markup — pass the new props/handlers:
```svelte
      {#if $tool === "crop"}
        <CropView id={$activeId} params={$params} imgW={oW} imgH={oH}
                  bind:rect {lockRatio} {rot90} {flipH} {flipV} {angle}
                  on:custom={() => (aspect = "custom")} on:straighten={(e) => onStraighten(e.detail)} />
      {:else}
        <Viewport id={$activeId} params={$params} imgW={effW} imgH={effH} imageCrop={imageCrop}
                  rot90={cRot} flipH={committed?.flipH ?? false} flipV={committed?.flipV ?? false} angle={committed?.angle ?? 0} />
      {/if}
```
```svelte
      {:else if $tool === "crop"}
        <CropPanel bind:aspect bind:orientation bind:angle
                   on:preset={(e) => onPreset(e.detail)} on:swap={onSwap} on:reset={onReset}
                   on:rotate={(e) => onRotate(e.detail)} on:flip={(e) => onFlip(e.detail)} />
      {/if}
```

- [ ] **Step 3: Typecheck + unit tests**
`cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -15 && npx vitest run 2>&1 | tail -5`
Expected: NO new errors anywhere now (the Task-4 Develop `CropRect`-field errors are resolved by `draftCrop()`); only the pre-existing `workflow.test.ts` error. All vitest pass.

- [ ] **Step 4: Commit**
```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/viewport/Viewport.svelte app/src/lib/tabs/Develop.svelte
git commit -m "feat(app): wire rotate/flip/straighten into Develop + Viewport (box-follows, shortcuts)"
```

---

## Task 10: Verification + manual smoke

- [ ] **Step 1: Automated**
```
source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml 2>&1 | grep "test result"
source "$HOME/.cargo/env" && cargo clippy --manifest-path app/src-tauri/Cargo.toml 2>&1 | tail -3
cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -1 && npx vitest run 2>&1 | tail -4
```
Expected: backend tests pass; clippy clean; svelte-check only the pre-existing error; vitest all pass.

- [ ] **Step 2: Manual smoke (user)**
In Develop → Crop on a developed image:
- **Rotate CW/CCW** buttons and **⌘/Ctrl+] / [** turn the image 90°; the crop box follows (a 16×9 becomes the matching 9×16 region); ⌘/Ctrl+]/[ also works in the Edit view (rotates the committed image).
- **Flip H / Flip V** mirror the image; a set straighten angle stays correct.
- **Straighten slider** and **dragging just outside a corner** tilt the image live; the crop box stays inside the rotated image (no blank wedges); double-click the slider resets to 0.
- **Enter**/switching to Edit commits → the Edit view and **export** reflect rotation + flip + straighten + crop.
- Per-image (transforms on A don't affect B).

- [ ] **Step 3: Final commit (only if smoke needed fixups)**
```bash
cd /Users/mohaelder/Repos/filmrev
git add -A && git commit -m "fix: rotate/flip/straighten smoke fixups"
```

---

## Self-Review notes

- **Spec coverage:** orient/orient_dims (T1), rotate (T2), backend pipeline + export (T3), TS contract + CropRect (T4), rect transforms + constrainToRotated (T5), CropPanel slider/buttons + glyphs (T6), rotate-on-hover (T7), CropView oriented+CSS straighten (T8), Develop box-follows + shortcuts + dims + Viewport props (T9), verify (T10).
- **Convention consistency:** the clockwise-positive sampling matrix is identical in `convert::rotate` (T2) and `constrainToRotated` (T5), and matches CSS `rotate({angle}deg)` in CropView (T8).
- **No reactive loop:** `constrainToRotated` returns the input unchanged when already inside, so `$: if (angle!==0) rect = constrainToRotated(...)` converges in ≤2 ticks.
- **Type consistency:** `CropRect` (rot90/flipH/flipV/angle) is built via `draftCrop()`/`rotateCommitted` and consumed by `effW/effH/imageCrop` + export; `orientDims` (TS) mirrors `orient_dims` (Rust); ViewSpec `rot90/flip_h/flip_v/angle` match between Rust `#[serde(default)]` and TS, and `exportImage`'s `flipH/flipV` camelCase map to `flip_h/flip_v`.
- **Cross-task note:** Task 4 intentionally leaves `Develop.svelte` with transient missing-field type errors that Task 9 resolves; Task 6/7/8 typechecks tolerate those Develop-only errors.
- **Known carry-over:** the pre-existing `workflow.test.ts` `path` error is unrelated.
