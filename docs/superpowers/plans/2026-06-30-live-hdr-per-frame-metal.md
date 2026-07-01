# Live HDR Per-Frame (Metal/MSL) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the macOS EDR display surface render the full invert+finish pipeline per-frame from uniforms (~60 fps live editing) with a proper super-white-preserving HDR finish, replacing the settle-based CPU-render-and-blit.

**Architecture:** Port `INVERT_FRAG` + `FRAG` (the GLSL SDR pipeline) to MSL in the existing native `CAMetalLayer` surface. Upload the raw negative once; drive geometry + invert + finish per-frame via a packed uniforms buffer + tone-curve LUT texture, triggered by `Viewport.svelte`'s existing `finishKey`/invert reactive path through a lightweight rAF-throttled Tauri command. Add a real HDR finalize mode (color-managed tone-shoulder) to `film-core` first (as the tested reference) and mirror it in MSL, retiring the CPU `split-body+excess` workaround for the live path.

**Tech Stack:** Rust (Tauri 2, objc2/Metal, film-core), MSL (Metal Shading Language), TypeScript/Svelte, existing WebGL2 GLSL as the port reference.

## Global Constraints

- **macOS-only.** All new native code `#[cfg(target_os="macos")]`-gated; non-macOS compiles unchanged; Windows keeps routing to the gain-map fallback.
- **SDR WebGL path and HDR-off behavior unchanged.** The SDR canvas keeps rendering (hidden in live-edr) to drive the histogram.
- **MSL must match `crates/film-core/src/finish.rs` + `app/src/lib/viewport/gl/shaders.ts` GLSL exactly** — same steps, same constants (`BRIGHTNESS_DENSITY_RANGE=0.5`, OKLab consts, `SKIN_HUE=0.70`, `SKIN_DAMP=0.5`, tone slider gains 0.20/0.20/0.18/0.18, etc.). Parity is the correctness bar.
- **No early [0,1] clamp during the body phase** (per-zone WB uses `.max(0.0)` only; tone tools see super-white body). The HDR finalize preserves >1.0.
- **Per-frame param IPC only** (params ~hundreds of bytes), throttled to `requestAnimationFrame`. Source pixels never cross IPC per-frame — the raw negative is uploaded native-side on rare events only.
- **Native Cocoa/Metal objects are main-thread-only** — reuse A's `with_webview` main-thread hop + `unsafe impl Send/Sync` (main-thread-only invariant) + main-thread `Drop`.
- **Commit discipline:** exact-path `git add` only (user keeps long-lived WIP in `app/src-tauri/*.rs`); NEVER `-A`/`.`/`app`/`crates`. Work on `main`.
- **Build/test:** Tauri crate is a separate workspace — `cd app/src-tauri && cargo build`, `cargo test --lib` (package `app`, lib `app_lib`). film-core: `cargo test -p film-core` (root workspace). TS: `cd app && npm run test:unit` / `npm run check`. Do NOT run `npm run tauri dev` from a subagent (it hangs) — GUI checks are the human's.

---

## Reference map (port sources — read these before touching shaders)

- **GLSL finish** `app/src/lib/viewport/gl/shaders.ts`: `FRAG` finish (16-332), `INVERT_FRAG` (409-617), `USM_FRAG` (346-402); OKLab (256-280), rgb/hsl (142-174), tone_curve (88-96), oklabSaturate (256-280), colorGrade (116-127), colorMixer (177-189), pointColor (198-215), applyPerZoneWb (106-114).
- **GLSL uniforms** (the 48): listed in `FRAG` — tone (`u_contrast/highlights/shadows/whites/blacks`), `u_vibrance/saturation/texture/brightness`, color grade (`u_cg_*`), mixer (`u_cm_hue/sat/lum[8]`), point (`u_pc_*`), per-zone (`u_pz_*`), clip (`u_clip_*`, `u_soft_clip`), `u_lut`, `u_src`, `u_finalize_body`. Invert uniforms in `INVERT_FRAG`: `u_base`, `u_wb`, `u_exposure`, `u_black`, `u_gamma`, `u_tone_mode`, geometry (`crop_off`, `crop_scale`, `angle`, `orient`, `view_off`, `view_scale`, `aspect`).
- **Rust reference** `crates/film-core/src/finish.rs`: `finish_pixel` (659-681), `tone_curve` (522-539), `apply_saturation` (607-654), `color_grade` (429-442), `color_mix` (233-252), `point_color` (272-300), `apply_per_zone_wb` (89-103), `brightness_gain` (507-509); HDR knee/headroom consts in `crates/film-core/src/engine.rs` (`HDR_KNEE=0.8`, `HDR_HEADROOM`). LUT sampling: `crates/film-core/src/curve.rs::sample_lut` (115-124).
- **Renderer reference** `app/src/lib/viewport/gl/renderer.ts`: `setSourceFloat` (311-337), `drawInvertPass` (376-408), `drawFinishPass` (415-470), `draw` (527-531).
- **Current native surface** `app/src-tauri/src/hdr_surface/macos.rs`: `EDR_SHADER_SRC` blit (53-78), `create_surface` (297-396), `upload_texture` (401-464), `render` (500-541), `position` (470-496). Fused render path `app/src-tauri/src/commands.rs::hdr_surface_render_show` + `render_hdr_buffer`.
- **Frontend trigger** `app/src/lib/viewport/Viewport.svelte`: `finishKey` (676-700), `drawGL` (329-337), `scheduleHdr` (452-457), `encodeHdr` (423-446), reactive (704), `hdrViewSpec`, `canvasRect`.

---

## File structure

- **Modify** `app/src-tauri/src/hdr_surface/macos.rs` — replace blit shader with the ported MSL pipeline; add uniforms buffer + LUT texture; new render path.
- **Create** `app/src-tauri/src/hdr_surface/msl.rs` — the MSL shader source strings (invert + finish + HDR finalize) as `const &str`, isolated from the objc2 plumbing so the ~500-line shader is one focused file.
- **Create** `app/src-tauri/src/hdr_surface/uniforms.rs` — the `#[repr(C)] HdrUniforms` packed struct + a `from_params(...)` builder; the byte-layout test.
- **Modify** `app/src-tauri/src/hdr_surface/mod.rs` — new commands `hdr_surface_set_source`, `hdr_surface_set_uniforms`; keep `hdr_surface_hide`/`set_rect`.
- **Modify** `app/src-tauri/src/lib.rs` — register the new commands.
- **Modify** `crates/film-core/src/finish.rs` — add the HDR finalize mode (the tested reference for the MSL tone-shoulder).
- **Modify** `app/src-tauri/src/commands.rs` — the raw-negative source provider for `hdr_surface_set_source`; route the HDR render through the new finalize mode (retire `split-body+excess` for this path).
- **Modify** `app/src/lib/api.ts` — bindings for the new commands.
- **Modify** `app/src/lib/viewport/Viewport.svelte` — rAF-throttled per-frame uniforms push; source-upload trigger on rare events; retire the settle debounce + gesture crutch for live-edr.

---

## Task 1: Uniforms contract (Rust struct + MSL struct + LUT)

**Files:**
- Create: `app/src-tauri/src/hdr_surface/uniforms.rs`
- Create: `app/src-tauri/src/hdr_surface/msl.rs` (add the `HdrUniforms` MSL `struct` declaration + a stub `constant` binding; shader bodies filled in Tasks 2-4)
- Modify: `app/src-tauri/src/hdr_surface/mod.rs` (`mod uniforms; mod msl;`)

**Interfaces:**
- Produces: `#[repr(C)] pub struct HdrUniforms { ... }` — a packed struct mirroring the MSL `constant HdrUniforms&` layout (all 48 finish uniforms + invert uniforms + geometry), with `pub fn from_params(params: &InvertParams, view: &ViewSpec, clip: &ClipState) -> HdrUniforms`. Field order/types MUST match `msl.rs`'s `struct HdrUniforms` byte-for-byte (Metal `constant` uses C-like alignment; use explicit padding fields and 16-byte alignment for `float3`/arrays per MSL rules).

- [ ] **Step 1: Write the failing test** in `uniforms.rs` `#[cfg(test)]`:

```rust
#[test]
fn hdr_uniforms_layout_matches_msl() {
    // MSL `constant HdrUniforms` packs float=4, float3 aligned to 16, arrays of float aligned to 16.
    // These asserts pin the Rust layout so it can't drift from the shader struct.
    assert_eq!(std::mem::align_of::<HdrUniforms>(), 16);
    // size must be a multiple of 16 (MSL constant buffer rule)
    assert_eq!(std::mem::size_of::<HdrUniforms>() % 16, 0);
    // spot-check a couple of documented field offsets (update these to the real layout you define)
    assert_eq!(memoffset::offset_of!(HdrUniforms, contrast), 0);
}
```

(Add `memoffset` as a dev-dependency of the app crate if not present, OR compute offsets via a `const` assertion pattern the repo already uses — check `app/src-tauri/Cargo.toml` first and match the existing approach; if adding a dep is undesirable, assert only `align_of`/`size_of % 16` and document field order in a comment.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd app/src-tauri && cargo test --lib hdr_uniforms_layout_matches_msl`
Expected: FAIL (struct doesn't exist).

- [ ] **Step 3: Implement `HdrUniforms`** in `uniforms.rs` — a `#[repr(C, align(16))]` struct with every uniform from the GLSL `FRAG` + `INVERT_FRAG` (see Reference map), using `[f32;3]`+pad for `float3`, `[[f32;4];N]`-style padded arrays for the 8-band mixer / point-color / grade arrays (pad each `float3` array element to `float4` to match MSL array alignment). Add `from_params(...)` mapping `InvertParams`/`ViewSpec`/clip → the packed fields (mirror the JS `finishUniforms`/`colorGrade`/`colorMix`/`perZoneWb`/`clipUniforms` builders in `app/src/lib/viewport/gl/*` and Viewport.svelte `drawGL`). In `msl.rs`, declare the matching `struct HdrUniforms { ... }` in MSL with identical field order + padding.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd app/src-tauri && cargo test --lib hdr_uniforms_layout_matches_msl`
Expected: PASS

- [ ] **Step 5: Build** `cd app/src-tauri && cargo build` — clean (cargo clean -p app if `_anon` link bug recurs).

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/hdr_surface/uniforms.rs app/src-tauri/src/hdr_surface/msl.rs app/src-tauri/src/hdr_surface/mod.rs app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock
git commit -m "feat(hdr-b): HdrUniforms packed struct + MSL layout contract"
```

---

## Task 2: HDR finalize mode in film-core (the tested reference)

**Files:**
- Modify: `crates/film-core/src/finish.rs` (add `FinalizeMode::Hdr` alongside the existing Faithful-finalize / SDR-clamp behavior in `tone_curve`)
- Test: inline `#[cfg(test)]` in `finish.rs`

**Interfaces:**
- Produces: an HDR finalize path — a color-managed soft tone-shoulder in log-density that maps the finished super-white body into `[1.0, HDR_HEADROOM]`: monotonic, continuous with the body at the knee, luminance-aware so highlights don't skew hue. Exposed via the existing `finalize_body`/mode plumbing (add a third variant rather than a bool if cleaner). This is the reference the MSL finalize (Task 4) must match, and it also lets the CPU/gain-map path use a principled HDR curve instead of `split-body+excess`.

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn hdr_finalize_preserves_and_ceilings_superwhite() {
    // Below the knee: identity (matches SDR body).
    assert!((hdr_finalize(0.5) - 0.5).abs() < 1e-6);
    // At/above white: stays > 1.0 (super-white preserved), monotonic, and never exceeds the headroom ceiling.
    let a = hdr_finalize(1.2);
    let b = hdr_finalize(4.0);
    assert!(a > 1.0, "1.2 -> {a}");
    assert!(b > a, "monotonic: {a} !< {b}");
    assert!(b <= HDR_HEADROOM + 1e-4, "ceiling: {b}");
    // Continuity at the knee (no visible kink).
    let k = HDR_KNEE;
    assert!((hdr_finalize(k + 1e-4) - hdr_finalize(k - 1e-4)).abs() < 1e-2);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p film-core hdr_finalize_preserves_and_ceilings_superwhite`
Expected: FAIL (function missing).

- [ ] **Step 3: Implement `hdr_finalize`** — a soft shoulder: below `HDR_KNEE` identity; above, compress the remaining range into `[HDR_KNEE, HDR_HEADROOM]` with a smooth (e.g. `tanh`/rational) roll-off, applied to a luminance/max-channel factor so the three channels scale together (hue-preserving) rather than clamping per-channel. Wire it as the finalize choice for the HDR path. Keep Faithful and SDR-clamp behavior byte-identical for their existing callers (regression: the existing `filmic_anchors_black_and_white` and finish tests still pass).

- [ ] **Step 4: Run to verify it passes** + full film-core suite

Run: `cargo test -p film-core`
Expected: PASS (new test + no regression).

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(hdr-b): color-managed HDR finalize shoulder in film-core (reference for MSL)"
```

---

## Task 3: MSL invert port + raw-negative source path

**Files:**
- Modify: `app/src-tauri/src/hdr_surface/msl.rs` (add the invert MSL: geometry + inversion)
- Modify: `app/src-tauri/src/hdr_surface/macos.rs` (upload raw-negative source; two-target render: invert → intermediate)
- Modify: `app/src-tauri/src/hdr_surface/mod.rs` + `commands.rs` (`hdr_surface_set_source` provides the raw negative)

**Interfaces:**
- Consumes: `HdrUniforms` (Task 1) for invert + geometry uniforms.
- Produces: `hdr_surface_set_source(id, view)` uploads the raw-negative float texture native-side; the surface renders invert → an intermediate RGBA16F texture (the inverted positive, unclamped). Mirrors `renderer.ts::drawInvertPass`.

**Note:** MSL fragment shaders have no cheap unit test. This task's gate is (a) it compiles + renders without error, and (b) VISUAL parity vs the WebGL invert, checked in Task 6 / by the human. Keep the objc2 plumbing minimal and the MSL isolated in `msl.rs`.

- [ ] **Step 1: Port `INVERT_FRAG`** (`shaders.ts:409-617`) to MSL in `msl.rs`: geometry transform (crop/straighten/orient/view window from uniforms) + per-channel log-density inversion for the 4 modes, UNCLAMPED output. Match `film-core::engine.rs` invert math + constants exactly. Add an intermediate RGBA16F texture + render target in `macos.rs` (`allocInter`-equivalent).
- [ ] **Step 2: Implement `hdr_surface_set_source`** — a command that gets the raw-negative proxy (reuse whatever backend path feeds WebGL's `setSourceFloat`; likely a `render_view`-style raw path) and uploads it to the source Metal texture on the main thread. Called on image load/develop/proxy-change/hi-res-threshold.
- [ ] **Step 3: Build** `cd app/src-tauri && cargo build` clean.
- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/hdr_surface/msl.rs app/src-tauri/src/hdr_surface/macos.rs app/src-tauri/src/hdr_surface/mod.rs app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(hdr-b): MSL invert stage + raw-negative source upload"
```

---

## Task 4: MSL finish port (+ HDR finalize) driven by uniforms

**Files:**
- Modify: `app/src-tauri/src/hdr_surface/msl.rs` (add the finish MSL)
- Modify: `app/src-tauri/src/hdr_surface/macos.rs` (finish pass reads uniforms buffer + LUT texture; render intermediate → drawable)

**Interfaces:**
- Consumes: intermediate inverted texture (Task 3), `HdrUniforms` buffer + LUT texture (Task 1), the `hdr_finalize` reference (Task 2).
- Produces: the full finished HDR output into the layer drawable. The finish pass mirrors `FRAG` (`shaders.ts:16-332`) + `finish.rs::finish_pixel`, with the HDR finalize (Task 2) as the finalize step.

**Note:** Same as Task 3 — verified by parity harness (Task 6) + visual A/B, not unit test.

- [ ] **Step 1: Port `FRAG`** to MSL: per-zone WB (no early clamp) → brightness/density (`10^(b·0.5)`) → tone_curve (whites/blacks cubic, highlights/shadows smoothstep, contrast pivot; NO leading clamp) → OKLab saturation (chroma scale, vibrance, skin damping, gamut compression) → tone LUT sample (256×1) → color grade (3-region + global, clamp) → color mix (8-band hue kernel + gate) → point color (≤8 samples) → **HDR finalize (Task 2's shoulder)** → clipping overlay. Match every constant (Reference map).
- [ ] **Step 2: Wire the finish pass** in `macos.rs::render`: bind uniforms `MTLBuffer` (`setFragmentBuffer`), LUT texture (`setFragmentTexture` index 1), intermediate source (index 0); render into the drawable. Upload the LUT texture (256×1 RGBA) from `HdrUniforms`/params on curve change.
- [ ] **Step 3: Build** clean.
- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/hdr_surface/msl.rs app/src-tauri/src/hdr_surface/macos.rs
git commit -m "feat(hdr-b): MSL finish stage + HDR finalize, uniform-driven"
```

---

## Task 5: Per-frame plumbing — retire settle, go live

**Files:**
- Modify: `app/src-tauri/src/hdr_surface/mod.rs` + `lib.rs` (`hdr_surface_set_uniforms` command)
- Modify: `app/src/lib/api.ts` (bindings)
- Modify: `app/src/lib/viewport/Viewport.svelte` (rAF-throttled per-frame push; source-upload triggers; retire settle debounce + gesture crutch)

**Interfaces:**
- Consumes: `hdr_surface_set_source` (Task 3), the finish pipeline (Task 4).
- Produces: `hdr_surface_set_uniforms(id, params, view, clip)` — packs `HdrUniforms::from_params`, updates the surface's uniform `MTLBuffer`, re-renders (main-thread). Frontend: on `finishKey`/invert-key change in live-edr, call it throttled to `requestAnimationFrame`; on rare events call `hdr_surface_set_source`; the 200 ms `scheduleHdr` debounce and the hide-EDR/show-SDR-during-gesture logic are removed for the `live-edr` path (kept for `gainmap-fallback`).

- [ ] **Step 1: Add `hdr_surface_set_uniforms`** (Rust) + api.ts binding. Register in `lib.rs`.
- [ ] **Step 2: Rewire `Viewport.svelte`** live-edr path: replace the settle `scheduleHdr`→`encodeHdr`→`hdrSurfaceRenderShow` flow with (a) `hdr_surface_set_source` on the rare-event key (image/develop/proxy/hires), (b) an rAF-throttled `hdr_surface_set_uniforms(params…)` on `finishKey`/invert/geom change. Keep `gainmap-fallback` unchanged. Keep the SDR WebGL canvas rendering (hidden) for the histogram. Keep hide/teardown on image-switch / HDR-off / unmount.
- [ ] **Step 3: Verify** `cd app && npm run check` (0 errors) + `npm run test:unit` (green). `cd app/src-tauri && cargo build` clean.
- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/hdr_surface/mod.rs app/src-tauri/src/lib.rs app/src/lib/api.ts app/src/lib/viewport/Viewport.svelte
git commit -m "feat(hdr-b): live per-frame EDR (set_uniforms + rAF push), retire settle"
```

---

## Task 6: Parity harness + manual acceptance

**Files:**
- Create: `app/src-tauri/src/hdr_surface/parity_test.rs` (offscreen MSL render vs `film-core::finish_pixel` on swatches) — OR, if a Metal offscreen harness proves too heavy, document visual A/B as the parity gate and skip the harness (state which in the commit).
- Verification only otherwise.

- [ ] **Step 1: Parity spot check** — render a small set of swatches × param sets through the MSL finish into an offscreen RGBA16F texture, read back, and assert per-pixel closeness to `finish_pixel` (below-white) within tolerance. If the harness is disproportionate, skip and rely on visual A/B — record the decision.
- [ ] **Step 2: MANUAL GUI acceptance (human, HDR display)** — `cd app && npm run tauri dev`:
  - Every slider (exposure, WB, tone, color, saturation) updates the EDR **live at ~60 fps** during a drag — no 200 ms settle.
  - Below-white, EDR matches the SDR canvas (toggle HDR off/on).
  - Highlights are color-managed (no cyan/hue skew); super-white visibly glows.
  - Clipping warning appears in HDR mode.
  - Pan/zoom stays aligned and live; deep-zoom re-uploads source correctly.
  - Image switch / HDR-off / leaving develop cleanly hide the surface.
  - Regression: HDR-off SDR unchanged; (Windows still gain-map; Linux hidden).
- [ ] **Step 3: Commit** any final tweaks + record acceptance:

```bash
git commit --allow-empty -m "test(hdr-b): parity + manual GUI acceptance for live per-frame HDR"
```

---

## Self-Review notes

- **Spec coverage:** raw-negative-once upload + rare-event re-upload (T3, T5) ✓; MSL invert+finish port (T3, T4) ✓; HDR finalize mode absorbing C's core + retiring split-body+excess (T2 reference, T4 MSL) ✓; per-frame uniforms + rAF, retire settle/crutch (T5) ✓; uniforms contract/parity discipline (T1) ✓; SDR-coexistence/histogram (T5) ✓; clipping-overlay-in-HDR bonus (T4) ✓; testing (T1 layout unit, T2 shoulder unit, T6 parity+GUI) ✓; macOS-gating/main-thread constraints (all native tasks) ✓.
- **Honest limitation:** MSL fragment shaders (T3/T4) are not unit-testable cheaply; their gate is the parity harness/visual A/B (T6) + the film-core reference (T2) they mirror. Unit-tested parts: uniform byte-layout (T1), HDR shoulder (T2).
- **USM/texture slider:** the plan ports invert+finish; the separable USM/texture pass (`USM_FRAG`) is NOT ported in these tasks — if the texture slider must be live in HDR, add a follow-up task; otherwise the texture slider settles (documented limitation). Confirm with the parity/acceptance pass.
- **Dust markers in HDR:** out of scope unless the invert port trivially includes them; documented as a possible follow-up (matches A's deferral).
- **Type consistency:** `HdrUniforms` (T1) consumed by T3/T4/T5; commands `hdr_surface_set_source`/`set_uniforms`/`hide`/`set_rect` consistent across T3/T5; `hdr_finalize` (T2) referenced by T4.
