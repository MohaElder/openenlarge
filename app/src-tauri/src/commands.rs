//! Tauri commands orchestrating film-core for the RedRoom UI.

use crate::convert::{crop, proxy, resize_to};
use crate::encode::{to_jpeg_b64, to_png_b64};
use crate::metadata::extract;
use crate::session::{CachedImage, Developed, ImageEntry, InvertParams, Quality, Session};
use film_core::calibrate::{auto_wb_gains, sample_base};
use film_core::decode::{decode_raw, decode_tiff};
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::spectral::Stock;
use serde::Deserialize;
use std::path::Path;
use tauri::State;

const THUMB_EDGE: u32 = 320;
const AUTOWB_EDGE: u32 = 256;
const PREVIEW_JPEG_QUALITY: u8 = 88;

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
    match stock_from(&p.stock) {
        Some(s) if p.mode == "b" => params_for_stock(s, base, p.exposure, p.black, p.gamma),
        _ => InversionParams { base, exposure: p.exposure, black: p.black, gamma: p.gamma, ..Default::default() },
    }
}

fn wb_from_temp_tint(temp: f32, tint: f32) -> [f32; 3] {
    let r = (1.0 + 0.4 * temp + 0.2 * tint).max(0.1);
    let g = (1.0 - 0.4 * tint).max(0.1);
    let b = (1.0 - 0.4 * temp + 0.2 * tint).max(0.1);
    [r, g, b]
}

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

    let mut images = session.images.lock().unwrap();
    let img = images.get_mut(&id).ok_or("unknown image id")?;
    img.metadata.width = w;
    img.metadata.height = h;
    img.developed = Some(Developed { working, thumb, base });
    Ok(ImageEntry {
        id: id.clone(),
        file_name: img.file_name.clone(),
        thumbnail: img.thumbnail.clone(),
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
    to_jpeg_b64(&inv, false, PREVIEW_JPEG_QUALITY)
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
    film_core::export::write_tiff16(&inv, Path::new(&out_path)).map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wb_temp_tint_directions() {
        let warm = wb_from_temp_tint(0.5, 0.0);
        assert!(warm[0] > 1.0 && warm[2] < 1.0);
        let green = wb_from_temp_tint(0.0, -0.5);
        assert!(green[1] > 1.0);
    }
}
