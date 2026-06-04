# RedRoom Zoom/Pan + Viewport-Bounded Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make RedRoom's Develop editing fast and add Lightroom-style zoom/pan by always rendering at ~viewport resolution through one `render_view` command, with the Auto-WB pass moved to the thumbnail.

**Architecture:** A single backend `render_view(id, params, view)` crops the appropriate source (proxy at fit, full-res when zoomed) to the visible region, resizes to viewport pixels, and inverts that small image once. The frontend `Viewport.svelte` owns zoom/pan state + loupe cursor and derives the crop via a pure, unit-tested `deriveView`. `film-core` is unchanged.

**Tech Stack:** Rust (`image` crate for crop/resize), Tauri commands, Svelte 5 (SvelteKit), TypeScript, vitest (frontend unit test).

**Reference spec:** `docs/superpowers/specs/2026-06-04-redroom-zoom-perf-design.md`

**Environment:** Work from `/Users/mohaelder/Repos/filmrev`, branch `feat/inversion-poc`. `cargo` is NOT on PATH — prefix cargo with `source "$HOME/.cargo/env" && `. Backend tests: `(cd app/src-tauri && cargo test <filter>)`. Frontend build: `(cd app && npm run build)`.

---

## File Structure

```
app/src-tauri/src/
├── convert.rs    ADD crop() + resize_to()
├── session.rs    ADD thumb_img: Image to CachedImage
└── commands.rs   ADD ViewSpec + choose_source() + render_view(); REMOVE raw_preview/inverted_preview;
                  Auto-WB pass uses thumb_img; export uses thumb_img too
app/src/lib/
├── viewport/view.ts            NEW pure deriveView() crop math
├── viewport/view.test.ts       NEW vitest unit tests
├── viewport/Viewport.svelte     NEW image canvas (cursor, click, wheel, pan, render)
├── api.ts                       ADD ViewSpec + renderView(); REMOVE rawPreview/invertedPreview
├── tabs/Develop.svelte          use <Viewport>; drop the manual invert/img logic
└── tabs/Library.svelte          use <Viewport raw interactive={false}>
```

---

## Task 1: Backend crop + resize helpers

**Files:** Modify `app/src-tauri/src/convert.rs`.

- [ ] **Step 1: Add `crop` and `resize_to` with tests**

Append to `crates`… NO — append to `app/src-tauri/src/convert.rs`, above the existing
`#[cfg(test)]` block, these two functions:

```rust
/// Crop a rectangle (in pixels) from the image, clamped to its bounds. Returns a
/// new Image; `ir` is dropped (previews don't need it).
pub fn crop(img: &Image, x: usize, y: usize, w: usize, h: usize) -> Image {
    let x = x.min(img.width);
    let y = y.min(img.height);
    let x2 = (x + w).min(img.width);
    let y2 = (y + h).min(img.height);
    let (cw, ch) = (x2 - x, y2 - y);
    let mut pixels = Vec::with_capacity(cw * ch);
    for yy in y..y2 {
        let row = yy * img.width;
        for xx in x..x2 {
            pixels.push(img.pixels[row + xx]);
        }
    }
    Image { width: cw, height: ch, pixels, ir: None }
}

/// Resize to exactly `w x h` (Triangle filter). No-op if already that size.
pub fn resize_to(img: &Image, w: u32, h: u32) -> Image {
    if img.width as u32 == w && img.height as u32 == h {
        return img.clone();
    }
    let buf = to_rgb32f(img);
    let r = image::imageops::resize(&buf, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
    from_rgb32f(&r)
}
```

Add to the `#[cfg(test)] mod tests` block in convert.rs (it already has a `solid` helper):

```rust
    #[test]
    fn crop_extracts_subrectangle() {
        // 4x4 where pixel value encodes position: r = x/10, g = y/10
        let mut img = Image { width: 4, height: 4, pixels: vec![[0.0; 3]; 16], ir: None };
        for y in 0..4 {
            for x in 0..4 {
                img.pixels[y * 4 + x] = [x as f32 / 10.0, y as f32 / 10.0, 0.0];
            }
        }
        let c = crop(&img, 1, 2, 2, 1);
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(c.pixels[0], [0.1, 0.2, 0.0]); // (x=1,y=2)
        assert_eq!(c.pixels[1], [0.2, 0.2, 0.0]); // (x=2,y=2)
    }

    #[test]
    fn crop_clamps_to_bounds_without_panic() {
        let img = solid(4, 4, [0.5, 0.5, 0.5]);
        let c = crop(&img, 3, 3, 10, 10); // overruns
        assert_eq!((c.width, c.height), (1, 1));
        let z = crop(&img, 9, 9, 2, 2); // fully outside
        assert_eq!((z.width, z.height), (0, 0));
    }

    #[test]
    fn resize_to_hits_target_dims_and_keeps_color() {
        let img = solid(10, 8, [0.2, 0.4, 0.6]);
        let r = resize_to(&img, 5, 4);
        assert_eq!((r.width, r.height), (5, 4));
        // solid color preserved through resize
        for c in 0..3 {
            assert!((r.pixels[0][c] - img.pixels[0][c]).abs() < 1e-3);
        }
    }
```

- [ ] **Step 2: Run tests**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo test convert::)`
Expected: existing convert tests + the 3 new ones PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/convert.rs
git commit -m "feat(redroom): convert crop() + resize_to() helpers"
```

---

## Task 2: `render_view` command (+ thumb_img, source selection, thumbnail Auto-WB)

**Files:** Modify `app/src-tauri/src/session.rs`, `app/src-tauri/src/commands.rs`, `app/src-tauri/src/lib.rs`.

- [ ] **Step 1: Cache a small thumbnail Image for Auto-WB**

In `app/src-tauri/src/session.rs`, add a field to `CachedImage`:

```rust
pub struct CachedImage {
    pub full_res: Image,
    pub proxy: Image,
    pub thumb_img: Image,
    pub file_name: String,
    pub metadata: Metadata,
    pub thumbnail: String,
}
```

- [ ] **Step 2: Populate `thumb_img` on import**

In `app/src-tauri/src/commands.rs`, in `import_image`, the thumbnail downscale is already
computed as `thumb_img`; stop dropping it. Change the `CachedImage { ... }` construction to:

```rust
    let cached = CachedImage {
        full_res: full,
        proxy: proxy_img,
        thumb_img,
        file_name,
        metadata,
        thumbnail,
    };
```
(The line `let thumb_img = proxy(&full, THUMB_EDGE);` already exists above; keep it. Note it must
come before `let thumbnail = to_png_b64(&thumb_img, true)?;` which borrows it — `proxy` returns an
owned Image so both the PNG and the stored field can use it: change the PNG line to borrow
`&thumb_img` which it already does, and move `thumb_img` into the struct afterwards.)

- [ ] **Step 3: Make `resolve_params` use a passed-in small image for Auto-WB**

In `commands.rs`, `resolve_params` currently inverts the proxy for the Auto-WB pass. Its second
argument is already an arbitrary `&film_core::Image`; no signature change is needed — callers
will pass `&img.thumb_img`. Confirm the function body inverts that argument (not the proxy):

```rust
fn resolve_params(p: &InvertParams, autowb_src: &film_core::Image, base: [f32; 3]) -> InversionParams {
    let manual = wb_from_temp_tint(p.temp, p.tint);
    let mut ip = build_params(p, base);
    ip.wb = manual;
    if p.auto_wb {
        let first = invert_image(autowb_src, &ip, mode_from(&p.mode));
        let auto = auto_wb_gains(&first);
        ip.wb = [manual[0] * auto[0], manual[1] * auto[1], manual[2] * auto[2]];
    }
    ip
}
```
(Rename the parameter to `autowb_src` for clarity.)

- [ ] **Step 4: Add `ViewSpec` + `choose_source` with a unit test, and `render_view`**

In `commands.rs`, add near the top (after the existing `use` lines add `use serde::Deserialize;`
if not present — `InvertParams` already derives Deserialize via session, but `ViewSpec` is defined
here so it needs the import):

```rust
use serde::Deserialize;

/// The visible region to render, in FULL-RES pixel coordinates, plus the output
/// (≈ viewport) pixel size. `raw` selects the un-inverted scan.
#[derive(Debug, Clone, Deserialize)]
pub struct ViewSpec {
    pub crop: [f64; 4], // x, y, w, h in full-res px
    pub out_w: u32,
    pub out_h: u32,
    pub raw: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Proxy,
    FullRes,
}

/// Choose the cheapest source that still has enough detail: use the proxy when
/// the cropped region sampled at proxy scale already meets the output width;
/// otherwise the full-res image.
fn choose_source(crop_w_full: f64, out_w: u32, proxy_scale: f64) -> Source {
    if crop_w_full * proxy_scale >= out_w as f64 {
        Source::Proxy
    } else {
        Source::FullRes
    }
}
```

Add the command:

```rust
#[tauri::command]
pub fn render_view(id: String, params: InvertParams, view: ViewSpec, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;

    let proxy_scale = img.proxy.width as f64 / img.full_res.width.max(1) as f64;
    let source = choose_source(view.crop[2], view.out_w, proxy_scale);
    let (src_img, s_scale) = match source {
        Source::Proxy => (&img.proxy, proxy_scale),
        Source::FullRes => (&img.full_res, 1.0),
    };

    // Map the full-res crop into source pixel coords.
    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;

    let cropped = crop(src_img, cx, cy, cw, ch);
    if cropped.pixels.is_empty() {
        return Err("empty crop".into());
    }
    let scaled = resize_to(&cropped, view.out_w.max(1), view.out_h.max(1));

    if view.raw {
        return to_png_b64(&scaled, true);
    }
    let base = sample_base(&img.proxy, None);
    let ip = resolve_params(&params, &img.thumb_img, base);
    let inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    to_png_b64(&inv, false)
}
```

Add a unit test for `choose_source` to a `#[cfg(test)]` block at the bottom of `commands.rs`
(create the block if none exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_source_uses_proxy_at_fit_fullres_when_zoomed() {
        // Fit: whole 4000px image, 250px out, proxy 0.5x → 2000 ≥ 250 → Proxy.
        assert_eq!(choose_source(4000.0, 250, 0.5), Source::Proxy);
        // Zoomed: 250px crop, 250px out, proxy 0.5x → 125 < 250 → FullRes.
        assert_eq!(choose_source(250.0, 250, 0.5), Source::FullRes);
    }
}
```

- [ ] **Step 5: Remove old commands; register `render_view`**

In `commands.rs`, DELETE the `raw_preview` and `inverted_preview` functions entirely.
In `app/src-tauri/src/lib.rs`, update the handler list:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::import_image,
            commands::render_view,
            commands::export_image,
        ])
```

- [ ] **Step 6: Export also uses the thumbnail for Auto-WB**

In `export_image`, change the `resolve_params` call to pass the thumbnail:

```rust
    let ip = resolve_params(&params, &img.thumb_img, base);
```
(Leave the rest: it still inverts `img.full_res` for the actual export.)

- [ ] **Step 7: Build + test**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo build 2>&1 | tail -8 && cargo test 2>&1 | grep 'test result')`
Expected: compiles; convert + choose_source tests pass; no unused-import warnings for the removed commands.

- [ ] **Step 8: Commit**

```bash
git add app/src-tauri
git commit -m "feat(redroom): render_view (viewport-bounded) + thumbnail Auto-WB; drop raw/inverted_preview"
```

---

## Task 3: Frontend API + `deriveView` math (unit-tested)

**Files:** Modify `app/src/lib/api.ts`; create `app/src/lib/viewport/view.ts`, `app/src/lib/viewport/view.test.ts`; modify `app/package.json` (vitest).

- [ ] **Step 1: Update `api.ts`**

In `app/src/lib/api.ts`, REMOVE `rawPreview` and `invertedPreview` from the `api` object, and add
the `ViewSpec` type + `renderView`:

```ts
export interface ViewSpec {
  crop: [number, number, number, number];
  out_w: number;
  out_h: number;
  raw: boolean;
}
```
and in the `api` object (replacing the two removed entries):
```ts
  renderView: (id: string, params: InvertParams, view: ViewSpec) =>
    invoke<string>("render_view", { id, params, view }),
```

- [ ] **Step 2: Pure crop math**

Create `app/src/lib/viewport/view.ts`:

```ts
import type { ViewSpec } from "../api";

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/** Scale that fits the whole image in the viewport (display px per image px). */
export function fitScale(imgW: number, imgH: number, vpW: number, vpH: number): number {
  if (!imgW || !imgH || !vpW || !vpH) return 1;
  return Math.min(vpW / imgW, vpH / imgH);
}

/**
 * Derive the render view from zoom/pan state.
 * `scale` = display px per image px. `(cx,cy)` = image-space point centered in the viewport.
 * The crop is the visible region in full-res image px; out_w/out_h ≈ its on-screen size.
 */
export function deriveView(
  scale: number, cx: number, cy: number,
  imgW: number, imgH: number, vpW: number, vpH: number,
  raw = false,
): ViewSpec {
  const visW = Math.min(vpW / scale, imgW);
  const visH = Math.min(vpH / scale, imgH);
  const x = clamp(cx - visW / 2, 0, Math.max(0, imgW - visW));
  const y = clamp(cy - visH / 2, 0, Math.max(0, imgH - visH));
  return {
    crop: [x, y, visW, visH],
    out_w: Math.max(1, Math.round(visW * scale)),
    out_h: Math.max(1, Math.round(visH * scale)),
    raw,
  };
}
```

- [ ] **Step 3: Add vitest and a unit test**

In `app/`, run: `cd /Users/mohaelder/Repos/filmrev/app && npm install -D vitest`
Add a script to `app/package.json` `"scripts"`: `"test:unit": "vitest run"`.

Create `app/src/lib/viewport/view.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { fitScale, deriveView } from "./view";

describe("deriveView", () => {
  it("fit view covers the whole image", () => {
    const s = fitScale(1000, 500, 250, 250); // = 0.25
    const v = deriveView(s, 500, 250, 1000, 500, 250, 250);
    expect(v.crop).toEqual([0, 0, 1000, 500]);
    expect(v.out_w).toBe(250);
    expect(v.out_h).toBe(125);
  });

  it("100% yields a viewport-sized crop centered on the point", () => {
    const v = deriveView(1.0, 500, 250, 1000, 500, 250, 250);
    expect(v.crop).toEqual([375, 125, 250, 250]);
    expect(v.out_w).toBe(250);
    expect(v.out_h).toBe(250);
  });

  it("clamps the crop at the edges", () => {
    const v = deriveView(1.0, 0, 0, 1000, 500, 250, 250);
    expect(v.crop[0]).toBe(0);
    expect(v.crop[1]).toBe(0);
  });
});
```

- [ ] **Step 4: Run the unit test**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/viewport/view.test.ts`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/api.ts app/src/lib/viewport app/package.json app/package-lock.json
git commit -m "feat(redroom): renderView API + pure deriveView crop math (vitest)"
```

---

## Task 4: `Viewport.svelte`

**Files:** Create `app/src/lib/viewport/Viewport.svelte`.

- [ ] **Step 1: Write the component**

Create `app/src/lib/viewport/Viewport.svelte`:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { api, type InvertParams } from "../api";
  import { deriveView, fitScale } from "./view";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;
  export let imgH = 0;
  export let raw = false;
  export let interactive = true;

  let el: HTMLDivElement;
  let src = "";
  let vpW = 0, vpH = 0;
  let scale = 0;        // display px per image px (0 = uninitialised)
  let cx = 0, cy = 0;   // image-space centre
  let prevId: string | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  $: fit = fitScale(imgW, imgH, vpW, vpH);
  $: zoomed = interactive && scale > fit + 1e-6;
  $: label = scale <= fit + 1e-6 ? "Fit" : Math.round(scale * 100) + "%";

  function measure() {
    if (!el) return;
    vpW = el.clientWidth; vpH = el.clientHeight;
    if (scale === 0 && imgW) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
  }

  onMount(() => {
    measure();
    const ro = new ResizeObserver(measure);
    if (el) ro.observe(el);
    return () => ro.disconnect();
  });

  // reset to fit when the image changes
  $: if (id !== prevId) { prevId = id; scale = fit || 1; cx = imgW / 2; cy = imgH / 2; }

  async function render() {
    if (!id || !imgW || !vpW) { src = ""; return; }
    const v = deriveView(interactive ? scale : fit, cx, cy, imgW, imgH, vpW, vpH, raw);
    try { src = await api.renderView(id, params, v); } catch { /* keep previous frame */ }
  }
  function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(render, 100); }

  // re-render on any relevant change
  $: if (id && vpW && imgW && (params, scale, cx, cy, raw)) schedule();

  function imgPoint(e: { clientX: number; clientY: number }): [number, number] {
    const v = deriveView(scale, cx, cy, imgW, imgH, vpW, vpH);
    const rect = el.getBoundingClientRect();
    const offX = (vpW - v.out_w) / 2, offY = (vpH - v.out_h) / 2;
    const px = (e.clientX - rect.left - offX) / v.out_w;
    const py = (e.clientY - rect.top - offY) / v.out_h;
    return [v.crop[0] + px * v.crop[2], v.crop[1] + py * v.crop[3]];
  }

  function onClick(e: MouseEvent) {
    if (!interactive) return;
    const [ix, iy] = imgPoint(e);
    if (zoomed) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
    else { scale = 1.0; cx = ix; cy = iy; }
  }

  function onWheel(e: WheelEvent) {
    if (!interactive) return;
    e.preventDefault();
    const [ix, iy] = imgPoint(e);
    const ns = Math.min(8, Math.max(fit, scale * Math.exp(-e.deltaY * 0.0015)));
    cx = ix + (cx - ix) * (scale / ns);
    cy = iy + (cy - iy) * (scale / ns);
    scale = ns;
  }

  let dragging = false, lastX = 0, lastY = 0;
  function onDown(e: PointerEvent) {
    if (!zoomed) return;
    dragging = true; lastX = e.clientX; lastY = e.clientY;
    (e.target as Element).setPointerCapture?.(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!dragging) return;
    cx -= (e.clientX - lastX) / scale;
    cy -= (e.clientY - lastY) / scale;
    lastX = e.clientX; lastY = e.clientY;
  }
  function onUp() { dragging = false; }
</script>

<div
  class="vp" class:interactive class:zoomed
  bind:this={el}
  on:click={onClick} on:wheel={onWheel}
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointerleave={onUp}
>
  {#if src}<img {src} alt="preview" draggable="false" />{:else}<div class="hint">…</div>{/if}
  {#if id && interactive}<div class="zoom">{label}</div>{/if}
</div>

<style>
  .vp { position: relative; width: 100%; height: 100%; display: grid; place-items: center;
    overflow: hidden; user-select: none; }
  .vp.interactive { cursor: zoom-in; }
  .vp.zoomed { cursor: grab; }
  .vp.zoomed:active { cursor: grabbing; }
  img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 10px; display: block; }
  .hint { color: var(--text-dim); }
  .zoom { position: absolute; bottom: 8px; right: 10px; font-size: 11px; color: var(--text-dim);
    background: rgba(0,0,0,0.45); padding: 2px 8px; border-radius: 6px; }
</style>
```

- [ ] **Step 2: Build check**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6`
Expected: builds (the component isn't wired in yet but must compile). If Svelte complains that
`on:click` on a div needs a keyboard handler (a11y warning), it's a warning not an error — leave
it; the build still succeeds.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(redroom): Viewport component — loupe cursor, click 1:1, wheel zoom, pan"
```

---

## Task 5: Wire Viewport into Develop and Library

**Files:** Modify `app/src/lib/tabs/Develop.svelte`, `app/src/lib/tabs/Library.svelte`.

- [ ] **Step 1: Develop tab uses Viewport (inverted, interactive), drops the manual invert flow**

Replace `app/src/lib/tabs/Develop.svelte` entirely with:

```svelte
<script lang="ts">
  import { activeId, params, images } from "../store";
  import Adjustments from "../panels/Adjustments.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";

  $: active = $images.find((i) => i.id === $activeId);
</script>

<div class="layout">
  <aside class="left"></aside>
  <section class="center">
    <Viewport id={$activeId} params={$params}
              imgW={active?.metadata.width ?? 0} imgH={active?.metadata.height ?? 0} />
  </section>
  <aside class="right"><Adjustments /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; min-height: 0; }
  .bottom { grid-area: bottom; }
</style>
```

(The left column is intentionally empty now — the explicit Invert button is gone since Develop
renders the inverted result live. We keep the column for layout symmetry; a later task can add
history/presets there.)

- [ ] **Step 2: Library tab uses Viewport (raw, non-interactive)**

Replace the `<script>` and the center `<section>` in `app/src/lib/tabs/Library.svelte`. New full file:

```svelte
<script lang="ts">
  import { activeId, params, images } from "../store";
  import Source from "../panels/Source.svelte";
  import Metadata from "../panels/Metadata.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";

  $: active = $images.find((i) => i.id === $activeId);
</script>

<div class="layout">
  <aside class="left"><Source /></aside>
  <section class="center">
    {#if active}
      <Viewport id={$activeId} params={$params} raw={true} interactive={false}
                imgW={active.metadata.width} imgH={active.metadata.height} />
    {:else}<div class="hint">Import a film scan to begin</div>{/if}
  </section>
  <aside class="right"><Metadata /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; display: grid; place-items: center; min-height: 0; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
```

- [ ] **Step 3: Build**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6`
Expected: builds. (`store.ts` already exports `images`, `activeId`, `params`.)

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/tabs
git commit -m "feat(redroom): wire Viewport into Develop (live inverted) and Library (raw fit)"
```

---

## Task 6: Manual E2E + findings

**Files:** `docs/superpowers/poc-findings.md`.

- [ ] **Step 1: Run the app**

`cd /Users/mohaelder/Repos/filmrev/app && source "$HOME/.cargo/env" && npm run tauri dev`
Verify on the V600 DNG + GFX RAF:
- Develop editing (sliders, stock, Auto-WB) feels noticeably faster than before.
- Hover shows the loupe cursor; click toggles Fit↔100% centered on the click; the 100% jump is
  bigger for the larger GFX image.
- Scroll/pinch zooms; drag pans only when zoomed; zoom indicator updates.
- Zoomed-in view is sharp (full-res region), fit view is fast.
- Export still writes a full-res 16-bit TIFF.

- [ ] **Step 2: Record results**

Add a "Zoom/perf — results" section to `docs/superpowers/poc-findings.md`: the speed change
(qualitative), zoom/pan feel, and any rough edges (e.g. cursor-anchored zoom drift, debounce
timing) to tune next.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/poc-findings.md
git commit -m "docs: zoom/pan + viewport-bounded rendering results"
```

---

## Definition of Done

- [ ] `(cd app/src-tauri && cargo test)` green incl. crop/resize + choose_source.
- [ ] `npx vitest run` green for deriveView.
- [ ] `npm run build` succeeds; backend compiles with `render_view` registered and old preview
      commands removed.
- [ ] App: Develop editing is faster; loupe cursor; click Fit↔100%; scroll/pinch zoom; pan only
      when zoomed; sharp zoomed view; export still full-res.
- [ ] Findings recorded.
```
