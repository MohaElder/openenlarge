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

## M_post fitting — results (2026-06-03)

Built the density-unmixing matrix fitter from a physical forward model (real per-dye spectral
densities from spektrafilm, analytic Gaussian sensor, Planckian-D55 stand-in illuminant).

**The math works strongly (synthetic + real-data held-out):**
- Synthetic overlapping dyes: fitted `M_post` beats identity by >20% on held-out patches.
- **Real Portra 400 dye data: RMS ΔC 0.094 (fit) vs 0.303 (identity) — a 3.2× error reduction**
  on held-out concentration patches, with significant off-diagonal crosstalk terms.
  → Spec Assumption #2 RESOLVED positively: the physical model produces a *strongly*
  non-trivial unmixing matrix. The "more scientific" differentiator is real, not marginal.

**But on a real scan, the generic matrix shifts color rather than clearly improving it.**
- V600 color frame, Mode B with `--stock portra400` vs identity: outputs DIFFER (as designed),
  but the Portra matrix pushes the image visibly **warmer / magenta**, not obviously "more
  correct."
- Cause: the matrix was fit against an *assumed* Gaussian sensor + Planckian illuminant that do
  NOT match the V600's actual sensor spectral sensitivity and lamp. The unmixing is calibrated
  to the wrong optics, so on a mismatched scanner it re-tints instead of neutralizing.

**Verdict / honest conclusion:** the M_post machinery and the physics are validated; real dye
data gives a large *modeled* improvement. Converting that into a real-scan quality win requires
matching the **sensor term** to the actual capture device. So the clear next unlock is
**per-camera/scanner spectral-sensitivity fitting** (e.g. from a ColorChecker shot) — it is the
gating step, not an optional refinement. The generic baked matrices are a stepping stone, not a
shippable default.

### Revised next priorities
1. **Per-camera/scanner SS fitting** (ColorChecker) — the real unlock; makes `M_post` correct
   for the actual device.
2. Frame/rebate detection + crop (camera-scan base sampling).
3. Then Phase 7 Tauri shell + WB/tone defaults.

---

## Library-first develop workflow + preview quality (2026-06-04)

Implemented the import → "Develop all" → Develop flow with a Preview Quality setting. All
backend (13) + frontend (5 vitest) tests green; builds clean.

**What shipped:**
- **Light import** — `import_image` no longer full-decodes; it reads the DNG embedded preview
  for the thumbnail + metadata + stores the path. (`develop_image` does the heavy decode.)
- **`develop_image`** — decodes → builds the working image at the **quality cap** (Performance
  4096 / Quality full) + a 256px auto-WB thumb + base; **drops full_res** (memory-bounded).
- **`set_quality`** + render/export now operate on the developed working image; **export always
  re-decodes full-res** from the path.
- **UI** — "Develop all" button (Source panel), full-screen **progress overlay**, auto-switch to
  Develop, **Develop tab disabled when empty**, **confirm popup** on early jump, right-click
  **Quality context menu** (changing quality re-develops all).

**Measured (real DNG, release/dev-optimized):**
- Light-import thumbnail via `decode_tiff` embedded preview: works (173×256). ~1.1s/image **from
  external disk** (IO-bound; far faster than old full decode; quicker from internal SSD). Each
  file is currently read twice (preview at import, full at develop) — a future optimization.
- Develop: decode → working 1549×2292 → orange-mask base `[0.44,0.20,0.11]` ✓.

**Memory model:** cache = N × working-image (Performance ≈ 147MB cap) + thumbnails; full_res only
transient during `develop_image`/export. Bounded for medium-format on 16GB; Quality mode is the
user's opt-in via the context menu.

**Manual E2E checklist (run live, `npm run tauri dev`):** drop a batch → instant-ish Library +
metadata; Develop tab disabled when empty; "Develop all" → progress → lands in Develop at Fit
(no first-frame magnify); jump-to-Develop-early → popup; Performance vs Quality sharpness; export
full-res in both. (GUI checks pending — done by the user.)

**Known follow-ups:** RAF (non-TIFF) light-import falls back to a placeholder thumbnail (no
embedded-preview extraction yet); files read twice (import preview + develop decode); quality is
session-global (not persisted); develop is sequential (no parallelism).

## Library redesign + app polish (2026-06-04)

Mockup validated via the visual companion, then implemented. Backend 13 + vitest 8 green; builds clean.

**Shipped:**
- **Window opens at 90% of screen, titled "RedRoom"** (sized at startup from the monitor).
- **Library = folder navigator + zoomable grid** (replaces the bottom filmstrip in Library;
  Develop keeps its filmstrip). macOS-style tree (inline Lucide icons: hard-drive/folder/chevron),
  per-folder counts, built from this session's imported paths (`buildTree`, vitest-tested). Import
  button at the bottom (label "Import").
- **Grid just shows the images** (no badges/wash); selecting a cell sets the active image; a
  "Thumb size" slider scales cells (120–320px).
- **Live thumbnails:** `develop_image` now renders an **inverted** thumbnail (grid flips to
  positive after develop); editing the active image refreshes its grid cell via a new
  `thumbnail(id,params)` command (debounced) — reflects white/black/tone edits.
- **Develop tab badge** = count of undeveloped images.
- Polish: tightened spacing, Lucide iconography, glass panels, `--text-faint` token.

**Out of scope (separate task):** file persistence (remembering imports across launches); the
navigator is session-only for now.

**Manual E2E (run live, `npm run tauri dev`):** window opens ~90% titled RedRoom; import from two
folders → tree shows both volumes/folders with counts → select each → grid filters; thumb-size
slider; "Develop all" → thumbnails flip to positives; edit black/white on the active image → its
grid thumbnail updates; Develop badge counts undeveloped.

## Real-file verdict (2026-06-03) — PIPELINE VALIDATED ✅

Ran `film-cli --compare` on both user files.

- **`Image 4.dng`** (Epson V600, color negative): confirmed a **linear RGB DNG**. Routed via
  `decode_tiff` (NOT rawler). Auto-detected orange mask `[0.43, 0.19, 0.11]` (R>G>B ✓).
  Inverted to a believable color positive (street/crosswalk frame). The linear-DNG open item
  is RESOLVED: `.dng` with PhotometricInterpretation=RGB must go through `decode_tiff`.
- **`_DSF0072.RAF`** (GFX100RF, 207MB/102MP): a camera photo of a **Shanghai GP3 B&W negative**
  strip on a lightbox. Decoded via rawler in ~8s full-res (3 inversions). Inverted correctly.

**Key findings:**
1. **B == C right now** (identity `M_pre`/`M_post`) — byte-identical output. The engine is
   currently negadoctor-level (per-channel log-density). It modestly beats naive (better
   shadows/highlights), but the scientific differentiator (cross-channel matrices) is unfit
   and therefore inert. **Fitting `M_pre`/`M_post` is the #1 substantive next step.**
2. **Base auto-sampling is too naive for camera scans.** Whole-image 95th percentile grabbed
   the white lightbox surround (GFX) rather than the film rebate, giving a wrong base. Camera
   scans include rebate + sprockets + lightbox. Need **frame/rebate detection + crop** before
   base sampling. (Scanner DNG was fine because it's tightly cropped to the frame.)
3. **Perf is acceptable:** 102MP/207MB full-res invert in ~8s on CPU; proxy preview will make
   the UI interactive.

### Prioritized next work (post-POC)
1. **Frame + rebate detection / crop** (biggest immediate quality lever for camera scans).
2. **`M_pre`/`M_post` calibration fitting** (the real "more scientific than NLP" differentiator).
3. White-balance + tone defaults for pleasing out-of-box color.
4. Then Phase 7 Tauri shell with proxy preview.

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

## GPU Develop Pipeline — Phase 0 spike (2026-06-05)

Spec: `specs/2026-06-04-gpu-develop-pipeline-design.md`, Assumption #1.

**Verdict: `[SPIKE float16] ok=true reason="RGBA16F render target OK"`** — the app's WKWebView
WebGL2 supports `EXT_color_buffer_float` and an `RGBA16F` render target is framebuffer-complete.

→ **Green light.** Plans 2–3 (float-texture upload, GPU inversion, offscreen tiled export) proceed
as designed; no RGBA8 encoded-range fallback or WebGPU detour needed. Probe added in
`app/src/lib/viewport/gl/renderer.ts::float16RenderTargetSupported()`.

## GPU Develop — Plan 2 results (2026-06-05)

Phases 2–4 landed on `main`: working image uploaded once as `RGBA16F` (half-float, ≤8192 cap),
GPU two-pass invert→finish, inversion + geometry driven by uniforms. The per-frame base64-JPEG
path is gone for the common (no-dust) case; exposure/temp/tint/mode/stock/zoom update with no
backend round-trip. Dust/IR active, raw view, or no-WebGL2 → unchanged CPU `render_view` path
(no parity gap). Export still CPU (Plan 3 moves it to offscreen GPU).

**User-verified (E2E):** inverted preview renders correctly; one bug found + fixed —
**vertical flip in crop mode** (root cause: float source uploaded `UNPACK_FLIP_Y=false` while
the display convention expected the 8-bit path's `true`; `sourceUV` mapped clip-space y-up
straight into top-down image space). Fixed in `INVERT_FRAG` by converting `v_uv` to image space
(`uv.y = 1.0 - uv.y`) before crop/orient/straighten — full frame and crop both correct after.

**Automated:** frontend build clean; 88 vitest; 5 `gpu_upload` Rust tests. Every Rust↔TS↔GLSL
contract (byte layout, JSON keys, uniform names, column-major matrices) traced consistent in a
final integration review, which also caught + fixed two bugs (double-render on eligible loads;
`useFloat` never reset → dust/IR fallback drew the un-healed invert).

**Still to confirm before Plan 3** (low risk, isolated sign tweaks if off): rotate-90 / flip-V /
straighten directions (this fix moved geometry into the image-space convention those matrices
assume, so it likely corrected them too).

## GPU Develop — Plan 3a results (2026-06-05)

Phase 5 landed on `main`: dust-eraser / IR-removal images now stay on the fast GPU path. When
dust/IR is active the frontend requests a *baked* working texture — Rust applies geometry
(orient/rotate/crop) and runs the Telea inpaint (`dust::apply`/`apply_ir`) on the **raw negative**
(pre-invert), returns half-float RGBA; the GPU inverts+finishes with identity geometry. Re-bake
fires only on stroke-commit / IR-toggle / geometry change (off the session lock; `info` skips the
inpaint so it runs once per upload).

**User-verified (E2E):** dust heals correctly and exposure stays instant with dust on; the
post→pre-invert heal-domain change looks equivalent to the prior CPU-fallback heal. CPU
`render_view` is now used only for raw view and no-WebGL2.

## GPU Develop — Plan 3b results (2026-06-05) — ARC COMPLETE

Phase 6 landed on `main`: export now renders through the SAME WebGL2 shader as the preview
(single source of truth). Rust decodes full-res, `bake_working` applies geometry + pre-invert
dust/IR heal, ships half-float bytes + resolved uniforms; a dedicated offscreen `FinishRenderer`
runs invert→finish into an FBO and reads back (RGBA8 for 8-bit formats, RGBA16F→f32 for 16-bit);
Rust builds an `Image` and encodes with the existing encoders. Oversize (> `MAX_TEXTURE_SIZE`),
no-WebGL2, or a non-renderable FBO format → unchanged CPU `export_image` fallback.

**User-verified (E2E):** preview unregressed after the `draw()` split; JPEG / 8-bit PNG / 16-bit
TIFF export correctly; GPU export pixel-matches the CPU export. Because export reuses
`bake_working`, preview and export now heal in the same (pre-invert) domain.

**Known minor (deferred):** on the oversize/null fallback path, `export_begin` has already stashed
a full-res half-float buffer in `Session.pending_export` that the GPU path won't consume; it's
bounded (overwritten by the next export, released by `export_finish` on success) but lingers until
then. Add an `export_cancel` / release on fallback if it ever matters.

### Whole arc summary (Plans 1 → 3b, all on `main`)
1. **rayon** — `invert_image`/`finish_image`/`apply_texture` multi-core.
2. **GPU inversion** — decode-once RGBA16F upload, two-pass invert→finish shader, exposure/temp/
   tint/zoom with no backend round-trip; killed the per-frame base64-JPEG path.
3. **dust/IR on GPU** — Rust-baked pre-invert heal keeps dust/IR images on the fast path.
4. **GPU export** — same shader for preview and export, CPU fallback for edge cases.
