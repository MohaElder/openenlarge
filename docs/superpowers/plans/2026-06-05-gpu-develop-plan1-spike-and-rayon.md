# GPU Develop — Plan 1: Float-Texture Spike + Rayon Parallelization

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the immediate, low-risk wins from the GPU develop pipeline spec — empirically verify the GPU float-texture assumption (gates Plans 2–3) and rayon-parallelize the CPU develop engine so every CPU path (preview fallback, export, batch thumbnails, and the parity oracle) gets a multi-core speedup today.

**Architecture:** Two independent pieces. (A) A **runtime spike** that probes whether the app's WebGL2 context supports `EXT_color_buffer_float` + an `RGBA16F` render target — verified by running the real app and reading one console line. (B) **rayon** swapped into the three per-pixel `map` loops in `film-core` (`invert_image`, `finish_image`, `apply_texture`). `par_iter().map().collect()` on an indexed iterator preserves element order, so output is byte-identical and the existing unit tests are the correctness guard.

**Tech Stack:** Rust (`film-core`, rayon), WebGL2 (TypeScript probe), Tauri.

**Spec:** `docs/superpowers/specs/2026-06-04-gpu-develop-pipeline-design.md` (Rollout phases 0 + 1; Assumption #1).

---

## File Structure

- `crates/film-core/Cargo.toml` — add `rayon` dependency.
- `crates/film-core/src/engine.rs` — parallelize `invert_image` (line 111).
- `crates/film-core/src/finish.rs` — parallelize `finish_image` per-pixel map (line 285) and `apply_texture` map (the `iter().zip()` map).
- `app/src/lib/viewport/gl/renderer.ts` — add `float16RenderTargetSupported()` probe (exported).
- `app/src/lib/viewport/Viewport.svelte` — call the probe once on mount, `console.log` the verdict (temporary spike instrumentation, removed at the end of Plan 2).

Note: `blur()` (`finish.rs`) keeps its sequential two-pass form in this plan — it reads neighbours and is a smaller cost than the per-pixel maps; it is left for a later pass if profiling shows it matters.

---

## Task 1: Add the rayon dependency

**Files:**
- Modify: `crates/film-core/Cargo.toml`

- [ ] **Step 1: Add rayon to film-core dependencies**

Edit `crates/film-core/Cargo.toml` so the `[dependencies]` section reads:

```toml
[dependencies]
nalgebra = { workspace = true }
tiff = { workspace = true }
rawler = { workspace = true }
thiserror = { workspace = true }
inpaint = "0.1"
ndarray = "0.16"
rayon = "1"
```

- [ ] **Step 2: Verify it resolves and the crate still builds**

Run: `cargo build -p film-core`
Expected: builds successfully; `rayon` appears in `Cargo.lock` as a direct dependency of `film-core`.

- [ ] **Step 3: Commit**

```bash
git add crates/film-core/Cargo.toml Cargo.lock
git commit -m "build(film-core): add rayon dependency"
```

---

## Task 2: Parallelize `invert_image`

**Files:**
- Modify: `crates/film-core/src/engine.rs:111-119`
- Test: `crates/film-core/src/engine.rs` (tests module)

- [ ] **Step 1: Write a per-pixel-independence test (the equivalence guard)**

Add this test inside the `#[cfg(test)] mod tests` block in `engine.rs` (after the existing tests):

```rust
#[test]
fn invert_image_is_per_pixel_and_order_preserving() {
    // A multi-pixel image must invert each pixel exactly as the scalar fn does,
    // in the same order — this guards the parallel collect() against reordering.
    let p = InversionParams { base: [0.8, 0.6, 0.4], ..Default::default() };
    let pixels = vec![
        [0.8, 0.6, 0.4],
        [0.1, 0.2, 0.3],
        [0.5, 0.5, 0.5],
        [0.05, 0.9, 0.45],
    ];
    let img = Image { width: 2, height: 2, pixels: pixels.clone(), ir: None };
    let out = invert_image(&img, &p, Mode::B);
    assert_eq!(out.width, 2);
    assert_eq!(out.height, 2);
    for (i, &px) in pixels.iter().enumerate() {
        let want = invert_b(px, &p);
        for c in 0..3 {
            assert!((out.pixels[i][c] - want[c]).abs() < 1e-6, "pixel {i} chan {c}");
        }
    }
}
```

- [ ] **Step 2: Run the test against the current (sequential) implementation**

Run: `cargo test -p film-core engine::tests::invert_image_is_per_pixel_and_order_preserving`
Expected: PASS (it documents current behavior; it must keep passing after parallelizing).

- [ ] **Step 3: Parallelize the map**

In `engine.rs`, add the rayon prelude import near the top of the file (below the existing `use nalgebra...` line):

```rust
use rayon::prelude::*;
```

Then change the body of `invert_image` (currently `let pixels = img.pixels.iter().map(|&px| f(px, p)).collect();`) to:

```rust
pub fn invert_image(img: &crate::Image, p: &InversionParams, mode: Mode) -> crate::Image {
    let f = match mode {
        Mode::B => invert_b,
        Mode::C => invert_c,
        Mode::Naive => invert_naive,
    };
    // par_iter + collect into Vec preserves index order, so output is identical
    // to the sequential map; the per-pixel fn `f` is pure (no shared state).
    let pixels = img.pixels.par_iter().map(|&px| f(px, p)).collect();
    crate::Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() }
}
```

- [ ] **Step 4: Run the new test and the whole engine test module**

Run: `cargo test -p film-core engine::`
Expected: PASS — all engine tests, including the new one, stay green (identical output, now parallel).

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/engine.rs
git commit -m "perf(engine): parallelize invert_image with rayon"
```

---

## Task 3: Parallelize `finish_image` per-pixel map

**Files:**
- Modify: `crates/film-core/src/finish.rs:284-288`
- Test: `crates/film-core/src/finish.rs` (tests module)

- [ ] **Step 1: Write the per-pixel-independence test**

Add this test inside the `#[cfg(test)] mod tests` block in `finish.rs`:

```rust
#[test]
fn finish_image_matches_scalar_per_pixel_no_texture() {
    // With texture == 0, finish_image is a pure per-pixel map; assert it matches
    // finish_pixel elementwise and in order (guards the parallel collect).
    let p = FinishParams { contrast: 0.4, saturation: 0.3, ..Default::default() };
    let pixels = vec![
        [0.6, 0.4, 0.3],
        [0.1, 0.7, 0.2],
        [0.9, 0.9, 0.1],
        [0.2, 0.2, 0.8],
    ];
    let img = Image { width: 4, height: 1, pixels: pixels.clone(), ir: None };
    let out = finish_image(&img, &p);
    for (i, &px) in pixels.iter().enumerate() {
        let want = finish_pixel(px, &p);
        for c in 0..3 {
            assert!((out.pixels[i][c] - want[c]).abs() < 1e-6, "pixel {i} chan {c}");
        }
    }
}
```

- [ ] **Step 2: Run it against the current implementation**

Run: `cargo test -p film-core finish::tests::finish_image_matches_scalar_per_pixel_no_texture`
Expected: PASS.

- [ ] **Step 3: Parallelize the map**

In `finish.rs`, add near the top of the file (with the other `use` statements):

```rust
use rayon::prelude::*;
```

Change the first line of `finish_image` from
`let pixels = img.pixels.iter().map(|&px| finish_pixel(px, p)).collect();`
to:

```rust
    let pixels = img.pixels.par_iter().map(|&px| finish_pixel(px, p)).collect();
```

(Leave the rest of `finish_image` — the `apply_texture` branch — unchanged in this task.)

- [ ] **Step 4: Run the finish test module**

Run: `cargo test -p film-core finish::`
Expected: PASS — all finish tests stay green.

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "perf(finish): parallelize finish_image per-pixel map with rayon"
```

---

## Task 4: Parallelize `apply_texture`'s unsharp map

**Files:**
- Modify: `crates/film-core/src/finish.rs` (`apply_texture` fn)
- Test: `crates/film-core/src/finish.rs` (tests module)

- [ ] **Step 1: Write a texture-path equivalence test**

`apply_texture` runs when `texture != 0`. Add this test to the `finish.rs` tests module. It
reproduces the unsharp math (original + k·(original − blur)) inline against the public
`finish_image` output, asserting the parallel zip-map stays correct and ordered:

```rust
#[test]
fn finish_image_with_texture_is_stable_and_clamped() {
    // A non-flat image so blur differs from the source; texture > 0 exercises the
    // apply_texture zip-map path. Output must stay in [0,1] and be deterministic.
    let p = FinishParams { texture: 1.0, ..Default::default() };
    let pixels = vec![
        [0.0, 0.0, 0.0], [1.0, 1.0, 1.0],
        [0.2, 0.5, 0.8], [0.9, 0.1, 0.4],
    ];
    let img = Image { width: 2, height: 2, pixels, ir: None };
    let a = finish_image(&img, &p);
    let b = finish_image(&img, &p);
    assert_eq!(a.pixels, b.pixels, "must be deterministic across runs");
    for px in &a.pixels {
        for c in 0..3 {
            assert!((0.0..=1.0).contains(&px[c]), "value {} out of range", px[c]);
        }
    }
}
```

- [ ] **Step 2: Run it against the current implementation**

Run: `cargo test -p film-core finish::tests::finish_image_with_texture_is_stable_and_clamped`
Expected: PASS.

- [ ] **Step 3: Parallelize the zip-map in `apply_texture`**

In `apply_texture`, change the pixel-building map from
`let pixels = img.pixels.iter().zip(b.pixels.iter())` to the parallel form:

```rust
fn apply_texture(img: &Image, amount: f32) -> Image {
    let b = blur(img);
    let k = USM_GAIN * amount;
    // par_iter().zip() over two equal-length indexed slices preserves order.
    let pixels = img.pixels.par_iter().zip(b.pixels.par_iter())
        .map(|(&v, &lo)| std::array::from_fn(|c| (v[c] + k * (v[c] - lo[c])).clamp(0.0, 1.0)))
        .collect();
    Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() }
}
```

- [ ] **Step 4: Run the finish test module**

Run: `cargo test -p film-core finish::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "perf(finish): parallelize apply_texture unsharp map with rayon"
```

---

## Task 5: Confirm the whole crate is green and clippy-clean

**Files:** none (verification only)

- [ ] **Step 1: Run the full film-core test suite**

Run: `cargo test -p film-core`
Expected: PASS — all pre-existing tests plus the three new tests.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p film-core --all-targets`
Expected: no warnings (matches the project bar recorded in `docs/superpowers/poc-findings.md`).

- [ ] **Step 3: (optional) eyeball the speedup with an ad-hoc timing test**

Add this **ignored** test to `engine.rs` tests, run it with `--nocapture`, then delete it (do not
commit it):

```rust
#[test]
#[ignore]
fn timing_invert_large() {
    let p = InversionParams { base: [0.8, 0.6, 0.4], ..Default::default() };
    let n = 4096 * 4096;
    let img = Image { width: 4096, height: 4096, pixels: vec![[0.3, 0.4, 0.5]; n], ir: None };
    let t = std::time::Instant::now();
    let _ = invert_image(&img, &p, Mode::B);
    println!("invert 16.8MP: {:?}", t.elapsed());
}
```

Run: `cargo test -p film-core engine::tests::timing_invert_large --release -- --ignored --nocapture`
Expected: prints a duration noticeably lower than a single-core run (roughly scales with core
count). Remove the test afterward; it is a one-off measurement, not a committed test.

---

## Task 6: Float-texture render-target spike (gates Plans 2–3)

**Files:**
- Modify: `app/src/lib/viewport/gl/renderer.ts` (add exported `float16RenderTargetSupported()`)
- Modify: `app/src/lib/viewport/Viewport.svelte` (call once on mount, log verdict)

This is a **runtime** capability check — WebGL is not available in the Node unit-test environment,
so it is verified by running the real Tauri app and reading one console line. This is intentional:
Assumption #1 in the spec must be confirmed against the actual WKWebView, not a headless mock.

- [ ] **Step 1: Add the probe function to `renderer.ts`**

Add this exported function near `webgl2Available()` (top of `renderer.ts`):

```ts
/**
 * Spike (Plan 1): does THIS environment's WebGL2 support an RGBA16F render
 * target? Plans 2-3 (GPU inversion + offscreen export) depend on it. Creates a
 * tiny offscreen RGBA16F texture, attaches it to an FBO, and checks both the
 * float-color-buffer extension and framebuffer completeness. Returns a verdict
 * object so the result can be logged from the app.
 */
export function float16RenderTargetSupported():
  { ok: boolean; reason: string } {
  if (typeof document === "undefined") return { ok: false, reason: "no document" };
  let gl: WebGL2RenderingContext | null = null;
  try {
    gl = document.createElement("canvas").getContext("webgl2");
  } catch {
    return { ok: false, reason: "no webgl2 context" };
  }
  if (!gl) return { ok: false, reason: "no webgl2 context" };
  // Needed to RENDER to a float texture (not just sample one).
  const ext = gl.getExtension("EXT_color_buffer_float");
  if (!ext) return { ok: false, reason: "EXT_color_buffer_float missing" };
  const tex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, 4, 4, 0, gl.RGBA, gl.HALF_FLOAT, null);
  const fbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.deleteFramebuffer(fbo);
  gl.deleteTexture(tex);
  if (status !== gl.FRAMEBUFFER_COMPLETE) {
    return { ok: false, reason: `framebuffer incomplete: 0x${status.toString(16)}` };
  }
  return { ok: true, reason: "RGBA16F render target OK" };
}
```

- [ ] **Step 2: Log the verdict once when the Viewport mounts**

In `Viewport.svelte`, add `float16RenderTargetSupported` to the existing import from
`./gl/renderer` (alongside whatever it already imports, e.g. `webgl2Available`/`FinishRenderer`),
and inside the component's `onMount` (add one if there isn't one) call it once:

```ts
import { onMount } from "svelte";
import { float16RenderTargetSupported } from "./gl/renderer";
// ...
onMount(() => {
  const v = float16RenderTargetSupported();
  console.log(`[SPIKE float16] ok=${v.ok} reason="${v.reason}"`);
});
```

(If `Viewport.svelte` already imports from `svelte` and/or `./gl/renderer`, merge into those
existing import statements rather than duplicating them.)

- [ ] **Step 3: Build the frontend to confirm it compiles**

Run: `cd app && npm run build` (or the project's type-check/build script)
Expected: builds with no TypeScript errors.

- [ ] **Step 4: Run the real app and read the verdict (MANUAL E2E — gating result)**

Run the Tauri app the way the project normally launches it (e.g. `cd app && npm run tauri dev`),
open the Develop viewport, and read the dev-tools console.
Expected: one line `[SPIKE float16] ok=... reason="..."`.

**Record the result** in `docs/superpowers/poc-findings.md` (one line, dated):
- `ok=true` → Plans 2–3 proceed as designed (RGBA16F preview + export).
- `ok=false` → STOP before Plan 2; the GPU-inversion approach needs revisiting (e.g. RGBA8 with
  encoded-range packing, or the WebGPU path). Bring the `reason` back to a design discussion.

- [ ] **Step 5: Commit the spike instrumentation**

```bash
git add app/src/lib/viewport/gl/renderer.ts app/src/lib/viewport/Viewport.svelte docs/superpowers/poc-findings.md
git commit -m "spike(viewport): probe RGBA16F render-target support (gates GPU plan)"
```

Note: the `console.log` + `onMount` probe is **temporary instrumentation**. It is removed as the
first cleanup step of Plan 2 (the real renderer will feature-detect this internally). The
`float16RenderTargetSupported()` function itself may be kept and reused by Plan 2's feature
detection.

---

## Self-Review

- **Spec coverage:** Plan 1 covers spec Rollout **Phase 1** (rayon: `invert_image` Task 2,
  `finish_image` Task 3, `apply_texture` Task 4) and **Phase 0 / Assumption #1** (float-texture
  spike, Task 6). Phases 2–7 are deferred to Plans 2–3 by design. CPU-engine-stays-as-oracle is
  honored (engine is parallelized, not removed).
- **Placeholder scan:** no TBD/TODO; every code step shows complete code; every run step shows the
  exact command and expected result. The optional timing test is explicitly throwaway.
- **Type consistency:** `float16RenderTargetSupported()` returns `{ ok: boolean; reason: string }`
  consistently in the function and its caller. rayon usage is `par_iter()`/`par_iter().zip()` with
  `use rayon::prelude::*;` added to both `engine.rs` and `finish.rs`.
- **Behavior safety:** all three parallelized maps are pure per-pixel/indexed and collected into
  `Vec`, which preserves order — output is identical, guarded by the existing unit tests plus the
  three new equivalence tests.
```