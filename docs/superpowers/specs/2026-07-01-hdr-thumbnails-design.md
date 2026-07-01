# HDR Thumbnails — display-only, gain-map baked

**Date:** 2026-07-01
**Status:** Design approved — ready for implementation plan
**Parent effort:** Live HDR. Sub-projects A (macOS EDR display surface), B (per-frame live Metal), and C (chroma-preserving HDR finish + gain-map export) are COMPLETE and shipped on `main`.

---

## Background

The develop preview can show a per-image HDR rendition (the `hdr` flag in the image's develop params). But the small thumbnails shown elsewhere are always SDR: `thumbnail_compute` (`app/src-tauri/src/commands.rs`) renders each frame through `finish_image` (SDR) and encodes a plain JPEG data URL via `to_jpeg_b64`. That baked data URL (`img.thumbnail`) is what every on-screen thumbnail surface displays:

- the export window (`app/src/lib/export/ExportModal.svelte` — `<img src={img.thumbnail}>`),
- the develop bottom strip (`app/src/lib/panels/Filmstrip.svelte` / `app/src/lib/roll/FramePreview.svelte`),
- the library grid (`app/src/lib/library/Grid.svelte`).

So an image the user has marked HDR still shows a flat SDR thumbnail in those places.

**The display mechanism.** On macOS (WKWebView) a **gain-map JPEG** shown as an ordinary `<img>` renders in HDR natively — this is exactly how the original single-image HDR preview worked (`encode_hdr` → base64 data URL → `<img>` glows). Because a grid or strip shows many tiles at once, a gain-map `<img>` per tile is the only scalable HDR path (you cannot put a native Metal EDR surface behind each of dozens of tiles). Sub-project C already produces gain-map JPEGs via `encode_gain_map_jpeg(sdr, hdr, quality)` from the shared chroma-preserving finish.

**Decisions locked in brainstorming:**
- **Display-only.** This project makes the on-screen thumbnails HDR. The exported contact-sheet *file* (`app/src/lib/roll/exportSheet.ts`) stays SDR for now (HDR compositing of a stitched sheet is deferred as separate future work).
- **Gated by the per-image `hdr` flag.** A thumbnail is HDR only when that image's `params.hdr` is true. Non-HDR images are unchanged (byte-identical SDR JPEG). This matches the user's "those should be HDR if that image is HDR."
- **Approach A (bake a gain-map thumbnail when `hdr`).** Rejected alternatives: (B) always bake gain-map for every image — bloats every thumbnail and makes non-HDR images glow, contradicting the gate; (C) keep SDR baked thumbs and render HDR on-demand per visible tile — adds scroll-time latency and a redundant render path for hundreds of grid tiles.

---

## Goal

When an image's `hdr` flag is on, its baked thumbnail carries a gain map so it renders in HDR (highlights glow) in the export window, the develop bottom strip, and the library grid — using Sub-project C's chroma-preserving finish, so those thumbnails match the develop HDR preview. SDR (non-HDR) thumbnails are byte-identical to today.

**Non-goals:** HDR for the exported contact-sheet/film-strip *file* (deferred); any frontend display changes (the `<img>` tags already render gain-map data URLs); new HDR tuning (uses C's `hdr_finish` as-is); changes to the import-time light thumbnail (embedded preview, pre-develop — always SDR).

---

## Architecture

### The one change: `thumbnail_compute` (`app/src-tauri/src/commands.rs`)

Today the tail of `thumbnail_compute` is:

```rust
let fin = finish_image(&inv, &finish_from(params));
to_jpeg_b64(&fin, false, 82)
```

Change it to branch on `params.hdr`:

```rust
let finish = finish_from(params);
let sdr = finish_image(&inv, &finish);
if params.hdr {
    let hdr = film_core::finish::finish_image_hdr(&inv, &finish);
    let jpeg = crate::hdr::encode_gain_map_jpeg(&sdr, &hdr, THUMB_QUALITY)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    Ok(format!("data:image/jpeg;base64,{b64}"))
} else {
    to_jpeg_b64(&sdr, false, THUMB_QUALITY)
}
```

Notes:
- `finish_image_hdr` (Sub-project C, already `pub`) derives its own pre-shoulder super-white body from the same `inv` and applies `hdr_finish`. No separate `hdr=true` invert is needed — this is consistent with C, where highlight headroom comes from the un-clamped Faithful tone body, not an invert-side expansion.
- `inv` is the existing single invert computed earlier in `thumbnail_compute` — feed it to both `finish_image` and `finish_image_hdr` (compute `finish_from(params)` once).
- The bytes→data-URL wrapping mirrors `encode_hdr` (`commands.rs:1416-1418`): `encode_gain_map_jpeg` returns `Vec<u8>`, base64-encode into a `data:image/jpeg;base64,…` string. `base64::Engine` is already imported at the top of `commands.rs`.
- **`THUMB_QUALITY`**: introduce a module const `const THUMB_QUALITY: u8 = 82;` (82 is the current thumbnail JPEG quality) and use it for BOTH branches, so the SDR branch stays byte-identical to today (same quality 82) and the HDR branch matches.

### Why nothing else changes

The per-image `hdr` flag already flows into every bake and re-bake, and every display surface already renders the baked data URL:

- **Active develop frame:** `refreshThumb()` (`app/src/lib/tabs/Develop.svelte:340-361`) is reactive on `$params` (which includes `hdr`). Toggling HDR re-runs `api.thumbnail(id, effParams, view)` and `api.saveThumbnail`, so the baked thumbnail switches to/from gain-map automatically and persists.
- **Roll / library frames:** `thumbRegen.regenOne` (`app/src/lib/develop/thumbRegen.ts`) bakes with each frame's saved edits (which carry `hdr`); the `thumb_stale` worker re-bakes on engine-version bumps and apply-to-roll.
- **Display:** `ExportModal.svelte`, `Filmstrip.svelte`/`FramePreview.svelte`, and `Grid.svelte` all render `<img src={img.thumbnail}>`. A gain-map data URL glows with no frontend change.

So the feature is one backend branch plus its tests. No TypeScript changes.

---

## Cross-platform behavior

- **macOS (WKWebView):** renders the gain map → thumbnails glow. (Same mechanism as the original single-image HDR preview.)
- **Windows (WebView2 / Chromium):** renders MPF gain maps in `<img>` → glows. `params.hdr` is reachable on Windows (gain-map was the Windows HDR path).
- **Linux (WebKitGTK):** no gain-map HDR → `<img>` shows the SDR base (graceful). The HDR toggle is hidden on Linux, so `params.hdr` stays false and the thumbnail takes the unchanged SDR branch.

---

## Edge cases & constraints

- **SDR byte-identical:** for `params.hdr == false`, the encode is `to_jpeg_b64(&sdr, false, 82)` exactly as today (the shared `THUMB_QUALITY = 82` preserves the value). No non-HDR thumbnail changes.
- **Tone mode:** `finish_image_hdr` needs the Faithful super-white body for real headroom (per C). Production `build_params` always sets Faithful, so this is a non-issue; a hypothetical Filmic+`hdr` thumbnail would simply carry a near-flat gain map (no glow), not an error.
- **Size/cost:** an HDR-flagged thumbnail is ~1.3–1.8× larger (SDR base + gain map) and costs one extra small `finish_image_hdr` render. Only HDR-flagged images pay this. Acceptable for thumbnail sizes.
- **Zero-dimension / tiny thumbnails:** `encode_gain_map_jpeg` already guards zero dimensions and sdr/hdr dimension mismatch (returns `Err`); `sdr` and `hdr` here are the same `inv` at the same size, so dimensions always match.

---

## Testing

- **Backend unit tests** (`app/src-tauri/src`, `cargo test --lib`):
  - `thumbnail_compute` (or a thin extract of its encode tail) with `params.hdr = true` on a blown-highlight fixture emits a **gain-map** JPEG — assert the MPF/gain-map marker is present, mirroring the existing `encode_gain_map_jpeg_emits_a_gain_map` test (`app/src-tauri/src/hdr.rs:139`).
  - With `params.hdr = false`, emits a **plain** JPEG data URL (`data:image/jpeg;base64,…`) with **no** gain-map marker (the SDR-parity guard: the non-HDR path is unchanged by construction — same `to_jpeg_b64(&sdr, false, THUMB_QUALITY)` call — so the test asserts absence of the gain map rather than golden bytes).
- **On-device visual (the real gate, macOS HDR display):**
  - Toggle HDR on a frame → its tile glows in the develop bottom strip, the library grid, and the export window; SDR-only images look unchanged; toggling HDR off reverts the tile to SDR.
  - Sanity: a grid/strip with several HDR tiles renders without compositor issues.

---

## Decomposition (tasks for the implementation plan)

1. **Backend HDR-thumbnail branch** — add `THUMB_QUALITY` const; branch `thumbnail_compute` on `params.hdr` to dual-render (`finish_image` + `finish_image_hdr`) and encode a gain-map data URL, else the unchanged SDR path; unit tests for the gain-map-present (hdr) and SDR-parity (non-hdr) cases.
2. **On-device visual acceptance** (USER) — glow in all three surfaces, SDR unchanged, toggle on/off, multi-tile sanity.

---

## Constraints (Global)

- **SDR byte-identical** — `params.hdr == false` thumbnails are unchanged (`to_jpeg_b64(&sdr, false, 82)`); introduce `THUMB_QUALITY = 82` and use it in both branches so the value is preserved in one place.
- **Reuse Sub-project C** — the HDR rendition uses `film_core::finish::finish_image_hdr` + `crate::hdr::encode_gain_map_jpeg`; no new imaging logic, no third copy of the HDR finish.
- **Display-only** — no frontend/TypeScript changes; the exported contact-sheet file stays SDR (`exportSheet.ts` untouched).
- **Gated by `params.hdr`** — HDR thumbnails only for images whose `hdr` flag is set.
- **Commit discipline** — work on `main`; exact-path `git add app/src-tauri/src/commands.rs` (the WIP-in-src-tauri caveat: never `-A`/`.`/`app`/`crates`). If an `_anon.*.llvm.*` link error appears, `cargo clean -p app`.
- Build/test: `cd app/src-tauri && cargo build` / `cargo test --lib`.
