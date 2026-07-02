# Windows Live WebGPU EDR Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring live, per-frame HDR editing to Windows via an in-webview WebGPU EDR renderer that mirrors the WebGL preview plus `hdr_finish`, swapped in when the display+GPU support extended tone mapping.

**Architecture:** Frontend-only (TypeScript + WGSL). A new `WebGPUFinishRenderer` parallels the WebGL `FinishRenderer` (`app/src/lib/viewport/gl/renderer.ts`) but targets an `rgba16float` canvas configured `toneMapping:{mode:'extended'}` and runs the full invert→finish→`hdr_finish` pipeline. WGSL is ported from the tested MSL in `app/src-tauri/src/hdr_surface/msl.rs`. It reuses the exact frontend data that drives WebGL (source texture, geometry, resolved uniforms, LUT). On Windows `live-edr`, the WebGPU canvas swaps in for WebGL; dust/eraser markers get a DOM overlay.

**Tech Stack:** Svelte/TypeScript, WebGPU + WGSL, Vitest. Dev/build on the Windows box (`calen@192.168.1.134`, `C:\Users\calen\filmrev`).

## Global Constraints

- **Frontend only** — no new Rust/backend code; reuse the WebGL renderer's existing data (source texture, geometry, resolved uniforms, LUT).
- **Windows-scoped** — macOS native path and Linux (`hidden`) untouched; pure capability upgrade gated on the WebGPU probe (no regression where unavailable).
- **Parity is the bar** — WebGPU-SDR must equal the WebGL preview pixel-for-pixel; the WGSL mirrors the MSL/Rust math exactly, sharing identical constants. WGSL derives from MSL — no divergent 4th logic copy.
- **EDR mechanism** — `rgba16float` + `toneMapping:{mode:'extended'}`; probe before enabling; `srgbToLinearExt3` on output (extended-linear surface), mirroring `msl.rs`.
- **Swap, not overlay** — one preview canvas visible at a time (WebGL for SDR, WebGPU for HDR).
- **Shared constants (must match across Rust `engine.rs`/`finish.rs`, GLSL `shaders.ts`, MSL `msl.rs`, and the new WGSL):** `HDR_KNEE=0.8`, `HDR_HEADROOM=2.5`, `HDR_W_HI=1.2`, `FAITHFUL_GAMMA=1.590`, `FAITHFUL_KNEE=0.892`, `FAITHFUL_SCALE=1/0.700`, `LOOK_K=2.0`, `EXPO_K=0.14`, `FAITHFUL_EXPO_K=1.0`, `CMY_STRENGTH=1.6`, `FILMIC_K=5.0`, `FILMIC_PIVOT=0.44`, `FILMIC_WHITE_T=1.05`, tone gains `0.20`(whites/blacks)/`0.18`(highlights/shadows), `BRIGHTNESS_DENSITY_RANGE=0.5`, `INV_EPS=1e-5`, `LOG10=0.30102999566`, plus the OKLab matrices, color-mixer band centres, and point-color tolerances.
- **Build/test on Windows:** `npm run test:unit` (vitest) for frontend units; visual/HDR acceptance only on the physical Windows HDR display (SSH/RDP can't show HDR). Build via VS Dev Shell + `LIBCLANG_PATH` (`C:\Users\calen\run-dev.ps1`).
- **Commit discipline** — work on `main`; `git add <exact paths>` only; never `-A`/`.`/`app`/`crates` (user keeps long-lived WIP in `app/src-tauri/*.rs`). This plan touches only `app/src/lib/**` files.

---

### Task 1: WebGPU capability probe + Windows gating

**Files:**
- Modify: `app/src/lib/viewport/hdrCapability.ts`
- Test: `app/src/lib/viewport/hdrCapability.test.ts` (create if absent; else add cases)

**Interfaces:**
- Consumes: nothing new.
- Produces: `probeWebGpuExtended(): Promise<boolean>` (async, environment-touching); `detectHdrMode(env)` unchanged signature but now Windows can pass `surfaceSupported=true`; `probeHdrEnv()` sets Windows `surfaceSupported` from `probeWebGpuExtended()`; **export the existing `detectOs()`** (currently module-private) so Task 4 can branch the live-edr drive by OS.

- [ ] **Step 1: Write the failing test** — add to `hdrCapability.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { detectHdrMode } from "./hdrCapability";

describe("detectHdrMode — Windows WebGPU", () => {
  it("windows + hdr display + webgpu surface → live-edr", () => {
    expect(detectHdrMode({ os: "windows", displayHdr: true, surfaceSupported: true })).toBe("live-edr");
  });
  it("windows + hdr display + no webgpu → gainmap-fallback", () => {
    expect(detectHdrMode({ os: "windows", displayHdr: true, surfaceSupported: false })).toBe("gainmap-fallback");
  });
  it("windows + webgpu but SDR display → gainmap-fallback", () => {
    expect(detectHdrMode({ os: "windows", displayHdr: false, surfaceSupported: true })).toBe("gainmap-fallback");
  });
  it("macos still live-edr; linux still hidden", () => {
    expect(detectHdrMode({ os: "macos", displayHdr: true, surfaceSupported: true })).toBe("live-edr");
    expect(detectHdrMode({ os: "linux", displayHdr: true, surfaceSupported: true })).toBe("hidden");
  });
});
```

- [ ] **Step 2: Run test to verify current state**

Run: `cd app && npx vitest run src/lib/viewport/hdrCapability.test.ts`
Expected: The four `detectHdrMode` cases PASS already (the rule table in `detectHdrMode` is unchanged — it always honored `surfaceSupported`). This test locks the behavior in before the probe change. If any fail, stop and reconcile.

- [ ] **Step 3: Add the WebGPU probe + wire Windows**

In `app/src/lib/viewport/hdrCapability.ts`, add the probe and use it for Windows (macOS stays `true`, Linux irrelevant — `hidden`):

```ts
/** True iff WebGPU is present AND an rgba16float canvas can be configured with
 *  toneMapping:'extended' (the EDR mechanism). Environment-touching; never throws. */
export async function probeWebGpuExtended(): Promise<boolean> {
  try {
    if (typeof navigator === "undefined" || !("gpu" in navigator) || !navigator.gpu) return false;
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) return false;
    const device = await adapter.requestDevice();
    const cv = typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(2, 2) : null;
    const ctx = cv?.getContext("webgpu") as GPUCanvasContext | null;
    if (!ctx) { device.destroy?.(); return false; }
    ctx.configure({ device, format: "rgba16float", alphaMode: "opaque", toneMapping: { mode: "extended" } } as GPUCanvasConfiguration);
    ctx.unconfigure?.();
    device.destroy?.();
    return true;
  } catch {
    return false;
  }
}
```

Then change `probeHdrEnv()`'s `surfaceSupported` line from `const surfaceSupported = os === "macos" ? true : false;` to:

```ts
  const surfaceSupported =
    os === "macos" ? true
    : os === "windows" ? await probeWebGpuExtended()
    : false;
```

(`GPUCanvasConfiguration`'s `toneMapping` may be missing from the installed `@webgpu/types`; if TS errors, the `as GPUCanvasConfiguration` cast above already narrows it — keep the cast.)

- [ ] **Step 4: Run tests**

Run: `cd app && npx vitest run src/lib/viewport/hdrCapability.test.ts`
Expected: PASS (all `detectHdrMode` cases green; the probe is not unit-tested — it is thin environment wiring per the file's own doc comment).

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/viewport/hdrCapability.ts app/src/lib/viewport/hdrCapability.test.ts
git commit -m "feat(hdr-win): WebGPU extended-tonemapping probe gates Windows live-edr"
```

---

### Task 2: WGSL shaders (port from MSL) + constants-parity test

**Files:**
- Create: `app/src/lib/viewport/webgpu/shaders.ts` (exports WGSL source strings)
- Test: `app/src/lib/viewport/webgpu/shaders.test.ts`

**Interfaces:**
- Consumes: the canonical MSL in `app/src-tauri/src/hdr_surface/msl.rs` (`HDR_UNIFORMS_STRUCT_MSL`, `INVERT_FRAG_MSL`, `FINISH_FRAG_MSL`) and the `HdrUniforms` field order in `app/src-tauri/src/hdr_surface/uniforms.rs`.
- Produces: `export const WGSL_UNIFORMS: string`, `export const INVERT_WGSL: string`, `export const FINISH_WGSL: string` — WGSL source for the uniform struct, the invert fragment stage, and the finish fragment stage (finish includes `hdr_finalize_scalar`, `hdr_finish`, the clip overlay, and `srgbToLinearExt3` on output).

**Port method (read `msl.rs` as the source of truth and apply these MSL→WGSL rules — do NOT invent the math; mirror it line-for-line):**
- Types: `float`→`f32`, `float2/3/4`→`vec2f/vec3f/vec4f`, `float3x3`→`mat3x3<f32>`, `float2x2`→`mat2x2<f32>`, `int`→`i32`, `bool` stays. Struct fields keep names/order from `HdrUniforms` (uniforms.rs) so the JS packer (Task 3) can match offsets.
- Functions: `float f(float x){...}` → `fn f(x: f32) -> f32 {...}`; `constant float K = ...;` → `const K: f32 = ...;`. `return` unchanged.
- Builtins: `mix`, `clamp`, `max`, `min`, `abs`, `pow`, `exp`, `log`, `tanh`, `smoothstep`, `floor`, `fract`, `dot`, `normalize` are all WGSL builtins (same names). `mix(a,b,t)` and `smoothstep(e0,e1,x)` match. `pow(x,y)` matches. MSL `metal::` qualifiers dropped.
- Matrix math: MSL `M * v` (column-major) → WGSL `M * v` (also column-major) — same. Build `mat3x3<f32>(c0, c1, c2)` from column vectors, matching how the packer lays out `m_pre`/`m_post`/`orient`.
- Texture sampling: MSL `tex.sample(s, uv)` → WGSL `textureSample(tex, s, uv)` (fragment stage). The source texture is `texture_2d<f32>`; the LUT is `texture_2d<f32>` sampled at `(x, 0.5)`.
- Uniform access: MSL `u.field` → WGSL `u.field` where `u` is `var<uniform> u: HdrUniforms` in `@group(0) @binding(0)`.
- Entry points: write a shared full-screen-triangle vertex stage (`@vertex`) emitting a `uv` in `[0,1]`; the invert stage is `@fragment fn fs_invert(...) -> @location(0) vec4f`; the finish stage `@fragment fn fs_finish(...) -> @location(0) vec4f`. Output of finish is `vec4f(srgbToLinearExt3(outc), 1.0)` exactly as MSL `finish_frag` does.
- Keep the geometry helper `hdr_source_uv` (invert stage) and every finish sub-function (`applyPerZoneWb`, `toneBody`, `shoulderOnly`, `lookS`, `displayFinalize`, `colorGrade`, color-mixer HSL fns, point color, OKLab saturation, `sampleLut`, `hdr_finalize_scalar`, `hdr_finish`, `clipOverlay`) — one WGSL fn per MSL fn, same names, same constants.

- [ ] **Step 1: Write the failing test** — `app/src/lib/viewport/webgpu/shaders.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { WGSL_UNIFORMS, INVERT_WGSL, FINISH_WGSL } from "./shaders";

describe("WGSL shaders — constants parity with the canonical pipeline", () => {
  const all = INVERT_WGSL + "\n" + FINISH_WGSL;
  // Values copied from crates/film-core/src/{engine,finish}.rs — MUST match.
  const consts: Record<string, string> = {
    HDR_KNEE: "0.8", HDR_HEADROOM: "2.5", HDR_W_HI: "1.2",
    FAITHFUL_GAMMA: "1.590", FAITHFUL_KNEE: "0.892",
    LOOK_K: "2.0", EXPO_K: "0.14", CMY_STRENGTH: "1.6",
    FILMIC_K: "5.0", FILMIC_PIVOT: "0.44", FILMIC_WHITE_T: "1.05",
  };
  for (const [name, val] of Object.entries(consts)) {
    it(`declares const ${name} = ${val}`, () => {
      // matches e.g.  const HDR_KNEE: f32 = 0.8;
      const re = new RegExp(`const\\s+${name}\\s*:\\s*f32\\s*=\\s*${val.replace(".", "\\.")}\\b`);
      expect(all).toMatch(re);
    });
  }
  it("finish output linearizes via srgbToLinearExt3", () => {
    expect(FINISH_WGSL).toMatch(/srgbToLinearExt3\s*\(/);
  });
  it("finish contains the chroma-preserving hdr_finish", () => {
    expect(FINISH_WGSL).toMatch(/fn\s+hdr_finish\s*\(/);
    expect(FINISH_WGSL).toMatch(/fn\s+hdr_finalize_scalar\s*\(/);
  });
  it("uniforms struct declares the invert + finish fields", () => {
    for (const f of ["base", "wb", "d_max", "mode", "tone_mode", "contrast", "saturation", "crop_off", "orient"]) {
      expect(WGSL_UNIFORMS).toMatch(new RegExp(`\\b${f}\\b`));
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && npx vitest run src/lib/viewport/webgpu/shaders.test.ts`
Expected: FAIL — module `./shaders` not found.

- [ ] **Step 3: Write the WGSL** — create `app/src/lib/viewport/webgpu/shaders.ts` exporting `WGSL_UNIFORMS`, `INVERT_WGSL`, `FINISH_WGSL`, porting `msl.rs`'s three shader strings using the rules above. Declare each shared constant as `const NAME: f32 = <value>;` using the exact values in the parity test / Global Constraints. `INVERT_WGSL` and `FINISH_WGSL` should each `@include` (string-concatenate) `WGSL_UNIFORMS` + the vertex stage so they compile standalone.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app && npx vitest run src/lib/viewport/webgpu/shaders.test.ts`
Expected: PASS (all constants present, `hdr_finish`/`srgbToLinearExt3` present, uniform fields present). NOTE: WGSL cannot be *compiled* in the vitest/Node environment — actual shader compilation is verified at runtime on the Windows box in Task 6.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/viewport/webgpu/shaders.ts app/src/lib/viewport/webgpu/shaders.test.ts
git commit -m "feat(hdr-win): WGSL invert+finish+hdr_finish ported from MSL + constants-parity test"
```

---

### Task 3: `WebGPUFinishRenderer` + uniform packer

**Files:**
- Create: `app/src/lib/viewport/webgpu/renderer.ts`
- Create: `app/src/lib/viewport/webgpu/uniforms.ts` (the ArrayBuffer packer)
- Test: `app/src/lib/viewport/webgpu/uniforms.test.ts`

**Interfaces:**
- Consumes: `WGSL_UNIFORMS`/`INVERT_WGSL`/`FINISH_WGSL` (Task 2); the WebGL renderer's public method shapes in `app/src/lib/viewport/gl/renderer.ts` (`setSourceFloat(pixels: Uint16Array, w, h): boolean`, `setGeometry({crop_off, crop_scale, angle, aspect, orient, raw, outW, outH})`, `setUniforms(FinishUniforms)`, `setLut`, `setColorGrade`, `setColorMix`, `setPerZoneWb`) — mirror these names/params so `Viewport` can drive either renderer identically; the `FinishUniforms`/`ColorGrade`/`ColorMix`/per-zone types exported from `gl/renderer.ts` or its shaders module.
- Produces:
  - `packHdrUniforms(inputs): ArrayBuffer` in `uniforms.ts` — packs the resolved finish+invert+geometry values into the byte layout of the WGSL `HdrUniforms` struct (16-byte alignment: scalars pack into vec4 lanes, `vec3f` fields align to 16 and consume 12+4 pad, `mat3x3<f32>` is 3× 16-byte columns, arrays stride 16). The field order MUST match `WGSL_UNIFORMS`.
  - `class WebGPUFinishRenderer` in `renderer.ts` — `static async create(canvas): Promise<WebGPUFinishRenderer | null>` (null if WebGPU init fails); instance methods mirroring the WebGL renderer (`setSourceFloat`, `setGeometry`, `setUniforms`, `setLut`, `setColorGrade`, `setColorMix`, `setPerZoneWb`, `render()`, `dispose()`).

- [ ] **Step 1: Write the failing test** — `app/src/lib/viewport/webgpu/uniforms.test.ts` (the packer is the only unit-testable piece; the renderer needs a GPU):

```ts
import { describe, it, expect } from "vitest";
import { packHdrUniforms, HDR_UNIFORMS_BYTES } from "./uniforms";

describe("packHdrUniforms", () => {
  it("produces a 16-byte-aligned buffer of the fixed struct size", () => {
    const buf = packHdrUniforms(SAMPLE); // SAMPLE = a full inputs fixture (all fields set)
    expect(buf.byteLength).toBe(HDR_UNIFORMS_BYTES);
    expect(buf.byteLength % 16).toBe(0);
  });
  it("writes scalar fields at their declared offsets", () => {
    const buf = packHdrUniforms({ ...SAMPLE, contrast: 0.5, d_max: 1.5 });
    const f = new Float32Array(buf);
    // OFF_CONTRAST / OFF_D_MAX are exported byte offsets / 4; assert the values land there.
    expect(f[OFF_CONTRAST / 4]).toBeCloseTo(0.5);
    expect(f[OFF_D_MAX / 4]).toBeCloseTo(1.5);
  });
});
```

(Define `SAMPLE`, `OFF_CONTRAST`, `OFF_D_MAX` in the test from the offsets `uniforms.ts` exports; keep the fixture minimal but with every field present.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && npx vitest run src/lib/viewport/webgpu/uniforms.test.ts`
Expected: FAIL — module `./uniforms` not found.

- [ ] **Step 3: Implement `uniforms.ts`** — mirror the field order of `WGSL_UNIFORMS`/`msl.rs`'s `HdrUniforms`. Export `HDR_UNIFORMS_BYTES` (the total aligned size) and a byte-offset constant per field. `packHdrUniforms` writes into a `new ArrayBuffer(HDR_UNIFORMS_BYTES)` via `DataView`/typed arrays at those offsets, applying the same resolution the WebGL path already applies to its uniforms (the inputs are the already-resolved values `Viewport` hands the WebGL renderer — sliders already divided by 100, etc.; this packer only lays them out, it does not re-resolve).

- [ ] **Step 4: Implement `renderer.ts`** — `WebGPUFinishRenderer`:
  - `create(canvas)`: `requestAdapter`/`requestDevice`; `ctx = canvas.getContext('webgpu')`; `ctx.configure({ device, format: 'rgba16float', alphaMode: 'opaque', toneMapping: { mode: 'extended' } })`; build two render pipelines (`INVERT_WGSL` → intermediate `rgba16float` texture; `FINISH_WGSL` → canvas), a linear sampler, the uniform buffer (`HDR_UNIFORMS_BYTES`, `usage: UNIFORM | COPY_DST`), the source texture (created/resized on `setSourceFloat`), and the LUT texture (256×1 `rgba8unorm`). Return `null` on any failure.
  - `setSourceFloat(pixels, w, h)`: (re)create the `rgba16float` source texture at w×h, `queue.writeTexture` the `Uint16Array` (16 bytes/row-px). Return `false` on size 0.
  - `setGeometry`/`setUniforms`/`setLut`/`setColorGrade`/`setColorMix`/`setPerZoneWb`: stash inputs; on the next `render()` call `packHdrUniforms(...)` and `queue.writeBuffer` the uniform buffer, and upload the LUT if changed.
  - `render()`: encode invert pass (draw 3) → intermediate; finish pass (draw 3) sampling intermediate+LUT → canvas; submit. Guard if no source yet.
  - `dispose()`: destroy textures/buffers/device.

- [ ] **Step 5: Run tests**

Run: `cd app && npx vitest run src/lib/viewport/webgpu/uniforms.test.ts`
Expected: PASS (packer offsets/size correct). The renderer class is exercised on-device in Task 6 (needs a real GPU).

- [ ] **Step 6: Commit**

```bash
git add app/src/lib/viewport/webgpu/renderer.ts app/src/lib/viewport/webgpu/uniforms.ts app/src/lib/viewport/webgpu/uniforms.test.ts
git commit -m "feat(hdr-win): WebGPUFinishRenderer + HdrUniforms ArrayBuffer packer"
```

---

### Task 4: Viewport swap wiring (drive WebGPU on Windows live-edr)

**Files:**
- Modify: `app/src/lib/viewport/Viewport.svelte`

**Interfaces:**
- Consumes: `WebGPUFinishRenderer` (Task 3); existing `$hdrMode`/`liveEdr` gating, `hdrGeom()`, `hdrViewSpec()`, `clipArg()`, `pushHdrSource()`/`pushHdrUniforms()`/`scheduleHdrUniforms()` (`Viewport.svelte:505/403/498/524/546/555`), and the values already assembled to drive the WebGL renderer (source `Uint16Array`, geometry, `FinishUniforms`, LUT, cg/cm/pz).
- Produces: an OS-branched live-edr path; a `<canvas>` sibling for WebGPU shown when the WebGPU HDR preview is active.

- [ ] **Step 1: Add a WebGPU canvas + renderer handle.** Add a sibling `<canvas bind:this={gpuCanvas}>` next to the WebGL `canvas` (same CSS box/size), hidden by default. Add `let gpuRenderer: WebGPUFinishRenderer | null = null;` and a reactive `isWinLiveEdr = liveEdr && detectOs() === "windows"` (import the now-exported `detectOs` from `hdrCapability.ts`, Task 1). Keep `liveEdr` semantics; only the *drive target* branches.

- [ ] **Step 2: Branch source/uniform pushes.** Where `pushHdrSource()` and `pushHdrUniforms()` currently call `api.hdrSurfaceSetSource`/`api.hdrSurfaceSetUniforms` (macOS native), branch: if `isWinLiveEdr`, instead call the local `gpuRenderer` — create it lazily via `WebGPUFinishRenderer.create(gpuCanvas)`, `setSourceFloat(...)` with the same source pixels the WebGL path uses, `setGeometry(hdrGeom()+outW/outH)`, `setUniforms/setLut/setColorGrade/setColorMix/setPerZoneWb(...)` with the same resolved values, then `gpuRenderer.render()`. If `create` returns null (WebGPU init failed at runtime), set a flag that forces `hdrMode='gainmap-fallback'` for the session and fall back. macOS branch unchanged.

- [ ] **Step 3: Swap visibility.** When the WebGPU preview is active (`isWinLiveEdr && params.hdr && gpuRenderer`), hide the WebGL `canvas` and show `gpuCanvas`; otherwise show WebGL and hide `gpuCanvas`. Keep the existing rAF coalescing (`scheduleHdrUniforms`) — on Windows it calls `gpuRenderer.render()` after packing uniforms, per frame.

- [ ] **Step 4: Teardown.** On HDR off / image switch / component destroy, `gpuRenderer?.dispose()` and clear the handle; ensure the WebGL canvas is shown again (mirrors the existing `hdrSurfaceHide` teardown at `Viewport.svelte:566`).

- [ ] **Step 5: Manual/type check.**

Run: `cd app && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | tail -20`
Expected: no new type errors from `Viewport.svelte`. (Visual behavior is verified in Task 6 — this step only guards types/compile.)

- [ ] **Step 6: Commit**

```bash
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(hdr-win): drive WebGPUFinishRenderer on Windows live-edr (canvas swap)"
```

---

### Task 5: DOM marker overlay (dust/eraser markers visible in HDR)

**Files:**
- Create: `app/src/lib/viewport/HdrMarkerOverlay.svelte`
- Modify: `app/src/lib/viewport/Viewport.svelte` (mount the overlay when the WebGPU HDR preview is active)

**Interfaces:**
- Consumes: the dust/eraser marker positions + the viewport geometry the current markers use (find how markers are drawn today — search `Viewport.svelte` and its GL layer for the dust/eraser marker draw + their normalized coordinates; reuse that source of truth and the same geometry mapping used for `hdrGeom()`).
- Produces: a geometry-driven DOM overlay component; absolutely-positioned marker elements above the preview canvas.

- [ ] **Step 1: Identify the marker data.** Locate where dust/eraser markers are currently produced (the store/props feeding the GL marker draw) and the normalized→screen mapping. Document the exact source in the component header comment.

- [ ] **Step 2: Build `HdrMarkerOverlay.svelte`.** Props: the marker list (normalized coords) + the geometry/rect needed to map normalized → CSS px (same mapping the GL overlay uses). Render one absolutely-positioned element per marker (a small ring/dot matching the existing marker style) inside a `pointer-events:none` layer sized to the canvas box.

- [ ] **Step 3: Mount conditionally.** In `Viewport.svelte`, render `<HdrMarkerOverlay .../>` only when the WebGPU HDR preview is active (`isWinLiveEdr && params.hdr && gpuRenderer`), positioned over `gpuCanvas`. Do NOT mount it in SDR/WebGL mode (WebGL still draws its own markers there).

- [ ] **Step 4: Type check.**

Run: `cd app && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | tail -20`
Expected: no new type errors.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/viewport/HdrMarkerOverlay.svelte app/src/lib/viewport/Viewport.svelte
git commit -m "feat(hdr-win): DOM marker overlay so dust/eraser markers show in HDR preview"
```

---

### Task 6: On-device acceptance + parity (USER)

Not a code task — the real gate, run on the Windows HDR display after Tasks 1–5 ship. Iterate: implement on Mac → commit/push → `git pull` on the box → `& "C:\Users\calen\run-dev.ps1"`.

Checklist:
- [ ] App launches; toggling **HDR on** shows the WebGPU canvas; highlights **glow** on the HDR display.
- [ ] **Live smoothness:** dragging sliders updates the HDR preview per-frame (no settle/lag).
- [ ] **Parity:** with headroom visually neutral (or a debug toggle), the WebGPU preview matches the WebGL SDR preview — no color/tone drift.
- [ ] **Clip overlay** shows (in-shader); **dust/eraser markers** visible via the DOM overlay.
- [ ] Toggling **HDR off** returns cleanly to the WebGL canvas; switching images works.
- [ ] On an SDR display or a machine without WebGPU-extended, it **falls back** to gain-map with no errors.

---

## Notes for the executor

- Read the canonical sources before porting: `app/src-tauri/src/hdr_surface/msl.rs` (shader math), `app/src-tauri/src/hdr_surface/uniforms.rs` (field order/layout), `app/src/lib/viewport/gl/renderer.ts` (the renderer interface to mirror), `app/src/lib/viewport/Viewport.svelte:393-588` (the live-edr drive loop to branch).
- WGSL cannot be compiled in vitest/Node; shader-compile + visual correctness are verified on-device (Task 6). Unit tests cover the pure/​layout pieces (gating rules, constants parity, uniform packing).
- The frontend already assembles every value the WebGPU renderer needs (it drives WebGL with them); if any value turns out to be missing (e.g. `base`/`d_max`/`cam_balance`/LUT not reachable at the branch point), surface it — do not fetch new data from Rust without checking the WebGL path first, since it must already have it.
