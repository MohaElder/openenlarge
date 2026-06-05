# GPU Develop — Plan 2: Float Upload + Shader Inversion + GPU Geometry (Phases 2–4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upload each decoded working image to the GPU **once** as an `RGBA16F` float texture, move film **inversion** into a WebGL2 shader pass driven by uniforms, and let zoom/pan/exposure/temp/tint/mode/stock change with **no backend round-trip** — eliminating the per-frame base64-JPEG path for the common (no-dust) case.

**Architecture:** Two GPU passes — **INVERT** (samples the raw negative float texture, applies Mode B/C/Naive + tone using resolved uniforms, writes an intermediate `RGBA16F` FBO) then the existing **FINISH** pass (reads that FBO, applies tone/sat/grade/texture to the canvas). The Rust backend ships (a) the raw linear working buffer once as half-float bytes via `tauri::ipc::Response`, and (b) the already-existing `resolve_params` output as JSON uniforms. Geometry (orient/flip/straighten/persistent-crop) becomes a UV transform in the INVERT pass; zoom/pan stay pure CSS (the viewport already renders the whole image and positions via CSS). Dust/IR active, or no-WebGL2 → fall back to the existing CPU `render_view` JPEG path (unchanged), so there is **no feature-parity gap**.

**Tech Stack:** Rust (Tauri 2, `half` crate, film-core), WebGL2 (GLSL 300es, RGBA16F, FBO), TypeScript, Svelte, Vitest.

**Spec:** `docs/superpowers/specs/2026-06-04-gpu-develop-pipeline-design.md` (Rollout Phases 2, 3, 4). Spike verdict `ok=true` recorded in `poc-findings.md`.

---

## Context the implementer needs

- The working image is `film_core::Image { width, height, pixels: Vec<[f32;3]> /* linear */, ir: Option<Vec<f32>> }` (`crates/film-core/src/image.rs:7`). It is the **raw negative**, pre-inversion.
- `Developed { working, thumb, base }` lives in the session (`app/src-tauri/src/session.rs:112`).
- `resolve_params(&InvertParams, &thumb, base) -> film_core::InversionParams` (`commands.rs:135`) computes the stock matrix (`m_post`), white-balance gains (from Kelvin/tint), exposure `2^ev`, etc. `InversionParams { base:[f32;3], m_pre:Matrix3, m_post:Matrix3, exposure, black, gamma, wb:[f32;3] }` (`engine.rs:12`).
- `mode_from(&str) -> Mode` (`commands.rs:119`): `"c"→C`, else `B`. (The UI only sends `"b"`/`"c"`; Naive is internal but the shader will support all three.)
- The inversion math to port is `invert_b`/`invert_c`/`invert_naive` + `tone` (`engine.rs:47–98`). `tone(v,gain,p) = max(v*exposure*gain - black, 0).powf(gamma)`. `log10(x) = log2(x)*0.30102999566`. `EPS=1e-5`.
- The viewport (`Viewport.svelte`) **already renders the whole image**: `render()` (line ~104) passes `crop:[0,0,imgW,imgH]`, sizes the canvas to `out_w×out_h`, and the element is positioned/scaled by CSS (`dispW=imgW*eff`, `left`, `top`). So "upload once + CSS zoom" fits the existing display model; only the *resolution* changes today.
- `FinishRenderer` (`renderer.ts`) currently uploads an 8-bit `<img>` as the source and runs one finishing program. We extend it with a float source, an invert program, and an intermediate FBO.
- Tauri is v2 (`app/src-tauri/Cargo.toml`). A command returning `tauri::ipc::Response` sends raw bytes; on the JS side `invoke()` resolves to an `ArrayBuffer`.

## File Structure

```
app/src-tauri/
├── Cargo.toml                     ADD: half = "2"
└── src/
    ├── gpu_upload.rs   (NEW)      pack_rgba16f(&Image,cap)->(u32,u32,Vec<u8>); ResolvedInversion + resolve_to_uniforms
    ├── commands.rs                ADD commands: working_info, working_pixels, resolved_inversion
    └── lib.rs                     register the 3 new commands in the invoke_handler

app/src/lib/viewport/gl/
├── invert.ts          (NEW)      InversionUniforms type + toInversionUniforms(json) (mat3 flatten, mode int)
├── invert.test.ts     (NEW)      vitest for the mapping
├── shaders.ts                    ADD INVERT_FRAG (+ geometry UV); keep VERT/FRAG
└── renderer.ts                   ADD invert program, RGBA16F source upload, intermediate FBO, geometry uniforms, 2-pass draw

app/src/lib/
└── api.ts                        ADD workingInfo, workingPixels (ArrayBuffer), resolvedInversion

app/src/lib/viewport/
└── Viewport.svelte               upload-once on id/quality; resolved-inversion + geometry as uniforms; collapse srcKey;
                                  CPU fallback when dust/IR active or no GL
```

---

## Task 1: Rust — half-float packing helper

**Files:**
- Modify: `app/src-tauri/Cargo.toml`
- Create: `app/src-tauri/src/gpu_upload.rs`
- Modify: `app/src-tauri/src/lib.rs` (add `mod gpu_upload;`)

- [ ] **Step 1: Add the `half` dependency**

In `app/src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
half = "2"
```

Run: `cargo build -p app` *(if the tauri crate has a different package name, use the name from `app/src-tauri/Cargo.toml`'s `[package] name`; build to confirm `half` resolves).*

- [ ] **Step 2: Write the failing test for `pack_rgba16f`**

Create `app/src-tauri/src/gpu_upload.rs` with ONLY the test first:

```rust
//! Pack a linear-RGB working image into half-float RGBA bytes for a one-shot
//! WebGL2 `RGBA16F` texture upload, and resolve inversion params into a flat,
//! serialisable uniform set the GPU shader consumes.

#[cfg(test)]
mod tests {
    use super::*;
    use film_core::Image;
    use half::f16;

    #[test]
    fn pack_rgba16f_one_pixel_round_trips_with_alpha_one() {
        let img = Image { width: 1, height: 1, pixels: vec![[0.25, 0.5, 0.75]], ir: None };
        let (w, h, bytes) = pack_rgba16f(&img, 8192);
        assert_eq!((w, h), (1, 1));
        assert_eq!(bytes.len(), 1 * 1 * 4 * 2, "RGBA, 2 bytes per channel");
        // Decode the 4 channels back from little-endian u16 half-floats.
        let chan = |i: usize| f16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]).to_f32();
        assert!((chan(0) - 0.25).abs() < 1e-3, "r");
        assert!((chan(1) - 0.50).abs() < 1e-3, "g");
        assert!((chan(2) - 0.75).abs() < 1e-3, "b");
        assert!((chan(3) - 1.0).abs() < 1e-3, "a defaults to 1.0");
    }

    #[test]
    fn pack_rgba16f_caps_long_edge() {
        // 10x4 image, cap 5 → downscaled so long edge <= 5, bytes match the capped dims.
        let img = Image { width: 10, height: 4, pixels: vec![[0.1, 0.2, 0.3]; 40], ir: None };
        let (w, h, bytes) = pack_rgba16f(&img, 5);
        assert!(w <= 5 && h <= 5, "long edge capped: {w}x{h}");
        assert_eq!(bytes.len(), (w * h * 4 * 2) as usize);
    }
}
```

- [ ] **Step 3: Run it to confirm it fails (no `pack_rgba16f` yet)**

Run: `cargo test -p film-core 2>/dev/null; cargo test --manifest-path app/src-tauri/Cargo.toml gpu_upload::tests::pack_rgba16f_one_pixel -- --nocapture`
Expected: FAIL to compile — `pack_rgba16f` not found.

- [ ] **Step 4: Implement `pack_rgba16f`**

Add above the `#[cfg(test)]` block in `gpu_upload.rs`:

```rust
use crate::convert::proxy;
use film_core::Image;
use half::f16;

/// Max GPU texture long-edge we will upload. WebGL2 guarantees at least 2048,
/// real GPUs >= 16384; 8192 is a safe, ample bound for the live proxy.
pub const MAX_GPU_EDGE: u32 = 8192;

/// Downscale (if needed) so the long edge <= `cap`, then pack the linear-RGB
/// pixels as little-endian half-float RGBA (alpha = 1.0). Returns the (possibly
/// reduced) dimensions and the byte buffer ready for `texImage2D(RGBA16F)`.
pub fn pack_rgba16f(img: &Image, cap: u32) -> (u32, u32, Vec<u8>) {
    let capped = proxy(img, cap); // no-op if already within cap
    let one = f16::from_f32(1.0).to_le_bytes();
    let mut bytes = Vec::with_capacity(capped.pixels.len() * 8);
    for px in &capped.pixels {
        bytes.extend_from_slice(&f16::from_f32(px[0]).to_le_bytes());
        bytes.extend_from_slice(&f16::from_f32(px[1]).to_le_bytes());
        bytes.extend_from_slice(&f16::from_f32(px[2]).to_le_bytes());
        bytes.extend_from_slice(&one);
    }
    (capped.width as u32, capped.height as u32, bytes)
}
```

Add `mod gpu_upload;` to `app/src-tauri/src/lib.rs` near the other `mod` declarations.

- [ ] **Step 5: Run the tests to confirm they pass**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml gpu_upload::tests`
Expected: PASS (both tests).

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/src/gpu_upload.rs app/src-tauri/src/lib.rs
git commit -m "feat(gpu): pack working image to half-float RGBA for one-shot upload"
```

---

## Task 2: Rust — resolved-inversion uniform struct + helper

**Files:**
- Modify: `app/src-tauri/src/gpu_upload.rs`

- [ ] **Step 1: Write the failing test for `resolve_to_uniforms`**

Add to the `#[cfg(test)] mod tests` block in `gpu_upload.rs`:

```rust
    use crate::commands_test_support::sample_invert_params; // see Step 3

    #[test]
    fn uniforms_none_stock_mode_c_is_identity_matrices_mode_1() {
        let mut p = sample_invert_params();
        p.stock = "none".into();
        p.mode = "c".into();
        p.exposure = 1.0; // 1 EV → 2.0x
        let u = resolve_to_uniforms(&p, [0.8, 0.6, 0.4]);
        assert_eq!(u.mode, 1, "c → 1");
        assert_eq!(u.base, [0.8, 0.6, 0.4]);
        // identity m_pre/m_post (column-major 9-vec)
        assert_eq!(u.m_pre, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(u.m_post, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert!((u.exposure - 2.0).abs() < 1e-5, "2^1");
    }

    #[test]
    fn uniforms_portra_mode_b_fits_nonidentity_mpost_mode_0() {
        let mut p = sample_invert_params();
        p.stock = "portra400".into();
        p.mode = "b".into();
        let u = resolve_to_uniforms(&p, [0.8, 0.6, 0.4]);
        assert_eq!(u.mode, 0, "b → 0");
        // m_post from fit_m_post is NOT identity for a real stock
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_ne!(u.m_post, identity, "stock fit produces a real matrix");
    }
```

- [ ] **Step 2: Run it to confirm failure**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml gpu_upload::tests::uniforms_`
Expected: FAIL to compile — `resolve_to_uniforms`, `ResolvedInversion`, and `sample_invert_params` not found.

- [ ] **Step 3: Provide a test-support constructor for `InvertParams`**

`InvertParams` has many fields; tests need a baseline. Add a tiny shared helper. In `app/src-tauri/src/lib.rs`, add (near other `mod`s):

```rust
#[cfg(test)]
pub mod commands_test_support {
    use crate::session::InvertParams;
    /// A neutral InvertParams for tests (mirrors default_invert_params()).
    pub fn sample_invert_params() -> InvertParams {
        InvertParams {
            mode: "b".into(), stock: "none".into(), base_rect: None,
            exposure: 0.0, black: 0.0, gamma: 0.4545, auto_wb: true,
            temp: 5500.0, tint: 0.0,
            contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
            texture: 0.0, vibrance: 0.0, saturation: 0.0,
            tc_highlights: 0.0, tc_lights: 0.0, tc_darks: 0.0, tc_shadows: 0.0,
            tc_curve: crate::session::identity_curve(),
            tc_red: crate::session::identity_curve(),
            tc_green: crate::session::identity_curve(),
            tc_blue: crate::session::identity_curve(),
            cg_sh_hue: 0.0, cg_sh_sat: 0.0, cg_sh_lum: 0.0,
            cg_mid_hue: 0.0, cg_mid_sat: 0.0, cg_mid_lum: 0.0,
            cg_hi_hue: 0.0, cg_hi_sat: 0.0, cg_hi_lum: 0.0,
            cg_glob_hue: 0.0, cg_glob_sat: 0.0, cg_glob_lum: 0.0,
            cg_blending: 50.0, cg_balance: 0.0,
        }
    }
}
```

- [ ] **Step 4: Implement `ResolvedInversion` + `resolve_to_uniforms`**

`build_params`/`wb_from_params` are private in `commands.rs`. Make them `pub(crate)` (change `fn build_params` → `pub(crate) fn build_params`, and `fn wb_from_params` → `pub(crate) fn wb_from_params`, and `fn mode_from` → `pub(crate) fn mode_from` in `commands.rs`). Then add to `gpu_upload.rs` (above the tests):

```rust
use crate::commands::{build_params, mode_from, wb_from_params};
use crate::session::InvertParams;
use film_core::Mode;
use serde::Serialize;

/// Flat, JS-friendly inversion uniforms. Matrices are column-major 9-vecs to
/// match GLSL `mat3` constructor/`uniformMatrix3fv` layout.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedInversion {
    pub base: [f32; 3],
    pub wb: [f32; 3],
    pub m_pre: [f32; 9],
    pub m_post: [f32; 9],
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    /// 0 = Mode B (density matrix), 1 = Mode C (per-channel), 2 = Naive.
    pub mode: u8,
}

fn mat3_col_major(m: &nalgebra::Matrix3<f32>) -> [f32; 9] {
    // nalgebra stores column-major; as_slice() is already column-major.
    let s = m.as_slice();
    [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8]]
}

/// Resolve the UI params (+ sampled film base) into GPU uniforms, reusing the
/// exact same param construction the CPU path uses (build_params + wb).
pub fn resolve_to_uniforms(p: &InvertParams, base: [f32; 3]) -> ResolvedInversion {
    let mut ip = build_params(p, base);
    ip.wb = wb_from_params(p.temp, p.tint);
    let mode = match mode_from(&p.mode) {
        Mode::B => 0u8,
        Mode::C => 1,
        Mode::Naive => 2,
    };
    ResolvedInversion {
        base: ip.base,
        wb: ip.wb,
        m_pre: mat3_col_major(&ip.m_pre),
        m_post: mat3_col_major(&ip.m_post),
        exposure: ip.exposure,
        black: ip.black,
        gamma: ip.gamma,
        mode,
    }
}
```

Add `use crate::commands_test_support::sample_invert_params;` at the top of the `#[cfg(test)] mod tests` (it already references it). Ensure `film_core` re-exports `Mode` (it does: `film_core::Mode`, used in `commands.rs`).

- [ ] **Step 5: Run the tests**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml gpu_upload::tests`
Expected: PASS (all four gpu_upload tests).

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/gpu_upload.rs app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(gpu): resolve inversion params to flat GPU uniforms"
```

---

## Task 3: Rust — the three Tauri commands

**Files:**
- Modify: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs` (register in `invoke_handler`)

- [ ] **Step 1: Add the commands to `commands.rs`**

Add near the other commands. `WorkingInfo` carries the capped GPU texture dims:

```rust
use crate::gpu_upload::{pack_rgba16f, resolve_to_uniforms, ResolvedInversion, MAX_GPU_EDGE};

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkingInfo {
    /// Capped dimensions of the float texture working_pixels will return.
    pub w: u32,
    pub h: u32,
}

/// Dimensions of the GPU float texture for this image (after the MAX_GPU_EDGE cap).
#[tauri::command]
pub fn working_info(id: String, session: State<Session>) -> Result<WorkingInfo, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (w, h, _) = pack_rgba16f(&dev.working, MAX_GPU_EDGE);
    Ok(WorkingInfo { w, h })
}

/// Raw half-float RGBA bytes of the linear working image (pre-inversion), for a
/// one-shot WebGL2 RGBA16F upload. Returned as raw IPC bytes (no base64/JPEG).
#[tauri::command]
pub fn working_pixels(id: String, session: State<Session>) -> Result<tauri::ipc::Response, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (_, _, bytes) = pack_rgba16f(&dev.working, MAX_GPU_EDGE);
    Ok(tauri::ipc::Response::new(bytes))
}

/// Resolve inversion params (+ this image's sampled base) into GPU uniforms.
#[tauri::command]
pub fn resolved_inversion(
    id: String, params: InvertParams, session: State<Session>,
) -> Result<ResolvedInversion, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    Ok(resolve_to_uniforms(&params, dev.base))
}
```

*(Note: if the codebase has an "ensure resident / lazy cache load" helper used by `render_view`, mirror that here so these work on a cache-restored image. Check how `render_view` obtains `dev` — if it calls a resident-loader before `.developed`, call the same one in all three commands. If `render_view` simply does `img.developed.as_ref().ok_or(...)`, the code above already matches.)*

- [ ] **Step 2: Register the commands**

In `app/src-tauri/src/lib.rs`, find `tauri::generate_handler![ ... ]` and add `working_info, working_pixels, resolved_inversion` to the list (using the same `commands::` path prefix as the existing entries).

- [ ] **Step 3: Build to confirm it compiles**

Run: `cargo build --manifest-path app/src-tauri/Cargo.toml`
Expected: builds clean. (No unit test here — command wiring is verified by the frontend integration + manual E2E in Task 8/9; the logic inside was unit-tested in Tasks 1–2.)

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(gpu): working_info/working_pixels/resolved_inversion commands"
```

---

## Task 4: TS — inversion uniform mapping (pure, unit-tested)

**Files:**
- Create: `app/src/lib/viewport/gl/invert.ts`
- Create: `app/src/lib/viewport/gl/invert.test.ts`

- [ ] **Step 1: Write the failing vitest**

Create `app/src/lib/viewport/gl/invert.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { toInversionUniforms, type ResolvedInversion } from "./invert";

const RES: ResolvedInversion = {
  base: [0.8, 0.6, 0.4],
  wb: [1.1, 1.0, 0.9],
  m_pre: [1, 0, 0, 0, 1, 0, 0, 0, 1],
  m_post: [2, 0, 0, 0, 1, 0, 0, 0, 1],
  exposure: 2.0,
  black: 0.05,
  gamma: 0.4545,
  mode: 0,
};

describe("toInversionUniforms", () => {
  it("passes scalars through and builds Float32Array mat3s", () => {
    const u = toInversionUniforms(RES);
    expect(u.exposure).toBe(2.0);
    expect(u.black).toBeCloseTo(0.05);
    expect(u.gamma).toBeCloseTo(0.4545);
    expect(u.mode).toBe(0);
    expect(Array.from(u.base)).toEqual([0.8, 0.6, 0.4]);
    expect(Array.from(u.wb)).toEqual([1.1, 1.0, 0.9]);
    expect(u.m_post).toBeInstanceOf(Float32Array);
    expect(u.m_post.length).toBe(9);
    expect(Array.from(u.m_post)).toEqual([2, 0, 0, 0, 1, 0, 0, 0, 1]);
  });
});
```

- [ ] **Step 2: Run it to confirm failure**

Run: `cd app && npx vitest run src/lib/viewport/gl/invert.test.ts`
Expected: FAIL — module `./invert` not found.

- [ ] **Step 3: Implement `invert.ts`**

Create `app/src/lib/viewport/gl/invert.ts`:

```ts
/** Mirrors the Rust `ResolvedInversion` JSON from the `resolved_inversion` command. */
export interface ResolvedInversion {
  base: [number, number, number];
  wb: [number, number, number];
  m_pre: number[];   // column-major 9
  m_post: number[];  // column-major 9
  exposure: number;
  black: number;
  gamma: number;
  mode: number;      // 0=B, 1=C, 2=Naive
}

/** GL-ready uniform buffers for the INVERT pass. */
export interface InversionUniforms {
  base: Float32Array;   // 3
  wb: Float32Array;     // 3
  m_pre: Float32Array;  // 9 (column-major, for uniformMatrix3fv)
  m_post: Float32Array; // 9
  exposure: number;
  black: number;
  gamma: number;
  mode: number;
}

export function toInversionUniforms(r: ResolvedInversion): InversionUniforms {
  return {
    base: new Float32Array(r.base),
    wb: new Float32Array(r.wb),
    m_pre: new Float32Array(r.m_pre),
    m_post: new Float32Array(r.m_post),
    exposure: r.exposure,
    black: r.black,
    gamma: r.gamma,
    mode: r.mode,
  };
}
```

- [ ] **Step 4: Run the test**

Run: `cd app && npx vitest run src/lib/viewport/gl/invert.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/viewport/gl/invert.ts app/src/lib/viewport/gl/invert.test.ts
git commit -m "feat(gpu): inversion uniform mapping (resolved JSON -> GL buffers)"
```

---

## Task 5: TS — the INVERT shader (port engine.rs + geometry UV)

**Files:**
- Modify: `app/src/lib/viewport/gl/shaders.ts`

No unit test (GLSL compiles at runtime; verified by `renderer` build + manual E2E in Task 8). Keep the existing `VERT` and `FRAG` exports unchanged.

- [ ] **Step 1: Add `INVERT_FRAG` (and a geometry-aware vertex stage)**

Append to `shaders.ts`:

```ts
// INVERT pass: samples the raw linear negative (RGBA16F), applies geometry
// (orient/flip/straighten/crop) as a UV transform, then ports engine.rs
// invert_b/c/naive + tone. Writes the inverted positive to an RGBA16F FBO that
// the existing FRAG (finishing) pass then reads. Geometry uniforms map the
// output [0,1] UV into source [0,1] UV; out-of-source samples render black.
export const INVERT_FRAG = `#version 300 es
precision highp float;
in vec2 v_uv;                 // output UV in [0,1]
out vec4 o;
uniform sampler2D u_src;      // raw negative, RGBA16F
uniform vec3 u_base;
uniform vec3 u_wb;
uniform mat3 u_m_pre;
uniform mat3 u_m_post;
uniform float u_exposure, u_black, u_gamma;
uniform int u_mode;           // 0=B 1=C 2=Naive
uniform bool u_raw;           // true → output the scan (display gamma), no inversion
// Geometry: output→source UV mapping. crop sub-rect (in source UV) + straighten
// rotation about the crop centre; orient handled by remapping in JS-set u_uvA/u_uvB.
uniform vec2 u_crop_off;      // source-UV offset of the crop origin
uniform vec2 u_crop_scale;    // source-UV size of the crop
uniform float u_angle;        // straighten radians (clockwise)
uniform mat2 u_orient;        // rot90/flip as a 2x2 on centred UV
uniform vec2 u_orient_flip;   // post-orient sign fix kept in JS; identity by default

const float EPS = 1e-5;
const float LOG10 = 0.30102999566; // 1/log2(10): log10(x) = log2(x)*LOG10

float tone(float v, float gain) {
  v = max(v * u_exposure * gain - u_black, 0.0);
  return pow(v, u_gamma);
}

vec3 invert(vec3 rgbIn) {
  // normalise against base, clamp like engine.rs
  vec3 r = clamp(vec3(
    rgbIn.r / max(u_base.r, EPS),
    rgbIn.g / max(u_base.g, EPS),
    rgbIn.b / max(u_base.b, EPS)), EPS, 1.0);
  if (u_mode == 2) {           // Naive: 1 - clamp(I/base,0,1)
    vec3 n = clamp(vec3(rgbIn.r/max(u_base.r,EPS), rgbIn.g/max(u_base.g,EPS), rgbIn.b/max(u_base.b,EPS)), 0.0, 1.0);
    return 1.0 - n;
  }
  if (u_mode == 1) {           // Mode C: per-channel log density
    vec3 dens = -vec3(log2(r.r), log2(r.g), log2(r.b)) * LOG10;
    return vec3(tone(dens.r, u_wb.r), tone(dens.g, u_wb.g), tone(dens.b, u_wb.b));
  }
  // Mode B: M_post * (-log10(M_pre * r)) then tone
  vec3 mixed = u_m_pre * r;
  vec3 dens = -vec3(
    log2(max(mixed.r, EPS)), log2(max(mixed.g, EPS)), log2(max(mixed.b, EPS))) * LOG10;
  vec3 unmixed = u_m_post * dens;
  return vec3(tone(unmixed.r, u_wb.r), tone(unmixed.g, u_wb.g), tone(unmixed.b, u_wb.b));
}

// Map output UV → source UV through crop + straighten + orient.
vec2 sourceUV(vec2 uv) {
  // centre, apply orient (rot90/flip) and straighten rotation, then map into crop.
  vec2 c = uv - 0.5;
  c = u_orient * c;
  float s = sin(u_angle), co = cos(u_angle);
  c = mat2(co, -s, s, co) * c;
  vec2 cuv = c + 0.5;                         // back to [0,1] within the (oriented) crop
  return u_crop_off + cuv * u_crop_scale;     // into full source UV
}

void main() {
  vec2 suv = sourceUV(v_uv);
  if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
    o = vec4(0.0, 0.0, 0.0, 1.0); return;     // outside source (straighten corners) = black
  }
  vec3 rgb = texture(u_src, suv).rgb;
  if (u_raw) { o = vec4(pow(clamp(rgb, 0.0, 1.0), vec3(1.0/2.2)), 1.0); return; }
  o = vec4(invert(rgb), 1.0);
}`;
```

*(Implementation note for the engineer: the geometry uniforms have sensible identity defaults — `u_orient = mat2(1,0,0,1)`, `u_angle = 0`, `u_crop_off = vec2(0)`, `u_crop_scale = vec2(1)` — which reproduce "whole image, no rotation". Get the invert path correct with identities FIRST (Task 8 Step 4), then enable geometry (Task 8 Step 6). `u_orient_flip` is reserved; leave it unused/identity for now.)*

- [ ] **Step 2: Confirm the module still builds**

Run: `cd app && npm run build`
Expected: builds (the new export is just a string; no type errors).

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/viewport/gl/shaders.ts
git commit -m "feat(gpu): INVERT fragment shader (ports engine.rs B/C/Naive + geometry UV)"
```

---

## Task 6: TS — extend `FinishRenderer` to two passes with a float source

**Files:**
- Modify: `app/src/lib/viewport/gl/renderer.ts`

No unit test (WebGL needs a real context — verified by build + manual E2E). Keep all existing finishing behavior; add the invert pass *before* it.

- [ ] **Step 1: Add the invert program, intermediate FBO, float upload, and geometry/inversion uniforms**

Make these changes in `renderer.ts`:

1. Import the invert shader + uniform type at the top:

```ts
import { VERT, FRAG, INVERT_FRAG } from "./shaders";
import { type InversionUniforms } from "./invert";
```

2. Add fields to the class (next to the existing private fields):

```ts
  private invProg: WebGLProgram | null = null;
  private srcTexF: WebGLTexture | null = null;   // RGBA16F raw negative
  private interTex: WebGLTexture | null = null;   // RGBA16F inverted intermediate
  private fbo: WebGLFramebuffer | null = null;
  private inv: InversionUniforms | null = null;
  private invLoc: Record<string, WebGLUniformLocation | null> = {};
  private geom = {
    crop_off: new Float32Array([0, 0]),
    crop_scale: new Float32Array([1, 1]),
    angle: 0,
    orient: new Float32Array([1, 0, 0, 1]),
    raw: false,
  };
  private useFloat = false; // true once setSourceFloat is used
```

3. In the constructor, after building the finishing program, build the invert program and create the float source texture + intermediate texture + FBO. Add this just before `this.available = true;`:

```ts
    // Invert program (pass 1). Requires float color-buffer for the FBO.
    if (!gl.getExtension("EXT_color_buffer_float")) { this.available = false; return; }
    const ivs = this.compile(gl, gl.VERTEX_SHADER, VERT);
    const ifs = this.compile(gl, gl.FRAGMENT_SHADER, INVERT_FRAG);
    if (!ivs || !ifs) { this.available = false; return; }
    const ip = gl.createProgram()!;
    gl.attachShader(ip, ivs); gl.attachShader(ip, ifs); gl.linkProgram(ip);
    if (!gl.getProgramParameter(ip, gl.LINK_STATUS)) {
      console.error("invert link:", gl.getProgramInfoLog(ip)); this.available = false; return;
    }
    this.invProg = ip;
    for (const n of [
      "u_src","u_base","u_wb","u_m_pre","u_m_post","u_exposure","u_black","u_gamma",
      "u_mode","u_raw","u_crop_off","u_crop_scale","u_angle","u_orient",
    ]) this.invLoc[n] = gl.getUniformLocation(ip, n);
    gl.useProgram(ip); gl.uniform1i(this.invLoc.u_src, 0);

    // RGBA16F raw-negative source texture.
    this.srcTexF = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.srcTexF);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);

    // RGBA16F intermediate (inverted) + FBO.
    this.interTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.interTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    this.fbo = gl.createFramebuffer();
```

4. Add a float-source uploader + geometry/inversion setters (methods on the class):

```ts
  /** Upload the raw linear negative as an RGBA16F texture (once per image). */
  setSourceFloat(pixels: Uint16Array, w: number, h: number) {
    const gl = this.gl; if (!gl || !this.srcTexF || !this.interTex) return;
    this.srcW = w; this.srcH = h; this.useFloat = true;
    gl.bindTexture(gl.TEXTURE_2D, this.srcTexF);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false); // geometry handled in-shader
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, pixels);
    // (Re)allocate the intermediate to the OUTPUT size; default = source size.
    this.allocInter(w, h);
    this.hasSource = true;
  }

  /** Size the intermediate FBO texture (output dims = post-geometry canvas). */
  private allocInter(w: number, h: number) {
    const gl = this.gl; if (!gl || !this.interTex) return;
    gl.bindTexture(gl.TEXTURE_2D, this.interTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
  }

  setInversion(u: InversionUniforms) { this.inv = u; }

  /** Geometry from the host (identity-safe). out{W,H} = post-geometry canvas size. */
  setGeometry(g: {
    crop_off: [number, number]; crop_scale: [number, number];
    angle: number; orient: [number, number, number, number];
    raw: boolean; outW: number; outH: number;
  }) {
    this.geom.crop_off = new Float32Array(g.crop_off);
    this.geom.crop_scale = new Float32Array(g.crop_scale);
    this.geom.angle = g.angle;
    this.geom.orient = new Float32Array(g.orient);
    this.geom.raw = g.raw;
    this.canvas.width = g.outW; this.canvas.height = g.outH;
    this.allocInter(g.outW, g.outH);
    this.srcW = g.outW; this.srcH = g.outH; // finishing pass uses these for u_texel/viewport
  }
```

5. Rewrite `draw()` so that, when a float source is present, it runs the invert pass into the FBO, then the finishing pass reads the FBO. Replace the body of `draw()` with:

```ts
  draw() {
    const gl = this.gl; if (!gl || !this.hasSource) return;
    if (this.useFloat && this.invProg && this.inv) {
      // PASS 1: INVERT raw negative → intermediate FBO (output-sized).
      gl.useProgram(this.invProg);
      gl.bindVertexArray(this.vao);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.interTex, 0);
      gl.viewport(0, 0, this.srcW, this.srcH);
      gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, this.srcTexF);
      const L = this.invLoc, u = this.inv;
      gl.uniform3fv(L.u_base, u.base); gl.uniform3fv(L.u_wb, u.wb);
      gl.uniformMatrix3fv(L.u_m_pre, false, u.m_pre);
      gl.uniformMatrix3fv(L.u_m_post, false, u.m_post);
      gl.uniform1f(L.u_exposure, u.exposure); gl.uniform1f(L.u_black, u.black);
      gl.uniform1f(L.u_gamma, u.gamma); gl.uniform1i(L.u_mode, u.mode);
      gl.uniform1i(L.u_raw, this.geom.raw ? 1 : 0);
      gl.uniform2fv(L.u_crop_off, this.geom.crop_off);
      gl.uniform2fv(L.u_crop_scale, this.geom.crop_scale);
      gl.uniform1f(L.u_angle, this.geom.angle);
      gl.uniformMatrix2fv(L.u_orient, false, this.geom.orient);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    }
    // PASS 2: FINISH (existing program) reads the intermediate, draws to canvas.
    const p = this.prog, fu = this.uniforms;
    if (!p || !fu) return;
    gl.useProgram(p);
    gl.bindVertexArray(this.vao);
    gl.viewport(0, 0, this.srcW, this.srcH);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.useFloat ? this.interTex : this.tex);
    gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, this.lutTex);
    gl.uniform2f(this.loc.u_texel, 1 / this.srcW, 1 / this.srcH);
    for (const n of UNIFORM_NAMES) gl.uniform1f(this.loc[`u_${n}`], (fu as unknown as Record<string, number>)[n]);
    const cg = this.cg;
    if (cg) {
      for (const [uu, k] of CG_VEC3) gl.uniform3fv(this.loc[uu], cg[k] as [number, number, number]);
      for (const [uu, k] of CG_FLOAT) gl.uniform1f(this.loc[uu], cg[k] as number);
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
```

*(The finishing pass reads the intermediate without Y-flip; the existing `setSource` 8-bit path — used by the CPU fallback — is untouched and still works via `this.useFloat === false`.)*

- [ ] **Step 2: Build**

Run: `cd app && npm run build`
Expected: builds clean (TypeScript happy).

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/viewport/gl/renderer.ts
git commit -m "feat(gpu): two-pass renderer — invert (RGBA16F FBO) then finish"
```

---

## Task 7: TS — api bindings for the new commands

**Files:**
- Modify: `app/src/lib/api.ts`

- [ ] **Step 1: Add the three bindings**

In `api.ts`, add to the exported api object (matching the existing `invoke<...>(...)` style):

```ts
  workingInfo: (id: string) =>
    invoke<{ w: number; h: number }>("working_info", { id }),

  // Tauri returns the command's `Response` bytes as an ArrayBuffer.
  workingPixels: (id: string) =>
    invoke<ArrayBuffer>("working_pixels", { id }),

  resolvedInversion: (id: string, params: InvertParams) =>
    invoke<import("./viewport/gl/invert").ResolvedInversion>("resolved_inversion", { id, params }),
```

- [ ] **Step 2: Build to confirm types resolve**

Run: `cd app && npm run build`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/api.ts
git commit -m "feat(gpu): api bindings for working_info/working_pixels/resolved_inversion"
```

---

## Task 8: Svelte — wire upload-once + uniform-driven redraw + CPU fallback

**Files:**
- Modify: `app/src/lib/viewport/Viewport.svelte`

This is the integration task; verification is **manual E2E** (run the app). Build after each step.

- [ ] **Step 1: Decide the GPU vs fallback condition and import helpers**

Add to the script imports:

```ts
import { toInversionUniforms } from "./gl/invert";
```

Add a reactive flag — GPU path is used only when WebGL2 is available AND no dust/IR is active (dust/IR keep the proven CPU path until Plan 3):

```ts
$: gpuEligible = !!(useGL && renderer && !raw && dust.length === 0 && !irRemoval.enabled);
```

- [ ] **Step 2: Upload the working buffer once per image (and on quality change)**

Add an upload routine + a key that changes only when the *texture* must change (image id or quality). `quality` arrives via the existing develop store / props the same way `render()` learns it — reuse whatever signals a quality switch today (the existing code re-fetches on quality because `develop_image` re-runs; key off `id` plus a `qualityRev` counter if present, else just `id`):

```ts
let uploadedId: string | null = null;

async function uploadWorking() {
  if (!gpuEligible || !id || !renderer) return;
  if (uploadedId === id) return;               // already on the GPU
  const info = await api.workingInfo(id);
  const buf = await api.workingPixels(id);
  renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h);
  uploadedId = id;
  texW = info.w; texH = info.h;
  await refreshInversion();                     // set uniforms before first draw
  applyGeometryAndDraw();
}

$: if (gpuEligible) { id; uploadWorking(); }   // re-run when id changes
```

Add `let texW = 0, texH = 0;` to the component state. *(If the app exposes a quality-changed signal, also reset `uploadedId = null` when it fires so the next `uploadWorking()` re-uploads at the new cap. If quality switching currently calls `developImage` again, hook the same place to clear `uploadedId`.)*

- [ ] **Step 3: Fetch resolved inversion uniforms on inversion-param change**

```ts
async function refreshInversion() {
  if (!gpuEligible || !id || !renderer) return;
  const res = await api.resolvedInversion(id, params);
  renderer.setInversion(toInversionUniforms(res));
}
```

- [ ] **Step 4: Geometry uniforms + draw (start with identities)**

For Plan 2 step-1 correctness, wire geometry as **identity** first (whole image, no orient/crop/straighten) so you can validate the invert path alone:

```ts
function applyGeometryAndDraw() {
  if (!gpuEligible || !renderer) return;
  // Identity geometry: whole texture, no orient/crop/straighten.
  renderer.setGeometry({
    crop_off: [0, 0], crop_scale: [1, 1], angle: 0,
    orient: [1, 0, 0, 1], raw: false, outW: texW, outH: texH,
  });
  drawGL();
}
```

`drawGL()` already sets finishing uniforms/LUT/colour-grade then calls `renderer.draw()` and snapshots the histogram — keep it as-is; it now drives the 2-pass path.

- [ ] **Step 5: Collapse `srcKey` for the GPU path; keep CPU path for fallback**

Today `srcKey` (line ~148) triggers a backend fetch on exposure/temp/tint/mode/stock/geometry/zoom. Split the reactive behavior:

```ts
// Inversion params now drive GPU uniforms (no fetch) when eligible.
$: invKey = `${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${params.black}|${params.gamma}`;
$: if (gpuEligible) { invKey; refreshInversion().then(applyGeometryAndDraw); }

// Geometry still drives GPU uniforms (no fetch) when eligible.
$: geomKey = `${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}`;
$: if (gpuEligible) { geomKey; texW; applyGeometryAndDraw(); }

// CPU fallback path: only fetch from the backend when NOT eligible
// (dust/IR active, raw view, or no WebGL2). Reuses the existing render()/srcKey.
$: cpuKey = gpuEligible ? '' :
  `${id}|${raw}|${eff}|${vpW}|${vpH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}`;
$: cpuKey, imgW, imgH, scheduleIfReady();
```

Remove the old `$: srcKey, imgW, imgH, scheduleIfReady();` line so the CPU path fires only via `cpuKey`. Keep `finishKey` exactly as-is (finishing is always GPU when `useGL`).

When falling *back* (e.g. user enables dust mid-edit), `render()` still runs and `setSource()` (8-bit) is used; clear `uploadedId = null` whenever `gpuEligible` flips false→true so re-entry re-uploads:

```ts
$: if (gpuEligible && uploadedId !== id) uploadWorking();
$: if (!gpuEligible) uploadedId = null;
```

- [ ] **Step 6: Build, then MANUAL E2E — validate the GPU inversion path**

Run: `cd app && npm run build` → expect clean.
Then: `cd app && npm run tauri dev`. Load a DNG from `/Volumes/Disk2/Film Scans`. Verify:
1. The image displays **correctly inverted** (matches how it looked before this plan) at Mode B with a stock.
2. Dragging **exposure / temp / tint** updates **instantly** with **no flicker/reload** and (in devtools Network/console) **no `render_view` call**.
3. Switching **Mode B ↔ C** and **stock** updates live.
4. **Zoom/pan** is smooth (pure CSS; no fetch).
5. Enabling **dust eraser** or **IR removal** falls back to the CPU path and still works (image stays correct).
6. The **histogram** still updates.

If inversion looks wrong (e.g. negative, wrong colors), debug the INVERT shader against `engine.rs` (most likely: matrix order — try `u_m_pre`/`u_m_post` as-is first; nalgebra `as_slice()` is column-major and `uniformMatrix3fv(..., false, ...)` expects column-major, so no transpose).

- [ ] **Step 7: MANUAL E2E — enable geometry**

Replace `applyGeometryAndDraw()`'s identity body with real geometry derived from `rot90/flipH/flipV/angle/imageCrop`:

```ts
function applyGeometryAndDraw() {
  if (!gpuEligible || !renderer) return;
  // orient as a 2x2 on centred UV: rot90 (clockwise) + flips.
  const a = (rot90 % 4) * Math.PI / 2;
  const s = Math.sin(a), c = Math.cos(a);
  let o = [c, -s, s, c];                       // rotation
  const fx = flipH ? -1 : 1, fy = flipV ? -1 : 1;
  o = [o[0] * fx, o[1] * fy, o[2] * fx, o[3] * fy];
  // persistent crop in source UV (imageCrop is normalized [x,y,w,h] or null).
  const [cx, cy, cw, ch] = imageCrop ?? [0, 0, 1, 1];
  // output canvas = oriented+cropped aspect; for rot90 odd, swap dims.
  const baseW = texW * cw, baseH = texH * ch;
  const swap = (rot90 % 2) === 1;
  const outW = Math.max(1, Math.round(swap ? baseH : baseW));
  const outH = Math.max(1, Math.round(swap ? baseW : baseH));
  renderer.setGeometry({
    crop_off: [cx, cy], crop_scale: [cw, ch],
    angle: (angle * Math.PI) / 180, orient: o as [number, number, number, number],
    raw: false, outW, outH,
  });
  drawGL();
}
```

Re-run the app. Verify rotate-90, flip H/V, straighten (angle slider), and a persistent crop all match the previous backend-rendered geometry. Debug UV signs against `convert.rs::orient`/`rotate` if a transform is mirrored/rotated the wrong way (flip a sign in `o` or negate `u_angle`).

- [ ] **Step 8: Commit**

```bash
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(gpu): upload-once + uniform-driven inversion/geometry; CPU fallback for dust/IR"
```

---

## Task 9: Parity verification pass + record findings

**Files:**
- Modify: `docs/superpowers/poc-findings.md`

- [ ] **Step 1: Walk the Plan-2 parity checklist in the running app**

With `npm run tauri dev`, confirm each and note PASS/FAIL:
1. Inverted preview (Mode B + each stock) visually matches pre-Plan-2 output.
2. Mode C and Mode-B/none produce the same result as the CPU path (switch quality to force a CPU render of the same params and compare — they should look identical within preview tolerance).
3. exposure / black / gamma / temp / tint / mode / stock — all live, no `render_view` calls (verify in devtools).
4. Zoom/pan smooth; quality toggle does not re-decode/re-upload unnecessarily (only on actual quality change).
5. orient / flip / straighten / persistent-crop match the old geometry.
6. Raw (un-inverted) view still works (Library uses `raw:true` → CPU path; confirm unaffected).
7. Dust eraser + IR removal still heal correctly (CPU fallback).
8. Finishing (contrast…saturation, tone curve, color grading) unchanged.
9. Export (TIFF/PNG/JPEG) still matches (export path is the unchanged CPU engine).
10. Histogram updates.

- [ ] **Step 2: Record the result**

Append a dated section to `docs/superpowers/poc-findings.md`:

```markdown
## GPU Develop — Plan 2 results (2026-06-05)

Float-once upload + GPU inversion + GPU geometry landed. Per-frame base64-JPEG eliminated for
the no-dust case; exposure/temp/tint/mode/stock/zoom now update with no backend round-trip.
Dust/IR and raw view use the CPU fallback (Plan 3 moves them onto the GPU + offscreen export).
Parity checklist walked: <PASTE PASS/FAIL PER ITEM>.
```

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/poc-findings.md
git commit -m "docs(gpu): record Plan 2 parity results"
```

---

## Self-Review

- **Spec coverage:** Phase 2 (float upload + raw IPC bytes) → Tasks 1,3 (`pack_rgba16f`, `working_pixels` via `tauri::ipc::Response`) + Task 8 (upload-once). Phase 3 (shader inversion B/C/Naive + tone + raw bypass, two passes) → Tasks 5,6 + uniforms in Tasks 2,4. Phase 4 (geometry on GPU) → Task 5 shader UV + Task 8 Step 7. Single-source-of-truth/CPU-as-oracle preserved: CPU engine untouched and used as fallback for dust/IR/raw + export. Quality toggle = re-upload at cap (Task 8 Step 2 note). Deferred to Plan 3 (per spec): dust/IR re-domaining onto GPU, offscreen tiled export, removing the JPEG path entirely — all explicitly out of scope here and covered by the CPU fallback so there is no parity gap.
- **Placeholder scan:** No TBD/TODO. Each code step has complete code. The two "Implementation note" parentheticals describe identity defaults and debug hints, not missing code. Manual-E2E steps (Tasks 8–9) are inherent to GPU work and give explicit expected results.
- **Type consistency:** `ResolvedInversion` fields (base, wb, m_pre, m_post, exposure, black, gamma, mode) are identical across Rust (`gpu_upload.rs`), the api binding, and TS (`invert.ts`). `InversionUniforms` (Float32Array mat3s) is produced by `toInversionUniforms` and consumed by `renderer.setInversion` + `draw()` (`uniformMatrix3fv`). `setSourceFloat(Uint16Array,w,h)`, `setGeometry({...})`, `setInversion(...)` names match between renderer and Viewport. Matrix layout: nalgebra column-major `as_slice()` → `uniformMatrix3fv(..., false, ...)` (column-major) — no transpose, consistent end to end.
- **Known risks flagged for the implementer:** (a) Tauri `Response`→`ArrayBuffer` invoke return shape — if `invoke` doesn't yield an ArrayBuffer directly in this Tauri version, use the `@tauri-apps/api/core` `invoke` raw-response option; verify in Task 8 Step 6. (b) `HALF_FLOAT` upload requires the data as `Uint16Array` (it is). (c) The "resident/lazy cache load" note in Task 3 — match `render_view`'s pattern. (d) Geometry UV signs may need empirical flips (Task 8 Step 7 calls this out).
```