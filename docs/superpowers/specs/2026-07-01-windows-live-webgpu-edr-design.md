# Windows Live HDR — in-webview WebGPU EDR surface

**Date:** 2026-07-01
**Status:** Design approved — ready for implementation plan
**Parent effort:** Live HDR. macOS sub-projects A (native CAMetalLayer EDR surface), B (per-frame live Metal), C (chroma-preserving finish + gain-map export), and HDR thumbnails are COMPLETE on `main`. This brings **live** HDR to Windows.

---

## Background

Today HDR mode is gated in `app/src/lib/viewport/hdrCapability.ts`: `surfaceSupported = os === "macos" ? true : false`, so **Windows falls back to the static gain-map path** (`gainmap-fallback`) — it renders and *settles*, and cannot update smoothly while editing. On-device validation (2026-07-01, Windows 11 build 26200, Chromium/WebView2 149) confirmed that an in-webview WebGPU canvas with `rgba16float` + `toneMapping:{mode:'extended'}` produces **real EDR headroom** (a 4-band 0.5/1.0/2.0/4.0 spike glowed past SDR white in extended mode, clipped in standard mode). WebView2 149 ≥ the Chromium 128 floor for `toneMapping:'extended'`.

The macOS live path (sub-project B) is a **native** CAMetalLayer behind the webview, driven by Rust commands (`hdr_surface_set_source` / `set_uniforms`) running MSL shaders. Windows cannot use that (no native HDR canvas needed — WebGPU works *inside* the webview), so the Windows live path is a **frontend** WebGPU renderer, not a native surface.

**Decisions locked in brainstorming:**
- **Swap model:** when HDR is on (Windows), the WebGPU canvas replaces the WebGL canvas — one visible preview at a time.
- **DOM-overlay markers now:** dust/eraser markers render as a DOM layer above the preview canvas so they stay visible in HDR (WebGL currently GPU-draws them into its own canvas, which the swap would hide).
- **Full-pipeline parity:** the WebGPU preview must match the WebGL SDR preview exactly *plus* add headroom — a partial port would visibly mismatch on saturation/grade, so the whole finish is ported.
- **Frontend-only:** no new Rust, no FFI, no native compositing. Reuses the exact frontend data that already drives the WebGL renderer.

---

## Goal

On Windows, when the display is HDR and WebGPU supports extended tone mapping, the develop preview renders **live** (per-frame, no settle) in HDR through an in-webview WebGPU EDR canvas at **parity with the macOS surface** and with the WebGL SDR preview. Where WebGPU/HDR is unavailable, behavior is unchanged (gain-map fallback). macOS and Linux paths are untouched.

**Non-goals:** macOS native path changes; Linux HDR (still `hidden`); new Rust/backend code; consolidating the shader copies into one source (larger refactor, deferred); changing the exported gain-map/thumbnail behavior (already works on Windows).

---

## Architecture

Entirely frontend (TypeScript + WGSL). A new **`WebGPUFinishRenderer`** parallels the existing WebGL `FinishRenderer` (`app/src/lib/viewport/gl/renderer.ts`): same driving interface (`setSourceFloat`, `setGeometry`, `setUniforms`, `render`), but it targets an `rgba16float` canvas configured `toneMapping:{mode:'extended'}` and runs the full pipeline **including `hdr_finish`** (chroma-preserving highlight extension, which the GLSL `FRAG` omits). It is driven by the **same** frontend data that already feeds WebGL — RGBA16F source texture, `buildGeometry()` output, resolved finish/invert uniforms, and the 256×1 tone LUT — so there is no new data path and no Rust round-trip.

### Components

1. **`app/src/lib/viewport/webgpu/shaders.ts`** — WGSL shader source (as TS strings), the port of `INVERT_FRAG_MSL` + `FINISH_FRAG_MSL` + the `HdrUniforms` struct from `app/src-tauri/src/hdr_surface/msl.rs`. **Derived from the existing MSL**, which is the only shader version that already contains `hdr_finish` + the in-shader clip overlay. Includes: the invert (all 4 modes, Filmic/Faithful), the finish (per-zone WB → brightness → tone body → display finalize → OKLab saturation → LUT → color grade → color mixer → point color), `hdr_finalize_scalar` + `hdr_finish`, `srgbToLinearExt3` output, and the clip overlay.

2. **`app/src/lib/viewport/webgpu/renderer.ts`** — `WebGPUFinishRenderer` class. Requests the adapter/device, configures the canvas context (`format: 'rgba16float'`, `toneMapping: { mode: 'extended' }`, `alphaMode: 'opaque'`), creates the invert + finish render pipelines, uploads the source texture (RGBA16F) and LUT (256×1), and packs a single `HdrUniforms` uniform buffer whose byte layout matches the WGSL struct (16-byte / std140 alignment, mirroring the macOS `#[repr(C, align(16))]` `HdrUniforms`). Exposes the same method surface the WebGL renderer does so `Viewport` can drive either identically. Two-pass like WebGL: invert → intermediate RGBA16F texture → finish → canvas.

3. **`app/src/lib/viewport/hdrCapability.ts`** — make Windows capability real and async:
   - Add an async WebGPU probe: `navigator.gpu` present AND an offscreen `rgba16float` + `toneMapping:'extended'` `configure()` succeeds (no throw) AND `requestAdapter()`/`requestDevice()` resolve.
   - `detectHdrMode`: `os==='windows' && displayHdr && surfaceSupported → 'live-edr'`. `surfaceSupported` for Windows is the WebGPU-probe result (was hardcoded `false`). macOS unchanged (`true`), Linux `'hidden'`.

4. **`app/src/lib/viewport/Viewport.svelte`** — branch the existing `liveEdr` path by OS. macOS `live-edr` → the native surface commands (`hdrSurfaceSetSource`/`hdrSurfaceSetUniforms`), unchanged. Windows `live-edr` → drive the `WebGPUFinishRenderer` in-process on the same rAF loop that today schedules `set_uniforms`. The swap: when the WebGPU preview is active, hide the WebGL canvas and show the WebGPU canvas (a sibling `<canvas>` — WebGL and WebGPU cannot share one context).

5. **DOM marker overlay** — a Svelte component that draws dust/eraser markers as absolutely-positioned DOM elements above the preview canvas, using the same viewport geometry the markers use today. Active when the WebGPU HDR preview is shown. Built platform-agnostic (geometry-driven), so it can later also address the macOS "markers not shown on the EDR surface" limitation; that macOS wiring is out of scope here.

### Data flow

Per-frame, identical to WebGL: `Viewport` already has the RGBA16F source pixels, `buildGeometry()` geometry, all resolved finish/invert uniform values, the clip toggles, and the tone LUT (it must, to drive the WebGL renderer today). The WebGPU renderer consumes the same values; the only new work is packing them into a WebGPU uniform buffer (vs individual `glUniform` calls). Source uploads are rare (on `id|developRev|tier` change); uniforms/geometry update per frame via rAF. Result: live HDR while dragging, no 200 ms settle.

---

## Shader duplication — primary risk, mitigated

This adds a **third** copy of the pipeline math (GLSL `shaders.ts` + MSL `msl.rs` + new WGSL) against the Rust canonical (`engine.rs` invert, `finish.rs` finish/`hdr_finish`), sharing ~30+ constants (HDR_KNEE=0.8, HDR_HEADROOM=2.5, HDR_W_HI=1.2, FAITHFUL_GAMMA=1.590, FAITHFUL_KNEE=0.892, FAITHFUL_SCALE=1/0.700, LOOK_K=2.0, EXPO_K=0.14, CMY_STRENGTH=1.6, FILMIC_K=5.0, FILMIC_PIVOT=0.44, FILMIC_WHITE_T=1.05, tone gains 0.20/0.18, OKLab matrices, color-mixer band centres, point-color tolerances, …).

Mitigation:
- **Derive from MSL, not from scratch.** WGSL and MSL are both C-like; the port is mechanical, and MSL is already the tested superset (has `hdr_finish` + clip overlay).
- **Constants-parity test.** A frontend unit test extracts the shared numeric constants from the canonical source (the Rust files, or the MSL strings) and asserts the WGSL carries matching values — same spirit as the existing `hdr_uniforms_layout_matches_msl` test that guards the uniform layout.
- **On-device A/B parity.** With HDR headroom disabled, the WebGPU output must match the WebGL preview pixel-for-pixel (same invert+finish), confirming no drift crept into the port.

Full consolidation (a single source generating GLSL/MSL/WGSL) is explicitly out of scope — a larger refactor for a later effort.

---

## Testing

- **Frontend unit tests** (`vitest`, `npm run test:unit`):
  - `detectHdrMode`: Windows + `displayHdr` + `surfaceSupported=true` → `'live-edr'`; Windows without WebGPU (`surfaceSupported=false`) → `'gainmap-fallback'`; macOS/Linux branches unchanged.
  - Uniform-buffer packer: given a params fixture, the packed `ArrayBuffer` has the expected field offsets/values for the WGSL `HdrUniforms` layout (alignment correctness).
  - Constants-parity guard: WGSL shader strings contain the canonical constant values.
- **On-device (the real gate, Windows HDR monitor):**
  - Live HDR updates **smoothly while dragging** sliders (no settle) with headroom glow.
  - **WebGPU-SDR == WebGL** pixel-for-pixel (headroom disabled) — parity.
  - Clip-overexposure overlay shows; **dust/eraser markers visible** via the DOM overlay.
  - Toggling HDR off cleanly returns to the WebGL canvas.
  - On a machine/display without WebGPU-extended or HDR, it **falls back** to gain-map with no regression.

---

## Decomposition (tasks for the implementation plan)

1. **WebGPU capability probe + gating** — async `navigator.gpu` + extended-config probe in `hdrCapability.ts`; flip Windows `surfaceSupported`; `detectHdrMode` unit tests.
2. **WGSL shaders** — port `HdrUniforms` struct + `INVERT_FRAG_MSL` + `FINISH_FRAG_MSL` (incl. `hdr_finish`, clip overlay, `srgbToLinearExt3`) to WGSL; constants-parity test.
3. **`WebGPUFinishRenderer`** — device/context/pipelines, source+LUT upload, uniform-buffer packer (with unit test), two-pass render.
4. **Viewport swap wiring** — OS branch of `liveEdr`; show/hide WebGPU vs WebGL canvas; drive the renderer on the rAF loop with the existing epoch/source-key logic.
5. **DOM marker overlay** — dust/eraser markers as a geometry-driven DOM layer above the preview, active in HDR mode.
6. **On-device acceptance + parity tuning** (USER) — smoothness, WebGPU-SDR==WebGL parity, overlays, fallback.

---

## Constraints (Global)

- **Frontend only** — no new Rust/backend code; reuse the WebGL renderer's existing data (source texture, geometry, resolved uniforms, LUT).
- **Windows-scoped** — macOS native path and Linux (`hidden`) untouched; pure capability upgrade gated on the WebGPU probe (no regression where unavailable).
- **Parity is the bar** — WebGPU-SDR must equal WebGL pixel-for-pixel; the WGSL mirrors the MSL/Rust math exactly (shared constants identical). No divergent 4th copy of the logic — WGSL derives from MSL.
- **`toneMapping:'extended'` + `rgba16float`** — the EDR mechanism; probe before enabling; `srgbToLinearExt3` on output (extended-linear surface).
- **Swap, not overlay** — one preview canvas visible at a time (WebGL for SDR, WebGPU for HDR).
- **Dev/build on Windows** — box `calen@192.168.1.134`, repo `C:\Users\calen\filmrev`; build via VS Dev Shell + `LIBCLANG_PATH`; iterate by push-from-Mac / pull-on-box + `npm run tauri dev`. Visual HDR acceptance is on the physical Windows HDR display (SSH/RDP can't show HDR).
