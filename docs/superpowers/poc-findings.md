# POC Findings

**Date:** 2026-06-03
**Branch:** `feat/inversion-poc`

## Status: code-complete, green; real-file verdict pending samples

The density-domain inversion engine (Approach B) and the CLI are implemented, tested, and
clean. What remains for the actual "is the color right?" verdict is running `--compare` on
real V600 + GFX files (Phase 6), which needs sample files.

## What was built (Phases 0–5 + compare)

- **`film-core`** (pure Rust lib, 16 unit tests, clippy-clean):
  - `Image` — f32 linear RGB + optional IR plane.
  - `engine` — Mode B (`Ĉ = M_post · log10(M_pre · (base/I))`), Mode C (per-channel
    log-density), naive (`1-x`). Validated against a synthetic Beer-Lambert forward model:
    neutral scenes recover neutral; brightness order preserved; naive provably differs from B.
  - `calibrate::sample_base` — 95th-percentile film-base (orange mask) estimate, empty-region safe.
  - `decode::decode_tiff` — 8/16-bit RGB/RGBA TIFF + linear DNG; 4th channel → `ir`.
  - `decode::decode_raw` — RAF/DNG via rawler 0.7 (`decode_file` → `RawDevelop` with
    Rescale/Demosaic/Crop, excluding WhiteBalance/Calibrate/SRgb → camera-native linear RGB).
  - `export::write_tiff16` — 16-bit RGB TIFF.
- **`film-cli`** — `invert <in> -o <out> [--mode b|c|naive] [--base-rect x,y,w,h] [--exposure]
  [--black] [--gamma] [--compare]`. `--compare` emits `*_b/_c/_naive.tiff` side by side.
  Decode dispatched by extension (tif/tiff → decode_tiff, else → decode_raw).

Verified: `cargo build --release`, `cargo test` (16 pass), `cargo clippy --all-targets` (clean).
Smoke-tested CLI single + compare on synthetic TIFF fixtures.

## Open items to resolve with real files (Phase 6)

1. **Linear-DNG vs Bayer path (most important).** The CLI sends `.dng` → `decode_raw`
   (demosaic path). A SilverFast V600 DNG is likely `LinearRaw` (already RGB, no Bayer) — the
   demosaic step may be wrong for it. Action on first file: check rawler's photometric
   interpretation; if `LinearRaw`/`Cfa` mismatch, route linear DNGs to `decode_tiff` (or a
   no-demosaic rawler path). GFX `.raf` (true Bayer) should use `decode_raw` as-is.
2. **Spec §9 assumption — "positive" scan.** Confirm the V600 DNG is the un-inverted negative
   (orange, linear). If it's an already-inverted positive, re-export the raw/HDR linear scan.
3. **The 4th/IR channel.** Confirm the "64-bit" V600 DNG carries an IR plane and that it lands
   in `Image.ir` (preserved for future dust removal).
4. **The verdict.** Run `film-cli ... --compare`, open B/C/naive; confirm B's neutrals are
   visibly cleaner than naive and B beats C in saturated regions. Record per-file results here.

## Deferred (Phase 7)

Tauri shell, GPU/proxy preview, M_pre/M_post calibration fitting, ICC/spectral, AI dust/color.
