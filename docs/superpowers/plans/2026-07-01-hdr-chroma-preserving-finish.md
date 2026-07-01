# Chroma-preserving HDR Finish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One shared `film-core` HDR finalize that preserves highlight chroma into headroom (no more gray), used by both the live MSL shader and the CPU gain-map export — retiring the split-body+excess workaround, SDR untouched.

**Architecture:** Add `hdr_finish(body, sdr)` to `film-core`: below the knee it returns `sdr` (parity); above, it tones the highlight *luminance* via the existing `hdr_finalize` shoulder and reconstructs RGB from the pre-shoulder body's chromaticity, blended across the knee. An HDR-mode finish (`finish_pixel_hdr`/`finish_image_hdr`) exposes the pre-shoulder body and applies it. The CPU export routes the HDR rendition through it (dropping split-body+excess); the MSL `finish_frag` mirrors it.

**Tech Stack:** Rust (`film-core`, Tauri `app` crate), MSL (Metal Shading Language), existing GLSL/CPU finish as the parity reference.

## Global Constraints

- **SDR / HDR-off byte-identical.** `finish_pixel`, `display_finalize`, and the SDR output are unchanged. Existing finish/tone tests must stay green. The HDR finish is invoked ONLY for the HDR rendition/mode.
- **Single source of truth:** `film-core::hdr_finish` is the reference; MSL mirrors it exactly; the CPU export calls it. No third divergent copy.
- **Below-knee == SDR exactly** — the extension is identity below `HDR_KNEE`. Parity is the correctness bar.
- **Retire split-body+excess** from `commands.rs::render_hdr_image`.
- **Constants:** `HDR_KNEE=0.8`, `HDR_HEADROOM=2.5` (`engine.rs`); reuse `hdr_finalize` (scalar tanh shoulder), `smoothstep`, `luma`.
- **Build/test:** `cargo test -p film-core` (root workspace) for film-core; `cd app/src-tauri && cargo build` / `cargo test --lib` for the Tauri crate; MSL can't compile in-env (runtime/visual-verified). Commit discipline: exact-path `git add` only (user WIP in `app/src-tauri/*.rs`), NEVER `-A`/`.`/`app`/`crates`; work on `main`. `_anon.*.llvm` link bug → `cargo clean -p app`.
- On-device visual tuning is expected (Task 5); the math gets close, the eye finalizes the tunables.

---

## Reference map

- `crates/film-core/src/finish.rs`: `hdr_finalize` (570-598, scalar tanh shoulder → `[KNEE,HEADROOM)`, `#[allow(dead_code)]` today), `hdr_finalize_rgb` (570-598 area, dead), `tone_curve` (523-540), `finish_pixel` (718-740), `luma` (45), `smoothstep` (used in tone_curve, scalar `smoothstep(e0,e1,x)`), `finish_image` (applies `finish_pixel` per pixel + the USM/texture spatial pass).
- `crates/film-core/src/engine.rs`: `display_finalize` (277-279), `shoulder_only` (254-262), `look_s` (173-184), `HDR_KNEE=0.8` (115), `HDR_HEADROOM=2.5` (118).
- `app/src-tauri/src/commands.rs`: `render_hdr_image` split-body+excess (1305-1350), `render_and_encode_hdr` (1372-1391), `export_image_hdr` (2076-2149).
- `app/src-tauri/src/hdr_surface/msl.rs`: `finish_frag` HDR gain block (597-610) — has `bodyU` (pre-shoulder body) and `disp` (SDR color) in scope.

---

## File structure

- **Modify** `crates/film-core/src/finish.rs` — add `hdr_finish`; extract `tone_body` from `tone_curve` (byte-identical); add `finish_pixel_hdr` + `finish_image_hdr`.
- **Modify** `crates/film-core/src/engine.rs` — add `HDR_W_HI` const (blend upper bound); make `hdr_finalize` reachable (it's in finish.rs actually — keep there). (engine.rs only if a const must live with `HDR_KNEE`.)
- **Modify** `app/src-tauri/src/commands.rs` — `render_and_encode_hdr` computes the body invert once, `sdr = finish_image`, `hdr = finish_image_hdr`; delete `render_hdr_image` split-body+excess.
- **Modify** `app/src-tauri/src/hdr_surface/msl.rs` — mirror `hdr_finish` in `finish_frag`.

---

## Task 1: `hdr_finish` in film-core (the shared reference)

**Files:**
- Modify: `crates/film-core/src/finish.rs` (add `hdr_finish`)
- Modify: `crates/film-core/src/finish.rs` (add `HDR_W_HI` const near the other HDR consts, or `engine.rs` if it must sit with `HDR_KNEE`)
- Test: inline `#[cfg(test)]` in `finish.rs`

**Interfaces:**
- Consumes: `hdr_finalize(v: f32) -> f32` (existing), `HDR_KNEE`, `HDR_HEADROOM`, `smoothstep(e0,e1,x)`.
- Produces: `pub(crate) fn hdr_finish(body: [f32;3], sdr: [f32;3]) -> [f32;3]` — below-knee returns `sdr`; above, blends `sdr → (body/max(body))·hdr_finalize(max(body))` via `smoothstep(HDR_KNEE, HDR_W_HI, max(body))`. `HDR_W_HI: f32 = 1.2` (blend upper bound, tunable).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn hdr_finish_below_knee_is_sdr_exactly() {
    // max(body) <= HDR_KNEE (0.8) → identity: the HDR extension must not touch it.
    let sdr = [0.42, 0.31, 0.55];
    let body = [0.5, 0.4, 0.3];
    assert_eq!(hdr_finish(body, sdr), sdr);
}

#[test]
fn hdr_finish_preserves_highlight_chroma_and_extends_luma() {
    // A blown WARM highlight: body is warm (R>G>B) & super-white; sdr is the shoulder-
    // desaturated near-white. hdr_finish must keep the warm hue and push luminance >1.
    let body = [2.0, 1.4, 0.9];
    let sdr = [0.98, 0.96, 0.93]; // near-white (what the shoulder produced)
    let out = hdr_finish(body, sdr);
    // Well above the blend top (HDR_W_HI=1.2) → fully the reconstructed highlight.
    // Luminance extended into headroom:
    let m = out[0].max(out[1]).max(out[2]);
    assert!(m > 1.0, "expected super-white luminance, got {m}");
    assert!(m <= HDR_HEADROOM + 1e-4);
    // Chroma preserved (NOT gray): R>G>B, and the ratios match body's chromaticity.
    assert!(out[0] > out[1] && out[1] > out[2], "warm order lost: {out:?}");
    let rg_body = body[0] / body[1];
    let rg_out = out[0] / out[1];
    assert!((rg_body - rg_out).abs() < 1e-3, "hue drift: body R/G {rg_body} vs out {rg_out}");
}

#[test]
fn hdr_finish_continuous_at_knee() {
    // Just above the knee, output ≈ sdr (w=0 at the knee) — no ring/edge.
    let body = [0.8001, 0.6, 0.4];
    let sdr = [0.8, 0.6, 0.4];
    let out = hdr_finish(body, sdr);
    for c in 0..3 { assert!((out[c] - sdr[c]).abs() < 1e-2, "kink at knee: {out:?} vs {sdr:?}"); }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p film-core hdr_finish`
Expected: FAIL (function missing).

- [ ] **Step 3: Implement `hdr_finish`** in `finish.rs`:

```rust
/// Blend top for the HDR highlight reconstruction: below HDR_KNEE the extension
/// is identity (== SDR); by HDR_W_HI it is fully the chroma-preserved highlight.
/// Tunable on-device (Task 5). MUST equal the MSL `hdr_finish` in msl.rs.
pub(crate) const HDR_W_HI: f32 = 1.2;

/// Chroma-preserving HDR finalize. `body` = the pre-shoulder tone body (per-channel,
/// super-white — carries the real highlight chroma); `sdr` = the finished SDR color
/// in [0,1] (byte-identical to finish_pixel). Below the knee returns `sdr` (parity);
/// above, tones the highlight LUMINANCE via `hdr_finalize` and reconstructs RGB from
/// `body`'s chromaticity (`body/max(body)`), blended sdr→highlight across the shoulder.
/// The single source of truth mirrored by the MSL shader and used by the CPU export.
pub(crate) fn hdr_finish(body: [f32; 3], sdr: [f32; 3]) -> [f32; 3] {
    let m_u = body[0].max(body[1]).max(body[2]);
    if m_u <= crate::engine::HDR_KNEE {
        return sdr;
    }
    let l_hdr = hdr_finalize(m_u); // scalar tanh shoulder → [HDR_KNEE, HDR_HEADROOM)
    let inv = l_hdr / m_u; // chromaticity (body/m_u) scaled to the HDR luminance
    let highlight: [f32; 3] = std::array::from_fn(|c| body[c] * inv);
    let w = smoothstep(crate::engine::HDR_KNEE, HDR_W_HI, m_u); // 0 at knee → 1 by HDR_W_HI
    std::array::from_fn(|c| sdr[c] + (highlight[c] - sdr[c]) * w)
}
```

(If `hdr_finalize`/`HDR_KNEE`/`HDR_HEADROOM`/`smoothstep` visibility doesn't reach here, bump to `pub(crate)`; do not change their behavior. Remove the now-used `#[allow(dead_code)]` on `hdr_finalize` since it's live.)

- [ ] **Step 4: Run to verify it passes** + full suite

Run: `cargo test -p film-core`
Expected: PASS (3 new tests + no regression).

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(hdr-c): chroma-preserving hdr_finish (luma-tone + body chromaticity)"
```

---

## Task 2: HDR-mode finish (expose the pre-shoulder body; SDR byte-identical)

**Files:**
- Modify: `crates/film-core/src/finish.rs` (extract `tone_body`; add `finish_pixel_hdr`, `finish_image_hdr`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `hdr_finish` (Task 1), existing `finish_pixel`, `apply_per_zone_wb`, `brightness_gain`, `display_finalize`.
- Produces:
  - `fn tone_body(v: f32, p: &FinishParams) -> f32` — the tone-slider math of `tone_curve` WITHOUT the finalize/clamp (Whites/Blacks/Highlights/Shadows/Contrast). `tone_curve` is refactored to call it (byte-identical).
  - `pub fn finish_pixel_hdr(rgb: [f32;3], p: &FinishParams) -> [f32;3]` — `sdr = finish_pixel(rgb, p)`; `body[c] = tone_body(per_zone_wb(rgb)[c] * brightness_gain, p)`; returns `hdr_finish(body, sdr)`.
  - `pub fn finish_image_hdr(img: &Image, p: &FinishParams) -> Image` — like `finish_image` but per-pixel `finish_pixel_hdr`; the USM/texture spatial pass is SKIPPED for the HDR rendition (matches the live surface, which has no USM pass — documented). 

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn tone_body_extraction_keeps_tone_curve_identical() {
    // tone_curve must be byte-identical after extracting tone_body.
    let p = test_finish_params(); // finalize_body = true
    for v in [0.0f32, 0.3, 0.7, 0.892, 1.0, 1.6, 2.4] {
        let got = tone_curve(v, &p);
        // display_finalize(tone_body(v)) is the finalize_body=true definition:
        let expect = crate::engine::display_finalize(tone_body(v, &p));
        assert!((got - expect).abs() < 1e-6, "v={v}: {got} != {expect}");
    }
}

#[test]
fn finish_pixel_hdr_below_knee_equals_finish_pixel() {
    // A dark/mid pixel whose tone body stays under the knee → HDR == SDR exactly.
    let p = test_finish_params();
    let rgb = [0.2, 0.18, 0.15];
    assert_eq!(finish_pixel_hdr(rgb, &p), finish_pixel(rgb, &p));
}

#[test]
fn finish_pixel_hdr_extends_bright_highlight() {
    // A super-white input (blown highlight) → HDR output exceeds 1.0 (SDR clamps ~1).
    let p = test_finish_params();
    let rgb = [2.2, 1.6, 1.0];
    let hdr = finish_pixel_hdr(rgb, &p);
    let sdr = finish_pixel(rgb, &p);
    let mh = hdr[0].max(hdr[1]).max(hdr[2]);
    let ms = sdr[0].max(sdr[1]).max(sdr[2]);
    assert!(mh > 1.0 && mh > ms, "hdr {mh} should exceed sdr {ms} and 1.0");
}
```

(Reuse/define `test_finish_params()` matching existing finish tests — `finalize_body: true`, neutral sliders.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p film-core finish_pixel_hdr`
Expected: FAIL.

- [ ] **Step 3: Implement.** Extract `tone_body` from `tone_curve` and rewire `tone_curve` to call it (byte-identical), then add `finish_pixel_hdr` + `finish_image_hdr`:

```rust
/// The tone-slider math (Whites/Blacks/Highlights/Shadows/Contrast) — the body of
/// tone_curve WITHOUT finalize/clamp. Shared by tone_curve and finish_pixel_hdr so
/// they can't drift.
fn tone_body(v: f32, p: &FinishParams) -> f32 {
    let mut v = v;
    v += p.whites * 0.20 * v.powi(3);
    v += p.blacks * 0.20 * (1.0 - v).powi(3);
    v += p.highlights * 0.18 * smoothstep(0.5, 1.0, v);
    v += p.shadows * 0.18 * (1.0 - smoothstep(0.0, 0.5, v));
    0.5 + (v - 0.5) * (1.0 + p.contrast)
}

fn tone_curve(v: f32, p: &FinishParams) -> f32 {
    if p.finalize_body {
        crate::engine::display_finalize(tone_body(v, p))
    } else {
        // Positive/HDR-rendition passthrough: leading clamp → sliders → trailing clamp
        // (byte-identical to the pre-extraction behavior).
        tone_body(v.clamp(0.0, 1.0), p).clamp(0.0, 1.0)
    }
}

/// HDR-mode per-pixel finish: the byte-identical SDR result PLUS a chroma-preserving
/// highlight extension via hdr_finish. `body` is the per-channel tone body (super-white).
pub fn finish_pixel_hdr(rgb: [f32; 3], p: &FinishParams) -> [f32; 3] {
    let sdr = finish_pixel(rgb, p); // unmodified SDR (byte-identical)
    let wb = apply_per_zone_wb(rgb, &p.per_zone);
    let g = brightness_gain(p.brightness);
    let body = [tone_body(wb[0] * g, p), tone_body(wb[1] * g, p), tone_body(wb[2] * g, p)];
    hdr_finish(body, sdr)
}
```

For `finish_image_hdr`: copy `finish_image`'s structure but call `finish_pixel_hdr` per pixel and SKIP the USM/texture spatial pass (add a doc comment: HDR rendition has no USM pass, matching the live EDR surface; texture-slider-on-HDR is a documented follow-up).

- [ ] **Step 4: Run to verify it passes** + full suite (SDR byte-identical regression)

Run: `cargo test -p film-core`
Expected: PASS (new tests + all existing finish/tone tests unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/film-core/src/finish.rs
git commit -m "feat(hdr-c): finish_pixel_hdr/finish_image_hdr (expose body, apply hdr_finish); tone_body extracted"
```

---

## Task 3: Route the CPU gain-map export through the HDR finish

**Files:**
- Modify: `app/src-tauri/src/commands.rs` (`render_and_encode_hdr`; delete `render_hdr_image`'s split-body+excess)
- Test: the existing `render_and_encode_hdr_emits_gain_map` test (should still pass) + a new assertion.

**Interfaces:**
- Consumes: `film_core::finish::finish_image_hdr` (Task 2), existing `finish_image`, `invert_image_core`, `encode_gain_map_jpeg`.
- Produces: `render_and_encode_hdr` computes the inverted body ONCE, `sdr = finish_image(&inv, finish)`, `hdr = finish_image_hdr(&inv, finish)`, then `encode_gain_map_jpeg(&sdr, &hdr, quality)`. `render_hdr_image` (split-body+excess) is removed.

- [ ] **Step 1: Confirm the failing/limiting state.** The existing `render_and_encode_hdr_emits_gain_map` test (commands.rs ~4169) passes today with the workaround. Add an assertion that the HDR rendition carries real super-white AND is not merely the clamped-body+excess: e.g. render a synthetic blown-warm image and assert the HDR rendition's highlight retains chroma (R>G>B) and >1.0, whereas the split approach would (document why). Write it to FAIL against the current split path if feasible; else keep it as the post-change guard.

- [ ] **Step 2: Implement.** In `render_and_encode_hdr` (commands.rs:1372-1391): keep the single `invert_image_core` + dust/IR, then:

```rust
    let sdr = finish_image(&inv, finish);              // Faithful SDR (finalize_body=true)
    let hdr = film_core::finish::finish_image_hdr(&inv, finish); // chroma-preserving HDR
    crate::hdr::encode_gain_map_jpeg(&sdr, &hdr, quality)
```

Delete `render_hdr_image` (the split-body+excess fn, commands.rs:1305-1350) and any now-unused imports. (Note: `finish_image_hdr` needs `pub` from film-core — Task 2 exposes it.) The HDR rendition input is the SAME `inv` body the SDR uses (it's the super-white Faithful body; `finish_image_hdr` derives its own body internally), so no separate `hdr=true` invert is needed.

- [ ] **Step 3: Run tests**

Run: `cd app/src-tauri && cargo test --lib render_and_encode_hdr` then full `cargo test --lib`
Expected: PASS (gain-map still emitted; new chroma assertion passes; no regression). `cargo build` clean.

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/commands.rs
git commit -m "feat(hdr-c): gain-map export uses chroma-preserving finish_image_hdr; retire split-body+excess"
```

---

## Task 4: Mirror `hdr_finish` in the live MSL shader

**Files:**
- Modify: `app/src-tauri/src/hdr_surface/msl.rs` (`finish_frag`, replace the gain block 597-610)

**Note:** MSL can't compile in-env; gate = compiles (Rust builds) + code-review + runtime/visual at Task 5.

**Interfaces:**
- Consumes: `bodyU` (pre-shoulder body) + `disp` (SDR color), both already in `finish_frag` scope; `hdr_finalize_scalar` (already in MSL), `HDR_KNEE`, `HDR_HEADROOM`. Add `HDR_W_HI = 1.2` MSL constant (MUST equal film-core `HDR_W_HI`).

- [ ] **Step 1: Replace the HDR gain block** in `finish_frag` (msl.rs:597-610) with the MSL port of `hdr_finish` — same math as `film-core::hdr_finish`:

```metal
    // Chroma-preserving HDR finalize (mirror of film-core::hdr_finish). disp = SDR color.
    float mU = max(bodyU.r, max(bodyU.g, bodyU.b));
    float3 outc;
    if (mU <= HDR_KNEE) {
        outc = disp;                              // below knee: exact SDR parity
    } else {
        float lHdr = hdr_finalize_scalar(mU);     // tanh shoulder → [KNEE, HEADROOM)
        float3 highlight = bodyU * (lHdr / mU);   // body chromaticity at HDR luminance
        float w = smoothstep(HDR_KNEE, HDR_W_HI, mU);
        outc = mix(disp, highlight, w);
    }
    int code = clipCode(disp, u);
    return float4(srgbToLinearExt3(clipOverlay(outc, code, u)), 1.0);
```

Add `constant float HDR_W_HI = 1.2;` alongside the other constants. Keep the `srgbToLinearExt3` output (the linearization fix from B) and the clip overlay. Match `film-core::hdr_finish` exactly.

- [ ] **Step 2: Build**

Run: `cd app/src-tauri && cargo build`
Expected: compiles (cargo clean -p app if `_anon`). MSL validity is inspected + runtime-verified at Task 5.

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/hdr_surface/msl.rs
git commit -m "feat(hdr-c): MSL finish_frag mirrors hdr_finish (chroma-preserving highlights)"
```

---

## Task 5: On-device visual acceptance + tuning (USER)

**Files:** none (verification + optional const tweaks).

- [ ] **Step 1: MANUAL GUI acceptance (human, macOS HDR display)** — `cd app && npm run tauri dev`:
  - **SDR unchanged:** HDR-off looks exactly as before (byte-identical Faithful look).
  - **HDR highlight color:** with HDR on, crank exposure — highlights now KEEP their color (a warm blown highlight stays warm/bright, not gray). The exposure→gray is gone.
  - **Knee blend:** no ring/edge/banding at the highlight boundary (the `sdr→highlight` blend is seamless).
  - **Live + zoom + all B behaviors** still hold; no shader-compile error (`create_surface failed` in stderr).
- [ ] **Step 2: Export A/B** — export a JPEG with HDR on; the exported gain-map's HDR highlights match the live surface (view in Apple Photos / Preview; confirm HDR + SDR fallback).
- [ ] **Step 3: Tune if needed** — if highlights transition too fast/slow or the hue drifts, adjust `HDR_W_HI` (blend top) in BOTH `film-core` (finish.rs) and MSL (msl.rs) to keep them in lockstep; optionally add the spec's hue-reconcile (rotate `body` chromaticity toward `disp`'s hue) if grade/mixer highlight hue must be respected. Re-verify. Commit any const changes:

```bash
git commit --allow-empty -m "test(hdr-c): on-device HDR chroma acceptance (SDR unchanged, highlights keep color, export matches)"
```

---

## Self-Review notes

- **Spec coverage:** shared `hdr_finish` in film-core (T1) ✓; luma-tone + body-chromaticity + knee blend (T1) ✓; HDR-mode finish exposing the body, SDR byte-identical (T2) ✓; retire split-body+excess + export wiring (T3) ✓; MSL mirror (T4) ✓; on-device tuning + export A/B (T5) ✓; single source of truth (film-core reference, MSL mirrors, export calls) ✓; SDR-untouched constraint (T2 regression + T1 below-knee parity) ✓.
- **TDD coverage:** T1 (hdr_finish math) and T2 (tone_body extraction byte-identical + finish_pixel_hdr) are fully unit-tested; T3 has the gain-map + chroma guard; T4 is review-gated (MSL can't compile in-env); T5 is the human visual/tuning gate. Honest, matching the HDR work's pattern.
- **Type consistency:** `hdr_finish(body,sdr)→[f32;3]` (T1) consumed by `finish_pixel_hdr` (T2); `finish_image_hdr` (T2) consumed by export (T3); `HDR_W_HI=1.2` shared film-core (T1) ↔ MSL (T4); `tone_body` extraction keeps `tone_curve` identical.
- **Known documented decision:** the HDR rendition skips the USM/texture spatial pass (matches the live surface; texture-slider-on-HDR is a follow-up).
