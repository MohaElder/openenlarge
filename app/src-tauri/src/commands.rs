//! Tauri commands orchestrating film-core for the RedRoom UI.

use crate::convert::{crop, proxy, resize_to};
use crate::encode::{to_jpeg_b64, to_png_b64};
use crate::metadata::extract;
use crate::session::{CachedImage, ImageEntry, InvertParams, Session};
use film_core::calibrate::{auto_wb_gains, sample_base, Rect};
use film_core::decode::{decode_raw, decode_tiff};
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::spectral::Stock;
use serde::Deserialize;
use std::path::Path;
use tauri::State;

const PROXY_EDGE: u32 = 2048;
const THUMB_EDGE: u32 = 256;
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

/// Manual white-balance gains from Temp/Tint controls (each ~[-1,1]).
/// temp>0 warms (more R, less B); tint>0 pushes magenta (less G).
fn wb_from_temp_tint(temp: f32, tint: f32) -> [f32; 3] {
    let r = (1.0 + 0.4 * temp + 0.2 * tint).max(0.1);
    let g = (1.0 - 0.4 * tint).max(0.1);
    let b = (1.0 - 0.4 * temp + 0.2 * tint).max(0.1);
    [r, g, b]
}

/// Build the final inversion params: matrices/exposure from `build_params`, plus
/// manual Temp/Tint WB and (if `auto_wb`) gray-world gains from a first pass over
/// `autowb_src`. Auto gains are computed on a small fixed image (the thumbnail)
/// for consistency between preview and export, independent of zoom/crop.
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

#[tauri::command]
pub fn import_image(path: String, session: State<Session>) -> Result<ImageEntry, String> {
    let p = Path::new(&path);
    let full = decode_any(p)?;
    let proxy_img = proxy(&full, PROXY_EDGE);
    let thumb_img = proxy(&full, THUMB_EDGE);
    let thumbnail = to_png_b64(&thumb_img, true)?;
    let metadata = extract(p, full.width as u32, full.height as u32);
    let base = sample_base(&proxy_img, None);
    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let cached = CachedImage { full_res: full, proxy: proxy_img, thumb_img, base, file_name, metadata, thumbnail };
    Ok(session.insert(cached))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Proxy,
    FullRes,
}

/// Cheapest source with enough detail: proxy when the crop sampled at proxy scale
/// already meets the output width; otherwise full-res.
fn choose_source(crop_w_full: f64, out_w: u32, proxy_scale: f64) -> Source {
    if crop_w_full * proxy_scale >= out_w as f64 {
        Source::Proxy
    } else {
        Source::FullRes
    }
}

#[tauri::command]
pub fn render_view(id: String, params: InvertParams, view: ViewSpec, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;

    let proxy_scale = img.proxy.width as f64 / img.full_res.width.max(1) as f64;
    let source = choose_source(view.crop[2], view.out_w, proxy_scale);
    let (src_img, s_scale) = match source {
        Source::Proxy => (&img.proxy, proxy_scale),
        Source::FullRes => (&img.full_res, 1.0),
    };

    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;

    let cropped = crop(src_img, cx, cy, cw, ch);
    if cropped.pixels.is_empty() {
        return Err("empty crop".into());
    }
    let scaled = resize_to(&cropped, view.out_w.max(1), view.out_h.max(1));

    if view.raw {
        return to_jpeg_b64(&scaled, true, PREVIEW_JPEG_QUALITY);
    }
    let ip = resolve_params(&params, &img.thumb_img, img.base);
    let inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    to_jpeg_b64(&inv, false, PREVIEW_JPEG_QUALITY)
}

#[tauri::command]
pub fn export_image(id: String, params: InvertParams, out_path: String, session: State<Session>) -> Result<(), String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let rect = params.base_rect.map(|r| Rect { x: r[0], y: r[1], w: r[2], h: r[3] });
    let base = sample_base(&img.proxy, rect);
    let ip = resolve_params(&params, &img.thumb_img, base);
    let inv = invert_image(&img.full_res, &ip, mode_from(&params.mode));
    film_core::export::write_tiff16(&inv, Path::new(&out_path)).map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_source_uses_proxy_at_fit_fullres_when_zoomed() {
        assert_eq!(choose_source(4000.0, 250, 0.5), Source::Proxy);
        assert_eq!(choose_source(250.0, 250, 0.5), Source::FullRes);
    }
}
