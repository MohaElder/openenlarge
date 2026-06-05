//! Tauri commands orchestrating film-core for the RedRoom UI.

use crate::convert::{crop, orient, orient_dims, proxy, resize_to, rotate};
use crate::encode::{to_jpeg_b64, to_png_b64, write_jpeg, write_png, write_tiff8};
use crate::metadata::extract;
use crate::session::{CachedImage, Developed, ImageEntry, InvertParams, Quality, Session};
use film_core::calibrate::{auto_wb_gains, sample_base};
use film_core::decode::{decode_raw, decode_tiff};
use film_core::dust::{self, Stamp};
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::finish::{finish_image, FinishParams};
use film_core::wb::{gains_to_cct, wb_from_kelvin};
use film_core::spectral::Stock;
use serde::Deserialize;
use std::path::Path;
use tauri::State;

fn default_bits() -> u8 { 16 }

/// Output format chosen in the Export modal. Mirrors the JS `ExportFormat` object.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFormat {
    pub kind: String, // "jpeg" | "tiff" | "png"
    #[serde(default = "default_bits")]
    pub bit_depth: u8, // 8 | 16 (tiff/png)
    #[serde(default)]
    pub quality: u8, // jpeg, 1–100
    #[serde(default)]
    pub max_bytes: Option<u64>, // jpeg
}

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

/// Map a normalized crop rect [x,y,w,h] (0..1) to integer pixels on a w×h image,
/// clamped to bounds with a 1px minimum.
fn crop_px(norm: [f64; 4], w: usize, h: usize) -> (usize, usize, usize, usize) {
    let x = (norm[0] * w as f64).round().clamp(0.0, (w - 1) as f64) as usize;
    let y = (norm[1] * h as f64).round().clamp(0.0, (h - 1) as f64) as usize;
    let cw = (norm[2] * w as f64).round().clamp(1.0, (w - x) as f64) as usize;
    let ch = (norm[3] * h as f64).round().clamp(1.0, (h - y) as f64) as usize;
    (x, y, cw, ch)
}

/// Map normalized strokes → `Stamp`s in OUTPUT pixel space.
/// `base_w/base_h` are the WORKING image dims BEFORE the view crop is applied
/// (oriented + straightened + persistent image_crop, but NOT the per-render view
/// crop) — this is the image the UI normalizes stroke coords against. `(cx,cy,cw,ch)`
/// is the view-crop window within that base; `out_w/out_h` is the rendered output size.
#[allow(clippy::too_many_arguments)]
fn view_stamps(
    dust: &[DustStroke], base_w: usize, base_h: usize,
    cx: usize, cy: usize, cw: usize, ch: usize, out_w: u32, out_h: u32,
) -> Vec<Stamp> {
    if cw == 0 || ch == 0 {
        return Vec::new();
    }
    let sx = out_w as f64 / cw as f64;
    let sy = out_h as f64 / ch as f64;
    let mut out = Vec::new();
    for stroke in dust {
        // r is normalized to base WIDTH (matching the UI brush); scale by the x-axis factor.
        let r = (stroke.r * base_w as f64 * sx).max(0.5);
        for pt in &stroke.points {
            let bx = pt[0] * base_w as f64;
            let by = pt[1] * base_h as f64;
            out.push(Stamp {
                cx: ((bx - cx as f64) * sx) as f32,
                cy: ((by - cy as f64) * sy) as f32,
                r: r as f32,
            });
        }
    }
    out
}

/// Map normalized strokes → `Stamp`s on a full-res `w`×`h` image (no view crop).
/// Mirrors `view_stamps` but for export: points normalize to image dims, radius to width.
fn export_stamps(dust: &[DustStroke], w: usize, h: usize) -> Vec<Stamp> {
    let mut out = Vec::new();
    for stroke in dust {
        let r = (stroke.r * w as f64).max(0.5) as f32;
        for pt in &stroke.points {
            out.push(Stamp { cx: (pt[0] * w as f64) as f32, cy: (pt[1] * h as f64) as f32, r });
        }
    }
    out
}

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
    let has_ir = working.ir.is_some();
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
        has_ir,
    })
}

#[tauri::command]
pub fn set_quality(quality: Quality, session: State<Session>) -> Result<(), String> {
    *session.quality.lock().unwrap() = quality;
    Ok(())
}

/// A brush stroke from the UI: a polyline of points normalized to the DISPLAYED
/// image ([0,1] each), with radius `r` normalized to the displayed image WIDTH.
#[derive(Debug, Clone, Deserialize)]
pub struct DustStroke {
    pub points: Vec<[f64; 2]>,
    pub r: f64,
}

/// IR-driven auto dust removal settings from the UI.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IrRemoval {
    pub enabled: bool,
    pub sensitivity: f32,
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
    /// Normalized [x,y,w,h] persistent crop on the original image; applied before
    /// the zoom/view crop. None = whole image.
    #[serde(default)]
    pub image_crop: Option<[f64; 4]>,
    #[serde(default)] pub rot90: u8,
    #[serde(default)] pub flip_h: bool,
    #[serde(default)] pub flip_v: bool,
    #[serde(default)] pub angle: f32,
    #[serde(default)] pub dust: Vec<DustStroke>,
    #[serde(default)] pub ir_removal: IrRemoval,
}

#[tauri::command]
pub fn render_view(id: String, params: InvertParams, view: ViewSpec, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;

    // Geometry: orient (lossless) → straighten → persistent crop, then the view crop.
    let oriented = orient(&dev.working, view.rot90, view.flip_h, view.flip_v);
    let straightened = rotate(&oriented, view.angle);
    let base_img = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, ix, iy, iw, ih)
        }
        None => straightened,
    };
    // The view crop is in oriented full-res coords → map to working px via the
    // oriented metadata width (orientation is lossless, so the ratio is preserved).
    let (ometa_w, _) = orient_dims(img.metadata.width as usize, img.metadata.height as usize, view.rot90);
    let s_scale = oriented.width as f64 / ometa_w.max(1) as f64;
    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;
    let cropped = crop(&base_img, cx, cy, cw, ch);
    if cropped.pixels.is_empty() {
        return Err("empty crop".into());
    }
    let (cw_px, ch_px) = (cropped.width, cropped.height);
    let scaled = resize_to(&cropped, view.out_w.max(1), view.out_h.max(1));

    if view.raw {
        return to_jpeg_b64(&scaled, true, PREVIEW_JPEG_QUALITY);
    }
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let mut inv = invert_image(&scaled, &ip, mode_from(&params.mode));
    let stamps = view_stamps(
        &view.dust, base_img.width, base_img.height,
        cx, cy, cw_px, ch_px, view.out_w.max(1), view.out_h.max(1),
    );
    dust::apply(&mut inv, &stamps);
    if view.ir_removal.enabled {
        if let Some(ir) = scaled.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, view.ir_removal.sensitivity);
        }
    }
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

/// Re-decode the file at full resolution and export it in the chosen format.
#[allow(clippy::too_many_arguments)] // Tauri command: flat args mirror the JS invoke contract
#[tauri::command]
pub fn export_image(
    id: String, params: InvertParams, out_path: String,
    image_crop: Option<[f64; 4]>,
    rot90: u8, flip_h: bool, flip_v: bool, angle: f32,
    dust: Vec<DustStroke>,
    ir_removal: IrRemoval,
    format: ExportFormat,
    session: State<Session>,
) -> Result<(), String> {
    let (path, base, thumb) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (img.path.clone(), dev.base, dev.thumb.clone())
    };
    let full = decode_any(Path::new(&path))?;
    let full = orient(&full, rot90, flip_h, flip_v);
    let full = rotate(&full, angle);
    let full = match image_crop {
        Some(nc) => {
            let (x, y, w, h) = crop_px(nc, full.width, full.height);
            crop(&full, x, y, w, h)
        }
        None => full,
    };
    let ip = resolve_params(&params, &thumb, base);
    let mut inv = invert_image(&full, &ip, mode_from(&params.mode));
    let stamps = export_stamps(&dust, inv.width, inv.height);
    dust::apply(&mut inv, &stamps);
    if ir_removal.enabled {
        if let Some(ir) = full.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, ir_removal.sensitivity);
        }
    }
    let fin = finish_image(&inv, &finish_from(&params));
    let out = Path::new(&out_path);
    match format.kind.as_str() {
        "tiff" => {
            if format.bit_depth == 16 {
                film_core::export::write_tiff16(&fin, out).map_err(|e| format!("{e}"))
            } else {
                write_tiff8(&fin, out)
            }
        }
        "png" => write_png(&fin, out, format.bit_depth),
        "jpeg" => write_jpeg(&fin, out, format.quality, format.max_bytes),
        other => Err(format!("unknown export format: {other}")),
    }
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
    fn crop_px_maps_and_clamps_normalized_rect() {
        assert_eq!(crop_px([0.0, 0.0, 1.0, 1.0], 100, 80), (0, 0, 100, 80));
        assert_eq!(crop_px([0.25, 0.25, 0.5, 0.5], 100, 80), (25, 20, 50, 40));
        let (x, y, w, h) = crop_px([0.9, 0.9, 0.5, 0.5], 100, 80);
        assert!(x < 100 && y < 80 && w >= 1 && h >= 1 && x + w <= 100 && y + h <= 80);
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

    #[test]
    fn viewspec_dust_defaults_empty_and_parses_points() {
        let d: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#).unwrap();
        assert!(d.dust.is_empty(), "dust defaults to empty when omitted");
        let p: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,
                "dust":[{"points":[[0.5,0.5],[0.6,0.5]],"r":0.02}]}"#).unwrap();
        assert_eq!(p.dust.len(), 1);
        assert_eq!(p.dust[0].points.len(), 2);
    }

    #[test]
    fn view_stamps_maps_normalized_points_to_output_pixels() {
        // base image 200x100; view crop = whole base; output 400x200 (2x).
        let dust = vec![DustStroke { points: vec![[0.5, 0.5]], r: 0.01 }];
        let s = view_stamps(&dust, 200, 100, 0, 0, 200, 100, 400, 200);
        assert_eq!(s.len(), 1);
        assert!((s[0].cx - 200.0).abs() < 0.5, "x: 0.5*200*2 = 200");
        assert!((s[0].cy - 100.0).abs() < 0.5, "y: 0.5*100*2 = 100");
        // r normalized to base width: 0.01*200 = 2 base px → *2 scale = 4 out px.
        assert!((s[0].r - 4.0).abs() < 0.5, "r mapped to output px, got {}", s[0].r);
    }

    #[test]
    fn export_stamps_maps_normalized_points_to_full_res_pixels() {
        let dust = vec![DustStroke { points: vec![[0.25, 0.5]], r: 0.01 }];
        let s = export_stamps(&dust, 400, 200);
        assert_eq!(s.len(), 1);
        assert!((s[0].cx - 100.0).abs() < 0.5, "0.25*400");
        assert!((s[0].cy - 100.0).abs() < 0.5, "0.5*200");
        assert!((s[0].r - 4.0).abs() < 0.5, "0.01*400");
    }

    #[test]
    fn viewspec_ir_removal_defaults_off_and_parses() {
        let d: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#).unwrap();
        assert!(!d.ir_removal.enabled, "ir_removal defaults disabled");
        let p: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,
                "ir_removal":{"enabled":true,"sensitivity":60}}"#).unwrap();
        assert!(p.ir_removal.enabled);
        assert!((p.ir_removal.sensitivity - 60.0).abs() < 1e-6);
    }
}
