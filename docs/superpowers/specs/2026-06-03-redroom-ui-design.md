# RedRoom — Consumer UI (Tauri Shell) Design

**Date:** 2026-06-03
**Status:** Approved (design phase)
**Depends on:** `film-core` (engine), `docs/superpowers/specs/2026-06-03-film-inversion-poc-design.md`

## Product

**RedRoom** — a consumer film-negative inversion app. Name + accent color reference the
darkroom safelight (red). Dark "liquid glass" aesthetic, Adobe Lightroom-style module layout.
Cross-platform (macOS + Windows) via Tauri, reusing the Rust `film-core` engine unchanged.

## Goal of this build

Wrap the working `film-core` engine in an interactive consumer shell with two Lightroom-style
modules — **Library** (import + metadata, no inversion) and **Develop** (invert + live
adjustments) — with proxy-based live preview and 16-bit TIFF export.

## Visual identity

- **Liquid glassmorphism, dark:** near-black gradient/vignetted background; floating **frosted
  translucent panels** (backdrop-blur, 1px light-tinted borders, soft inner glow, rounded
  corners) — the Apple "liquid glass" vibe.
- **Accent: red** (darkroom safelight) — used for the active module tab, primary actions
  (Invert, Export), slider fills, focus states, selection highlights.
- Built with the **frontend-design skill** after the spec, to reach production-grade polish.

## Layout (Lightroom modules)

```
┌───────────────────────────────────────────────────────────────┐
│  ◐ RedRoom          [ Library ][ Develop ]            (glass)   │  top bar + module tabs
├──────────┬──────────────────────────────────────┬─────────────┤
│  SOURCE  │            IMAGE PREVIEW              │  RIGHT PANEL │
│ (glass)  │   Library: raw negative as-is        │  (glass)     │
│ import   │   Develop: inverted result           │  Library →   │
│ list     │                                      │   METADATA   │
│          │                                      │  Develop →   │
│          │                                      │   ADJUST     │
├──────────┴──────────────────────────────────────┴─────────────┤
│  ▣ ▣ ▣ ▣ ▣   filmstrip of imported thumbnails (glass strip)    │
└───────────────────────────────────────────────────────────────┘
```

- **Top bar:** RedRoom wordmark + module tabs (Library | Develop); active tab in red.
- **Left (Source):** import button (multi-file), list of imported images.
- **Center:** image preview — **raw un-inverted negative in Library**, inverted result in Develop.
- **Right panel:** swaps by module — **Metadata** (Library) / **Adjustments** (Develop).
- **Bottom:** filmstrip of imported thumbnails; click to select active image.

### Library tab
Import DNG / TIFF / RAF (multi-select). Center + filmstrip show the **raw scan, not inverted**.
Right = **Metadata**: camera make/model, lens, ISO, shutter, aperture, dimensions, file size,
capture date — best-effort from RAW/TIFF tags (some fields blank for scanner DNGs).

### Develop tab
Center shows the active image; a prominent red **Invert** button runs the engine. Right =
**Adjustments** stack:
- **Mode:** B (density) / C (per-channel).
- **Film stock:** none / Portra 400 / Fuji C200 (sets fitted `M_post` for Mode B).
- **Base sample:** Auto (whole-image 95th pct) + manual rectangle pick on the preview.
- **Sliders:** Exposure, Black point, Gamma.
- **Export** (red): writes full-res 16-bit TIFF via a save dialog.

Live: adjustments re-run the engine on the proxy (debounced) and update the preview instantly.

## Architecture

```
app/                          Tauri project
├── src/                      Svelte + Vite + TypeScript frontend
│   ├── App.svelte            layout + module-tab state
│   ├── lib/
│   │   ├── tabs/Library.svelte, Develop.svelte
│   │   ├── panels/Metadata.svelte, Adjustments.svelte, Filmstrip.svelte, Source.svelte
│   │   ├── glass/GlassPanel.svelte         reusable frosted container
│   │   ├── store.ts          Svelte stores: imported images, active id, params
│   │   └── api.ts            typed wrappers over Tauri commands
│   └── styles/               theme tokens (dark glass + red accent)
└── src-tauri/
    └── src/
        ├── main.rs           Tauri builder, register commands
        ├── commands.rs       import / metadata / preview / invert / export
        └── session.rs        in-memory image cache (full-res + proxy + thumb)
```

- **`film-core` is reused unchanged.** `src-tauri` adds only: a session cache and a metadata
  extractor; commands orchestrate `film-core` (decode, sample_base, params_for_stock,
  invert_image, write_tiff16).
- **Frontend decomposition:** small, single-purpose Svelte components; `GlassPanel` centralizes
  the glass styling; `store.ts` holds session + params; `api.ts` is the only Tauri boundary.

## Tauri command interface

```rust
// All ids are session-local u64 strings. Images are base64 PNG for transport.
import_image(path: String) -> ImageEntry { id, file_name, thumbnail_png_b64, metadata }
list_images() -> Vec<ImageEntry>
get_raw_preview(id) -> String            // base64 PNG of the (un-inverted) proxy, for Library
get_inverted_preview(id, params: InvertParams) -> String  // base64 PNG, engine on proxy
export_image(id, params: InvertParams, out_path: String) -> ()   // full-res 16-bit TIFF
get_metadata(id) -> Metadata             // (also returned by import)
```

`InvertParams` (serde, mirrors engine knobs the UI exposes): `mode` ("b"/"c"), `stock`
("none"/"portra400"/"fujic200"), `base_rect` (optional [x,y,w,h]), `exposure`, `black`, `gamma`.
`Metadata`: optional strings for camera, lens, iso, shutter, aperture; width, height, file_size,
date.

## Data flow

```
import(path) ─► film-core decode (once) ─► session cache { full_res, proxy(~2–4MP), thumb }
                                          └─► ImageEntry { id, thumb_b64, metadata }
Library:  get_raw_preview(id) ─► proxy → PNG b64 → <img>
Develop:  get_inverted_preview(id, params) ─► engine on proxy → PNG b64 → <img>  (debounced)
Export:   export_image(id, params, out) ─► engine on full_res → write_tiff16
```

- Proxy generated at import by downscaling the decoded full-res to fit ~2048px long edge.
- Preview returns base64 PNG (small, fast over IPC) — no GPU needed for v1.
- Metadata extracted at import from RAW/TIFF tags (best-effort).

## Error handling

- Decode failure (bad/unsupported file) → command returns `Err`; UI shows a red inline toast,
  skips that file, continues importing the rest.
- Unknown stock / malformed params → command returns `Err`; UI ignores the change, keeps prior
  preview.
- Export path not writable → `Err` → toast.
- All Tauri commands return `Result<T, String>`; the frontend `api.ts` surfaces errors as toasts.

## Testing

- **`film-core`:** already covered (25 tests).
- **`src-tauri` (Rust unit tests):** metadata extraction returns expected fields for a known
  TIFF; proxy downscale preserves aspect ratio and caps the long edge; `export_image` writes a
  valid 16-bit TIFF (reuse decode_tiff to assert). Commands tested by calling the underlying
  functions directly (not through the Tauri runtime).
- **Frontend:** lightweight — a smoke test that `api.ts` wrappers call the right command names;
  component-level rendering checks are optional for v1.
- **Manual E2E:** import the V600 DNG + GFX RAF; verify Library metadata, Develop invert + stock
  + sliders + export. Record in `poc-findings.md`.

## Scope

**In v1:** two tabs (Library/Develop), multi-import, metadata, filmstrip, raw preview, invert,
Mode B/C, stock selector, base auto + manual rect, exposure/black/gamma, live proxy preview,
16-bit TIFF export, dark glass theme with red accent, cross-platform build.

**Out (later):** GPU/wgpu preview; per-camera SS calibration; frame/rebate auto-detect; AI
dust/color; crop/rotate/straighten; catalog persistence (v1 session is in-memory, resets on
quit); naive mode in UI (engine keeps it; UI exposes B/C only); multi-image batch export.

## Assumptions

1. Tauri 2.x; Svelte 5 + Vite + TS scaffold via `create-tauri-app`.
2. Metadata available from `rawler` (camera/lens/exposure for RAF/DNG) and the `tiff` crate tags
   (for TIFF/linear DNG); fields absent in the source are shown blank, not errored.
3. base64-PNG-over-IPC preview is fast enough for ~2–4MP proxies; if it ever feels slow, the
   Tauri asset protocol or wgpu are the later optimizations (out of scope now).
4. In-memory session only (assumption confirmed): imported list resets on quit.
