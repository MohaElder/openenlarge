# Live HDR Display Surface (Sub-project A) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the static gain-map `<img>` HDR preview with a true EDR display surface — native `CAMetalLayer` on macOS, in-webview WebGPU canvas on Windows — fed by the existing Rust HDR render, settling on edits, with clean fallback on SDR displays and the HDR button hidden on Linux.

**Architecture:** A capability detector picks one of three modes per display (`live-edr` / `gainmap-fallback` / `hidden`). In `live-edr`, the existing `Viewport.svelte` settle state machine renders the current HDR buffer (display-referred sRGB with >1.0 highlights, linearized) into a real EDR surface instead of crossfading a gain-map `<img>`. macOS uses a native Metal layer composited *behind* a transparent webview region; Windows uses a WebGPU `rgba16float` + `toneMapping:'extended'` canvas in the DOM. No new tone math and no per-frame live render — that is Sub-project B.

**Tech Stack:** Rust (Tauri 2, `film-core`, `half`, new: `raw-window-handle` + `objc2`/`objc2-app-kit`/`objc2-quartz-core`/`objc2-metal` for macOS), TypeScript/Svelte frontend, WebGPU (browser, Windows), WebGL2 (existing SDR canvas), Vitest + `cargo test`.

## Global Constraints

- **Scope is display mechanism only.** Keep today's settle triggers (render on gesture-end, 200 ms debounce). No per-frame live render, no WGSL port, no HDR-aware tools — those are Sub-projects B/C. (verbatim from spec "Scope decisions 1")
- **Render source = existing Rust render.** Reuse `film-core` `invert_d` with `hdr=true`; do not write new tone math. (spec "Scope decisions 2")
- **EDR surface replaces the SDR canvas only at rest.** During an active gesture in HDR mode, keep showing the SDR WebGL canvas live (today's crossfade); settle back to the EDR surface. (spec "Scope decisions 3")
- **Reuse `Viewport.svelte` settle/crossfade state machine** (`hdrSrc`/`hdrShown`/`hdrTimer`/`hdrPrevId` + 200 ms debounce); only the thing shown changes. (spec "Scope decisions 4")
- **Linux:** HDR toggle button not rendered; no HDR params offered; no gain-map HDR export. (spec "Capability detection & fallback")
- **macOS/Windows on SDR display:** unchanged current gain-map `<img>` behavior. (spec "Capability detection & fallback")
- **HDR buffer color space:** the `film-core` HDR rendition is **display-referred sRGB-encoded with highlights >1.0**, NOT linear. Linearize with the existing `srgb_to_linear_ext()` pattern (`app/src-tauri/src/hdr.rs:111-121`) before handing pixels to any EDR surface. (discovered fact)
- **No regression to the SDR path.** With HDR off, the existing WebGL2 SDR render must be byte-for-byte unchanged. (spec "Testing")
- **Tests:** Rust via `cargo test`; TS via `npm run test:unit` (from `app/`). Match existing styles (`crates/film-core/src/engine.rs` tests; `app/src/lib/perImage.test.ts`).

---

## File Structure

**Create:**
- `app/src-tauri/src/hdr_surface/mod.rs` — platform-agnostic entry: the `encode_hdr_raw` Tauri command + the `HdrBuffer` DTO.
- `app/src-tauri/src/hdr_surface/macos.rs` — `#[cfg(target_os="macos")]` native CAMetalLayer EDR overlay (create/attach/blit/resize/destroy).
- `app/src/lib/viewport/hdrSurface.ts` — frontend surface abstraction: `HdrSurface` interface + `createHdrSurface()` factory (Windows WebGPU impl + a macOS no-op proxy that just signals "native handles it").
- `app/src/lib/viewport/hdrCapability.ts` — capability detection → `HdrMode = "live-edr" | "gainmap-fallback" | "hidden"`.
- `app/src/lib/viewport/hdrCapability.test.ts` — Vitest for the detector.
- `app/src-tauri/src/hdr_surface/mod.rs` tests (inline `#[cfg(test)]`) — buffer correctness.

**Modify:**
- `app/src-tauri/src/commands.rs` — extract the HDR render half of `render_and_encode_hdr` into a reusable `render_hdr_image()` so both `encode_hdr` and the new raw command share it.
- `app/src-tauri/src/lib.rs:125-186` — register `encode_hdr_raw` in `generate_handler!`; call macOS overlay init.
- `app/src-tauri/src/main.rs`/`lib.rs` module list — add `mod hdr_surface;`.
- `app/src-tauri/Cargo.toml` — add `raw-window-handle`, `objc2*` (macOS-gated), enable any required `tauri` webview-handle feature.
- `app/src/lib/api.ts:197` — add `encodeHdrRaw()` binding next to `encodeHdr()`.
- `app/src/lib/viewport/Viewport.svelte:351-398` — branch the settle state machine on `HdrMode`; drive the surface in `live-edr`, keep `<img>` in `gainmap-fallback`.
- `app/src/lib/develop/Basic.svelte:319-323` — hide the HDR toggle when mode is `hidden` (Linux).

---

## Phase 0 — Proving spikes (GATE)

> These two tasks retire the project's core unknowns. They are **exploratory** (native compositing / GPU EDR can only be verified by eye on real HDR hardware), so they use **manual acceptance**, not unit tests. Hardcode a synthetic HDR gradient — do NOT wire real image data yet. **If either platform cannot light up EDR, STOP and report before Phase 1.**

### Task 1: macOS — EDR CAMetalLayer overlay spike

**Files:**
- Create: `app/src-tauri/src/hdr_surface/macos.rs`
- Modify: `app/src-tauri/src/hdr_surface/mod.rs` (add `#[cfg(target_os="macos")] pub mod macos;`)
- Modify: `app/src-tauri/Cargo.toml`
- Modify: `app/src-tauri/src/lib.rs` (call the spike on window-ready)

**Interfaces:**
- Produces: `macos::attach_edr_spike(window: &tauri::WebviewWindow) -> Result<(), String>` — attaches a Metal layer rendering a static HDR gradient behind the webview.

**Goal:** prove an embedded WKWebView in a Tauri window grants EDR to a sibling `CAMetalLayer`, that the layer composites *behind* a transparent webview region, and that input falls through. This is greenfield — no native-handle precedent exists in the repo.

- [ ] **Step 1: Add dependencies** to `app/src-tauri/Cargo.toml`:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
raw-window-handle = "0.6"
objc2 = "0.5"
objc2-foundation = "0.2"
objc2-app-kit = "0.2"
objc2-quartz-core = "0.2"
objc2-metal = "0.2"
```

(Pin to whatever versions resolve cleanly with the installed Tauri 2; the implementer iterates `cargo build` until it compiles. Record final versions in the commit.)

- [ ] **Step 2: Implement `attach_edr_spike`** in `macos.rs`. The concrete API checklist the implementer must hit (iterating against the compiler — do not guess-and-ship un-built code):
  1. Get the `NSWindow`/content `NSView` from `window.with_webview(|w| ...)` (Tauri 2 `PlatformWebview` exposes the WKWebView / its `ns_window`). Fall back to `window.ns_window()` if available.
  2. Create an `NSView` subview backed by a `CAMetalLayer` (`wantsLayer = true`, set `layer`).
  3. On the layer: `setWantsExtendedDynamicRangeContent(true)`, `setPixelFormat(MTLPixelFormatRGBA16Float)`, `setColorspace(CGColorSpace(extendedLinearDisplayP3))`, and an `edrMetadata` (start with `nil`/reference-EDR; HDR10/HLG metadata is a later tuning knob).
  4. Composite behind the webview: webview `setDrawsBackground(false)`; native view `layer.zPosition = -1.0`.
  5. Input passthrough: subclass/override `hitTest:` to return `nil` so clicks/scroll reach the webview.
  6. Render a **static** HDR gradient (e.g. a horizontal ramp 0.0 → 4.0 linear) into the layer's drawable each frame (a `CADisplayLink` or a one-shot draw is fine for the spike).

- [ ] **Step 3: Call it on window-ready** in `lib.rs` setup, guarded `#[cfg(target_os="macos")]`, near the existing macOS block at `lib.rs:76`.

- [ ] **Step 4: Build**

Run: `cd app/src-tauri && cargo build`
Expected: compiles (iterate objc2 calls until it does).

- [ ] **Step 5: MANUAL ACCEPTANCE — run the app on a real HDR display (macOS 14+):**

Run: `cd app && npm run tauri dev`
Verify by eye (record a note/screenshot in the commit message):
  1. The gradient's bright end is **visibly brighter than the webview's white UI** (EDR confirmed — not just clamped to 1.0).
  2. The gradient sits behind/within the window with the webview UI drawn over it where the DOM is opaque.
  3. Clicking and scrolling over the gradient region reaches the webview (buttons still work).
  4. Resizing the window does not crash; layer stays attached.

If the bright end is NOT brighter than white → EDR is not granted to embedded WKWebView; STOP and report (fall back to gain-map on macOS).

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/src/hdr_surface/ app/src-tauri/src/lib.rs
git commit -m "spike(hdr): macOS CAMetalLayer EDR overlay behind webview (static gradient)"
```

### Task 2: Windows — WebGPU HDR canvas spike

**Files:**
- Create: `app/src/lib/viewport/hdrSpike.ts` (temporary; deleted in Phase 2)
- Modify: a dev entry point to mount it (e.g. a temporary import in `Viewport.svelte`, reverted after the spike)

**Interfaces:**
- Produces: `mountHdrSpike(container: HTMLElement): void` — mounts a WebGPU canvas drawing a static HDR gradient.

**Goal:** prove WebGPU `toneMapping:'extended'` lights up EDR inside a packaged Tauri WebView2 build (not just stock Edge).

- [ ] **Step 1: Implement `mountHdrSpike`** in `hdrSpike.ts`:

```typescript
export async function mountHdrSpike(container: HTMLElement): Promise<void> {
  if (!("gpu" in navigator)) { console.warn("no WebGPU"); return; }
  const adapter = await navigator.gpu.requestAdapter();
  const device = await adapter!.requestDevice();
  const canvas = document.createElement("canvas");
  canvas.width = 512; canvas.height = 128;
  canvas.style.cssText = "position:absolute;left:0;top:0;z-index:0;";
  container.appendChild(canvas);
  const ctx = canvas.getContext("webgpu")!;
  ctx.configure({
    device,
    format: "rgba16float",
    toneMapping: { mode: "extended" } as GPUCanvasToneMapping,
    alphaMode: "premultiplied",
  });
  // Minimal pipeline: fullscreen triangle, fragment outputs a 0..4 horizontal ramp.
  // (Implementer fills the WGSL — a ramp on vec4(x*4,x*4,x*4,1).)
}
```

- [ ] **Step 2: Mount it temporarily** in `Viewport.svelte` `onMount` (guarded behind a `// SPIKE` comment) to render over the viewport.

- [ ] **Step 3: MANUAL ACCEPTANCE — build and run on a Windows machine with an HDR display + Windows "Use HDR" ON:**

Run: `cd app && npm run tauri dev` (on Windows)
Verify by eye:
  1. The ramp's bright end is **visibly brighter than the white UI** (EDR confirmed).
  2. No flag was needed (WebGPU `toneMapping:'extended'` is unflagged on recent WebView2). If it required `--enable-experimental-web-platform-features`, note that the implementer must add it via Tauri `additional_browser_args` and record it.

If it does not light up → re-check WebView2 runtime version / Windows HDR toggle; if still no, report before Phase 1.

- [ ] **Step 4: Revert the temporary mount** (keep `hdrSpike.ts` for reference until Task 7), commit:

```bash
git add app/src/lib/viewport/hdrSpike.ts
git commit -m "spike(hdr): Windows WebGPU rgba16float toneMapping:extended canvas (static gradient)"
```

---

## Phase 1 — Data path & capability (TDD)

### Task 3: Rust `encode_hdr_raw` command (raw rgba16f buffer)

**Files:**
- Modify: `app/src-tauri/src/commands.rs` (extract `render_hdr_image`, add `encode_hdr_raw`)
- Create: `app/src-tauri/src/hdr_surface/mod.rs` (the `HdrBuffer` DTO)
- Modify: `app/src-tauri/src/lib.rs:125-186` (register command)
- Test: inline `#[cfg(test)]` in `commands.rs` (or `hdr_surface/mod.rs`)

**Interfaces:**
- Produces:
  - `pub struct HdrBuffer { pub width: u32, pub height: u32, pub rgba16f: Vec<u16> }` (serde-serializable; `rgba16f` is row-major RGBA half-float, **linear extended-P3**, 4 channels).
  - `fn render_hdr_image(src: &film_core::Image, ip: &InversionParams, mode: Mode, finish: &FinishParams, stamps: &[Stamp], ir_removal: &IrRemoval) -> film_core::Image` — the HDR-rendition half factored out of `render_and_encode_hdr` (`commands.rs:1293-1319`): clones params with `hdr=true`, `finalize_body=false`, runs invert→dust→ir→finish.
  - `#[tauri::command] pub fn encode_hdr_raw(id, params, view, session) -> Result<HdrBuffer, String>` — same geometry/param resolution as `encode_hdr` (`commands.rs:1327-1388`) but returns the linearized half-float buffer instead of a gain-map data URL.
- Consumes: existing `srgb_to_linear_ext` (make it `pub(crate)` in `hdr.rs`) and `half::f16`.

- [ ] **Step 1: Write the failing test** in `commands.rs` `#[cfg(test)]`:

```rust
#[test]
fn hdr_raw_buffer_has_dims_and_superwhite() {
    // A 2x1 synthetic developed image with a bright highlight in one pixel.
    let src = test_developed_image_2x1_bright(); // helper: one near-base px (bright), one dense px
    let ip = test_inversion_params();
    let finish = test_finish_params();
    let img = render_hdr_image(&src, &ip, Mode::Negative, &finish, &[], &IrRemoval::default());
    let buf = hdr_image_to_rgba16f(&img); // the packing helper used by encode_hdr_raw
    assert_eq!(buf.width, 2);
    assert_eq!(buf.height, 1);
    assert_eq!(buf.rgba16f.len(), 2 * 1 * 4);
    // Bright pixel must exceed SDR white (1.0) in linear space after expansion+linearize.
    let r0 = f16::from_bits(buf.rgba16f[0]).to_f32();
    assert!(r0 > 1.0, "expected super-white highlight, got {r0}");
    // Alpha channel is 1.0.
    assert_eq!(f16::from_bits(buf.rgba16f[3]).to_f32(), 1.0);
}
```

(Write the small `test_developed_image_2x1_bright` / `test_inversion_params` / `test_finish_params` helpers alongside, mirroring existing `engine.rs` test fixtures.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app/src-tauri && cargo test hdr_raw_buffer_has_dims_and_superwhite`
Expected: FAIL ("cannot find function `render_hdr_image`").

- [ ] **Step 3: Implement.** Factor `render_hdr_image` out of `render_and_encode_hdr`, add the packing helper and command:

```rust
pub struct HdrBuffer { pub width: u32, pub height: u32, pub rgba16f: Vec<u16> }

fn render_hdr_image(src: &film_core::Image, ip: &InversionParams, mode: Mode,
                    finish: &FinishParams, stamps: &[Stamp], ir_removal: &IrRemoval) -> film_core::Image {
    let render = |ip: &InversionParams, fin: &FinishParams| -> film_core::Image {
        let mut inv = invert_image_core(src, ip, mode);
        dust::apply(&mut inv, stamps);
        if ir_removal.enabled {
            if let Some(ir) = src.ir.as_ref() { dust::apply_ir(&mut inv, ir, ir_removal.sensitivity); }
        }
        finish_image(&inv, fin)
    };
    let mut ip_hdr = ip.clone(); ip_hdr.hdr = true;
    let mut finish_hdr = finish.clone(); finish_hdr.finalize_body = false;
    render(&ip_hdr, &finish_hdr)
}

fn hdr_image_to_rgba16f(img: &film_core::Image) -> HdrBuffer {
    let (w, h) = (img.width, img.height);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for px in &img.pixels {
        // Linearize display-referred sRGB (>1.0 allowed) to linear extended-P3 half-float.
        out.push(half::f16::from_f32(crate::hdr::srgb_to_linear_ext(px[0])).to_bits());
        out.push(half::f16::from_f32(crate::hdr::srgb_to_linear_ext(px[1])).to_bits());
        out.push(half::f16::from_f32(crate::hdr::srgb_to_linear_ext(px[2])).to_bits());
        out.push(half::f16::from_f32(1.0).to_bits());
    }
    HdrBuffer { width: w, height: h, rgba16f: out }
}
```

Then add `encode_hdr_raw` by copying the geometry/param-resolution block from `encode_hdr` (`commands.rs:1327-1388`) and returning `hdr_image_to_rgba16f(&render_hdr_image(...))`. Refactor `render_and_encode_hdr` to call `render_hdr_image` for its HDR half (DRY).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app/src-tauri && cargo test hdr_raw_buffer_has_dims_and_superwhite`
Expected: PASS

- [ ] **Step 5: Register the command** in `lib.rs` `generate_handler!` after `commands::encode_hdr,` (line ~135):

```rust
commands::encode_hdr,
commands::encode_hdr_raw,
```

- [ ] **Step 6: Run the full Rust suite** (no regression in `render_and_encode_hdr`)

Run: `cd app/src-tauri && cargo test`
Expected: PASS (including existing gain-map / HDR tests).

- [ ] **Step 7: Commit**

```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/hdr_surface/mod.rs app/src-tauri/src/lib.rs
git commit -m "feat(hdr): encode_hdr_raw returns linearized rgba16f buffer; share render_hdr_image"
```

### Task 4: Frontend capability detector

**Files:**
- Create: `app/src/lib/viewport/hdrCapability.ts`
- Test: `app/src/lib/viewport/hdrCapability.test.ts`

**Interfaces:**
- Produces:
  - `type HdrMode = "live-edr" | "gainmap-fallback" | "hidden";`
  - `function detectHdrMode(env: HdrEnv): HdrMode` where `HdrEnv = { os: "macos"|"windows"|"linux"; displayHdr: boolean; surfaceSupported: boolean }`.
  - `async function probeHdrEnv(): Promise<HdrEnv>` — fills `os` via Tauri `platform()`, `displayHdr` via a media query / screen probe, `surfaceSupported` via `"gpu" in navigator` (Windows) or `os==="macos"` (native).
- Consumes: nothing from other tasks.

Rule table (pure, unit-tested):
- `os==="linux"` → `"hidden"` (always, regardless of display).
- `os in {macos,windows}` && `displayHdr` && `surfaceSupported` → `"live-edr"`.
- `os in {macos,windows}` && otherwise → `"gainmap-fallback"`.

- [ ] **Step 1: Write the failing test** `hdrCapability.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { detectHdrMode } from "./hdrCapability";

describe("detectHdrMode", () => {
  it("hides on linux regardless of display", () => {
    expect(detectHdrMode({ os: "linux", displayHdr: true, surfaceSupported: true })).toBe("hidden");
    expect(detectHdrMode({ os: "linux", displayHdr: false, surfaceSupported: false })).toBe("hidden");
  });
  it("live-edr on macos/windows with hdr display + surface", () => {
    expect(detectHdrMode({ os: "macos", displayHdr: true, surfaceSupported: true })).toBe("live-edr");
    expect(detectHdrMode({ os: "windows", displayHdr: true, surfaceSupported: true })).toBe("live-edr");
  });
  it("gainmap fallback on macos/windows without hdr display", () => {
    expect(detectHdrMode({ os: "macos", displayHdr: false, surfaceSupported: true })).toBe("gainmap-fallback");
    expect(detectHdrMode({ os: "windows", displayHdr: true, surfaceSupported: false })).toBe("gainmap-fallback");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app && npm run test:unit -- hdrCapability`
Expected: FAIL ("Cannot find module './hdrCapability'").

- [ ] **Step 3: Implement** `hdrCapability.ts`:

```typescript
export type HdrMode = "live-edr" | "gainmap-fallback" | "hidden";
export interface HdrEnv { os: "macos" | "windows" | "linux"; displayHdr: boolean; surfaceSupported: boolean }

export function detectHdrMode(env: HdrEnv): HdrMode {
  if (env.os === "linux") return "hidden";
  if (env.displayHdr && env.surfaceSupported) return "live-edr";
  return "gainmap-fallback";
}

export async function probeHdrEnv(): Promise<HdrEnv> {
  const { platform } = await import("@tauri-apps/plugin-os");
  const p = await platform(); // "macos" | "windows" | "linux" | ...
  const os = p === "macos" ? "macos" : p === "windows" ? "windows" : "linux";
  const displayHdr =
    typeof window !== "undefined" && "matchMedia" in window
      ? window.matchMedia("(dynamic-range: high)").matches
      : false;
  const surfaceSupported = os === "macos" ? true : os === "windows" ? "gpu" in navigator : false;
  return { os, displayHdr, surfaceSupported };
}
```

(Confirm the OS plugin import path matches what the repo already uses — `hotkeys.ts` uses `navigator.platform`; if no `@tauri-apps/plugin-os` is present, derive `os` from `navigator` like `hotkeys.ts` does and adjust the import. Keep `detectHdrMode` pure regardless.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app && npm run test:unit -- hdrCapability`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/viewport/hdrCapability.ts app/src/lib/viewport/hdrCapability.test.ts
git commit -m "feat(hdr): capability detector (live-edr / gainmap-fallback / hidden)"
```

### Task 5: Hide the HDR button on Linux

**Files:**
- Modify: `app/src/lib/develop/Basic.svelte:319-323`
- Modify: wherever `Basic.svelte` gets app context (pass the resolved `HdrMode` down, or read a small store)

**Interfaces:**
- Consumes: `HdrMode` from Task 4 (via a Svelte store `hdrMode` set once at startup from `probeHdrEnv()`/`detectHdrMode()`).

- [ ] **Step 1: Create the store + init.** Add to an existing app-init location (e.g. where other one-time probes run): `export const hdrMode = writable<HdrMode>("gainmap-fallback");` and set it once: `probeHdrEnv().then(e => hdrMode.set(detectHdrMode(e)))`.

- [ ] **Step 2: Gate the button** in `Basic.svelte`:

```svelte
{#if $hdrMode !== "hidden"}
  <button class="hdrtoggle" class:on={$params.hdr}
          title={$t('basic.hdrTitle')} aria-pressed={$params.hdr}
          on:click={() => { params.update((p) => ({ ...p, hdr: !p.hdr })); commitActive(); }}>
    {$t('basic.hdr')}
  </button>
{/if}
```

- [ ] **Step 3: Manual check (Linux build or forced `hidden`)**: button absent on Linux; present on macOS/Windows. (No unit test — trivial template gate; a forced-`hidden` story is enough.)

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/develop/Basic.svelte app/src/lib/<store-init-file>
git commit -m "feat(hdr): hide HDR toggle on Linux (hidden mode)"
```

---

## Phase 2 — Wire real renders into the surface

### Task 6: macOS — real buffer + rect sync into the Metal layer

**Files:**
- Modify: `app/src-tauri/src/hdr_surface/macos.rs` (turn the spike into a real, controllable surface)
- Modify: `app/src-tauri/src/hdr_surface/mod.rs` (Tauri commands to drive it)
- Modify: `app/src-tauri/src/lib.rs` (managed state for the surface handle)

**Interfaces:**
- Produces (Tauri commands):
  - `hdr_surface_show(rgba16f: Vec<u16>, width: u32, height: u32, rect: ViewportRect)` — upload buffer to a Metal texture, blit into the layer, position the layer at `rect`.
  - `hdr_surface_hide()` — hide/detach the layer (reveals the SDR canvas).
  - `hdr_surface_set_rect(rect: ViewportRect)` — reposition/resize on pan/zoom/window-resize.
  - `ViewportRect { x: f64, y: f64, w: f64, h: f64, dpr: f64 }` (CSS px + devicePixelRatio).

**Note:** Native EDR compositing cannot be unit-tested; this task is verified in Task 9's manual smoke. Keep the code factored so the texture-upload/packing logic is the only non-visual part.

- [ ] **Step 1: Replace the static gradient** with a texture uploaded from the passed `rgba16f` buffer (`MTLPixelFormatRGBA16Float`, `width`×`height`), blitting it scaled into the layer's drawable.
- [ ] **Step 2: Implement `set_rect`** — convert `ViewportRect` (CSS px × `dpr`) to the native view frame in the window's coordinate space; the rect comes from the frontend (the SDR canvas's bounding box).
- [ ] **Step 3: Implement show/hide** — `hide` sets the native view hidden; `show` un-hides + uploads + positions.
- [ ] **Step 4: Store the surface handle** in Tauri managed state (created lazily on first `show`); register the three commands in `generate_handler!`.
- [ ] **Step 5: Build**

Run: `cd app/src-tauri && cargo build`
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/hdr_surface/ app/src-tauri/src/lib.rs
git commit -m "feat(hdr): macOS Metal EDR surface — show/hide/set_rect with real rgba16f upload"
```

### Task 7: Windows — WebGPU surface implementation

**Files:**
- Create: `app/src/lib/viewport/hdrSurface.ts`
- Delete: `app/src/lib/viewport/hdrSpike.ts` (superseded)

**Interfaces:**
- Produces:
  - `interface HdrSurface { show(buf: HdrBuffer, rect: DOMRect): Promise<void>; hide(): void; setRect(rect: DOMRect): void; destroy(): void }`
  - `type HdrBuffer = { width: number; height: number; rgba16f: Uint16Array }`
  - `function createHdrSurface(os: HdrMode extends never ? never : "macos" | "windows", container: HTMLElement): HdrSurface` — returns a WebGPU-backed surface on Windows; on macOS returns a thin proxy that forwards `show/hide/setRect` to the Tauri commands from Task 6 (the native layer does the actual drawing).
- Consumes: Task 6's Tauri commands (macOS proxy path); `encodeHdrRaw` from Task 8's api binding.

- [ ] **Step 1: Implement the Windows WebGPU impl** — a `<canvas>` configured `format:"rgba16float", toneMapping:{mode:"extended"}`, positioned at `rect`; `show` uploads `buf.rgba16f` to a texture and blits; `hide` clears/hides; `setRect` repositions; `destroy` tears down. (Reuse the spike's pipeline from `hdrSpike.ts`.)
- [ ] **Step 2: Implement the macOS proxy** — `show` calls `invoke("hdr_surface_show", {...})`, etc. (No canvas; native layer draws.)
- [ ] **Step 3: Add `encodeHdrRaw` to `api.ts`** next to `encodeHdr` (`api.ts:197`):

```typescript
encodeHdrRaw: (id: string, params: InvertParams, view: ViewSpec) =>
    invoke<{ width: number; height: number; rgba16f: number[] }>(
      "encode_hdr_raw", { id, params, view: { ...view, dust: wireDust(view.dust) } }),
```

- [ ] **Step 4: Smoke the Windows path** is covered in Task 9; commit the abstraction:

```bash
git add app/src/lib/viewport/hdrSurface.ts app/src/lib/api.ts
git rm app/src/lib/viewport/hdrSpike.ts
git commit -m "feat(hdr): HdrSurface abstraction (Windows WebGPU impl + macOS native proxy)"
```

### Task 8: Viewport integration — branch settle state machine on mode

**Files:**
- Modify: `app/src/lib/viewport/Viewport.svelte:351-398`

**Interfaces:**
- Consumes: `hdrMode` store (Task 5), `createHdrSurface` (Task 7), `api.encodeHdrRaw` (Task 7), existing settle vars (`hdrShown`/`hdrTimer`/`hdrPrevId`).

The existing state machine stays; we branch what `encodeHdr` does and what gets shown:
- `gainmap-fallback` → unchanged: `api.encodeHdr` → `<img>` crossfade (current code).
- `live-edr` → `api.encodeHdrRaw` → `surface.show(buf, canvasRect())`; `scheduleHdr`'s "hide overlay" path calls `surface.hide()` (revealing the live SDR canvas during gestures); on settle, `surface.show()` + hide SDR canvas.

- [ ] **Step 1: Instantiate the surface** in `onMount` when `$hdrMode === "live-edr"`: `const surface = createHdrSurface(os, viewportEl)`. Track `canvasRect()` = the SDR canvas's `getBoundingClientRect()` (+ `devicePixelRatio`).
- [ ] **Step 2: Branch `encodeHdr()`** (`Viewport.svelte:369`):

```svelte
async function encodeHdr() {
  if (!params.hdr || !id || !imgW || !vpW) return;
  const curId = id;
  try {
    if ($hdrMode === "live-edr") {
      const raw = await api.encodeHdrRaw(id, params, hdrViewSpec());
      if (id !== curId || !params.hdr) return;
      await surface.show({ width: raw.width, height: raw.height, rgba16f: Uint16Array.from(raw.rgba16f) }, canvasRect());
      hdrShown = true; // hides the SDR canvas via existing class binding
    } else {
      const data = await api.encodeHdr(id, params, hdrViewSpec());
      if (id !== curId || !params.hdr) return;
      hdrSrc = data; hdrShown = true;
    }
  } catch (e) {
    if (!(typeof e === "string" && e === "not developed")) console.error("encodeHdr failed", e);
  }
}
```

- [ ] **Step 3: Branch the "hide on edit" path** (`scheduleHdr`, `Viewport.svelte:385`): when `$hdrMode === "live-edr"`, also call `surface.hide()` so the live SDR canvas shows through during the gesture.
- [ ] **Step 4: Keep rect in sync** — on pan/zoom/resize (the existing reactive geometry block), if `live-edr` && `hdrShown`, call `surface.setRect(canvasRect())`.
- [ ] **Step 5: Image-switch cleanup** (`Viewport.svelte:393`) — also `surface.hide()` on `id` change.
- [ ] **Step 6: SDR canvas visibility** — ensure the existing `hdrShown` binding that dims/hides the `<img>` also hides the WebGL canvas in `live-edr` (CSS: when `hdrShown && live-edr`, `canvas { visibility: hidden }`). This is the "EDR surface replaces SDR canvas at rest" behavior.
- [ ] **Step 7: Document the GPU-overlay limitation** with a code comment: in `live-edr` on macOS, the GPU-drawn clipping warning (`clip.ts`) and on-image dust markers live in the SDR canvas, so they are not visible over the native layer at rest (they remain visible during gestures while the SDR canvas shows). Deferred to Sub-project B/C, which render the surface itself and can re-add overlays. (No behavior change needed for the skeleton; just the comment + Task 9 acceptance note.)
- [ ] **Step 8: Type-check + unit suite**

Run: `cd app && npm run test:unit && npm run check`
Expected: PASS (no new unit tests here — integration is manual; ensure nothing else broke).

- [ ] **Step 9: Commit**

```bash
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(hdr): drive EDR surface from settle state machine in live-edr mode"
```

### Task 9: Manual GUI acceptance + regression

**Files:** none (verification only).

This is the real acceptance gate for the skeleton (per spec "Testing → Manual / GUI smoke").

- [ ] **Step 1: macOS (HDR display, macOS 14+)** — `cd app && npm run tauri dev`:
  - Toggle HDR on a developed image → image shows in **visible EDR** (specular highlights brighter than paper-white UI).
  - During a slider drag → live SDR shows (surface hidden); on release (~200 ms) → settles back to EDR.
  - Pan/zoom/resize → EDR layer stays aligned to the image rect.
  - Crop handles (DOM) still draw over the image. Note that clipping-warning + dust markers are absent at rest in HDR (documented limitation).
  - Switch images → surface hides, re-settles on the new image.
  - Toggle HDR off → SDR path identical to before (regression check).
- [ ] **Step 2: Windows (HDR display, Windows HDR ON)** — same checklist via WebGPU canvas.
- [ ] **Step 3: SDR-display fallback (either OS)** — force/normal SDR display → gain-map `<img>` path works exactly as today.
- [ ] **Step 4: Linux** — HDR button absent.
- [ ] **Step 5: Record results** (screenshots/notes) and commit any final tweaks:

```bash
git commit --allow-empty -m "test(hdr): manual GUI acceptance for live HDR display surface (A complete)"
```

---

## Self-Review notes

- **Spec coverage:** Goal/non-goals (Phase 0–2 scope guards) ✓; macOS native overlay (T1, T6) ✓; Windows WebGPU (T2, T7) ✓; capability detection + 3 modes (T4) ✓; Linux hidden (T5) ✓; SDR fallback unchanged (T8 branch + T9.3) ✓; reuse settle state machine (T8) ✓; render source = existing Rust render, linearized (T3) ✓; risks retired (T1/T2 manual gates, T9) ✓; compositing-correctness / GPU-overlay issue surfaced and handled as a documented limitation (T8.7) ✓.
- **Known honest limitation:** Phase 0 native/GPU tasks use manual acceptance, not unit tests — EDR lighting up is not machine-verifiable. The data path (T3), detector (T4), and gating (T5) are TDD.
- **Type consistency:** `HdrBuffer` (`{width,height,rgba16f}`) consistent across Rust (T3) and TS (T7/T8); `HdrMode` strings consistent (T4/T5/T8); surface methods `show/hide/setRect/destroy` consistent (T6/T7/T8).
