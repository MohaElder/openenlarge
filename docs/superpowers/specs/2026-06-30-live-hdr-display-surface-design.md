# Live HDR — Sub-project A: Display Surface (walking skeleton)

**Date:** 2026-06-30
**Status:** Design approved — ready for implementation plan
**Parent effort:** Live HDR preview (replaces the static gain-map `<img>` settle preview with a true live HDR pipeline)

---

## Background

Today's HDR feature shows HDR only as a **static gain-map JPEG** (libultrahdr, ISO 21496-1 / Apple) crossfaded in as an `<img>` overlay after a gesture settles. The live editing canvas is WebGL2 but **SDR-only**. The original design doc declared a live HDR canvas "impossible in our webview." A 2026 feasibility review found that verdict is now **outdated** and platform-dependent:

| Platform | Live HDR in-webview? | Native overlay? | Chosen path |
|---|---|---|---|
| **macOS (WKWebView)** | ❌ No HDR canvas API in WebKit at all | ✅ Native `CAMetalLayer` EDR layer (proven with wry) | **Native Metal/wgpu EDR overlay** |
| **Windows (WebView2)** | ✅ WebGPU `rgba16float` + `toneMapping:'extended'`, shipped & unflagged (Chrome 128+) | ⚠️ Possible but hard & unnecessary | **In-webview WebGPU HDR canvas** |
| **Linux (WebKitGTK)** | ❌ No HDR canvas, no desktop EDR | ❌ No EDR display API | **HDR button hidden entirely** |

The full effort decomposes into three sub-projects, unified by the fact that native `wgpu` (Metal) and browser WebGPU both speak **WGSL**:

- **A (this spec) — Display surface / walking skeleton.** Prove EDR pixels light up and composite correctly with the webview UI, on macOS + Windows. No new tone math, no per-frame live render.
- **B — Shared WGSL render core.** Port the invert + finish + unsharp pipeline (today's `shaders.ts` GLSL) to WGSL once; drive native `wgpu` (mac) and browser WebGPU (win) live, per-frame.
- **C — HDR-aware tools & UI.** Exposure / whites / highlights / tone-curve reach into the >1.0 headroom; tone-curve UI represents the super-white region; HDR becomes a workflow mode.

This spec covers **Sub-project A only.**

---

## Goal

On a real HDR display, toggling HDR shows the current image in **true EDR** via a native surface (macOS) or WebGPU canvas (Windows), correctly positioned beneath all webview UI, settling on edits, and falling back cleanly on SDR displays. Linux hides the HDR button.

**Explicit non-goals (deferred to B/C):**

- Per-frame live HDR rendering during gestures (B).
- Porting the tone/invert/finish pipeline to WGSL (B).
- HDR-aware tone tools or tone-curve UI (C).
- New HDR export formats (PQ/HLG AVIF/HEIC) — gain-map JPEG export stays as-is.

---

## Scope decisions (locked)

1. **Display mechanism, not live-ness.** A keeps today's settle triggers (render on gesture-end + debounce). It swaps only the *display* from a gain-map `<img>` to a true EDR surface. Per-frame live is B's job.
2. **Render source = existing Rust render.** A feeds the surface the existing `film-core` HDR render (`invert_d`, `hdr=true`) → an `rgba16f` (half-float linear, extended-P3) buffer → upload → blit. No shader porting; this also **removes the libultrahdr encode cost** on capable displays.
3. **EDR surface replaces the SDR canvas at rest.** In HDR mode on a capable display, the EDR surface is the settled preview; the SDR WebGL canvas is hidden/punched-through. *During* an active gesture, A still shows the SDR canvas live (today's crossfade), then settles back to the EDR surface. B retires the during-gesture SDR crutch.
4. **Reuse `Viewport.svelte` settle/crossfade state machine.** The existing `hdrSrc`/`hdrShown`/`hdrTimer`/debounce logic is kept; only the thing being shown changes (EDR surface instead of `<img src=dataURL>`).

---

## Architecture

### Shared (cross-platform)

- **Capability detection** decides per-session/per-display which display mode is active:
  - `live-edr` — HDR display + EDR headroom + platform surface available.
  - `gainmap-fallback` — macOS/Windows on an SDR display (today's gain-map `<img>` path).
  - `hidden` — Linux (HDR button not rendered).
- A single TS module exposes the active mode and an imperative surface API (`showHdrBuffer(rgba16f, rect)`, `hideHdr()`, `resizeHdr(rect)`) so `Viewport.svelte` doesn't branch on platform internally.

### macOS — native CAMetalLayer EDR overlay (Rust/Tauri)

- Access the window's `NSWindow` / content `NSView` via `window.ns_window()` (raw-window-handle).
- Add a sibling `NSView` backed by a `CAMetalLayer`:
  - `wantsExtendedDynamicRangeContent = true`
  - extended-linear Display P3 colorspace, `MTLPixelFormatRGBA16Float`
  - `edrMetadata` set appropriately (HDR10/HLG metadata or none for reference EDR — chosen during implementation spike).
- Compositing:
  - Webview `setDrawsBackground: NO` so transparent DOM regions reveal the layer.
  - Native layer `zPosition = -1.0` (behind the webview visually).
  - Override `hitTest:` → `nil` on the native view so pointer/scroll events fall through to the webview.
- The DOM punches a **transparent hole** where the image viewport sits; the native layer renders the image into exactly that rect.
- Rect sync: position, size, devicePixelRatio, and window resize keep the layer aligned to the viewport rect. The frontend sends the current viewport rect (CSS px + DPR) to Rust on layout changes.
- Blit: Rust uploads the `film-core` HDR `rgba16f` buffer into a Metal texture and draws a fullscreen blit into the layer.

### Windows — in-webview WebGPU HDR canvas (TS)

- A WebGPU canvas element positioned over the image viewport in the DOM (natural z-order with other DOM overlays — no punch-out needed).
- `context.configure({ device, format: 'rgba16float', toneMapping: { mode: 'extended' }, ... })`.
- Verify it works under Tauri's WebView2 launch args; if the experimental flag is needed for any required sub-feature, add it via `additional_browser_args` (WebGPU `toneMapping:'extended'` itself is unflagged on recent WebView2).
- Blit: upload the `film-core` HDR `rgba16f` buffer into a WebGPU texture and draw a fullscreen blit.

### Render source (both platforms)

- Reuse the existing `render_and_encode_hdr` plumbing in `commands.rs`, but add a path that returns the **raw `rgba16f` HDR buffer** (linear, extended-P3) instead of (or in addition to) the encoded gain-map JPEG.
  - macOS: the buffer stays in Rust and is uploaded to the Metal texture directly.
  - Windows: the buffer is handed to the WebView (e.g. via a Tauri command returning bytes / shared buffer) for WebGPU upload.

---

## Data flow (settled HDR display, capable display)

1. User edits; gesture ends → existing debounce fires (`Viewport.svelte`).
2. Frontend requests an HDR render for the current params + viewport rect.
3. Rust `film-core` produces the `rgba16f` HDR buffer (existing `invert_d`, `hdr=true`).
4. **macOS:** buffer uploaded to Metal texture in Rust; native layer blits it; frontend told "HDR ready" to punch the hole + hide the SDR canvas, crossfade in.
   **Windows:** buffer bytes returned to frontend; WebGPU canvas uploads + blits; SDR canvas hidden, crossfade in.
5. Next gesture start → hide EDR surface, reveal SDR canvas for live feedback (reused crossfade).

---

## Capability detection & fallback

- **HDR display present + EDR headroom:** query platform — macOS via screen `maximumExtendedDynamicRangeColorComponentValue` (> 1.0); Windows via WebGPU adapter/`window` HDR signals + display HDR enabled.
- **macOS/Windows, no HDR display:** `gainmap-fallback` — unchanged current behavior (gain-map `<img>` crossfade). The native layer / WebGPU canvas is not created.
- **Linux:** `hidden` — HDR toggle button is not rendered, HDR params are not offered, gain-map HDR export is not offered. (Compile-time / runtime OS check, whichever is cleaner in the existing platform-gating pattern.)
- Mode can change if the window moves between an SDR and an HDR display; detection re-runs on display change. (A may handle this lazily — re-evaluate on next settle — to keep the skeleton simple; documented as acceptable.)

---

## Compositing correctness (the core risk)

The macOS native-layer-behind-webview model must keep these DOM overlays rendering **above** the image, through the transparent hole:

- Crop handles / crop rectangle.
- Clipping (blown-highlight / crushed-shadow) warnings.
- Eraser / dust spot markers.
- Loupe, grid, any in-viewport HUD.

Implementation must confirm these are DOM/CSS overlays (not drawn into the GL canvas). Any overlay currently rendered *into* the WebGL canvas would be invisible over the native layer and must be moved to DOM — this is a discovery task in the implementation plan.

Windows has no such risk (WebGPU canvas is a normal DOM element).

---

## Risks this sub-project retires (definition of "done enough to proceed to B")

1. Embedded (non-Safari) **WKWebView actually grants EDR** to a sibling CAMetalLayer in a Tauri window. *(Research flagged this as empirically unverified.)*
2. **Input / scroll / resize sync** of the native layer is solid (`hitTest:` passthrough, rect tracking, DPR, window resize, fullscreen).
3. **Z-order vs. DOM overlays** works (transparent punch-out + `zPosition`).
4. **WebGPU HDR canvas under WebView2** in a packaged Tauri build (not just stock Edge) lights up real EDR.
5. **Capability detection + fallback** correctly routes capable vs. SDR vs. Linux.

If any of 1/4 fail outright, we fall back to the gain-map path on that platform and re-scope — but the research strongly indicates both work.

---

## Testing

- **Rust unit:** new raw-`rgba16f` render path returns a buffer with expected dims, extended-P3 linear values, and >1.0 highlight values when `hdr=true` (mirrors existing `invert_d` HDR tests).
- **Capability detection unit (TS):** given mocked platform/display signals → correct mode (`live-edr` / `gainmap-fallback` / `hidden`).
- **Manual / GUI smoke (the real acceptance):** on a real HDR display —
  - macOS: HDR toggle shows the image in visible EDR (specular highlights brighter than paper-white UI); crop/clipping/eraser overlays remain on top; pan/zoom/resize keeps the layer aligned; clicks/scroll reach the webview; SDR display falls back to gain-map; Linux build shows no HDR button.
  - Windows: same, via WebGPU canvas.
- Regression: with HDR off, the existing SDR path is byte-for-byte unchanged.

---

## Open questions for implementation (resolve in the plan / spike)

- Exact `edrMetadata` choice on macOS (reference EDR vs. HDR10/HLG metadata) and the matching tone normalization of the linear buffer.
- Cheapest reliable channel to hand the `rgba16f` buffer to the WebView on Windows (Tauri command bytes vs. shared memory) without a costly copy per settle.
- Whether any in-viewport overlay is currently canvas-drawn (must move to DOM).
- Whether to create the native layer eagerly at startup or lazily on first HDR-enable.
