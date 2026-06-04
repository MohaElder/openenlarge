//! Tauri commands orchestrating film-core for the RedRoom UI.

use crate::convert::{crop, proxy, resize_to};
use crate::encode::{to_jpeg_b64, to_png_b64};
use crate::metadata::extract;
use crate::session::{CachedImage, Developed, ImageEntry, InvertParams, Quality, Session};
use film_core::calibrate::{auto_wb_gains, sample_base};
use film_core::decode::{decode_raw, decode_tiff};
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::finish::{finish_image, FinishParams};
use film_core::wb::{gains_to_cct, wb_from_kelvin};
use film_core::spectral::Stock;
use serde::Deserialize;
use std::path::Path;
use tauri::State;

const THUMB_EDGE: u32 = 320;
const AUTOWB_EDGE: u32 = 256;
const PREVIEW_JPEG_QUALITY: u8 = 88;

fn default_invert_params() -> InvertParams {
    InvertParams {
        mode: "b".into(), stock: "none".into(), base_rect: None,
        exposure: 0.0, black: 0.0, gamma: 0.4545, auto_wb: true,
        temp: 5500.0, tint: 0.0,
        contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
        texture: 0.0, vibrance: 0.0, saturation: 0.0,
    }
}

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

fn mode_from(s: &str) -> Mode {
    match s { "c" => Mode::C, _ => Mode::B }
}

fn build_params(p: &InvertParams, base: [f32; 3]) -> InversionParams {
    let exposure = 2f32.powf(p.exposure); // EV stops → linear multiplier
    match stock_from(&p.stock) {
        Some(s) if p.mode == "b" => params_for_stock(s, base, exposure, p.black, p.gamma),
        _ => InversionParams { base, exposure, black: p.black, gamma: p.gamma, ..Default::default() },
    }
}

fn wb_from_params(temp: f32, tint: f32) -> [f32; 3] {
    wb_from_kelvin(temp, tint / 150.0)
}

fn resolve_params(p: &InvertParams, _autowb_src: &film_core::Image, base: [f32; 3]) -> InversionParams {
    let mut ip = build_params(p, base);
    ip.wb = wb_from_params(p.temp, p.tint);
    ip
}

fn finish_default() -> bool { true }

fn finish_from(p: &InvertParams) -> FinishParams {
    FinishParams {
        contrast: p.contrast / 100.0,
        highlights: p.highlights / 100.0,
        shadows: p.shadows / 100.0,
        whites: p.whites / 100.0,
        blacks: p.blacks / 100.0,
        texture: p.texture / 100.0,
        vibrance: p.vibrance / 100.0,
        saturation: p.saturation / 100.0,
    }
}

/// LIGHT import: thumbnail (embedded preview if available) + metadata + stored
/// path. No full decode — the heavy work happens in `develop_image`.
#[tauri::command]
pub fn import_image(path: String, session: State<Session>) -> Result<ImageEntry, String> {
    let p = Path::new(&path);
    let thumbnail = match decode_tiff(p) {
        Ok(prev) => to_png_b64(&proxy(&prev, THUMB_EDGE), true)?,
        Err(_) => "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
    };
    let metadata = extract(p, 0, 0);
    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let cached = CachedImage { path, file_name, metadata, thumbnail, developed: None };
    Ok(session.insert(cached))
}

/// HEAVY step: decode the file, build the working image at the quality cap, a
/// small auto-WB thumb, and sample the base. Drops full_res. Returns the updated
/// entry (real dimensions + developed=true).
#[tauri::command]
pub fn develop_image(id: String, session: State<Session>) -> Result<ImageEntry, String> {
    let cap = session.quality.lock().unwrap().cap();
    let path = {
        let images = session.images.lock().unwrap();
        images.get(&id).ok_or("unknown image id")?.path.clone()
    };
    let full = decode_any(Path::new(&path))?;
    let working = proxy(&full, cap);
    let thumb = proxy(&full, AUTOWB_EDGE);
    let base = sample_base(&working, None);
    let (w, h) = (full.width as u32, full.height as u32);
    drop(full);

    let small = proxy(&working, THUMB_EDGE);
    let defaults = default_invert_params();
    let ip = resolve_params(&defaults, &thumb, base);
    let inv_thumb = invert_image(&small, &ip, Mode::B);
    let inv_thumb = finish_image(&inv_thumb, &finish_from(&defaults));
    let thumbnail = to_jpeg_b64(&inv_thumb, false, 82)?;

    let mut images = session.images.lock().unwrap();
    let img = images.get_mut(&id).ok_or("unknown image id")?;
    img.metadata.width = w;
    img.metadata.height = h;
    img.thumbnail = thumbnail.clone();
    img.developed = Some(Developed { working, thumb, base });
    Ok(ImageEntry {
        id: id.clone(),
        path: img.path.clone(),
        file_name: img.file_name.clone(),
        thumbnail,
        metadata: img.metadata.clone(),
        developed: true,
    })
}

#[tauri::command]
pub fn set_quality(quality: Quality, session: State<Session>) -> Result<(), String> {
    *session.quality.lock().unwrap() = quality;
    Ok(())
}

/// The visible region to render, in FULL-RES pixel coordinates, plus the output
/// (≈ viewport) pixel size. `raw` selects the un-inverted scan.
#[derive(Debug, Clone, Deserialize)]
pub struct ViewSpec {
    pub crop: [f64; 4],
    pub out_w: u32,
    pub out_h: u32,
    pub raw: bool,
    /// When false, return the inverted+graded preview BEFORE the finishing layer
    /// (the GPU applies finishing live). Defaults true for the legacy path/export.
    #[serde(default = "finish_default")]
    pub finish: bool,
}

#[tauri::command]
pub fn render_view(id: String, params: InvertParams, view: ViewSpec, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;

    let s_scale = dev.working.width as f64 / img.metadata.width.max(1) as f64;

    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;

    let cropped = crop(&dev.working, cx, cy, cw, ch);
    if cropped.pixels.is_empty() {
        return Err("empty crop".into());
    }
    let scaled = resize_to(&cropped, view.out_w.max(1), view.out_h.max(1));

    if view.raw {
        return to_jpeg_b64(&scaled, true, PREVIEW_JPEG_QUALITY);
    }
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    let out = if view.finish { finish_image(&inv, &finish_from(&params)) } else { inv };
    to_jpeg_b64(&out, false, PREVIEW_JPEG_QUALITY)
}

/// Render a small (~320px) inverted JPEG of the developed image at the given
/// params — used to live-refresh the Library grid cell while editing.
#[tauri::command]
pub fn thumbnail(id: String, params: InvertParams, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let small = proxy(&dev.working, THUMB_EDGE);
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&small, &ip, mode_from(&params.mode));
    let fin = finish_image(&inv, &finish_from(&params));
    to_jpeg_b64(&fin, false, 82)
}

/// Re-decode the file at full resolution and export a 16-bit TIFF.
#[tauri::command]
pub fn export_image(id: String, params: InvertParams, out_path: String, session: State<Session>) -> Result<(), String> {
    let (path, base, thumb) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (img.path.clone(), dev.base, dev.thumb.clone())
    };
    let full = decode_any(Path::new(&path))?;
    let ip = resolve_params(&params, &thumb, base);
    let inv = invert_image(&full, &ip, mode_from(&params.mode));
    let fin = finish_image(&inv, &finish_from(&params));
    film_core::export::write_tiff16(&fin, Path::new(&out_path)).map_err(|e| format!("{e}"))
}

/// Estimated as-shot white point for the developed image, as (Kelvin, tint).
/// The UI seeds the Temp/Tint sliders with this when an image becomes active.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AsShotWb { pub temp: f32, pub tint: f32 }

#[tauri::command]
pub fn as_shot_wb(id: String, session: State<Session>) -> Result<AsShotWb, String> {
    let (base, thumb) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (dev.base, dev.thumb.clone())
    };
    // Lock released — the inversion + gray-world estimate run unlocked.
    let neutral = default_invert_params();
    let ip = build_params(&neutral, base);
    let first = invert_image(&thumb, &ip, mode_from(&neutral.mode));
    let gains = auto_wb_gains(&first);
    let (temp, tint) = gains_to_cct(gains);
    Ok(AsShotWb { temp, tint: tint * 150.0 }) // back to UI −150..150
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewspec_finish_defaults_true_and_parses_false() {
        let d: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#).unwrap();
        assert!(d.finish, "finish should default to true when omitted");
        let f: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,"finish":false}"#).unwrap();
        assert!(!f.finish);
    }

    #[test]
    fn wb_from_params_directions() {
        let warm = wb_from_params(3000.0, 0.0);
        let cool = wb_from_params(9000.0, 0.0);
        assert!(warm[0] < cool[0], "warm should cut red vs cool");
        // wb_from_kelvin normalises green to 1.0; negative tint (green cast)
        // suppresses R and B relative to G, i.e. R < 1 at neutral temp.
        let green = wb_from_params(5500.0, -150.0);
        assert!(green[0] < 1.0, "negative tint suppresses red relative to green");
        assert!(green[2] < 1.0, "negative tint suppresses blue relative to green");
    }
}
