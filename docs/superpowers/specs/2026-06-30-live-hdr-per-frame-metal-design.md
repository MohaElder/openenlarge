# Live HDR — Sub-project B: Per-frame live rendering (Metal/MSL)

**Date:** 2026-06-30
**Status:** Design approved — ready for implementation plan
**Parent effort:** Live HDR preview. Sub-project A (macOS EDR display surface, settle-based) is COMPLETE and shipped on `main`.

---

## Background

Sub-project A gave macOS a true-EDR native `CAMetalLayer` surface, but it is **settle-based**: on gesture-end (200 ms debounce) the Rust backend CPU-renders the finished image and uploads an `rgba16f` buffer, which a 20-line MSL "blit" shader samples. Editing latency is ~300 ms, and during a gesture the app hides the EDR surface and shows the live SDR WebGL canvas as a crutch.

The SDR WebGL path (`app/src/lib/viewport/gl/renderer.ts` + `shaders.ts`) is already the model we want: it uploads the raw negative **once**, then applies invert + finish **per-frame from uniforms** (~1–2 ms, 60 fps). Sub-project B gives the Metal EDR surface the same uniform-driven pipeline, so HDR editing is live.

**Decisions locked in brainstorming:**
- **Shading stack: raw Metal / MSL** (not wgpu/WGSL). Consistent with the existing native surface (objc2/Metal, no new deps) and the codebase's existing "keep Rust + GLSL finish in sync" pattern. Adds a 3rd copy of the finish math; a future Windows/WebGPU path will need its own WGSL port (accepted).
- **Live scope: all sliders** (invert + finish). Upload the raw negative once; do geometry + invert + finish per-frame in MSL. Every slider — exposure, WB, tone, color — updates the EDR live, like the SDR canvas.
- **B absorbs the proper HDR finish** (Sub-project C's core math): a live MSL shader cannot cleanly replicate the CPU `split-body+excess` hack, so B implements a real HDR tone mode that preserves and color-manages super-white. This retires the workaround and fixes highlight rendering. C shrinks to UI-only (headroom controls).

---

## Goal

In `live-edr` mode on macOS, every edit updates the native EDR surface **per-frame at ~60 fps** (no settle debounce, no SDR-during-gesture crutch), with a proper HDR finish that preserves color-managed highlights above white. SDR behavior and non-macOS platforms are unchanged.

**Non-goals (out of scope):**
- Windows/WebGPU live path (still deferred; needs a separate WGSL port).
- Sub-project C's UI work (tone-curve/exposure controls that visually extend into the >1.0 region).
- Per-frame *invert-source* re-computation beyond what WebGL already does (geometry stays in-shader; source re-upload only on the rare events listed below).
- Gain-map `<img>` export/fallback path — unchanged.

---

## Architecture

The EDR surface becomes a full pipeline renderer, 1:1 with the WebGL renderer.

### Source-texture lifecycle (rare uploads)
The Metal texture holds the **raw negative** (float RGBA16F), the same data WebGL's `setSourceFloat` uploads. It is (re)uploaded native-side (Rust renders/holds the proxy and writes the Metal texture directly — no JS pixel round-trip) only on:
- image load / switch,
- develop (re-decode) / channel-balance / base recalibration,
- proxy-resolution change,
- the hi-res-on-zoom threshold crossing (deep zoom uploads a hi-res source, mirroring the WebGL behavior).

Geometry (crop, straighten, orient, zoom-window pan/scale) is applied **in-shader** from the full source via uniforms — so pan/zoom is a uniform update, NOT a re-upload.

### The MSL pipeline (`app/src-tauri/src/hdr_surface/macos.rs`)
Replace the 20-line blit shader with a port of `INVERT_FRAG` + `FRAG` (~500 lines MSL total):
- **Invert stage** (port of `shaders.ts:409-617`): geometry transform + per-channel log-density inversion (4 modes; filmic/faithful), UNCLAMPED output (super-white preserved).
- **Finish stage** (port of `shaders.ts:16-332`): per-zone WB (no early clamp) → brightness/density → tone_curve → OKLab saturation → tone LUT → color grade → color mix (8-band) → point color → clipping overlay. Matches `crates/film-core/src/finish.rs::finish_pixel` and the GLSL exactly (all constants: `BRIGHTNESS_DENSITY_RANGE`, OKLab consts, `SKIN_HUE`, etc.).
- Two intermediate render targets if the texture (USM/unsharp) slider path is ported; otherwise a single pass. (USM/texture pass parity is a task-level detail — port `USM_FRAG` if the texture slider must be live, else note the limitation.)
- Inputs: a packed **uniforms buffer** (`MTLBuffer`, layout mirrored between the Rust struct and the MSL `constant HdrUniforms&`), the **256×1 tone-curve LUT texture**, and the raw-negative source texture.

### HDR finish mode (the new tone behavior — absorbs C's core)
Add a **third finalize mode: HDR**, alongside the existing Faithful-finalize and SDR-clamp. Instead of clamping to `[0,1]` or applying the SDR display-shoulder, the HDR mode applies a **soft ceiling in log-density** that maps the finished super-white body into the EDR headroom: highlights stay above 1.0, roll off smoothly toward a headroom ceiling, and remain color-managed (the shoulder is applied consistently across channels / in a luminance-aware way so highlights don't skew hue — fixing the cyan/ungraded-highlight problem at the root). This replaces the CPU `split-body+excess` workaround entirely for the live path. The exact shoulder shape is a small tone-curve design task (target: match the intent of the current `HDR_KNEE=0.8`/`HDR_HEADROOM` but color-managed and continuous with the body).

### Per-frame parameter flow
- `Viewport.svelte`'s existing `finishKey` (`:676-700`) and invert-key reactive triggers already fire on every param change and call `drawGL()` for WebGL. Add a sibling: when `hdrUsesSurface` is active, marshal the params → a new lightweight Tauri command `hdr_surface_set_uniforms(id, params, view, clip)` (params only, ~hundreds of bytes; NO pixels), **throttled to `requestAnimationFrame`** to avoid IPC flooding, which packs the uniform buffer and re-renders the layer.
- **Retire** the 200 ms `scheduleHdr` debounce and the "hide EDR / show SDR during gesture" logic for the `live-edr` path. The EDR surface stays shown and updates live.
- Source re-upload (rare events above) uses a separate command (`hdr_surface_set_source` or an extended render-show) that uploads the raw negative + sets initial uniforms.

### Coexistence with SDR WebGL
- The SDR WebGL canvas keeps rendering (hidden) in `live-edr` — it drives the histogram (`toDataURL`) and remains the SDR fallback. Both render live (~1 ms each); no visible dance.
- **Clipping-warning overlay works in HDR** now (it is part of the ported `FRAG`) — retires an A limitation. On-image dust markers: match whatever the WebGL invert pass does; if non-trivial, scope as a follow-up (documented).

---

## Data flow (live edit, live-edr mode)

1. Image opens / develops → backend uploads raw-negative float texture to the Metal surface + sets initial uniforms; surface shown.
2. User drags a slider → `finishKey`/invert-key changes → (a) WebGL `drawGL()` (hidden, for histogram); (b) rAF-throttled `hdr_surface_set_uniforms(params…)`.
3. Backend packs uniforms, updates the `MTLBuffer`, re-renders the layer on the main thread → EDR updates that frame (~2–3 ms).
4. Pan/zoom → geometry uniforms update (same path); deep-zoom threshold → source re-upload.
5. Image switch / HDR off / leave develop → hide surface (existing A teardown).

---

## Testing & parity

- **Rust unit:** the uniform-packing (Rust struct → byte layout) has a test asserting field offsets/size match the MSL `HdrUniforms` layout (catches struct-drift). The HDR tone-shoulder function has a unit test (monotonic, continuous with the body at the knee, maps to expected headroom).
- **Pixel-parity spot check:** a small Metal offscreen-render harness renders a handful of swatches + param sets through the MSL pipeline and compares to `film-core::finish_pixel` (and invert) within tolerance — catches constant mismatches. (If a Metal offscreen harness proves too heavy, fall back to documented visual A/B as the parity gate and note it.)
- **Manual GUI acceptance (the real gate):** on a real HDR display — every slider updates the EDR live at ~60 fps under drag; below-white the EDR matches the SDR canvas (toggle HDR off/on); highlights are color-managed (no cyan/hue skew); clipping warning appears in HDR; pan/zoom stays aligned and live; no regression to SDR or non-macOS.
- **Regression:** HDR off / Linux / Windows unchanged (Windows still routes to gain-map fallback).

---

## Decomposition (tasks for the implementation plan)

1. **Uniforms contract** — define the packed `HdrUniforms` Rust struct + matching MSL `constant` struct + the tone-curve LUT texture upload; unit-test the byte layout. (Foundation.)
2. **MSL invert port** — port `INVERT_FRAG` (geometry + inversion, unclamped) to MSL; wire the raw-negative source upload path; verify against the WebGL invert on swatches.
3. **MSL finish port** — port `FRAG` (all finish steps) to MSL reading the uniforms + LUT; verify parity vs `finish.rs`/GLSL.
4. **HDR finish mode** — add the super-white-preserving color-managed tone-shoulder; retire the `split-body+excess` path for live; unit-test the shoulder.
5. **Per-frame plumbing** — `hdr_surface_set_uniforms` command + source-upload command; `Viewport.svelte` rAF-throttled trigger; retire the settle debounce + gesture crutch for live-edr.
6. **Parity harness + manual acceptance** — offscreen pixel-parity spot check (or documented A/B) + the GUI acceptance pass.

---

## Constraints (Global)

- macOS-only; all new native code `#[cfg(target_os="macos")]`-gated; non-macOS compiles unchanged; Windows keeps routing to gain-map fallback.
- SDR WebGL path and its output unchanged; HDR-off unchanged.
- Native Cocoa/Metal objects main-thread-only (per A's established `with_webview` main-thread-hop + `unsafe impl Send/Sync` invariant + main-thread `Drop`).
- MSL finish must match `film-core::finish.rs` + `shaders.ts` GLSL constants exactly (parity is the correctness bar).
- Per-frame param IPC throttled to `requestAnimationFrame`; source pixels never cross IPC per-frame (upload native-side, rare).
- Commit discipline: exact-path `git add` only (user keeps WIP in `app/src-tauri/*.rs`); work on `main`.
- The macOS Tauri crate is a separate workspace: build `cd app/src-tauri && cargo build`, test `cargo test --lib`; TS `cd app && npm run test:unit` / `npm run check`.
