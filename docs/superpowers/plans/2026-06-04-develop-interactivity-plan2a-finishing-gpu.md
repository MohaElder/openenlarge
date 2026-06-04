# GPU Live Preview — Plan 2A: Finishing on GPU Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the 8 finishing sliders (contrast, highlights, shadows, whites, blacks, texture, vibrance, saturation) update per-frame on the GPU — the develop preview applies the finishing layer in a WebGL2 fragment shader instead of round-tripping to the backend.

**Architecture:** The backend gains a `finish` flag on `ViewSpec`; when false it returns the inverted+graded preview *before* `finish_image`. The develop `Viewport` renders that "pre-finish" preview into a WebGL2 `<canvas>` and applies the finishing layer in a single fragment shader (ported from `film-core/src/finish.rs`); dragging a finishing slider only updates shader uniforms + redraws (no backend call). Inversion-affecting params (mode, stock, exposure, temp, tint) and zoom still re-fetch the source, as today. Exposure/WB go to the GPU in Plan 2B. Export stays the authoritative Rust path. If WebGL2 is unavailable, the existing `<img>` + backend-finished path is the fallback.

**Tech Stack:** Rust (Tauri command), TypeScript, WebGL2, Svelte 5, vitest, cargo test. cargo not on PATH — prefix with `source "$HOME/.cargo/env" && `. Frontend cwd: `/Users/mohaelder/Repos/filmrev/app`.

**Spec:** `docs/superpowers/specs/2026-06-04-develop-interactivity-design.md` (Plan 2A section).
**Branch:** `feat/develop-redesign`.

**Parity anchor — `film-core/src/finish.rs` (the shader must match):**
- `tone_curve(v)`: clamp01; `v += whites*0.20*v^3`; `v -= blacks*0.20*(1-v)^3`; `v += shadows*0.30*(1-v)^2*v`; `v += highlights*0.30*v^2*(1-v)`; `v = 0.5+(v-0.5)*(1+contrast)`; clamp01.
- `apply_saturation(rgb)`: `y=0.2126r+0.7152g+0.0722b`; `mx=max,mn=min`; `cur=mx>1e-5?(mx-mn)/mx:0`; `f=1+saturation+vibrance*(1-cur)`; per-channel `clamp01(y+(c-y)*f)`.
- `finish_pixel` = tone each channel then saturation.
- texture: `blur` = separable 3-tap (0.25/0.5/0.25) H then V (≡ a 3×3 outer-product kernel), edges clamp; `out = clamp01(v + 1.5*amount*(v − blur(v)))`, applied to the `finish_pixel` result.
- All finishing params are −1..1 (UI value ÷ 100).

---

## File Structure

**Create:**
- `app/src/lib/viewport/gl/uniforms.ts` — pure `finishUniforms(params)` (UI ÷100 → −1..1), testable.
- `app/src/lib/viewport/gl/uniforms.test.ts` — vitest.
- `app/src/lib/viewport/gl/shaders.ts` — vertex + fragment GLSL source strings.
- `app/src/lib/viewport/gl/renderer.ts` — `webgl2Available()` + `FinishRenderer` class (program, source texture, uniforms, draw).

**Modify:**
- `app/src-tauri/src/commands.rs` — `ViewSpec` gains `finish: bool` (default true); `render_view` skips `finish_image` when false.
- `app/src/lib/api.ts` — `ViewSpec.finish?: boolean`.
- `app/src/lib/viewport/Viewport.svelte` — render into a WebGL canvas when interactive & WebGL2 available; split fetch vs. redraw triggers; publish `previewSrc` from the canvas; `<img>` fallback.

`Histogram.svelte` is unchanged — it reads `previewSrc`, which `Viewport` keeps publishing (now from the canvas).

---

## Task 1: Backend — `ViewSpec.finish` flag

**Files:**
- Modify: `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Add the failing test**

In `commands.rs` `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn viewspec_finish_defaults_true_and_parses_false() {
        let d: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#).unwrap();
        assert!(d.finish, "finish should default to true when omitted");
        let f: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,"finish":false}"#).unwrap();
        assert!(!f.finish);
    }
```

- [ ] **Step 2: Run to confirm it fails**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml viewspec_finish`
Expected: FAIL to compile (`ViewSpec` has no field `finish`).

- [ ] **Step 3: Add the field + default + skip logic**

In `commands.rs`, add a default helper near the top (after the `const` lines):

```rust
fn finish_default() -> bool { true }
```

Update the `ViewSpec` struct (currently `pub struct ViewSpec { pub crop: [f64;4], pub out_w: u32, pub out_h: u32, pub raw: bool }`):

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ViewSpec {
    pub crop: [f64; 4],
    pub out_w: u32,
    pub out_h: u32,
    pub raw: bool,
    /// When false, return the inverted+graded preview BEFORE the finishing layer
    /// (the GPU applies finishing live). Defaults true for the legacy path/export.
    #[serde(default = "finish_default")]
    pub finish: bool,
}
```

In `render_view`, the non-raw tail currently is:
```rust
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    let fin = finish_image(&inv, &finish_from(&params));
    to_jpeg_b64(&fin, false, PREVIEW_JPEG_QUALITY)
```
Replace with:
```rust
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    let out = if view.finish { finish_image(&inv, &finish_from(&params)) } else { inv };
    to_jpeg_b64(&out, false, PREVIEW_JPEG_QUALITY)
```

- [ ] **Step 4: Run tests**

Run: `source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: all pass (13 + the new one = 14). Also `cargo clippy --manifest-path app/src-tauri/Cargo.toml 2>&1 | tail -5` → no new warnings.

- [ ] **Step 5: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src-tauri/src/commands.rs
git commit -m "feat(backend): ViewSpec.finish flag — skip finishing layer for GPU preview"
```

---

## Task 2: TS contract + pure uniform mapping

**Files:**
- Modify: `app/src/lib/api.ts`
- Create: `app/src/lib/viewport/gl/uniforms.ts`
- Create: `app/src/lib/viewport/gl/uniforms.test.ts`

- [ ] **Step 1: Add `finish` to `ViewSpec` in `api.ts`**

The interface is currently:
```ts
export interface ViewSpec {
  crop: [number, number, number, number];
  out_w: number;
  out_h: number;
  raw: boolean;
}
```
Add the optional field:
```ts
export interface ViewSpec {
  crop: [number, number, number, number];
  out_w: number;
  out_h: number;
  raw: boolean;
  finish?: boolean; // omit/true = backend applies finishing; false = GPU does it
}
```
(`renderView` already forwards the whole `view` object — no other api.ts change.)

- [ ] **Step 2: Write the failing uniform-mapping test**

Create `app/src/lib/viewport/gl/uniforms.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { finishUniforms } from "./uniforms";
import type { InvertParams } from "../../api";

const base = {
  contrast: 50, highlights: -100, shadows: 0, whites: 25,
  blacks: -25, texture: 100, vibrance: 10, saturation: -40,
} as InvertParams;

describe("finishUniforms", () => {
  it("scales UI −100..100 down to −1..1 per channel", () => {
    const u = finishUniforms(base);
    expect(u.contrast).toBeCloseTo(0.5);
    expect(u.highlights).toBeCloseTo(-1);
    expect(u.shadows).toBeCloseTo(0);
    expect(u.whites).toBeCloseTo(0.25);
    expect(u.blacks).toBeCloseTo(-0.25);
    expect(u.texture).toBeCloseTo(1);
    expect(u.vibrance).toBeCloseTo(0.1);
    expect(u.saturation).toBeCloseTo(-0.4);
  });
});
```

- [ ] **Step 3: Run to confirm FAIL**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/viewport/gl/uniforms.test.ts`
Expected: FAIL (module not found).

- [ ] **Step 4: Implement `uniforms.ts`**

Create `app/src/lib/viewport/gl/uniforms.ts`:
```ts
import type { InvertParams } from "../../api";

/** The 8 finishing controls scaled to the engine's −1..1 (UI value ÷ 100).
 *  Mirrors `finish_from` in commands.rs / FinishParams in finish.rs. */
export interface FinishUniforms {
  contrast: number; highlights: number; shadows: number; whites: number;
  blacks: number; texture: number; vibrance: number; saturation: number;
}

export function finishUniforms(p: InvertParams): FinishUniforms {
  return {
    contrast: p.contrast / 100,
    highlights: p.highlights / 100,
    shadows: p.shadows / 100,
    whites: p.whites / 100,
    blacks: p.blacks / 100,
    texture: p.texture / 100,
    vibrance: p.vibrance / 100,
    saturation: p.saturation / 100,
  };
}
```

- [ ] **Step 5: Run to confirm PASS + typecheck**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/viewport/gl/uniforms.test.ts && npm run check 2>&1 | tail -12`
Expected: test PASS; no new typecheck errors (only the pre-existing `workflow.test.ts` error).

- [ ] **Step 6: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/api.ts app/src/lib/viewport/gl/uniforms.ts app/src/lib/viewport/gl/uniforms.test.ts
git commit -m "feat(app): ViewSpec.finish in TS + finishUniforms mapping (tested)"
```

---

## Task 3: WebGL2 renderer module

**Files:**
- Create: `app/src/lib/viewport/gl/shaders.ts`
- Create: `app/src/lib/viewport/gl/renderer.ts`

- [ ] **Step 1: Create the shaders**

Create `app/src/lib/viewport/gl/shaders.ts`:
```ts
// Fullscreen-triangle vertex shader (no buffers; uses gl_VertexID).
export const VERT = `#version 300 es
out vec2 v_uv;
void main() {
  vec2 uv = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  v_uv = uv;
  gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}`;

// Fragment shader: ports finish.rs. tone_curve + saturation per pixel; texture
// (unsharp) computed by re-evaluating finish() on a 3x3 (outer-product 0.25/0.5/
// 0.25) neighbourhood — numerically equal to blur(finish_pixel) then unsharp.
export const FRAG = `#version 300 es
precision highp float;
in vec2 v_uv;
out vec4 o;
uniform sampler2D u_src;
uniform vec2 u_texel;            // 1/width, 1/height
uniform float u_contrast, u_highlights, u_shadows, u_whites, u_blacks;
uniform float u_vibrance, u_saturation, u_texture;

float tone(float v) {
  v = clamp(v, 0.0, 1.0);
  v += u_whites * 0.20 * v * v * v;
  v -= u_blacks * 0.20 * pow(1.0 - v, 3.0);
  v += u_shadows * 0.30 * (1.0 - v) * (1.0 - v) * v;
  v += u_highlights * 0.30 * v * v * (1.0 - v);
  v = 0.5 + (v - 0.5) * (1.0 + u_contrast);
  return clamp(v, 0.0, 1.0);
}

vec3 finishAt(vec2 uv) {
  vec3 c = texture(u_src, uv).rgb;
  vec3 t = vec3(tone(c.r), tone(c.g), tone(c.b));
  float y = 0.2126 * t.r + 0.7152 * t.g + 0.0722 * t.b;
  float mx = max(max(t.r, t.g), t.b);
  float mn = min(min(t.r, t.g), t.b);
  float cur = mx > 1e-5 ? (mx - mn) / mx : 0.0;
  float f = 1.0 + u_saturation + u_vibrance * (1.0 - cur);
  return clamp(vec3(y) + (t - vec3(y)) * f, 0.0, 1.0);
}

void main() {
  vec3 c = finishAt(v_uv);
  if (abs(u_texture) < 1e-5) { o = vec4(c, 1.0); return; }
  vec2 d = u_texel;
  vec3 b =
    finishAt(v_uv + vec2(-d.x, -d.y)) * 0.0625 +
    finishAt(v_uv + vec2( 0.0, -d.y)) * 0.125  +
    finishAt(v_uv + vec2( d.x, -d.y)) * 0.0625 +
    finishAt(v_uv + vec2(-d.x,  0.0)) * 0.125  +
    c * 0.25 +
    finishAt(v_uv + vec2( d.x,  0.0)) * 0.125  +
    finishAt(v_uv + vec2(-d.x,  d.y)) * 0.0625 +
    finishAt(v_uv + vec2( 0.0,  d.y)) * 0.125  +
    finishAt(v_uv + vec2( d.x,  d.y)) * 0.0625;
  float k = 1.5 * u_texture;
  o = vec4(clamp(c + k * (c - b), 0.0, 1.0), 1.0);
}`;
```

- [ ] **Step 2: Create the renderer**

Create `app/src/lib/viewport/gl/renderer.ts`:
```ts
import { VERT, FRAG } from "./shaders";
import type { FinishUniforms } from "./uniforms";

/** True if the environment can create a WebGL2 context. */
export function webgl2Available(): boolean {
  if (typeof document === "undefined") return false;
  try {
    const c = document.createElement("canvas");
    return !!c.getContext("webgl2");
  } catch {
    return false;
  }
}

const UNIFORM_NAMES = [
  "contrast", "highlights", "shadows", "whites", "blacks",
  "vibrance", "saturation", "texture",
] as const;

/** Applies the finishing layer to a source preview texture via a fragment shader. */
export class FinishRenderer {
  readonly available: boolean;
  private gl: WebGL2RenderingContext | null = null;
  private prog: WebGLProgram | null = null;
  private tex: WebGLTexture | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private loc: Record<string, WebGLUniformLocation | null> = {};
  private uniforms: FinishUniforms | null = null;
  private srcW = 0;
  private srcH = 0;
  private hasSource = false;

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true, premultipliedAlpha: false });
    if (!gl) { this.available = false; return; }
    this.gl = gl;
    const prog = this.build(gl);
    if (!prog) { this.available = false; return; }
    this.prog = prog;
    this.vao = gl.createVertexArray(); // empty VAO required to draw in WebGL2
    this.tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.useProgram(prog);
    this.loc.u_src = gl.getUniformLocation(prog, "u_src");
    this.loc.u_texel = gl.getUniformLocation(prog, "u_texel");
    for (const n of UNIFORM_NAMES) this.loc[`u_${n}`] = gl.getUniformLocation(prog, `u_${n}`);
    gl.uniform1i(this.loc.u_src, 0);
    this.available = true;
  }

  private build(gl: WebGL2RenderingContext): WebGLProgram | null {
    const vs = this.compile(gl, gl.VERTEX_SHADER, VERT);
    const fs = this.compile(gl, gl.FRAGMENT_SHADER, FRAG);
    if (!vs || !fs) return null;
    const p = gl.createProgram()!;
    gl.attachShader(p, vs); gl.attachShader(p, fs); gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
      console.error("link:", gl.getProgramInfoLog(p)); return null;
    }
    return p;
  }
  private compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader | null {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      console.error("shader:", gl.getShaderInfoLog(s)); return null;
    }
    return s;
  }

  /** Upload a decoded preview image as the source texture; sizes the canvas. */
  setSource(img: TexImageSource, w: number, h: number) {
    const gl = this.gl; if (!gl || !this.tex) return;
    this.srcW = w; this.srcH = h;
    this.canvas.width = w; this.canvas.height = h;
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
    this.hasSource = true;
  }

  setUniforms(u: FinishUniforms) { this.uniforms = u; }

  draw() {
    const gl = this.gl, p = this.prog, u = this.uniforms;
    if (!gl || !p || !u || !this.hasSource) return;
    gl.useProgram(p);
    gl.bindVertexArray(this.vao);
    gl.viewport(0, 0, this.srcW, this.srcH);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.uniform2f(this.loc.u_texel, 1 / this.srcW, 1 / this.srcH);
    for (const n of UNIFORM_NAMES) gl.uniform1f(this.loc[`u_${n}`], (u as Record<string, number>)[n]);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}
```

- [ ] **Step 3: Typecheck**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -15`
Expected: no new errors from `shaders.ts` / `renderer.ts`. (WebGL types ship with `lib.dom`; `TexImageSource` is a DOM type.) Ignore the pre-existing `workflow.test.ts` error and a11y warnings.

- [ ] **Step 4: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/viewport/gl/shaders.ts app/src/lib/viewport/gl/renderer.ts
git commit -m "feat(app): WebGL2 finishing renderer (shader port of finish.rs)"
```

---

## Task 4: Viewport integration (canvas + split triggers + fallback)

**Files:**
- Modify: `app/src/lib/viewport/Viewport.svelte`

This replaces the whole file. **All zoom/pan/animation logic is preserved verbatim** — the only changes are: GL imports + state, a WebGL `<canvas>` element (used when `interactive` and WebGL2 is available; `<img>` otherwise), splitting the re-fetch trigger (inversion/zoom params) from the GPU-redraw trigger (finishing params), and publishing `previewSrc` from the canvas.

- [ ] **Step 1: Replace `app/src/lib/viewport/Viewport.svelte` with:**

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { api, type InvertParams } from "../api";
  import { previewSrc } from "../store";
  import { FinishRenderer, webgl2Available } from "./gl/renderer";
  import { finishUniforms } from "./gl/uniforms";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;
  export let imgH = 0;
  export let raw = false;
  export let interactive = true;

  const CAP = 5000;
  const PAD = 60;

  let el: HTMLDivElement;
  let canvas: HTMLCanvasElement | null = null;
  let renderer: FinishRenderer | null = null;
  // GPU path: only the interactive, non-raw develop canvas, when WebGL2 exists.
  const useGL = interactive && !raw && webgl2Available();

  let src = "";
  let vpW = 0, vpH = 0;
  let scale = 0;
  let cx = 0, cy = 0;
  let prevId: string | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let histTimer: ReturnType<typeof setTimeout> | null = null;
  let animating = false;
  let animTimer: ReturnType<typeof setTimeout> | null = null;

  $: ready = imgW > 0 && imgH > 0 && vpW > 0 && vpH > 0;
  $: pad = interactive ? PAD : 0;
  $: avW = Math.max(1, vpW - 2 * pad);
  $: avH = Math.max(1, vpH - 2 * pad);
  $: fit = ready ? Math.min(avW / imgW, avH / imgH) : 0;
  $: eff = interactive ? (scale > 0 ? scale : fit) : fit;
  $: zoomed = interactive && eff > fit + 1e-6;
  $: label = eff <= fit + 1e-6 ? "Fit" : Math.round(eff * 100) + "%";

  function clampCenter() {
    const halfW = avW / 2 / eff, halfH = avH / 2 / eff;
    cx = imgW * eff <= avW ? imgW / 2 : Math.max(halfW, Math.min(imgW - halfW, cx));
    cy = imgH * eff <= avH ? imgH / 2 : Math.max(halfH, Math.min(imgH - halfH, cy));
  }

  $: dispW = imgW * eff;
  $: dispH = imgH * eff;
  $: left = vpW / 2 - cx * eff;
  $: top = vpH / 2 - cy * eff;

  function measure() {
    if (!el) return;
    vpW = el.clientWidth; vpH = el.clientHeight;
  }
  onMount(() => {
    measure();
    if (useGL && canvas) {
      const r = new FinishRenderer(canvas);
      if (r.available) renderer = r;
    }
    const ro = new ResizeObserver(measure);
    if (el) ro.observe(el);
    return () => ro.disconnect();
  });

  $: if (id !== prevId) { prevId = id; scale = 0; cx = imgW / 2; cy = imgH / 2; }
  $: if (interactive && scale === 0 && fit > 0) scale = fit;

  // Decode a JPEG data-URL to an <img> we can upload as a texture.
  function loadImage(url: string): Promise<HTMLImageElement> {
    return new Promise((resolve, reject) => {
      const im = new Image();
      im.onload = () => resolve(im);
      im.onerror = reject;
      im.src = url;
    });
  }

  // Fetch the source preview. With GL, request the PRE-FINISH image (finish:false)
  // and apply finishing in the shader; otherwise fetch the finished image.
  async function render() {
    if (!id || !imgW || !vpW) { src = ""; return; }
    const rscale = Math.min(eff, CAP / Math.max(imgW, imgH));
    const out_w = Math.max(1, Math.round(imgW * rscale));
    const out_h = Math.max(1, Math.round(imgH * rscale));
    try {
      const data = await api.renderView(id, params, {
        crop: [0, 0, imgW, imgH], out_w, out_h, raw, finish: !(useGL && renderer),
      });
      if (useGL && renderer) {
        const im = await loadImage(data);
        renderer.setSource(im, out_w, out_h);
        drawGL();
      } else {
        src = data;
        if (interactive && !raw) previewSrc.set(src);
      }
    } catch { /* keep previous frame */ }
  }

  function drawGL() {
    if (!renderer) return;
    renderer.setUniforms(finishUniforms(params));
    renderer.draw();
    // Publish a snapshot for the histogram (debounced; toDataURL is cheap-ish).
    if (canvas) {
      if (histTimer) clearTimeout(histTimer);
      const cv = canvas;
      histTimer = setTimeout(() => previewSrc.set(cv.toDataURL("image/jpeg", 0.8)), 120);
    }
  }

  function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(render, 80); }
  function scheduleIfReady() { if (id && vpW && imgW) { clampCenter(); schedule(); } }

  // Re-fetch the SOURCE only when the inversion / zoom / view changes. In Plan 2A
  // exposure/temp/tint are still baked by the backend, so they live in this key.
  $: srcKey = `${id}|${raw}|${eff}|${vpW}|${vpH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}`;
  $: srcKey, imgW, imgH, scheduleIfReady();

  // Finishing-only change → GPU redraw, no backend fetch.
  $: finishKey = `${params.contrast}|${params.highlights}|${params.shadows}|${params.whites}|${params.blacks}|${params.texture}|${params.vibrance}|${params.saturation}`;
  $: if (useGL) { finishKey; if (renderer) drawGL(); }

  function imgPoint(e: { clientX: number; clientY: number }): [number, number] {
    const rect = el.getBoundingClientRect();
    return [(e.clientX - rect.left - left) / eff, (e.clientY - rect.top - top) / eff];
  }

  function startAnim() {
    animating = true;
    if (animTimer) clearTimeout(animTimer);
    animTimer = setTimeout(() => (animating = false), 200);
  }
  function stopAnim() {
    if (animTimer) { clearTimeout(animTimer); animTimer = null; }
    animating = false;
  }

  function onWheel(e: WheelEvent) {
    if (!interactive) return;
    stopAnim();
    e.preventDefault();
    const [ix, iy] = imgPoint(e);
    const ns = Math.min(8, Math.max(fit, eff * Math.exp(-e.deltaY * 0.0015)));
    cx = ix + (cx - ix) * (eff / ns);
    cy = iy + (cy - iy) * (eff / ns);
    scale = ns;
  }

  let lastX = 0, lastY = 0, downX = 0, downY = 0, moved = false, panning = false;
  function onDown(e: PointerEvent) {
    if (!interactive) return;
    stopAnim();
    downX = lastX = e.clientX; downY = lastY = e.clientY; moved = false;
    panning = zoomed;
    (e.target as Element).setPointerCapture?.(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!interactive || !(e.buttons & 1)) return;
    if (Math.abs(e.clientX - downX) > 3 || Math.abs(e.clientY - downY) > 3) moved = true;
    if (panning && moved) {
      cx -= (e.clientX - lastX) / eff;
      cy -= (e.clientY - lastY) / eff;
      clampCenter();
    }
    lastX = e.clientX; lastY = e.clientY;
  }
  function onUp(e: PointerEvent) {
    if (interactive && !moved) {
      const [ix, iy] = imgPoint(e);
      startAnim();
      if (zoomed) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
      else { scale = 1.0; cx = ix; cy = iy; }
    }
    panning = false; moved = false;
  }
  function onCancel() { panning = false; moved = false; }
</script>

<div
  class="vp" class:interactive class:zoomed
  bind:this={el}
  on:wheel={onWheel}
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onCancel}
>
  {#if useGL}
    <canvas
      bind:this={canvas} class:anim={animating}
      style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px;"
    ></canvas>
    {#if !id}<div class="hint">…</div>{/if}
  {:else if src}
    <img
      {src} alt="preview" draggable="false" class:anim={animating}
      style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px;"
    />
  {:else}<div class="hint">…</div>{/if}
  {#if id && interactive}<div class="zoom">{label}</div>{/if}
</div>

<style>
  .vp { position: relative; width: 100%; height: 100%; overflow: hidden; user-select: none;
    border-radius: 10px; }
  .vp.interactive { cursor: zoom-in; }
  .vp.zoomed { cursor: grab; }
  .vp.zoomed:active { cursor: grabbing; }
  img, canvas { display: block; will-change: left, top, width, height; }
  img.anim, canvas.anim { transition: left 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    top 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    width 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    height 180ms cubic-bezier(0.22, 0.61, 0.36, 1); }
  .hint { color: var(--text-dim); position: absolute; inset: 0; display: grid; place-items: center; }
  .zoom { position: absolute; bottom: 8px; right: 10px; font-size: 11px; color: var(--text-dim);
    background: rgba(0,0,0,0.45); padding: 2px 8px; border-radius: 6px; z-index: 2; }
</style>
```

- [ ] **Step 2: Typecheck**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -15`
Expected: no new errors from `Viewport.svelte`. Ignore the pre-existing `workflow.test.ts` error and a11y warnings (the canvas with pointer handlers may add an a11y warning like the div already has — acceptable, consistent with the rest of the file).

- [ ] **Step 3: Unit tests still green**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run 2>&1 | tail -6`
Expected: all pass (no Viewport unit tests; this confirms nothing else broke).

- [ ] **Step 4: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(app): GPU finishing preview in Viewport (WebGL canvas + split fetch/redraw, img fallback)"
```

---

## Task 5: Verification + manual smoke

**Files:** none (verification only)

- [ ] **Step 1: Rust + frontend automated checks**

Run:
```
source "$HOME/.cargo/env" && cargo test --manifest-path app/src-tauri/Cargo.toml 2>&1 | grep -E "test result"
cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -1 && npx vitest run 2>&1 | tail -4
```
Expected: backend tests pass (14); svelte-check shows only the pre-existing `workflow.test.ts` error; all vitest pass.

- [ ] **Step 2: Manual smoke (user, in the running app)**

In Develop on a developed image:
- Dragging **Contrast / Highlights / Shadows / Whites / Blacks / Texture / Vibrance / Saturation** updates the preview **smoothly, per-frame** (no backend hitch).
- Dragging **Exposure / Temp / Tint** still updates correctly (via backend re-fetch; expected to be less instant in 2A — fixed in 2B).
- The image is **not upside-down or mirrored** (texture flip correct), colors match what the exported TIFF looks like at the same settings (parity spot-check at a couple of settings).
- **Zoom (tap Fit↔100%, scroll), pan, and the zoom animation** all still work, now on the canvas.
- The **histogram** still updates as finishing sliders move.
- Switching images shows that image's edits (per-image, from Plan 1).

- [ ] **Step 3: Final commit (only if smoke required fixups)**

```bash
cd /Users/mohaelder/Repos/filmrev
git add -A && git commit -m "fix: GPU finishing preview smoke fixups"
```

---

## Self-Review notes

- **Spec coverage (Plan 2A):** backend `finish` flag + skip (Task 1); TS `finish` + uniform mapping (Task 2); WebGL renderer porting `finish.rs` incl. texture/unsharp (Task 3); Viewport canvas + split fetch/redraw + `<img>` fallback + `previewSrc` from canvas for the histogram (Task 4); parity + zoom/pan/anim preserved, verified (Task 5).
- **Placeholder scan:** none — full code in every step.
- **Type consistency:** `FinishUniforms` shape and the `UNIFORM_NAMES` list match the shader's `u_*` uniforms and `finishUniforms` keys; `ViewSpec.finish` matches between Rust (`#[serde(default)]`) and TS (`finish?`); `FinishRenderer.setSource(img, w, h)` / `setUniforms` / `draw` are used exactly so in Viewport.
- **Parity risk:** the GLSL `tone`/`finishAt`/texture math is a line-by-line port of `finish.rs`; export stays Rust-authoritative; manual parity spot-check in Task 5.
- **Known carry-over:** the pre-existing `workflow.test.ts` `path` error is unrelated and out of scope.
