# Global IR Smart Dust Removal (Plan B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the "Remove dust (IR)" toggle live for images that carry an infrared plane — auto-detect dust/scratches from IR and inpaint them non-destructively, full-res on export. Disabled (as already shipped) for non-IR images.

**Architecture:** Carry the IR plane as a 4th channel through the geometry pipeline (`proxy`/`crop`/`orient`/`rotate`/`resize_to`), so the IR aligns pixel-for-pixel with the rendered image. Detect defects late (on the geometry-aligned IR) in `render_view`/`export_image`, build a full-frame `Mask`, and reuse Plan A's `inpaint_masked` to heal `inv` before finishing.

**Tech Stack:** Rust (`film-core`, `src-tauri` `app` crate, `film-cli`), `inpaint` (Telea) + `ndarray` + `image` (Luma resize); SvelteKit + TypeScript (vitest).

**Builds on:** Plan A (`2026-06-04-eraser-dust-removal-manual-brush.md`). Reuses `film_core::dust::{Mask, inpaint_masked, RADIUS}` and the `dustById` store. Design: `2026-06-04-eraser-dust-removal-ir-design.md`.

**Build/test commands** (cargo is not on PATH — always prefix `source "$HOME/.cargo/env" && ...`):
- film-core: `cargo test -p film-core`
- app backend (excluded from workspace): `cargo test --manifest-path app/src-tauri/Cargo.toml`
- film-cli: `cargo test -p film-cli` / run `cargo run -p film-cli -- ...`
- TS: `cd app && npm run test:unit` and `npm run check`

**Pre-existing baseline:** `app` svelte-check has ONE pre-existing error (`workflow.test.ts` missing `path`); ignore it. film-core 46 tests, src-tauri 25 tests, vitest 47 — all green before this plan.

---

## File Structure

**Modify (Rust):**
- `app/src-tauri/src/convert.rs` — `proxy`/`crop`/`orient`(+`flip_h`/`flip_v`/`rotate_cw`)/`rotate`/`resize_to` carry the IR plane; new private `resize_ir` + `sample_scalar_bilinear` helpers.
- `app/src-tauri/src/session.rs` — `ImageEntry.has_ir`; `Session::insert` sets it.
- `app/src-tauri/src/commands.rs` — `develop_image` computes `has_ir`; `IrRemoval` DTO + `ViewSpec.ir_removal`; apply IR in `render_view` and `export_image`.
- `crates/film-core/src/dust.rs` — `ir_defect_mask` (detection) + `apply_ir` (detect → reuse `inpaint_masked`).
- `crates/film-cli/src/main.rs` — `--check-ir` flag.

**Modify (TS):**
- `app/src/lib/develop/dust.ts` — `IrRemoval` type, extend `DustEdits`, reducers, `emptyDust`.
- `app/src/lib/api.ts` — `ImageEntry.has_ir`, `ViewSpec.ir_removal`, `exportImage` ir arg, `renderView` wires `ir_removal`.
- `app/src/lib/develop/EraserPanel.svelte` — live toggle + sensitivity slider when `hasIr`.
- `app/src/lib/tabs/Develop.svelte` — pass `hasIr`, own `irRemoval` handlers, wire to Viewport/export.

---

## Task 1: `proxy` and `resize_to` carry the IR plane

**Files:** Modify `app/src-tauri/src/convert.rs`

- [ ] **Step 1: Write failing tests** — add to the `#[cfg(test)] mod tests` block:

```rust
    fn solid_ir(w: usize, h: usize, c: [f32; 3], ir: f32) -> Image {
        Image { width: w, height: h, pixels: vec![c; w * h], ir: Some(vec![ir; w * h]) }
    }

    #[test]
    fn proxy_carries_and_resizes_ir() {
        let img = solid_ir(4000, 2000, [0.4, 0.4, 0.4], 0.8);
        let p = proxy(&img, 2048);
        assert_eq!((p.width, p.height), (2048, 1024));
        let ir = p.ir.expect("ir preserved through proxy");
        assert_eq!(ir.len(), 2048 * 1024);
        assert!((ir[0] - 0.8).abs() < 1e-3, "ir value preserved on solid field");
    }

    #[test]
    fn proxy_noop_small_keeps_ir() {
        let img = solid_ir(10, 8, [0.1, 0.2, 0.3], 0.5);
        let p = proxy(&img, 2048);
        assert_eq!(p.ir.as_ref().map(|v| v.len()), Some(80));
    }

    #[test]
    fn resize_to_carries_ir() {
        let img = solid_ir(10, 8, [0.2, 0.4, 0.6], 0.7);
        let r = resize_to(&img, 5, 4);
        let ir = r.ir.expect("ir preserved through resize_to");
        assert_eq!(ir.len(), 20);
        assert!((ir[0] - 0.7).abs() < 1e-3);
    }

    #[test]
    fn resize_to_drops_none_ir() {
        let img = solid(10, 8, [0.2, 0.4, 0.6]);
        assert!(resize_to(&img, 5, 4).ir.is_none());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml proxy_carries`
Expected: FAIL (ir is None — current code drops it).

- [ ] **Step 3: Add the `resize_ir` helper** (near `to_rgb32f`, add the import + fn):

At the top imports of `convert.rs`, change `use image::{ImageBuffer, Rgb};` to:
```rust
use image::{ImageBuffer, Luma, Rgb};
```
Add the helper:
```rust
/// Resize a single-channel IR plane to `nw`×`nh` (same Triangle filter as RGB).
fn resize_ir(ir: &[f32], w: usize, h: usize, nw: u32, nh: u32) -> Vec<f32> {
    let buf: ImageBuffer<Luma<f32>, Vec<f32>> =
        ImageBuffer::from_raw(w as u32, h as u32, ir.to_vec()).expect("ir plane matches w*h");
    let r = image::imageops::resize(&buf, nw.max(1), nh.max(1), image::imageops::FilterType::Triangle);
    r.into_raw()
}
```

- [ ] **Step 4: Make `proxy` carry IR.** Replace the resize branch tail of `proxy`:
```rust
    let buf = to_rgb32f(img);
    let resized = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
    from_rgb32f(&resized)
```
with:
```rust
    let buf = to_rgb32f(img);
    let resized = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
    let mut out = from_rgb32f(&resized);
    out.ir = img.ir.as_ref().map(|ir| resize_ir(ir, img.width, img.height, nw, nh));
    out
```
(The early `if long <= max_edge { return img.clone(); }` path already preserves `ir` via clone — leave it.)

- [ ] **Step 5: Make `resize_to` carry IR.** Replace its tail:
```rust
    let buf = to_rgb32f(img);
    let r = image::imageops::resize(&buf, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
    from_rgb32f(&r)
```
with:
```rust
    let buf = to_rgb32f(img);
    let r = image::imageops::resize(&buf, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
    let mut out = from_rgb32f(&r);
    out.ir = img.ir.as_ref().map(|ir| resize_ir(ir, img.width, img.height, w.max(1), h.max(1)));
    out
```
(The early `if img.width as u32 == w && img.height as u32 == h { return img.clone(); }` path preserves ir via clone — leave it.)

- [ ] **Step 6: Run tests** — `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml` → all pass (new + existing). Also `cargo clippy --manifest-path app/src-tauri/Cargo.toml 2>&1 | tail -5` → no new warnings.

- [ ] **Step 7: Commit**
```bash
git add app/src-tauri/src/convert.rs
git commit -m "feat(backend): proxy/resize_to carry the IR plane"
```

---

## Task 2: `crop` and `orient` carry the IR plane

**Files:** Modify `app/src-tauri/src/convert.rs`

- [ ] **Step 1: Write failing tests** (add to `mod tests`):

```rust
    fn ramp_ir(w: usize, h: usize) -> Image {
        // pixels and ir both encode a per-pixel index so remaps are checkable.
        let mut img = Image { width: w, height: h, pixels: vec![[0.0; 3]; w * h], ir: Some(vec![0.0; w * h]) };
        for i in 0..w * h {
            img.pixels[i] = [i as f32, 0.0, 0.0];
            if let Some(ir) = img.ir.as_mut() { ir[i] = i as f32; }
        }
        img
    }

    #[test]
    fn crop_carries_ir_subrectangle() {
        let img = ramp_ir(4, 4);
        let c = crop(&img, 1, 2, 2, 1); // row 2, cols 1..3 → indices 9,10
        let ir = c.ir.expect("crop carries ir");
        assert_eq!(ir, vec![9.0, 10.0]);
    }

    #[test]
    fn crop_none_ir_stays_none() {
        let img = solid(4, 4, [0.5, 0.5, 0.5]);
        assert!(crop(&img, 0, 0, 2, 2).ir.is_none());
    }

    #[test]
    fn orient_flip_h_remaps_ir_like_pixels() {
        let img = ramp_ir(2, 3);
        let f = orient(&img, 0, true, false);
        let ir = f.ir.expect("orient carries ir");
        // flip_h swaps columns: pixel[0].r and ir[0] both come from old index 1.
        assert_eq!(f.pixels[0][0], ir[0]);
        assert_eq!(f.pixels[1][0], ir[1]);
    }

    #[test]
    fn orient_rot90_remaps_ir_like_pixels() {
        let img = ramp_ir(2, 3);
        let r = orient(&img, 1, false, false);
        let ir = r.ir.expect("orient carries ir through rot90");
        assert_eq!((r.width, r.height), (3, 2));
        // ir must track the same remap as the red channel everywhere.
        for i in 0..r.pixels.len() {
            assert_eq!(r.pixels[i][0], ir[i]);
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml crop_carries_ir`
Expected: FAIL (ir None).

- [ ] **Step 3: Make `crop` carry IR.** Replace the body's pixel-copy tail. Current:
```rust
    let mut pixels = Vec::with_capacity(cw * ch);
    for yy in y..y2 {
        let row = yy * img.width;
        for xx in x..x2 {
            pixels.push(img.pixels[row + xx]);
        }
    }
    Image { width: cw, height: ch, pixels, ir: None }
```
Replace with:
```rust
    let mut pixels = Vec::with_capacity(cw * ch);
    let mut ir: Option<Vec<f32>> = img.ir.as_ref().map(|_| Vec::with_capacity(cw * ch));
    for yy in y..y2 {
        let row = yy * img.width;
        for xx in x..x2 {
            pixels.push(img.pixels[row + xx]);
            if let (Some(dst), Some(src)) = (ir.as_mut(), img.ir.as_ref()) {
                dst.push(src[row + xx]);
            }
        }
    }
    Image { width: cw, height: ch, pixels, ir }
```

- [ ] **Step 4: Make the three orient primitives carry IR.** `orient` composes `flip_h`/`flip_v`/`rotate_cw`, so fixing those three propagates automatically. Replace each:

`flip_h`:
```rust
fn flip_h(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut px = vec![[0.0_f32; 3]; w * h];
    let mut ir = img.ir.as_ref().map(|_| vec![0.0_f32; w * h]);
    for y in 0..h { for x in 0..w {
        let (dst, src) = (y * w + x, y * w + (w - 1 - x));
        px[dst] = img.pixels[src];
        if let (Some(d), Some(s)) = (ir.as_mut(), img.ir.as_ref()) { d[dst] = s[src]; }
    } }
    Image { width: w, height: h, pixels: px, ir }
}
```
`flip_v`:
```rust
fn flip_v(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut px = vec![[0.0_f32; 3]; w * h];
    let mut ir = img.ir.as_ref().map(|_| vec![0.0_f32; w * h]);
    for y in 0..h { for x in 0..w {
        let (dst, src) = (y * w + x, (h - 1 - y) * w + x);
        px[dst] = img.pixels[src];
        if let (Some(d), Some(s)) = (ir.as_mut(), img.ir.as_ref()) { d[dst] = s[src]; }
    } }
    Image { width: w, height: h, pixels: px, ir }
}
```
`rotate_cw`:
```rust
fn rotate_cw(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let (nw, nh) = (h, w);
    let mut px = vec![[0.0_f32; 3]; nw * nh];
    let mut ir = img.ir.as_ref().map(|_| vec![0.0_f32; nw * nh]);
    for ny in 0..nh { for nx in 0..nw {
        let ox = ny; let oy = h - 1 - nx;
        let (dst, src) = (ny * nw + nx, oy * w + ox);
        px[dst] = img.pixels[src];
        if let (Some(d), Some(s)) = (ir.as_mut(), img.ir.as_ref()) { d[dst] = s[src]; }
    } }
    Image { width: nw, height: nh, pixels: px, ir }
}
```

- [ ] **Step 5: Run tests** — `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml` → all pass. Clippy clean.

- [ ] **Step 6: Commit**
```bash
git add app/src-tauri/src/convert.rs
git commit -m "feat(backend): crop/orient carry the IR plane"
```

---

## Task 3: `rotate` (straighten) carries the IR plane

**Files:** Modify `app/src-tauri/src/convert.rs`

- [ ] **Step 1: Write failing test** (add to `mod tests`):

```rust
    #[test]
    fn rotate_zero_preserves_ir() {
        let img = ramp_ir(3, 3);
        let r = rotate(&img, 0.0);
        assert_eq!(r.ir.as_ref().map(|v| v.len()), Some(9));
    }

    #[test]
    fn rotate_carries_ir_and_blacks_corners() {
        let img = ramp_ir(5, 5);
        let r = rotate(&img, 30.0);
        let ir = r.ir.expect("rotate carries ir");
        assert_eq!(ir.len(), 25);
        // Top-left corner is rotated out of frame → ir 0.0 (same as RGB black).
        assert_eq!(r.pixels[0], [0.0, 0.0, 0.0]);
        assert_eq!(ir[0], 0.0);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml rotate_carries_ir`
Expected: FAIL (ir None).

- [ ] **Step 3: Add a scalar bilinear sampler** (next to `sample_bilinear`):
```rust
/// Bilinear sample a single-channel plane; 0.0 for out-of-bounds (mirrors sample_bilinear).
fn sample_scalar_bilinear(plane: &[f32], w: usize, h: usize, sx: f32, sy: f32) -> f32 {
    let (wi, hi) = (w as i32, h as i32);
    if sx < 0.0 || sy < 0.0 || sx >= wi as f32 || sy >= hi as f32 {
        return 0.0;
    }
    let x0 = sx.floor() as i32; let y0 = sy.floor() as i32;
    let fx = sx - x0 as f32; let fy = sy - y0 as f32;
    let get = |x: i32, y: i32| -> f32 {
        let xc = x.clamp(0, wi - 1) as usize; let yc = y.clamp(0, hi - 1) as usize;
        plane[yc * w + xc]
    };
    let a = get(x0, y0) * (1.0 - fx) + get(x0 + 1, y0) * fx;
    let b = get(x0, y0 + 1) * (1.0 - fx) + get(x0 + 1, y0 + 1) * fx;
    a * (1.0 - fy) + b * fy
}
```

- [ ] **Step 4: Make `rotate` carry IR.** The `rotate` early-return `if deg.abs() < 1e-4 { return img.clone(); }` already preserves ir. For the main path, replace:
```rust
    let mut px = vec![[0.0_f32; 3]; w * h];
    for oy in 0..h { for ox in 0..w {
        let dx = ox as f32 + 0.5 - cx;
        let dy = oy as f32 + 0.5 - cy;
        let sx = cos * dx + sin * dy + cx - 0.5;
        let sy = -sin * dx + cos * dy + cy - 0.5;
        px[oy * w + ox] = sample_bilinear(img, sx, sy);
    } }
    Image { width: w, height: h, pixels: px, ir: None }
```
with:
```rust
    let mut px = vec![[0.0_f32; 3]; w * h];
    let mut ir = img.ir.as_ref().map(|_| vec![0.0_f32; w * h]);
    for oy in 0..h { for ox in 0..w {
        let dx = ox as f32 + 0.5 - cx;
        let dy = oy as f32 + 0.5 - cy;
        let sx = cos * dx + sin * dy + cx - 0.5;
        let sy = -sin * dx + cos * dy + cy - 0.5;
        px[oy * w + ox] = sample_bilinear(img, sx, sy);
        if let (Some(d), Some(s)) = (ir.as_mut(), img.ir.as_ref()) {
            d[oy * w + ox] = sample_scalar_bilinear(s, w, h, sx, sy);
        }
    } }
    Image { width: w, height: h, pixels: px, ir }
```

- [ ] **Step 5: Run tests** — `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml` → all pass. Clippy clean.

- [ ] **Step 6: Commit**
```bash
git add app/src-tauri/src/convert.rs
git commit -m "feat(backend): rotate (straighten) carries the IR plane"
```

---

## Task 4: `develop_image` keeps IR on `working` + `has_ir` on `ImageEntry`

**Files:** Modify `app/src-tauri/src/session.rs`, `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing test** (add to `session.rs` `mod tests`):

```rust
    #[test]
    fn insert_reports_has_ir_false_when_undeveloped() {
        let s = Session::default();
        let img = CachedImage {
            path: "/x/a.tif".into(), file_name: "a.tif".into(),
            metadata: Metadata::default(), thumbnail: "data:,".into(), developed: None,
        };
        let e = s.insert(img);
        assert!(!e.has_ir);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml insert_reports_has_ir`
Expected: FAIL — `ImageEntry` has no `has_ir`.

- [ ] **Step 3: Add `has_ir` to `ImageEntry`** in `session.rs`:
```rust
pub struct ImageEntry {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub thumbnail: String,
    pub metadata: Metadata,
    pub developed: bool,
    pub has_ir: bool,
}
```
Set it in `Session::insert` (the entry literal):
```rust
        let entry = ImageEntry {
            id: id.clone(),
            path: img.path.clone(),
            file_name: img.file_name.clone(),
            thumbnail: img.thumbnail.clone(),
            metadata: img.metadata.clone(),
            developed: img.developed.is_some(),
            has_ir: img.developed.as_ref().map(|d| d.working.ir.is_some()).unwrap_or(false),
        };
```

- [ ] **Step 4: Set `has_ir` in `develop_image`** (`commands.rs`). In `develop_image`, after `let working = proxy(&full, cap);` capture the flag, and add it to the returned `ImageEntry`. Find the end of `develop_image` where it builds the entry and add `has_ir`. Specifically, after `img.developed = Some(Developed { working, thumb, base });` the `working` is moved — so compute the flag BEFORE the move. Add right after `let working = proxy(&full, cap);`:
```rust
    let has_ir = working.ir.is_some();
```
Then in the returned `Ok(ImageEntry { ... })` at the end of `develop_image`, add the field:
```rust
    Ok(ImageEntry {
        id: id.clone(),
        path: img.path.clone(),
        file_name: img.file_name.clone(),
        thumbnail,
        metadata: img.metadata.clone(),
        developed: true,
        has_ir,
    })
```

- [ ] **Step 5: Run tests + build** — `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml` → all pass (the new test + existing `insert_reports_undeveloped...` still pass). `cargo build --manifest-path app/src-tauri/Cargo.toml` → compiles.

- [ ] **Step 6: Commit**
```bash
git add app/src-tauri/src/session.rs app/src-tauri/src/commands.rs
git commit -m "feat(backend): report has_ir; keep IR on developed working buffer"
```

---

## Task 5: `ir_defect_mask` + `apply_ir` (film-core dust engine)

**Files:** Modify `crates/film-core/src/dust.rs`

- [ ] **Step 1: Write failing tests** (add to dust.rs `mod tests`):

```rust
    #[test]
    fn ir_defect_mask_flags_low_ir_and_ignores_clean() {
        // 11x11 clean IR field at 0.9 with one defect pixel at (5,5) = 0.1.
        let n = 11usize;
        let mut ir = vec![0.9_f32; n * n];
        ir[5 * n + 5] = 0.1;
        let m = ir_defect_mask(n, n, &ir, 50.0); // sensitivity 50 → t=0.725 → thr=0.6525
        assert_eq!((m.x0, m.y0, m.w, m.h), (0, 0, n, n), "ir mask spans the whole frame");
        assert!(m.bits[5 * n + 5], "defect pixel flagged");
        assert!(!m.bits[0], "clean corner not flagged");
    }

    #[test]
    fn ir_defect_mask_sensitivity_widens_detection() {
        let n = 11usize;
        let mut ir = vec![0.9_f32; n * n];
        ir[5 * n + 5] = 0.7; // a FAINT defect (just below clean)
        let low = ir_defect_mask(n, n, &ir, 0.0);   // t=0.5 → thr=0.45 → 0.7 not flagged
        let high = ir_defect_mask(n, n, &ir, 100.0); // t=0.95 → thr=0.855 → 0.7 flagged
        assert!(!low.bits[5 * n + 5], "faint defect missed at low sensitivity");
        assert!(high.bits[5 * n + 5], "faint defect caught at high sensitivity");
    }

    #[test]
    fn ir_defect_mask_skips_zero_ir_corners() {
        // A pixel with ir exactly 0.0 (straighten out-of-frame) must NOT be flagged.
        let n = 5usize;
        let mut ir = vec![0.9_f32; n * n];
        ir[0] = 0.0;
        let m = ir_defect_mask(n, n, &ir, 100.0);
        assert!(!m.bits[0], "ir==0 (out-of-frame) is not a defect");
    }

    #[test]
    fn apply_ir_heals_defect_colocated_with_low_ir() {
        let n = 21usize;
        let mut img = Image { width: n, height: n, pixels: vec![[0.4, 0.4, 0.4]; n * n], ir: None };
        let mid = 10 * n + 10;
        img.pixels[mid] = [1.0, 1.0, 1.0]; // white speck
        let mut ir = vec![0.9_f32; n * n];
        ir[mid] = 0.05; // co-located low IR
        apply_ir(&mut img, &ir, 50.0);
        assert!(img.pixels[mid][0] < 0.6, "speck healed toward field, got {:?}", img.pixels[mid]);
    }

    #[test]
    fn apply_ir_noop_on_length_mismatch() {
        let mut img = Image { width: 4, height: 4, pixels: vec![[0.3; 3]; 16], ir: None };
        let before = img.clone();
        apply_ir(&mut img, &[0.1; 9], 50.0); // wrong length
        assert_eq!(img, before);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core dust::tests::ir_defect_mask`
Expected: FAIL — `ir_defect_mask`/`apply_ir` not defined.

- [ ] **Step 3: Implement** (add to `dust.rs`, after `apply`):

```rust
/// Build a whole-frame defect `Mask` from an IR plane. IR is high where the film is
/// clean and low where a defect blocks it. `clean` = 95th-percentile IR (robust to the
/// defect minority); a pixel is a defect when `0 < ir < clean * t`, where `t` comes from
/// `sensitivity` (0..100 → t 0.5..0.95). `ir==0` (straighten out-of-frame) is never a
/// defect. The mask is dilated by 1px to cover defect edges.
pub fn ir_defect_mask(w: usize, h: usize, ir: &[f32], sensitivity: f32) -> Mask {
    let empty = Mask { x0: 0, y0: 0, w: 0, h: 0, bits: Vec::new() };
    if w == 0 || h == 0 || ir.len() != w * h {
        return empty;
    }
    // 95th-percentile clean level over positive IR samples.
    let mut sorted: Vec<f32> = ir.iter().copied().filter(|v| *v > 0.0).collect();
    if sorted.is_empty() {
        return empty;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let clean = sorted[((sorted.len() as f32 * 0.95) as usize).min(sorted.len() - 1)];
    let t = 0.5 + 0.45 * (sensitivity.clamp(0.0, 100.0) / 100.0);
    let thr = clean * t;

    // Raw defect bits.
    let mut raw = vec![false; w * h];
    for i in 0..w * h {
        let v = ir[i];
        if v > 0.0 && v < thr {
            raw[i] = true;
        }
    }
    // Dilate by 1px (4-neighborhood is enough for single-px edge growth; use 8 for safety).
    let mut bits = raw.clone();
    for y in 0..h {
        for x in 0..w {
            if !raw[y * w + x] {
                continue;
            }
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        bits[ny as usize * w + nx as usize] = true;
                    }
                }
            }
        }
    }
    Mask { x0: 0, y0: 0, w, h, bits }
}

/// Detect defects from `ir` and inpaint them in place over the whole frame. No-op when
/// `ir` length doesn't match the image or no defects are found.
pub fn apply_ir(img: &mut Image, ir: &[f32], sensitivity: f32) {
    if ir.len() != img.pixels.len() {
        return;
    }
    let mask = ir_defect_mask(img.width, img.height, ir, sensitivity);
    inpaint_masked(img, &mask, RADIUS);
}
```

- [ ] **Step 4: Run tests** — `source "$HOME/.cargo/env" && cargo test -p film-core dust::` → all pass (Plan A's 5 + these 5). Clippy: `cargo clippy -p film-core 2>&1 | tail -5` → clean.

- [ ] **Step 5: Commit**
```bash
git add crates/film-core/src/dust.rs
git commit -m "feat(film-core): ir_defect_mask + apply_ir (IR-driven dust detection)"
```

---

## Task 6: Wire IR removal into `render_view`

**Files:** Modify `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing test** (add to commands.rs `mod tests`):

```rust
    #[test]
    fn viewspec_ir_removal_defaults_off_and_parses() {
        let d: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#).unwrap();
        assert!(!d.ir_removal.enabled, "ir_removal defaults disabled");
        let p: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,
                "ir_removal":{"enabled":true,"sensitivity":60}}"#).unwrap();
        assert!(p.ir_removal.enabled);
        assert!((p.ir_removal.sensitivity - 60.0).abs() < 1e-6);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml viewspec_ir_removal`
Expected: FAIL — no `ir_removal`.

- [ ] **Step 3: Add the `IrRemoval` DTO** (near `DustStroke` in commands.rs):
```rust
/// IR-driven auto dust removal settings from the UI.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IrRemoval {
    pub enabled: bool,
    pub sensitivity: f32,
}
```

- [ ] **Step 4: Add the field to `ViewSpec`** (after `dust`):
```rust
    #[serde(default)] pub ir_removal: IrRemoval,
```

- [ ] **Step 5: Apply IR in `render_view`.** After the Plan A manual-dust block (`dust::apply(&mut inv, &stamps);`) and BEFORE the finish branch, add:
```rust
    if view.ir_removal.enabled {
        if let Some(ir) = scaled.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, view.ir_removal.sensitivity);
        }
    }
```
(`scaled` is the resized cropped image and now carries `.ir` from Tasks 1-3; it is the same dims as `inv`. Confirm `scaled` is still in scope at this point — it is, since `invert_image(&scaled, ...)` borrows it.)

- [ ] **Step 6: Run tests + build** — `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml` → all pass. `cargo clippy --manifest-path app/src-tauri/Cargo.toml 2>&1 | tail -5` → clean.

- [ ] **Step 7: Commit**
```bash
git add app/src-tauri/src/commands.rs
git commit -m "feat(backend): apply IR dust removal in render_view"
```

---

## Task 7: Wire IR removal into `export_image`

**Files:** Modify `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Add the `ir_removal` parameter to `export_image`** (after `dust: Vec<DustStroke>`, before `session`):
```rust
    dust: Vec<DustStroke>,
    ir_removal: IrRemoval,
    session: State<Session>,
```

- [ ] **Step 2: Apply IR in the export pipeline.** After the Plan A manual-dust block (`dust::apply(&mut inv, &stamps);`) and before `finish_image`, add:
```rust
    if ir_removal.enabled {
        if let Some(ir) = full.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, ir_removal.sensitivity);
        }
    }
```
NOTE: `full` is the oriented+straightened+cropped full-res image; with Tasks 1-3 it carries `.ir` (fresh decode → orient → rotate → crop all preserve it). It has the same dims as `inv`. Confirm `full` is still in scope after `let mut inv = invert_image(&full, ...)` (it is — invert borrows it). If `full` was shadowed/moved, capture `full.ir` into a local before the invert.

- [ ] **Step 3: Verify there's no test for export here** (export needs a real file; covered by manual verification Task 11). Build + existing tests: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml` (25+1 pass) and `cargo build --manifest-path app/src-tauri/Cargo.toml`. Clippy clean.

- [ ] **Step 4: Commit**
```bash
git add app/src-tauri/src/commands.rs
git commit -m "feat(backend): apply IR dust removal at full-res in export_image"
```

---

## Task 8: TS `irRemoval` state, `has_ir`, and wire-through

**Files:** Modify `app/src/lib/develop/dust.ts`, `app/src/lib/develop/dust.test.ts`, `app/src/lib/api.ts`

- [ ] **Step 1: Write failing tests** (add to `dust.test.ts`):

```ts
import {
  emptyDust, setIrEnabled, setIrSensitivity,
} from "./dust";

describe("ir removal state", () => {
  it("defaults disabled at sensitivity 50", () => {
    const d = emptyDust();
    expect(d.irRemoval.enabled).toBe(false);
    expect(d.irRemoval.sensitivity).toBe(50);
  });
  it("toggles enabled and sets sensitivity immutably, preserving strokes", () => {
    const d0 = addStroke(emptyDust(), { points: [{ x: 0.5, y: 0.5 }], r: 0.02 });
    const d1 = setIrEnabled(d0, true);
    const d2 = setIrSensitivity(d1, 70);
    expect(d0.irRemoval.enabled).toBe(false); // original untouched
    expect(d2.irRemoval).toEqual({ enabled: true, sensitivity: 70 });
    expect(d2.strokes.length).toBe(1); // strokes preserved
  });
});
```
(Add `addStroke` to the existing import from `./dust` if not already imported in the test file.)

- [ ] **Step 2: Run to verify failure**

Run: `cd app && npm run test:unit -- dust.test`
Expected: FAIL — `setIrEnabled`/`setIrSensitivity` not exported; `irRemoval` missing.

- [ ] **Step 3: Extend `dust.ts`.** Add the type, extend `DustEdits`, update `emptyDust`, add reducers:
```ts
/** IR-driven automatic dust removal settings. */
export interface IrRemoval { enabled: boolean; sensitivity: number }
```
Change `DustEdits`:
```ts
export interface DustEdits { strokes: DustStroke[]; irRemoval: IrRemoval }
```
Change `emptyDust`:
```ts
export const emptyDust = (): DustEdits => ({ strokes: [], irRemoval: { enabled: false, sensitivity: 50 } });
```
Update `addStroke`/`undoStroke`/`resetDust` to preserve `irRemoval`:
```ts
export function addStroke(d: DustEdits, s: DustStroke): DustEdits {
  return { ...d, strokes: [...d.strokes, s] };
}
export function undoStroke(d: DustEdits): DustEdits {
  return { ...d, strokes: d.strokes.slice(0, -1) };
}
export function resetDust(d: DustEdits): DustEdits {
  return { ...d, strokes: [] };
}
export function setIrEnabled(d: DustEdits, enabled: boolean): DustEdits {
  return { ...d, irRemoval: { ...d.irRemoval, enabled } };
}
export function setIrSensitivity(d: DustEdits, sensitivity: number): DustEdits {
  return { ...d, irRemoval: { ...d.irRemoval, sensitivity } };
}
```
NOTE: `resetDust` now takes the current `DustEdits` (to preserve `irRemoval`) instead of being argument-less — this changes its signature. Two callers to update:
- The existing Plan A test in `dust.test.ts` has `expect(resetDust().strokes.length).toBe(0);` — change it to `expect(resetDust(emptyDust()).strokes.length).toBe(0);` (do this in this task, since it's the same file).
- Its caller in `Develop.svelte` (`updateDust(() => resetDust())` → `updateDust((d) => resetDust(d))`) — done in Task 9.

Decision: **Reset clears strokes only, NOT the IR toggle** (the IR pass has its own toggle; "Reset" is the brush-strokes reset). If you want Reset to also turn IR off, that's a Task 9 wiring choice — default here is strokes-only.

- [ ] **Step 4: Wire types in `api.ts`.** Add `has_ir` to `ImageEntry`:
```ts
export interface ImageEntry {
  id: string; path: string; file_name: string; thumbnail: string; metadata: Metadata; developed: boolean; has_ir: boolean;
}
```
Add an `IrRemoval` wire type + field on `ViewSpec`:
```ts
export interface IrRemoval { enabled: boolean; sensitivity: number }
```
```ts
  ir_removal?: IrRemoval;
```
(inside `ViewSpec`). Make `renderView` forward it (it already spreads `view`; `ir_removal` rides along since it's a plain object — confirm the spread `view: { ...view, dust: wireDust(view.dust) }` keeps `ir_removal`. It does, because `...view` includes it.)
Add an `irRemoval` arg to `exportImage`:
```ts
  exportImage: (
    id: string, params: InvertParams, outPath: string,
    imageCrop: [number, number, number, number] | null = null,
    geom: { rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number } = {},
    dust: DustStroke[] = [],
    irRemoval: IrRemoval = { enabled: false, sensitivity: 50 },
  ) =>
    invoke<void>("export_image", {
      id, params, outPath, imageCrop,
      rot90: geom.rot90 ?? 0, flipH: geom.flip_h ?? false,
      flipV: geom.flip_v ?? false, angle: geom.angle ?? 0,
      dust: wireDust(dust), irRemoval,
    }),
```
NOTE: the Rust `export_image` param is named `ir_removal` (snake), but Tauri's JS↔Rust arg mapping converts camelCase `irRemoval` → snake `ir_removal` automatically (same as `outPath`→`out_path`, `flipH`→`flip_h`). So pass `irRemoval` here. Verify after wiring (Task 11) that export actually receives it; if Tauri does NOT auto-convert this nested key, pass `ir_removal: irRemoval` instead.

- [ ] **Step 5: Run tests + check** — `cd app && npm run test:unit` (all green) and `npm run check` (only the pre-existing `workflow.test.ts` error).

- [ ] **Step 6: Commit**
```bash
git add app/src/lib/develop/dust.ts app/src/lib/develop/dust.test.ts app/src/lib/api.ts
git commit -m "feat(app): irRemoval edit-state, reducers, has_ir + wire types"
```

---

## Task 9: EraserPanel live toggle + sensitivity, Develop wiring

**Files:** Modify `app/src/lib/develop/EraserPanel.svelte`, `app/src/lib/tabs/Develop.svelte`

- [ ] **Step 1: Update `EraserPanel.svelte`.** Replace the disabled IR `<span class="ir-wrap">…</span>` block and add a sensitivity slider. New script + markup:

Script — add props/dispatcher:
```svelte
<script lang="ts">
  import { createEventDispatcher } from "svelte";

  /** Brush radius normalized to image width (0.005..0.2). */
  export let brush: number;
  /** Whether the active image carries an IR plane. */
  export let hasIr = false;
  /** IR auto-removal state. */
  export let irEnabled = false;
  export let irSensitivity = 50;

  const dispatch = createEventDispatcher<{
    reset: void; irEnabled: boolean; irSensitivity: number;
  }>();
</script>
```
Markup — replace the existing `.ir-wrap` button block with:
```svelte
  {#if hasIr}
    <button class="ir on" class:active={irEnabled}
            on:click={() => dispatch("irEnabled", !irEnabled)}>
      Remove dust (IR) <span class="state">{irEnabled ? "On" : "Off"}</span>
    </button>
    {#if irEnabled}
      <div class="sub">Sensitivity</div>
      <div class="slrow">
        <input type="range" min="0" max="100" step="1" value={irSensitivity}
               on:input={(e) => dispatch("irSensitivity", +(e.target as HTMLInputElement).value)} />
        <span class="val">{Math.round(irSensitivity)}</span>
      </div>
    {/if}
  {:else}
    <span class="ir-wrap" title="Requires an infrared scan channel">
      <button class="ir" disabled>
        Remove dust (IR) <span class="soon">soon</span>
      </button>
    </span>
  {/if}
```
Add CSS for the active toggle (next to the existing `.ir` rule):
```css
  .ir.on { cursor: pointer; opacity: 1; }
  .ir.on.active { background: rgba(224,52,52,0.18); border-color: rgba(224,52,52,0.5); }
  .state { font-size: 10px; border: 1px solid var(--glass-brd); border-radius: 4px;
    padding: 0 5px; color: var(--text-dim); }
```
(Keep the existing brush-size slider, Reset button, hint, and the other styles.)

- [ ] **Step 2: Wire `Develop.svelte`.** Update imports from `../develop/dust` to add the IR reducers:
```ts
  import { addStroke, undoStroke, resetDust, emptyDust, setIrEnabled, setIrSensitivity, type DustStroke, type DustEdits } from "../develop/dust";
```
The `resetDust` signature changed (now takes the current edits) — update `resetDustEdits`:
```ts
  function resetDustEdits() { updateDust((d) => resetDust(d)); }
```
Add IR handlers (next to `updateDust`):
```ts
  function setIrOn(on: boolean) { updateDust((d) => setIrEnabled(d, on)); }
  function setIrSens(v: number) { updateDust((d) => setIrSensitivity(d, v)); }
```
Compute `hasIr` for the active image:
```ts
  $: hasIr = active?.has_ir ?? false;
```
(`active` already exists: `$: active = $images.find((i) => i.id === $activeId);`.)

Pass IR settings to the eraser `<Viewport>` and `EraserPanel`. The Viewport must send `ir_removal` to the backend — add it to the eraser Viewport props:
```svelte
        <Viewport id={$activeId} params={$params} imgW={effW} imgH={effH} imageCrop={imageCrop}
                  rot90={cRot} flipH={committed?.flipH ?? false} flipV={committed?.flipV ?? false} angle={committed?.angle ?? 0}
                  eraser={$tool === "eraser"} {brush} dust={dust.strokes} irRemoval={dust.irRemoval} {dustRev}
                  on:stroke={(e) => commitStroke(e.detail)} on:brush={(e) => (brush = e.detail)} />
```
And the panel branch:
```svelte
      {:else if $tool === "eraser"}
        <EraserPanel bind:brush {hasIr}
                     irEnabled={dust.irRemoval.enabled} irSensitivity={dust.irRemoval.sensitivity}
                     on:reset={resetDustEdits}
                     on:irEnabled={(e) => setIrOn(e.detail)}
                     on:irSensitivity={(e) => setIrSens(e.detail)} />
```
Forward `dust.irRemoval` to export:
```ts
      await api.exportImage($activeId, $params, out, imageCrop, {
        rot90: committed?.rot90 ?? 0, flip_h: committed?.flipH ?? false,
        flip_v: committed?.flipV ?? false, angle: committed?.angle ?? 0,
      }, dust.strokes, dust.irRemoval);
```

- [ ] **Step 3: Pass `ir_removal` through the Viewport to `renderView`.** In `app/src/lib/viewport/Viewport.svelte`:
Add the prop (next to the existing `dust`/`dustRev` props):
```ts
  import { screenRadius, type DustStroke } from "../develop/dust";
  import type { IrRemoval } from "../api";
  export let irRemoval: IrRemoval = { enabled: false, sensitivity: 50 };
```
Pass it in `render()`'s `renderView` view object (alongside `dust`):
```ts
        ..., angle, dust, ir_removal: irRemoval,
```
Add IR settings to `srcKey` so toggling/tuning re-renders:
```ts
  $: srcKey = `...|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}`;
```
(Append `|${irRemoval.enabled}|${irRemoval.sensitivity}` to the existing template string before the closing backtick.)

- [ ] **Step 4: Run checks + tests** — `cd app && npm run check` (only pre-existing error) and `npm run test:unit` (47 green) and backend `source "$HOME/.cargo/env" && cargo build --manifest-path app/src-tauri/Cargo.toml`.

- [ ] **Step 5: Commit**
```bash
git add app/src/lib/develop/EraserPanel.svelte app/src/lib/tabs/Develop.svelte app/src/lib/viewport/Viewport.svelte
git commit -m "feat(app): live IR removal toggle + sensitivity, wired to render/export"
```

---

## Task 10: CLI `--check-ir`

**Files:** Modify `crates/film-cli/src/main.rs`

- [ ] **Step 1: Add the flag.** In the `Cli` struct, add:
```rust
    /// Decode the input and report whether it carries an infrared plane, then exit.
    #[arg(long)]
    check_ir: bool,
```

- [ ] **Step 2: Handle it early in `main`,** right after the image is decoded (`let img = match ext ... .with_context(...)?;`):
```rust
    if cli.check_ir {
        match &img.ir {
            Some(ir) => println!(
                "{:?}: {}x{} RGB+IR (4-channel); ir samples = {}",
                cli.input, img.width, img.height, ir.len()
            ),
            None => println!(
                "{:?}: {}x{} RGB only — no infrared plane",
                cli.input, img.width, img.height
            ),
        }
        return Ok(());
    }
```
NOTE: `--output` is a required arg on the CLI; `--check-ir` still requires `-o` to be passed. To avoid forcing a dummy output path, make `output` optional ONLY enough to not break existing usage is out of scope — simplest: document that `--check-ir` needs a dummy `-o`, e.g. `film-cli scan.tiff -o /dev/null --check-ir`. (Keeping `output` required avoids touching the existing happy path.) If you prefer, change `output` to `Option<PathBuf>` and guard the later `write_tiff16` calls with an `.expect("output required")` when not in check mode — but that's optional polish; the dummy `-o` works.

- [ ] **Step 3: Verify it compiles and runs** — `source "$HOME/.cargo/env" && cargo build -p film-cli`. If a 4-channel test TIFF is available, `cargo run -p film-cli -- <file> -o /dev/null --check-ir` prints the IR status. (No unit test — it's a thin I/O command.)

- [ ] **Step 4: Commit**
```bash
git add crates/film-cli/src/main.rs
git commit -m "feat(cli): --check-ir reports whether a scan carries an infrared plane"
```

---

## Task 11: End-to-end verification + calibration

**Files:** none (verification + a possible one-line constant tweak in `dust.rs`).

- [ ] **Step 1: Full automated suite**
```bash
source "$HOME/.cargo/env" && cargo test -p film-core && cargo test --manifest-path app/src-tauri/Cargo.toml && cargo test -p film-cli
cd app && npm run test:unit && npm run check
```
Expected: film-core (Plan A 5 dust + Plan B 5 + geometry IR tests) green; src-tauri green; vitest green; svelte-check only the pre-existing `workflow.test.ts` error.

- [ ] **Step 2: Confirm a real IR file exists.** Run `film-cli --check-ir` on the user's SilverFast scan:
```bash
source "$HOME/.cargo/env" && cargo run -p film-cli -- <scan.tiff> -o /dev/null --check-ir
```
If it reports "RGB only — no infrared plane", STOP and tell the user: the scan must be re-exported from SilverFast as **RGBI / 16-bit 4-channel** TIFF; without it the feature cannot be exercised.

- [ ] **Step 3: GUI verification** (user-driven). `cd app && npm run tauri dev`, import the RGBI scan, Develop it:
  - In the Eraser panel, "Remove dust (IR)" is now **enabled** (not greyed). For a non-IR image it stays disabled with the tooltip.
  - Toggle it on → dust/scratches vanish in the preview. The **Sensitivity** slider live-updates detection (higher = more aggressive).
  - Manual brush strokes and the IR pass both apply and coexist.
  - Crop / rotate / straighten the image → IR detection still aligns (the IR plane follows the geometry).
  - Export a TIFF → defects healed at full resolution.

- [ ] **Step 4: Calibrate if needed.** If IR detection over/under-flags on the real scan, tune ONLY the constants in `ir_defect_mask` (`dust.rs`): the `0.95` percentile and/or the `t = 0.5 + 0.45*…` range. The synthetic unit tests assert the *mechanism* (low IR flagged, sensitivity widens), so reasonable constant tweaks keep them green — re-run `cargo test -p film-core dust::` after any change. Commit any tweak:
```bash
git add crates/film-core/src/dust.rs
git commit -m "tune(film-core): IR defect thresholds against real RGBI scan"
```

---

## Self-Review (completed)

- **Spec coverage:** Carry-IR-through-geometry (design §Architecture) — Tasks 1-3. Keep IR on working + `has_ir` (§State) — Task 4. Detection `ir_defect_mask` + reuse `inpaint_masked` (§Detection) — Task 5. Apply in render_view/export before finish, after manual strokes (§Pipeline order) — Tasks 6-7. `irRemoval` edit-state + reducers + `hasIr` gating (§State & UI) — Tasks 8-9. Live toggle + sensitivity, no re-develop (§UI) — Task 9 (`srcKey` + dustRev). CLI `--check-ir` (§Testing) — Task 10. Synthetic-IR + geometry tests (§Testing) — Tasks 1-5. Calibration caveat (§Detection) — Task 11.
- **Placeholders:** none — every step has full code/commands.
- **Type consistency:** `ir_defect_mask(w,h,ir,sensitivity)->Mask`, `apply_ir(img,ir,sensitivity)`, `IrRemoval{enabled,sensitivity}` (Rust DTO + TS), `ViewSpec.ir_removal`, `ImageEntry.has_ir`, `setIrEnabled`/`setIrSensitivity`, `DustEdits{strokes,irRemoval}`, reducers spread-preserve, EraserPanel props `hasIr`/`irEnabled`/`irSensitivity` + events `irEnabled`/`irSensitivity`, Viewport `irRemoval` prop. The `resetDust` signature change (now takes `DustEdits`) is flagged in Task 8 and its caller updated in Task 9.
- **Reuse:** `apply_ir` builds a full-frame `Mask` and calls Plan A's `inpaint_masked` — no new inpaint code.
- **Known risks carried from Plan A:** erasing/IR-detecting then changing geometry re-maps strokes (strokes only; the IR pass re-detects each render so it always re-aligns — IR is robust to geometry changes, unlike manual strokes).
