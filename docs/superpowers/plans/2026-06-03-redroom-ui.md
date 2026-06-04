# RedRoom UI (Tauri Shell) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build RedRoom — a Tauri + Svelte desktop app wrapping `film-core`, with a dark liquid-glass Lightroom-style UI: a Library tab (import + metadata, raw preview) and a Develop tab (invert + live adjustments + export).

**Architecture:** Tauri 2 app under `app/`. The Rust backend (`src-tauri`) adds a session cache, proxy/thumbnail generation, PNG-base64 encoding, and metadata extraction, orchestrating `film-core` unchanged. The Svelte/Vite/TS frontend talks to it through six typed commands. Preview = engine on a downscaled proxy returned as base64 PNG; export = full-res 16-bit TIFF.

**Tech Stack:** Tauri 2, Svelte 5 + Vite + TypeScript, Rust (`film-core` path dep, `image` for resize/PNG, `base64`, `serde`), CSS (dark glass theme, red accent).

**Reference spec:** `docs/superpowers/specs/2026-06-03-redroom-ui-design.md`

**Environment:** Work from `/Users/mohaelder/Repos/filmrev`, branch `feat/inversion-poc`. `cargo` is NOT on PATH — prefix cargo with `source "$HOME/.cargo/env" && `. Node/npm assumed available (verify in Task 0).

---

## File Structure

```
app/
├── src/                                Svelte frontend
│   ├── App.svelte                      layout + module-tab state
│   ├── main.ts                         Svelte mount
│   ├── lib/
│   │   ├── store.ts                    stores: images, activeId, module, params
│   │   ├── api.ts                      typed Tauri command wrappers
│   │   ├── glass/GlassPanel.svelte     reusable frosted container
│   │   ├── panels/Source.svelte        import + image list (left)
│   │   ├── panels/Filmstrip.svelte     bottom thumbnails
│   │   ├── panels/Metadata.svelte      right panel (Library)
│   │   ├── panels/Adjustments.svelte   right panel (Develop)
│   │   ├── tabs/Library.svelte
│   │   └── tabs/Develop.svelte
│   └── styles/theme.css                dark glass tokens + red accent
└── src-tauri/
    └── src/
        ├── main.rs                     Tauri builder + command registration
        ├── convert.rs                  Image<->image crate, proxy resize
        ├── encode.rs                   Image -> base64 PNG (gamma option)
        ├── metadata.rs                 best-effort EXIF/dimension extraction
        ├── session.rs                  in-memory cache + InvertParams/Metadata/ImageEntry types
        └── commands.rs                 the six Tauri commands
```

---

## Phase 0 — Scaffold

### Task 0: Scaffold the Tauri + Svelte app

**Files:** Create `app/` (via scaffolder), then edit `app/src-tauri/Cargo.toml`.

- [ ] **Step 1: Verify toolchain**

Run: `node --version && npm --version && source "$HOME/.cargo/env" && cargo --version`
Expected: all print versions. If `npm`/`node` missing, STOP and report BLOCKED (need Node ≥18).

- [ ] **Step 2: Scaffold into `app/`**

Run (non-interactive):
```bash
cd /Users/mohaelder/Repos/filmrev
npm create tauri-app@latest app -- --template svelte-ts --manager npm --yes
```
If the flags differ for the installed version, run `npm create tauri-app@latest app` and choose:
TypeScript frontend, **Svelte**, package manager **npm**. The result must be a Tauri 2 project
with a Svelte+TS frontend under `app/` (`app/src-tauri/` Rust, `app/src/` Svelte).

- [ ] **Step 3: Add backend dependencies**

In `app/src-tauri/Cargo.toml`, under `[dependencies]` add:
```toml
film-core = { path = "../../crates/film-core" }
image = "0.25"
base64 = "0.22"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```
(`tauri`, `serde` may already be present — keep existing versions; add the rest.)

- [ ] **Step 4: Install JS deps and verify build**

Run:
```bash
cd /Users/mohaelder/Repos/filmrev/app && npm install
source "$HOME/.cargo/env" && npm run tauri build -- --debug 2>&1 | tail -15
```
Expected: the Rust side compiles (film-core links) and Vite builds. A full bundle may be slow;
if `tauri build` is too heavy, `source "$HOME/.cargo/env" && (cd src-tauri && cargo build)` plus
`npm run build` (Vite) both succeeding is sufficient. STOP/report if film-core fails to link.

- [ ] **Step 5: Add app/ to git and commit**

```bash
cd /Users/mohaelder/Repos/filmrev
printf '\n/app/node_modules\n/app/dist\n/app/src-tauri/target\n' >> .gitignore
git add -A
git commit -m "chore: scaffold RedRoom Tauri+Svelte app, link film-core"
```

---

## Phase 1 — Rust backend (TDD)

### Task 1: Image conversion + proxy resize

**Files:** Create `app/src-tauri/src/convert.rs`; register `mod convert;` in `main.rs`.

- [ ] **Step 1: Write the failing test + implementation**

Create `app/src-tauri/src/convert.rs`:

```rust
//! Convert between film_core::Image (f32 linear RGB) and the `image` crate,
//! and downscale to a preview proxy.

use film_core::Image;
use image::{ImageBuffer, Rgb};

/// film_core::Image -> f32 RGB image buffer.
pub fn to_rgb32f(img: &Image) -> ImageBuffer<Rgb<f32>, Vec<f32>> {
    let mut buf = ImageBuffer::new(img.width as u32, img.height as u32);
    for (i, px) in img.pixels.iter().enumerate() {
        let x = (i % img.width) as u32;
        let y = (i / img.width) as u32;
        buf.put_pixel(x, y, Rgb([px[0], px[1], px[2]]));
    }
    buf
}

/// f32 RGB image buffer -> film_core::Image (ir dropped).
pub fn from_rgb32f(buf: &ImageBuffer<Rgb<f32>, Vec<f32>>) -> Image {
    let (w, h) = (buf.width() as usize, buf.height() as usize);
    let pixels = buf.pixels().map(|p| [p[0], p[1], p[2]]).collect();
    Image { width: w, height: h, pixels, ir: None }
}

/// Downscale so the long edge is at most `max_edge` px (preserving aspect).
/// Returns the input unchanged if already within bounds.
pub fn proxy(img: &Image, max_edge: u32) -> Image {
    let long = img.width.max(img.height) as u32;
    if long <= max_edge {
        return img.clone();
    }
    let scale = max_edge as f32 / long as f32;
    let nw = (img.width as f32 * scale).round().max(1.0) as u32;
    let nh = (img.height as f32 * scale).round().max(1.0) as u32;
    let buf = to_rgb32f(img);
    let resized = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
    from_rgb32f(&resized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, c: [f32; 3]) -> Image {
        Image { width: w, height: h, pixels: vec![c; w * h], ir: None }
    }

    #[test]
    fn roundtrip_preserves_pixels() {
        let img = solid(3, 2, [0.25, 0.5, 0.75]);
        let back = from_rgb32f(&to_rgb32f(&img));
        assert_eq!(back.width, 3);
        assert_eq!(back.height, 2);
        assert_eq!(back.pixels[0], [0.25, 0.5, 0.75]);
    }

    #[test]
    fn proxy_caps_long_edge_and_keeps_aspect() {
        let img = solid(4000, 2000, [0.4, 0.4, 0.4]);
        let p = proxy(&img, 2048);
        assert_eq!(p.width, 2048);
        assert_eq!(p.height, 1024);
    }

    #[test]
    fn proxy_noop_when_small() {
        let img = solid(100, 80, [0.1, 0.2, 0.3]);
        let p = proxy(&img, 2048);
        assert_eq!((p.width, p.height), (100, 80));
    }
}
```

In `app/src-tauri/src/main.rs`, add near the top (after any existing items): `mod convert;`

- [ ] **Step 2: Run tests**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo test convert::)`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/convert.rs app/src-tauri/src/main.rs
git commit -m "feat(redroom): image<->buffer convert + proxy downscale"
```

---

### Task 2: PNG base64 encoding (with display gamma option)

**Files:** Create `app/src-tauri/src/encode.rs`; `mod encode;` in `main.rs`.

- [ ] **Step 1: Write the implementation + test**

Create `app/src-tauri/src/encode.rs`:

```rust
//! Encode a film_core::Image to a base64 PNG (8-bit) for the webview.

use base64::Engine;
use film_core::Image;
use image::{ImageBuffer, Rgb};

/// Encode to base64 PNG. If `apply_gamma`, apply sRGB-ish display gamma (1/2.2)
/// — use for raw (linear) previews. Pass false for engine output that is already
/// tone-mapped.
pub fn to_png_b64(img: &Image, apply_gamma: bool) -> Result<String, String> {
    let g = if apply_gamma { 1.0 / 2.2 } else { 1.0 };
    let mut buf: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::new(img.width as u32, img.height as u32);
    for (i, px) in img.pixels.iter().enumerate() {
        let x = (i % img.width) as u32;
        let y = (i / img.width) as u32;
        let enc = |v: f32| -> u8 {
            let v = v.clamp(0.0, 1.0).powf(g);
            (v * 255.0).round() as u8
        };
        buf.put_pixel(x, y, Rgb([enc(px[0]), enc(px[1]), enc(px[2])]));
    }
    let mut bytes: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&buf, img.width as u32, img.height as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("png encode: {e}"))?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_decodable_png_data_uri() {
        let img = Image { width: 2, height: 1, pixels: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], ir: None };
        let uri = to_png_b64(&img, false).unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        let b64 = uri.strip_prefix("data:image/png;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        // Decodes back to a 2x1 image.
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
    }

    #[test]
    fn gamma_brightens_midtones() {
        let img = Image { width: 1, height: 1, pixels: vec![[0.25, 0.25, 0.25]], ir: None };
        let lin = to_png_b64(&img, false).unwrap();
        let gam = to_png_b64(&img, true).unwrap();
        // Different encodings → different base64.
        assert_ne!(lin, gam);
    }
}
```

Add `use image::ImageEncoder;` is needed for `.write_image` — include it: at the top of
`encode.rs` add `use image::ImageEncoder;` alongside the other `use` lines.

In `main.rs` add: `mod encode;`

- [ ] **Step 2: Run tests**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo test encode::)`
Expected: 2 tests PASS. (If `write_image`/`ExtendedColorType` names differ in image 0.25, adapt
to the crate's actual PNG-encode API, keeping the function signature + behavior.)

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/encode.rs app/src-tauri/src/main.rs
git commit -m "feat(redroom): Image -> base64 PNG encode with gamma option"
```

---

### Task 3: Metadata extraction (best-effort)

**Files:** Create `app/src-tauri/src/metadata.rs`; `mod metadata;` in `main.rs`.

- [ ] **Step 1: Write the implementation + test**

Create `app/src-tauri/src/metadata.rs`:

```rust
//! Best-effort image metadata for the Library panel. Camera/lens/exposure come
//! from rawler when the file is a RAW/DNG; dimensions + file size are always set.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Metadata {
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<String>,
    pub shutter: Option<String>,
    pub aperture: Option<String>,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub date: Option<String>,
}

/// Extract metadata. `width`/`height` are passed in from the already-decoded
/// image (authoritative); the rest is best-effort from rawler's EXIF and may be
/// None. `file_size` from the filesystem.
pub fn extract(path: &Path, width: u32, height: u32) -> Metadata {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut md = Metadata { width, height, file_size, ..Default::default() };

    // Best-effort RAW EXIF via rawler. Failures are non-fatal.
    if let Ok(raw) = rawler::decode_file(path) {
        md.camera = Some(format!("{} {}", raw.clean_make, raw.clean_model).trim().to_string())
            .filter(|s| !s.is_empty());
        let exif = &raw.exif;
        md.iso = exif.iso.map(|v| v.to_string());
        md.shutter = exif.exposure_time.map(|r| format!("{}/{}", r.n, r.d));
        md.aperture = exif.fnumber.map(|r| format!("f/{:.1}", r.n as f32 / r.d as f32));
        md.lens = exif.lens_model.clone().filter(|s| !s.is_empty());
        md.date = exif.date_time_original.clone().filter(|s| !s.is_empty());
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_and_size_always_set_even_for_missing_exif() {
        // A path that isn't a RAW (this source file) → rawler fails → only dims/size set.
        let p = Path::new(file!());
        let md = extract(p, 1234, 567);
        assert_eq!(md.width, 1234);
        assert_eq!(md.height, 567);
        assert!(md.file_size > 0);
        assert!(md.camera.is_none());
    }
}
```

In `main.rs` add: `mod metadata;`

NOTE on rawler EXIF field names: `clean_make`, `clean_model`, `exif.iso`, `exif.exposure_time`,
`exif.fnumber`, `exif.lens_model`, `exif.date_time_original` are the expected rawler 0.7 fields,
but exact names/types may differ. If they don't compile, run
`source "$HOME/.cargo/env" && cargo doc -p rawler --no-deps` or read the rawler source
(`~/.cargo/registry/src/*/rawler-0.7.2/src/`) for the real `RawImage`/`Exif` field names and
adapt — keep the `Metadata` struct shape and the contract (dims/size always set, EXIF best-effort
None on failure). The unit test must still pass (it relies only on dims/size for a non-RAW file).

- [ ] **Step 2: Run tests**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo test metadata::)`
Expected: 1 test PASS (after any rawler field-name adaptation).

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/metadata.rs app/src-tauri/src/main.rs
git commit -m "feat(redroom): best-effort metadata extraction"
```

---

### Task 4: Session cache + shared types

**Files:** Create `app/src-tauri/src/session.rs`; `mod session;` in `main.rs`.

- [ ] **Step 1: Write the implementation + test**

Create `app/src-tauri/src/session.rs`:

```rust
//! In-memory session: decoded images (full-res + proxy) keyed by id, plus the
//! serde types shared with the frontend.

use crate::metadata::Metadata;
use film_core::Image;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Knobs the UI sends for an inversion (mirrors the engine's exposed controls).
#[derive(Debug, Clone, Deserialize)]
pub struct InvertParams {
    pub mode: String,             // "b" | "c"
    pub stock: String,            // "none" | "portra400" | "fujic200"
    pub base_rect: Option<[usize; 4]>, // [x,y,w,h]
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
}

/// What the frontend gets per imported image.
#[derive(Debug, Clone, Serialize)]
pub struct ImageEntry {
    pub id: String,
    pub file_name: String,
    pub thumbnail: String, // data:image/png;base64,...
    pub metadata: Metadata,
}

/// A cached decoded image.
pub struct CachedImage {
    pub full_res: Image,
    pub proxy: Image,
    pub file_name: String,
    pub metadata: Metadata,
    pub thumbnail: String,
}

#[derive(Default)]
pub struct Session {
    pub images: Mutex<HashMap<String, CachedImage>>,
    pub next_id: Mutex<u64>,
}

impl Session {
    pub fn insert(&self, img: CachedImage) -> ImageEntry {
        let mut id_guard = self.next_id.lock().unwrap();
        let id = format!("img{}", *id_guard);
        *id_guard += 1;
        drop(id_guard);
        let entry = ImageEntry {
            id: id.clone(),
            file_name: img.file_name.clone(),
            thumbnail: img.thumbnail.clone(),
            metadata: img.metadata.clone(),
        };
        self.images.lock().unwrap().insert(id, img);
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(name: &str) -> CachedImage {
        let img = Image { width: 1, height: 1, pixels: vec![[0.0; 3]], ir: None };
        CachedImage {
            full_res: img.clone(),
            proxy: img,
            file_name: name.to_string(),
            metadata: Metadata::default(),
            thumbnail: "data:,".to_string(),
        }
    }

    #[test]
    fn insert_assigns_unique_incrementing_ids() {
        let s = Session::default();
        let a = s.insert(dummy("a.dng"));
        let b = s.insert(dummy("b.raf"));
        assert_eq!(a.id, "img0");
        assert_eq!(b.id, "img1");
        assert_eq!(s.images.lock().unwrap().len(), 2);
        assert_eq!(a.file_name, "a.dng");
    }
}
```

In `main.rs` add: `mod session;`

- [ ] **Step 2: Run tests**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo test session::)`
Expected: 1 test PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/session.rs app/src-tauri/src/main.rs
git commit -m "feat(redroom): session cache + shared serde types"
```

---

### Task 5: The Tauri commands

**Files:** Create `app/src-tauri/src/commands.rs`; wire into `main.rs`.

- [ ] **Step 1: Write commands.rs**

Create `app/src-tauri/src/commands.rs`:

```rust
//! Tauri commands orchestrating film-core for the RedRoom UI.

use crate::convert::proxy;
use crate::encode::to_png_b64;
use crate::metadata::extract;
use crate::session::{ImageEntry, InvertParams, CachedImage, Session};
use film_core::calibrate::{sample_base, Rect};
use film_core::decode::{decode_raw, decode_tiff};
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::spectral::Stock;
use std::path::Path;
use tauri::State;

const PROXY_EDGE: u32 = 2048;
const THUMB_EDGE: u32 = 256;

fn decode_any(path: &Path) -> Result<film_core::Image, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "tif" | "tiff" => decode_tiff(path).map_err(|e| format!("{e}")),
        _ => decode_raw(path).map_err(|e| format!("{e}")),
    }
}

fn stock_from(s: &str) -> Option<Stock> {
    match s {
        "portra400" => Some(Stock::Portra400),
        "fujic200" => Some(Stock::FujiC200),
        _ => None,
    }
}

fn build_params(p: &InvertParams, base: [f32; 3]) -> InversionParams {
    match stock_from(&p.stock) {
        Some(s) if p.mode == "b" => params_for_stock(s, base, p.exposure, p.black, p.gamma),
        _ => InversionParams { base, exposure: p.exposure, black: p.black, gamma: p.gamma, ..Default::default() },
    }
}

fn mode_from(s: &str) -> Mode {
    match s { "c" => Mode::C, _ => Mode::B }
}

#[tauri::command]
pub fn import_image(path: String, session: State<Session>) -> Result<ImageEntry, String> {
    let p = Path::new(&path);
    let full = decode_any(p)?;
    let proxy_img = proxy(&full, PROXY_EDGE);
    let thumb_img = proxy(&full, THUMB_EDGE);
    let thumbnail = to_png_b64(&thumb_img, true)?; // raw negative thumb, display gamma
    let metadata = extract(p, full.width as u32, full.height as u32);
    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let cached = CachedImage { full_res: full, proxy: proxy_img, file_name, metadata, thumbnail };
    Ok(session.insert(cached))
}

#[tauri::command]
pub fn raw_preview(id: String, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    to_png_b64(&img.proxy, true) // un-inverted, display gamma
}

#[tauri::command]
pub fn inverted_preview(id: String, params: InvertParams, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let rect = params.base_rect.map(|r| Rect { x: r[0], y: r[1], w: r[2], h: r[3] });
    let base = sample_base(&img.proxy, rect);
    let inv = invert_image(&img.proxy, &build_params(&params, base), mode_from(&params.mode));
    to_png_b64(&inv, false) // engine already tone-mapped
}

#[tauri::command]
pub fn export_image(id: String, params: InvertParams, out_path: String, session: State<Session>) -> Result<(), String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    // sample base from the proxy (fast) but invert the full-res for export
    let rect = params.base_rect.map(|r| Rect { x: r[0], y: r[1], w: r[2], h: r[3] });
    let base = sample_base(&img.proxy, rect);
    let inv = invert_image(&img.full_res, &build_params(&params, base), mode_from(&params.mode));
    film_core::export::write_tiff16(&inv, Path::new(&out_path)).map_err(|e| format!("{e}"))
}
```

- [ ] **Step 2: Register commands + manage Session in main.rs**

In `app/src-tauri/src/main.rs`: ensure the module decls exist (`mod convert; mod encode; mod
metadata; mod session; mod commands;`), and in the `tauri::Builder` chain add `.manage(...)` and
the handler. The builder should look like:

```rust
fn main() {
    tauri::Builder::default()
        .manage(session::Session::default())
        .plugin(tauri_plugin_dialog::init()) // for open/save dialogs from the frontend
        .invoke_handler(tauri::generate_handler![
            commands::import_image,
            commands::raw_preview,
            commands::inverted_preview,
            commands::export_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RedRoom");
}
```

Add the dialog plugin dependency: in `app/src-tauri/Cargo.toml` add `tauri-plugin-dialog = "2"`,
and install the JS side: `cd app && npm install @tauri-apps/plugin-dialog @tauri-apps/api`.
(If the scaffold already added `@tauri-apps/api`, npm will no-op.)

- [ ] **Step 3: Build the backend**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo build) 2>&1 | tail -15`
Expected: compiles. Fix any rawler/image/tauri API drift per the notes in earlier tasks.

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri
git commit -m "feat(redroom): import/raw_preview/inverted_preview/export commands"
```

---

## Phase 2 — Frontend foundation

### Task 6: Theme, GlassPanel, stores, api, App layout + tabs

**Files:** Create `app/src/styles/theme.css`, `app/src/lib/glass/GlassPanel.svelte`,
`app/src/lib/store.ts`, `app/src/lib/api.ts`; replace `app/src/App.svelte`.

- [ ] **Step 1: Theme tokens**

Create `app/src/styles/theme.css`:

```css
:root {
  --bg-0: #0a0a0c;
  --bg-1: #141418;
  --glass-bg: rgba(28, 28, 34, 0.55);
  --glass-brd: rgba(255, 255, 255, 0.08);
  --glass-hi: rgba(255, 255, 255, 0.04);
  --text: #e8e8ea;
  --text-dim: #9a9aa2;
  --accent: #e03434;        /* darkroom red */
  --accent-dim: #7a1e1e;
  --radius: 14px;
}
* { box-sizing: border-box; }
html, body, #app { height: 100%; margin: 0; }
body {
  background:
    radial-gradient(1200px 800px at 70% -10%, #1a1320 0%, transparent 60%),
    radial-gradient(900px 600px at -10% 110%, #101820 0%, transparent 55%),
    var(--bg-0);
  color: var(--text);
  font: 13px/1.4 -apple-system, system-ui, sans-serif;
  overflow: hidden;
}
.accent { color: var(--accent); }
button { font: inherit; color: var(--text); cursor: pointer; }
```

- [ ] **Step 2: GlassPanel**

Create `app/src/lib/glass/GlassPanel.svelte`:

```svelte
<script lang="ts">
  export let pad = 12;
</script>

<div class="glass" style="padding:{pad}px">
  <slot />
</div>

<style>
  .glass {
    background: var(--glass-bg);
    border: 1px solid var(--glass-brd);
    border-radius: var(--radius);
    backdrop-filter: blur(22px) saturate(140%);
    -webkit-backdrop-filter: blur(22px) saturate(140%);
    box-shadow: inset 0 1px 0 var(--glass-hi), 0 8px 30px rgba(0,0,0,0.35);
    overflow: auto;
  }
</style>
```

- [ ] **Step 3: Types + API wrappers**

Create `app/src/lib/api.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export interface Metadata {
  camera?: string; lens?: string; iso?: string; shutter?: string;
  aperture?: string; width: number; height: number; file_size: number; date?: string;
}
export interface ImageEntry {
  id: string; file_name: string; thumbnail: string; metadata: Metadata;
}
export interface InvertParams {
  mode: "b" | "c";
  stock: "none" | "portra400" | "fujic200";
  base_rect: [number, number, number, number] | null;
  exposure: number; black: number; gamma: number;
}

export const api = {
  importImage: (path: string) => invoke<ImageEntry>("import_image", { path }),
  rawPreview: (id: string) => invoke<string>("raw_preview", { id }),
  invertedPreview: (id: string, params: InvertParams) =>
    invoke<string>("inverted_preview", { id, params }),
  exportImage: (id: string, params: InvertParams, outPath: string) =>
    invoke<void>("export_image", { id, params, outPath }),
};

export const defaultParams = (): InvertParams => ({
  mode: "b", stock: "none", base_rect: null, exposure: 1, black: 0, gamma: 0.4545,
});
```

- [ ] **Step 4: Stores**

Create `app/src/lib/store.ts`:

```ts
import { writable } from "svelte/store";
import type { ImageEntry, InvertParams } from "./api";
import { defaultParams } from "./api";

export const images = writable<ImageEntry[]>([]);
export const activeId = writable<string | null>(null);
export const module = writable<"library" | "develop">("library");
export const params = writable<InvertParams>(defaultParams());
```

- [ ] **Step 5: App layout + tabs**

Replace `app/src/App.svelte`:

```svelte
<script lang="ts">
  import "./styles/theme.css";
  import { module } from "./lib/store";
  import Library from "./lib/tabs/Library.svelte";
  import Develop from "./lib/tabs/Develop.svelte";
</script>

<div class="app">
  <header class="topbar">
    <div class="brand"><span class="dot"></span> RedRoom</div>
    <nav class="tabs">
      <button class:active={$module === "library"} on:click={() => module.set("library")}>Library</button>
      <button class:active={$module === "develop"} on:click={() => module.set("develop")}>Develop</button>
    </nav>
    <div class="spacer"></div>
  </header>
  <main>
    {#if $module === "library"}<Library />{:else}<Develop />{/if}
  </main>
</div>

<style>
  .app { display: flex; flex-direction: column; height: 100%; }
  .topbar {
    display: flex; align-items: center; gap: 18px;
    padding: 10px 16px; backdrop-filter: blur(20px);
    border-bottom: 1px solid var(--glass-brd);
  }
  .brand { font-weight: 600; letter-spacing: 0.3px; display: flex; align-items: center; gap: 8px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--accent);
         box-shadow: 0 0 12px var(--accent); }
  .tabs button {
    background: transparent; border: 0; padding: 6px 14px; border-radius: 8px;
    color: var(--text-dim);
  }
  .tabs button.active { color: var(--text); background: rgba(224,52,52,0.14);
    box-shadow: inset 0 0 0 1px rgba(224,52,52,0.4); }
  .spacer { flex: 1; }
  main { flex: 1; min-height: 0; padding: 12px; }
</style>
```

- [ ] **Step 6: Create placeholder tab files so it compiles**

Create `app/src/lib/tabs/Library.svelte` and `app/src/lib/tabs/Develop.svelte` each with:
```svelte
<div style="color:var(--text-dim)">coming soon</div>
```

- [ ] **Step 7: Verify the frontend builds**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -8`
Expected: Vite build succeeds.

- [ ] **Step 8: Commit**

```bash
git add app/src
git commit -m "feat(redroom): glass theme, GlassPanel, stores, api, app shell + tabs"
```

---

## Phase 3 — Library tab

### Task 7: Source (import), Filmstrip, Metadata panel, raw preview

**Files:** Create `app/src/lib/panels/{Source,Filmstrip,Metadata}.svelte`; rewrite
`app/src/lib/tabs/Library.svelte`.

- [ ] **Step 1: Source panel (import + list)**

Create `app/src/lib/panels/Source.svelte`:

```svelte
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { images, activeId } from "../store";
  import GlassPanel from "../glass/GlassPanel.svelte";

  let importing = false;
  let error = "";

  async function pickAndImport() {
    const sel = await open({ multiple: true, filters: [{ name: "Film scans", extensions: ["dng", "tif", "tiff", "raf"] }] });
    if (!sel) return;
    const paths = Array.isArray(sel) ? sel : [sel];
    importing = true; error = "";
    for (const path of paths) {
      try {
        const entry = await api.importImage(path as string);
        images.update((xs) => [...xs, entry]);
        activeId.update((id) => id ?? entry.id);
      } catch (e) { error = String(e); }
    }
    importing = false;
  }
</script>

<GlassPanel>
  <button class="import" on:click={pickAndImport} disabled={importing}>
    {importing ? "Importing…" : "Import"}
  </button>
  {#if error}<div class="err">{error}</div>{/if}
  <ul>
    {#each $images as img}
      <li class:active={$activeId === img.id} on:click={() => activeId.set(img.id)}>
        {img.file_name}
      </li>
    {/each}
  </ul>
</GlassPanel>

<style>
  .import { width: 100%; padding: 9px; border-radius: 10px; border: 0;
    background: var(--accent); color: white; font-weight: 600; }
  .import:disabled { opacity: 0.6; }
  .err { color: var(--accent); margin-top: 8px; font-size: 12px; }
  ul { list-style: none; padding: 0; margin: 12px 0 0; }
  li { padding: 7px 9px; border-radius: 8px; color: var(--text-dim); cursor: pointer;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  li.active { background: rgba(255,255,255,0.06); color: var(--text); }
</style>
```

- [ ] **Step 2: Filmstrip**

Create `app/src/lib/panels/Filmstrip.svelte`:

```svelte
<script lang="ts">
  import { images, activeId } from "../store";
</script>

<div class="strip">
  {#each $images as img}
    <button class:active={$activeId === img.id} on:click={() => activeId.set(img.id)}>
      <img src={img.thumbnail} alt={img.file_name} />
    </button>
  {/each}
</div>

<style>
  .strip { display: flex; gap: 8px; overflow-x: auto; padding: 6px; }
  button { padding: 0; border: 1px solid var(--glass-brd); border-radius: 8px; background: none;
    flex: 0 0 auto; }
  button.active { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  img { height: 64px; display: block; border-radius: 7px; }
</style>
```

- [ ] **Step 3: Metadata panel**

Create `app/src/lib/panels/Metadata.svelte`:

```svelte
<script lang="ts">
  import { images, activeId } from "../store";
  import GlassPanel from "../glass/GlassPanel.svelte";
  $: active = $images.find((i) => i.id === $activeId);
  const fmtSize = (b: number) => (b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : (b / 1e3).toFixed(0) + " KB");
</script>

<GlassPanel>
  {#if active}
    {@const m = active.metadata}
    <h3>{active.file_name}</h3>
    <dl>
      <dt>Camera</dt><dd>{m.camera ?? "—"}</dd>
      <dt>Lens</dt><dd>{m.lens ?? "—"}</dd>
      <dt>ISO</dt><dd>{m.iso ?? "—"}</dd>
      <dt>Shutter</dt><dd>{m.shutter ?? "—"}</dd>
      <dt>Aperture</dt><dd>{m.aperture ?? "—"}</dd>
      <dt>Dimensions</dt><dd>{m.width} × {m.height}</dd>
      <dt>Size</dt><dd>{fmtSize(m.file_size)}</dd>
      <dt>Date</dt><dd>{m.date ?? "—"}</dd>
    </dl>
  {:else}
    <div class="empty">No image selected</div>
  {/if}
</GlassPanel>

<style>
  h3 { margin: 0 0 12px; font-size: 13px; word-break: break-all; }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 6px 12px; margin: 0; }
  dt { color: var(--text-dim); } dd { margin: 0; text-align: right; }
  .empty { color: var(--text-dim); }
</style>
```

- [ ] **Step 4: Library tab layout (left / center raw preview / right metadata / filmstrip)**

Rewrite `app/src/lib/tabs/Library.svelte`:

```svelte
<script lang="ts">
  import { api } from "../api";
  import { activeId } from "../store";
  import Source from "../panels/Source.svelte";
  import Metadata from "../panels/Metadata.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";

  let preview = "";
  $: if ($activeId) { api.rawPreview($activeId).then((d) => (preview = d)).catch(() => (preview = "")); }
</script>

<div class="layout">
  <aside class="left"><Source /></aside>
  <section class="center">
    {#if preview}<img src={preview} alt="raw scan" />{:else}<div class="hint">Import a film scan to begin</div>{/if}
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
  .center img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 10px; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
```

- [ ] **Step 5: Verify build + manual smoke**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6`
Expected: builds. Then manual: `source "$HOME/.cargo/env" && npm run tauri dev` — import the V600
DNG, confirm it appears in Source + Filmstrip, the raw (orange negative) preview shows in center,
and Metadata populates (dims/size at minimum). Close dev when done.

- [ ] **Step 6: Commit**

```bash
git add app/src
git commit -m "feat(redroom): Library tab — import, filmstrip, raw preview, metadata"
```

---

## Phase 4 — Develop tab

### Task 8: Develop preview, Invert, Adjustments, Export

**Files:** Create `app/src/lib/panels/Adjustments.svelte`; rewrite `app/src/lib/tabs/Develop.svelte`.

- [ ] **Step 1: Adjustments panel**

Create `app/src/lib/panels/Adjustments.svelte`:

```svelte
<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { params, activeId } from "../store";
  import GlassPanel from "../glass/GlassPanel.svelte";

  let exporting = false;
  let msg = "";

  async function exportTiff() {
    if (!$activeId) return;
    const out = await save({ defaultPath: "redroom-export.tiff", filters: [{ name: "TIFF", extensions: ["tiff"] }] });
    if (!out) return;
    exporting = true; msg = "";
    try { await api.exportImage($activeId, $params, out); msg = "Exported ✓"; }
    catch (e) { msg = "Error: " + e; }
    exporting = false;
  }
</script>

<GlassPanel>
  <div class="grp">
    <label>Mode</label>
    <div class="seg">
      <button class:on={$params.mode === "b"} on:click={() => params.update((p) => ({ ...p, mode: "b" }))}>B · density</button>
      <button class:on={$params.mode === "c"} on:click={() => params.update((p) => ({ ...p, mode: "c" }))}>C · per-channel</button>
    </div>
  </div>

  <div class="grp">
    <label>Film stock</label>
    <select bind:value={$params.stock}>
      <option value="none">None (identity)</option>
      <option value="portra400">Kodak Portra 400</option>
      <option value="fujic200">Fuji C200</option>
    </select>
  </div>

  <div class="grp">
    <label>Exposure <span>{$params.exposure.toFixed(2)}</span></label>
    <input type="range" min="0.2" max="3" step="0.01" bind:value={$params.exposure} />
  </div>
  <div class="grp">
    <label>Black <span>{$params.black.toFixed(3)}</span></label>
    <input type="range" min="0" max="0.3" step="0.001" bind:value={$params.black} />
  </div>
  <div class="grp">
    <label>Gamma <span>{$params.gamma.toFixed(3)}</span></label>
    <input type="range" min="0.2" max="1" step="0.001" bind:value={$params.gamma} />
  </div>

  <button class="export" on:click={exportTiff} disabled={exporting || !$activeId}>
    {exporting ? "Exporting…" : "Export 16-bit TIFF"}
  </button>
  {#if msg}<div class="msg">{msg}</div>{/if}
</GlassPanel>

<style>
  .grp { margin-bottom: 14px; }
  label { display: flex; justify-content: space-between; color: var(--text-dim); margin-bottom: 6px; }
  label span { color: var(--text); }
  .seg { display: flex; gap: 6px; }
  .seg button { flex: 1; padding: 7px; border-radius: 8px; border: 1px solid var(--glass-brd);
    background: transparent; color: var(--text-dim); }
  .seg button.on { color: white; background: rgba(224,52,52,0.18);
    border-color: rgba(224,52,52,0.5); }
  select { width: 100%; padding: 7px; border-radius: 8px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); }
  input[type="range"] { width: 100%; accent-color: var(--accent); }
  .export { width: 100%; margin-top: 8px; padding: 10px; border: 0; border-radius: 10px;
    background: var(--accent); color: white; font-weight: 600; }
  .export:disabled { opacity: 0.5; }
  .msg { margin-top: 8px; color: var(--text-dim); }
</style>
```

- [ ] **Step 2: Develop tab (live preview + invert)**

Rewrite `app/src/lib/tabs/Develop.svelte`:

```svelte
<script lang="ts">
  import { api } from "../api";
  import { activeId, params } from "../store";
  import Adjustments from "../panels/Adjustments.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";

  let preview = "";
  let inverted = false;
  let busy = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function render() {
    if (!$activeId) return;
    busy = true;
    try { preview = await api.invertedPreview($activeId, $params); }
    catch (e) { preview = ""; }
    busy = false;
  }

  function scheduleRender() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(render, 120); // debounce slider drags
  }

  // Re-render live once inverted and whenever params/active change.
  $: if (inverted && ($params, $activeId)) scheduleRender();

  function invert() { inverted = true; render(); }
</script>

<div class="layout">
  <aside class="left">
    <button class="invert" class:done={inverted} on:click={invert} disabled={!$activeId}>
      {inverted ? "Re-invert" : "Invert"}
    </button>
  </aside>
  <section class="center">
    {#if preview}<img src={preview} alt="inverted" class:busy />
    {:else}<div class="hint">{$activeId ? "Press Invert" : "Select an image in Library"}</div>{/if}
  </section>
  <aside class="right"><Adjustments /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; display: grid; place-items: center; min-height: 0; }
  .center img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 10px;
    transition: opacity 0.1s; }
  .center img.busy { opacity: 0.75; }
  .invert { width: 100%; padding: 11px; border: 0; border-radius: 10px;
    background: var(--accent); color: white; font-weight: 700; letter-spacing: 0.3px; }
  .invert.done { background: rgba(224,52,52,0.18); box-shadow: inset 0 0 0 1px rgba(224,52,52,0.5); }
  .invert:disabled { opacity: 0.5; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
```

- [ ] **Step 3: Verify build + manual smoke**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6` (builds), then
`source "$HOME/.cargo/env" && npm run tauri dev`: select the V600 image, go to Develop, press
Invert → positive appears; change stock to Portra 400 and sliders → preview updates live; Export
→ save dialog → writes a TIFF. Close dev.

- [ ] **Step 4: Commit**

```bash
git add app/src
git commit -m "feat(redroom): Develop tab — invert, live adjustments, stock, export"
```

---

## Phase 5 — Polish

### Task 9: frontend-design polish pass

**Files:** `app/src/styles/theme.css` and component styles.

- [ ] **Step 1: Invoke the frontend-design skill** to elevate the glass/red aesthetic to
production grade — refined blur/lighting, micro-interactions, spacing rhythm, typography,
hover/active states, and a cohesive darkroom-red identity — WITHOUT changing the component
structure, store shape, or the `api.ts` contract. Keep all functionality from Tasks 6–8 working.

- [ ] **Step 2: Verify build still passes**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6`
Expected: builds; manual check that both tabs still function.

- [ ] **Step 3: Commit**

```bash
git add app/src
git commit -m "feat(redroom): frontend-design polish pass (glass + darkroom red)"
```

---

## Phase 6 — Validation

### Task 10: Manual E2E + findings

**Files:** `docs/superpowers/poc-findings.md`.

- [ ] **Step 1: Full manual run**

`source "$HOME/.cargo/env" && (cd app && npm run tauri dev)`. Verify, on the real V600 DNG and
GFX RAF: import (both formats), Library raw preview + metadata, Develop invert, Mode B/C, stock
Portra400, all three sliders live, and Export produces a valid 16-bit TIFF (open it / re-import).

- [ ] **Step 2: Record results**

Add a "RedRoom UI — v1 results" section to `docs/superpowers/poc-findings.md`: what works, any
rough edges, perf feel of the live proxy preview, and the next UI priorities.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/poc-findings.md
git commit -m "docs: RedRoom UI v1 manual validation results"
```

---

## Definition of Done

- [ ] `cd app/src-tauri && cargo test` green (convert, encode, metadata, session).
- [ ] `cd app && npm run build` succeeds; backend `cargo build` succeeds.
- [ ] `npm run tauri dev`: import DNG + RAF; Library shows raw preview + metadata; Develop
      inverts with live Mode/stock/sliders; Export writes a valid 16-bit TIFF.
- [ ] Dark liquid-glass theme with red darkroom accent; Lightroom-style two-module layout.
- [ ] Session is in-memory (resets on quit); naive mode not exposed in UI.
- [ ] Findings recorded.
```
