# HDR Thumbnails Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When an image's per-image `hdr` flag is set, bake its thumbnail as a gain-map JPEG so it renders in HDR in the export window, develop bottom strip, and library grid — reusing Sub-project C's chroma-preserving finish; non-HDR thumbnails stay byte-identical.

**Architecture:** One backend change in `app/src-tauri/src/commands.rs`. Extract the thumbnail encode tail of `thumbnail_compute` into a small `encode_thumb(inv, params)` helper that branches on `params.hdr`: HDR → dual-render (`finish_image` + `finish_image_hdr`) → `encode_gain_map_jpeg` → base64 data URL; SDR → the unchanged `to_jpeg_b64`. No frontend changes: the per-image `hdr` flag already flows into every bake/re-bake, and all three surfaces already render `<img src={img.thumbnail}>`.

**Tech Stack:** Rust (Tauri `app` crate), `film-core` (`finish_image`, `finish_image_hdr`), `crate::hdr::encode_gain_map_jpeg`, `base64`. Tests: `cargo test --lib` in `app/src-tauri`.

## Global Constraints

- **SDR byte-identical** — `params.hdr == false` thumbnails are unchanged: `to_jpeg_b64(&sdr, false, 82)`. Introduce `const THUMB_QUALITY: u8 = 82;` and use it in both branches so the value lives in one place.
- **Reuse Sub-project C** — HDR rendition uses `film_core::finish::finish_image_hdr` + `crate::hdr::encode_gain_map_jpeg`. No new imaging logic, no third copy of the HDR finish.
- **Display-only** — no frontend/TypeScript changes; the exported contact-sheet file stays SDR (`exportSheet.ts` untouched).
- **Gated by `params.hdr`** — HDR thumbnails only for images whose `hdr` flag is set.
- **No separate `hdr=true` invert** — feed the SAME existing `inv` to both `finish_image` and `finish_image_hdr` (per C, headroom comes from the un-clamped Faithful tone body, not an invert-side expansion).
- **Commit discipline** — work on `main`; `git add app/src-tauri/src/commands.rs` (EXACT path only; the user keeps long-lived uncommitted WIP in `app/src-tauri/*.rs` — never `-A`/`.`/`app`/`crates`). If an `_anon.*.llvm.*` link error appears, run `cargo clean -p app` then rebuild.

---

### Task 1: Gain-map thumbnail encode branch

**Files:**
- Modify: `app/src-tauri/src/commands.rs` — add `const THUMB_QUALITY: u8 = 82;` (near the other consts at `commands.rs:89-91`); extract `encode_thumb`; rewire the tail of `thumbnail_compute` (`commands.rs:1871-1872`); add two unit tests in the `#[cfg(test)] mod tests` block.
- Test: same file (Rust in-crate `mod tests`).

**Interfaces:**
- Consumes (all already imported at `commands.rs:17-21`): `film_core::finish::finish_image`, `film_core::finish::finish_image_hdr`, `crate::hdr::encode_gain_map_jpeg` (`fn(&Image, &Image, u8) -> Result<Vec<u8>, String>`), `to_jpeg_b64` (`fn(&Image, bool, u8) -> Result<String, String>`), `finish_from` (`fn(&InvertParams) -> FinishParams`), `base64::Engine` (in scope via `use base64::Engine;` at `commands.rs:21`). `InvertParams` has a `pub hdr: bool` field (`session.rs:73`). Test helper `crate::commands_test_support::sample_invert_params()` returns an `InvertParams` in Faithful tone mode (used by the existing `render_and_encode_hdr_hdr_rendition_preserves_highlight_chroma` test).
- Produces: `fn encode_thumb(inv: &film_core::Image, params: &InvertParams) -> Result<String, String>` — a module-private helper returning a JPEG data URL (`data:image/jpeg;base64,…`); gain-map when `params.hdr`, plain SDR otherwise. `thumbnail_compute` calls it as its final expression.

- [ ] **Step 1: Write the failing tests**

Add these two tests inside the existing `#[cfg(test)] mod tests { … }` block in `app/src-tauri/src/commands.rs` (the same module that already contains `render_and_encode_hdr_hdr_rendition_preserves_highlight_chroma`). They call `encode_thumb`, which does not exist yet — so the module will fail to compile (expected TDD red).

```rust
    // A gain-map JPEG carries either the ISO 21496-1 URN or Apple's `hdrgainmap`
    // marker in its bytes (same detection the encoder's own test uses, hdr.rs).
    fn has_gain_map(data_url: &str) -> bool {
        use base64::Engine;
        let b64 = data_url
            .strip_prefix("data:image/jpeg;base64,")
            .expect("thumbnail is a jpeg data url");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("valid base64");
        let iso = b"urn:iso";
        let apple = b"hdrgainmap";
        bytes.windows(iso.len()).any(|w| w == iso)
            || bytes.windows(apple.len()).any(|w| w == apple)
    }

    // A small warm-ish near-white positive (post-invert) buffer: bright enough that
    // the Faithful finish pushes the highlight into headroom for a real gain map.
    fn bright_inv() -> film_core::Image {
        film_core::Image {
            width: 8,
            height: 8,
            pixels: vec![[0.98, 0.90, 0.82]; 64],
            ir: None,
        }
    }

    #[test]
    fn encode_thumb_with_hdr_flag_emits_gain_map() {
        let mut params = crate::commands_test_support::sample_invert_params();
        params.hdr = true;
        let url = encode_thumb(&bright_inv(), &params).expect("encode");
        assert!(url.starts_with("data:image/jpeg;base64,"), "not a jpeg data url");
        assert!(has_gain_map(&url), "hdr thumbnail must carry a gain map");
    }

    #[test]
    fn encode_thumb_without_hdr_flag_has_no_gain_map() {
        let mut params = crate::commands_test_support::sample_invert_params();
        params.hdr = false;
        let url = encode_thumb(&bright_inv(), &params).expect("encode");
        assert!(url.starts_with("data:image/jpeg;base64,"), "not a jpeg data url");
        assert!(!has_gain_map(&url), "sdr thumbnail must not carry a gain map");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd app/src-tauri && cargo test --lib encode_thumb`
Expected: FAILS to compile — `cannot find function encode_thumb in this scope` (the helper doesn't exist yet).

- [ ] **Step 3: Add the `THUMB_QUALITY` constant**

In `app/src-tauri/src/commands.rs`, next to the existing thumbnail/quality consts (`commands.rs:89-91`, which currently read `const THUMB_EDGE: u32 = 320;` and `const PREVIEW_JPEG_QUALITY: u8 = 88;`), add:

```rust
/// JPEG quality for baked catalog thumbnails (SDR and HDR gain-map alike).
const THUMB_QUALITY: u8 = 82;
```

- [ ] **Step 4: Add the `encode_thumb` helper**

Add this module-private function to `app/src-tauri/src/commands.rs` (place it immediately above `fn thumbnail_compute`, i.e. just before `commands.rs:1815`'s `#[allow(clippy::too_many_arguments)]`):

```rust
/// Encode one finished thumbnail to a JPEG data URL. When `params.hdr` is set,
/// dual-render the SDR base (`finish_image`) and the chroma-preserving HDR
/// rendition (`finish_image_hdr`, Sub-project C) from the SAME inverted buffer
/// and mux them into a gain-map JPEG so the `<img>` glows on HDR displays; below
/// that flag it is the plain SDR JPEG, byte-identical to the pre-HDR-thumbnail
/// behavior. No separate `hdr=true` invert: `finish_image_hdr` derives its own
/// super-white body from `inv` (headroom comes from the un-clamped Faithful body).
fn encode_thumb(inv: &film_core::Image, params: &InvertParams) -> Result<String, String> {
    let finish = finish_from(params);
    let sdr = finish_image(inv, &finish);
    if params.hdr {
        let hdr = finish_image_hdr(inv, &finish);
        let jpeg = crate::hdr::encode_gain_map_jpeg(&sdr, &hdr, THUMB_QUALITY)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        Ok(format!("data:image/jpeg;base64,{b64}"))
    } else {
        to_jpeg_b64(&sdr, false, THUMB_QUALITY)
    }
}
```

- [ ] **Step 5: Rewire `thumbnail_compute` to use the helper**

In `app/src-tauri/src/commands.rs`, replace the final two lines of `thumbnail_compute` (`commands.rs:1871-1872`):

```rust
    let fin = finish_image(&inv, &finish_from(params));
    to_jpeg_b64(&fin, false, 82)
```

with:

```rust
    encode_thumb(&inv, params)
```

(`inv` is the already-computed inverted+dust+IR buffer; `params` is the `&InvertParams` argument. The SDR path is preserved exactly — `encode_thumb` calls `to_jpeg_b64(&sdr, false, THUMB_QUALITY)` with `THUMB_QUALITY == 82`.)

- [ ] **Step 6: Run the new tests to verify they pass**

Run: `cd app/src-tauri && cargo test --lib encode_thumb`
Expected: PASS — `encode_thumb_with_hdr_flag_emits_gain_map` and `encode_thumb_without_hdr_flag_has_no_gain_map` both green.

- [ ] **Step 7: Run the full app lib suite + build to verify no regression**

Run: `cd app/src-tauri && cargo test --lib && cargo build`
Expected: all tests pass (the prior count + 2), `cargo build` clean with no warnings. (If a linker error mentioning `_anon.*.llvm.*` appears, run `cargo clean -p app` then re-run.)

- [ ] **Step 8: Commit**

```bash
git add app/src-tauri/src/commands.rs
git commit -m "feat(hdr): bake gain-map thumbnails when the per-image hdr flag is set"
```

---

### Task 2: On-device visual acceptance (USER)

Not a code task — the real acceptance gate, run by the user on a macOS HDR display after Task 1 ships. No automated coverage (HDR glow is not unit-testable).

Checklist:
- [ ] Toggle HDR on a developed frame → its tile glows in the **develop bottom strip**, the **library grid**, and the **export window**.
- [ ] SDR-only (HDR-off) images look unchanged in all three surfaces.
- [ ] Toggling HDR **off** reverts the tile to SDR (the reactive re-bake in `Develop.svelte` swaps it back).
- [ ] A grid/strip showing several HDR tiles at once renders without compositor glitches.

---

## Notes for the executor

- This plan is a single code task (Task 1) plus a user visual gate (Task 2). Task 1's deliverable is independently testable via the two `encode_thumb` unit tests.
- `finish_image_hdr` and `finish_image` are both already imported at `commands.rs:18`; `base64::Engine` at `commands.rs:21`. No new `use` statements are required.
- Do not touch any `.ts`/`.svelte` files, `exportSheet.ts`, or `film-core`. The whole change is contained in `commands.rs`.
