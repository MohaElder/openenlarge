//! Tauri commands orchestrating film-core for the OpenEnlarge UI.

use crate::convert::{crop, orient, orient_dims, proxy, resize_to, rotate};
use crate::encode::{to_jpeg_b64, to_png_b64, write_jpeg, write_png, write_tiff8};
use crate::gpu_upload::{bake_geometry, bake_working, capped_dims, image_from_rgba8, image_from_rgba_f32, pack_rgba16f, resolve_to_uniforms, BakeSpec, ResolvedInversion, MAX_GPU_EDGE};
use crate::metadata::extract;
use crate::session::{CachedImage, Developed, ImageEntry, InvertParams, PreparedExport, Quality, Session};
use film_core::calibrate::{auto_wb_gains, sample_base};
use film_core::decode::{decode_ldr, decode_raw, decode_tiff};
use film_core::dust::{self, Stamp};
use film_core::engine::{invert_image, params_for_stock, InversionParams, Mode};
use film_core::finish::{finish_image, tone_luts, ColorGrade, ColorMix, FinishParams, PcSample};
use film_core::wb::{gains_to_cct, wb_from_kelvin};
use film_core::spectral::Stock;
use serde::Deserialize;
use std::path::Path;
use tauri::State;

fn default_bits() -> u8 { 16 }
fn default_quality() -> u8 { 85 }

/// Output format chosen in the Export modal. Mirrors the JS `ExportFormat` object.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFormat {
    pub kind: String, // "jpeg" | "tiff" | "png"
    #[serde(default = "default_bits")]
    pub bit_depth: u8, // 8 | 16 (tiff/png)
    #[serde(default = "default_quality")]
    pub quality: u8, // jpeg, 1–100
    #[serde(default)]
    pub max_bytes: Option<u64>, // jpeg
}

/// User-edited metadata overrides sent from the panel. Each field, when present
/// and non-blank, replaces the source EXIF value on export. Mirrors the JS
/// `MetaOverride`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetaOverride {
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<String>,
    pub shutter: Option<String>,
    pub aperture: Option<String>,
    pub date: Option<String>,
    pub note: Option<String>,
}

/// Overlay the override onto the source metadata: a non-blank override field wins,
/// otherwise the original is kept. (Note has no source value; it comes only from
/// the override.)
fn effective_metadata(
    orig: &crate::metadata::Metadata,
    ov: Option<&MetaOverride>,
) -> crate::metadata::Metadata {
    let mut m = orig.clone();
    let Some(o) = ov else { return m };
    let pick = |cur: &Option<String>, new: &Option<String>| -> Option<String> {
        new.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| cur.clone())
    };
    m.camera = pick(&m.camera, &o.camera);
    m.lens = pick(&m.lens, &o.lens);
    m.iso = pick(&m.iso, &o.iso);
    m.shutter = pick(&m.shutter, &o.shutter);
    m.aperture = pick(&m.aperture, &o.aperture);
    m.date = pick(&m.date, &o.date);
    m.note = pick(&m.note, &o.note);
    m
}

const THUMB_EDGE: u32 = 320;
const AUTOWB_EDGE: u32 = 256;
const PREVIEW_JPEG_QUALITY: u8 = 88;
const CACHE_WORKING_CAP: u32 = 4096;

pub(crate) fn default_invert_params() -> InvertParams {
    InvertParams {
        mode: "b".into(), stock: "none".into(), base_override: None,
        exposure: 0.0, black: 0.0, gamma: 0.4545, auto_wb: true,
        temp: 5500.0, tint: 0.0,
        contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
        texture: 0.0, vibrance: 0.0, saturation: 0.0,
        tc_highlights: 0.0, tc_lights: 0.0, tc_darks: 0.0, tc_shadows: 0.0,
        tc_curve: crate::session::identity_curve(),
        tc_red: crate::session::identity_curve(),
        tc_green: crate::session::identity_curve(),
        tc_blue: crate::session::identity_curve(),
        cg_sh_hue: 0.0, cg_sh_sat: 0.0, cg_sh_lum: 0.0,
        cg_mid_hue: 0.0, cg_mid_sat: 0.0, cg_mid_lum: 0.0,
        cg_hi_hue: 0.0, cg_hi_sat: 0.0, cg_hi_lum: 0.0,
        cg_glob_hue: 0.0, cg_glob_sat: 0.0, cg_glob_lum: 0.0,
        cg_blending: 50.0, cg_balance: 0.0,
        cm_red_hue: 0.0, cm_red_sat: 0.0, cm_red_lum: 0.0,
        cm_orange_hue: 0.0, cm_orange_sat: 0.0, cm_orange_lum: 0.0,
        cm_yellow_hue: 0.0, cm_yellow_sat: 0.0, cm_yellow_lum: 0.0,
        cm_green_hue: 0.0, cm_green_sat: 0.0, cm_green_lum: 0.0,
        cm_aqua_hue: 0.0, cm_aqua_sat: 0.0, cm_aqua_lum: 0.0,
        cm_blue_hue: 0.0, cm_blue_sat: 0.0, cm_blue_lum: 0.0,
        cm_purple_hue: 0.0, cm_purple_sat: 0.0, cm_purple_lum: 0.0,
        cm_magenta_hue: 0.0, cm_magenta_sat: 0.0, cm_magenta_lum: 0.0,
        pc_samples: Vec::new(),
    }
}

fn metadata_to_json(m: &crate::metadata::Metadata) -> Result<String, String> {
    serde_json::to_string(m).map_err(|e| e.to_string())
}

fn decode_any(path: &Path) -> Result<film_core::Image, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "tif" | "tiff" => decode_tiff(path).map_err(|e| format!("{e}")),
        "jpg" | "jpeg" | "png" => decode_ldr(path).map_err(|e| format!("{e}")),
        _ => decode_raw(path).map_err(|e| format!("{e}")),
    }
}

fn stock_from(s: &str) -> Option<Stock> {
    match s {
        "portra400" => Some(Stock::Portra400),
        "fujic200" => Some(Stock::FujiC200),
        "portra160" => Some(Stock::Portra160),
        "portra800" => Some(Stock::Portra800),
        "ektar100" => Some(Stock::Ektar100),
        "gold200" => Some(Stock::Gold200),
        "ultramax400" => Some(Stock::Ultramax400),
        "fujipro400h" => Some(Stock::FujiPro400H),
        "fujixtra400" => Some(Stock::FujiXtra400),
        "vision350d" => Some(Stock::Vision350D),
        "vision3200t" => Some(Stock::Vision3200T),
        "vision3250d" => Some(Stock::Vision3250D),
        "vision3500t" => Some(Stock::Vision3500T),
        _ => None,
    }
}

pub(crate) fn mode_from(s: &str) -> Mode {
    match s { "c" => Mode::C, _ => Mode::B }
}

pub(crate) fn build_params(p: &InvertParams, base: [f32; 3]) -> InversionParams {
    let exposure = 2f32.powf(p.exposure); // EV stops → linear multiplier
    match stock_from(&p.stock) {
        Some(s) if p.mode == "b" => params_for_stock(s, base, exposure, p.black, p.gamma),
        _ => InversionParams { base, exposure, black: p.black, gamma: p.gamma, ..Default::default() },
    }
}

/// The base to invert with: the per-image override if set, else the develop-time
/// auto base sampled at `develop_image` time.
pub(crate) fn effective_base(p: &InvertParams, dev_base: [f32; 3]) -> [f32; 3] {
    p.base_override.unwrap_or(dev_base)
}

pub(crate) fn wb_from_params(temp: f32, tint: f32) -> [f32; 3] {
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
pub(crate) fn crop_px(norm: [f64; 4], w: usize, h: usize) -> (usize, usize, usize, usize) {
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
pub(crate) fn export_stamps(dust: &[DustStroke], w: usize, h: usize) -> Vec<Stamp> {
    let mut out = Vec::new();
    for stroke in dust {
        let r = (stroke.r * w as f64).max(0.5) as f32;
        for pt in &stroke.points {
            out.push(Stamp { cx: (pt[0] * w as f64) as f32, cy: (pt[1] * h as f64) as f32, r });
        }
    }
    out
}

fn color_mix_from(p: &crate::session::InvertParams) -> ColorMix {
    let cm_hue = [
        p.cm_red_hue, p.cm_orange_hue, p.cm_yellow_hue, p.cm_green_hue,
        p.cm_aqua_hue, p.cm_blue_hue, p.cm_purple_hue, p.cm_magenta_hue,
    ];
    let cm_sat = [
        p.cm_red_sat, p.cm_orange_sat, p.cm_yellow_sat, p.cm_green_sat,
        p.cm_aqua_sat, p.cm_blue_sat, p.cm_purple_sat, p.cm_magenta_sat,
    ];
    let cm_lum = [
        p.cm_red_lum, p.cm_orange_lum, p.cm_yellow_lum, p.cm_green_lum,
        p.cm_aqua_lum, p.cm_blue_lum, p.cm_purple_lum, p.cm_magenta_lum,
    ];
    let samples = p.pc_samples.iter().map(|s| PcSample {
        hue: s.hue, sat: s.sat, lum: s.lum,
        hue_shift: s.hue_shift / 100.0, sat_shift: s.sat_shift / 100.0, lum_shift: s.lum_shift / 100.0,
        variance: s.variance, range: s.range,
    }).collect();
    ColorMix {
        cm_hue: cm_hue.map(|v| v / 100.0),
        cm_sat: cm_sat.map(|v| v / 100.0),
        cm_lum: cm_lum.map(|v| v / 100.0),
        samples,
    }
}

fn finish_from(p: &InvertParams) -> FinishParams {
    // Region sliders ordered [shadows, darks, lights, highlights] to match the engine.
    let regions = [
        p.tc_shadows / 100.0, p.tc_darks / 100.0, p.tc_lights / 100.0, p.tc_highlights / 100.0,
    ];
    let (lut_r, lut_g, lut_b) =
        tone_luts(regions, &p.tc_curve, &p.tc_red, &p.tc_green, &p.tc_blue);
    let cg = ColorGrade::new(
        ([p.cg_sh_hue, p.cg_sh_sat / 100.0], p.cg_sh_lum / 100.0),
        ([p.cg_mid_hue, p.cg_mid_sat / 100.0], p.cg_mid_lum / 100.0),
        ([p.cg_hi_hue, p.cg_hi_sat / 100.0], p.cg_hi_lum / 100.0),
        ([p.cg_glob_hue, p.cg_glob_sat / 100.0], p.cg_glob_lum / 100.0),
        p.cg_blending / 100.0,
        p.cg_balance / 100.0,
    );
    FinishParams {
        contrast: p.contrast / 100.0,
        highlights: p.highlights / 100.0,
        shadows: p.shadows / 100.0,
        whites: p.whites / 100.0,
        blacks: p.blacks / 100.0,
        texture: p.texture / 100.0,
        vibrance: p.vibrance / 100.0,
        saturation: p.saturation / 100.0,
        lut_r, lut_g, lut_b, cg,
        cm: color_mix_from(p),
    }
}

/// LIGHT import: thumbnail (embedded preview if available) + metadata + stored
/// path. No full decode — the heavy work happens in `develop_image`.
#[tauri::command]
pub fn import_image(
    path: String,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<ImageEntry, String> {
    let p = Path::new(&path);
    // Light thumbnail: TIFF and JPEG/PNG decode cheaply (no demosaic), so render a
    // real preview. RAW files fall back to the 1x1 placeholder until develop.
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let preview = match ext.as_str() {
        "jpg" | "jpeg" | "png" => decode_ldr(p).map_err(|e| format!("{e}")),
        _ => decode_tiff(p).map_err(|e| format!("{e}")),
    };
    let thumbnail = match preview {
        Ok(prev) => to_png_b64(&proxy(&prev, THUMB_EDGE), true)?,
        Err(_) => "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
    };
    let metadata = extract(p, 0, 0);
    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let metadata_json = metadata_to_json(&metadata)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let id = catalog
        .upsert_image(&path, &file_name, &metadata_json, &thumbnail, now)
        .map_err(|e| e.to_string())?;
    let cached = CachedImage { path, file_name, metadata, thumbnail, developed: None };
    Ok(session.insert_with_id(id, cached))
}

/// HEAVY step: decode the file, build the working image at the quality cap, a
/// small auto-WB thumb, and sample the base. Drops full_res. Returns the updated
/// entry (real dimensions + developed=true).
#[tauri::command]
pub fn develop_image(
    id: String,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<ImageEntry, String> {
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

    // Build a cache-bounded copy of working (≤CACHE_WORKING_CAP long edge) for the sidecar.
    // Clone thumb too before the move into Developed.
    let cache_working = if working.width.max(working.height) > CACHE_WORKING_CAP as usize {
        crate::convert::proxy(&working, CACHE_WORKING_CAP)
    } else {
        working.clone()
    };
    let cache_thumb = thumb.clone();

    // Mutate session state and build the entry inside the lock, then release
    // the guard before the expensive cache write (tens of MB, zstd + file IO).
    let (entry, metadata_json) = {
        let mut images = session.images.lock().unwrap();
        let img = images.get_mut(&id).ok_or("unknown image id")?;
        img.metadata.width = w;
        img.metadata.height = h;
        img.thumbnail = thumbnail.clone();
        img.developed = Some(Developed { working, thumb, base });
        let metadata_json = metadata_to_json(&img.metadata)?;
        let entry = ImageEntry {
            id: id.clone(),
            path: img.path.clone(),
            file_name: img.file_name.clone(),
            thumbnail,
            metadata: img.metadata.clone(),
            developed: true,
            has_ir,
            offline: false,
        };
        (entry, metadata_json)
    }; // lock released here

    if let Err(e) = catalog.update_image_render(&id, &entry.thumbnail, &metadata_json) {
        eprintln!("[catalog] update_image_render failed for {id}: {e}");
    }

    // Write cache sidecar (best-effort; never fails the develop command).
    if let Err(e) = crate::cache::write(&session.cache_path(&id), base, &cache_working, &cache_thumb) {
        eprintln!("[cache] write failed for {id}: {e}");
    }

    Ok(entry)
}

#[tauri::command]
pub fn set_quality(quality: Quality, session: State<Session>) -> Result<(), String> {
    *session.quality.lock().unwrap() = quality;
    Ok(())
}

/// Drop an image from the session. With `delete_file`, also move the original
/// file to the OS trash (recoverable). Removing from the session always happens
/// first so the app forgets the image even if the trash step fails.
#[tauri::command]
pub fn delete_image(
    id: String,
    delete_file: bool,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    let removed = session.images.lock().unwrap().remove(&id);
    if let Err(e) = catalog.delete_image(&id) {
        eprintln!("[catalog] delete_image failed for {id}: {e}");
    }
    let _ = std::fs::remove_file(session.cache_path(&id));
    if delete_file {
        let img = removed.ok_or_else(|| "unknown image".to_string())?;
        trash::delete(&img.path).map_err(|e| format!("{e}"))?;
    }
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

/// Ensure the image's decoded `Developed` is resident in the Session: if absent
/// but a cache file exists, load it. Drops the lock during file IO.
fn ensure_resident(session: &Session, id: &str) -> Result<(), String> {
    {
        let images = session.images.lock().unwrap();
        match images.get(id) {
            Some(c) if c.developed.is_some() => return Ok(()),
            Some(_) => {}
            None => return Err("unknown image id".into()),
        }
    }
    let path = session.cache_path(id);
    if !path.exists() {
        return Err("not developed".into());
    }
    let (base, working, thumb) =
        crate::cache::read(&path).map_err(|e| format!("cache read: {e}"))?;
    let mut images = session.images.lock().unwrap();
    if let Some(c) = images.get_mut(id) {
        if c.developed.is_none() {
            c.developed = Some(Developed { working, thumb, base });
        }
    }
    Ok(())
}

#[tauri::command]
pub fn render_view(id: String, params: InvertParams, view: ViewSpec, session: State<Session>) -> Result<String, String> {
    ensure_resident(&session, &id)?;
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
    let ip = resolve_params(&params, &dev.thumb, effective_base(&params, dev.base));
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

/// The persistent per-image edits that shape a thumbnail's geometry and retouching.
/// Mirrors the relevant `ViewSpec` fields but without the zoom/view crop — a
/// thumbnail always shows the whole (cropped) frame. All fields default so the
/// grid can request a plain develop-only thumbnail with `{}`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThumbView {
    #[serde(default)] pub image_crop: Option<[f64; 4]>,
    #[serde(default)] pub rot90: u8,
    #[serde(default)] pub flip_h: bool,
    #[serde(default)] pub flip_v: bool,
    #[serde(default)] pub angle: f32,
    #[serde(default)] pub dust: Vec<DustStroke>,
    #[serde(default)] pub ir_removal: IrRemoval,
}

/// Render a small (~320px) inverted JPEG of the developed image at the given
/// params and persistent edits — used to live-refresh the Library grid cell and
/// filmstrip while editing. Applies orientation/straighten/crop, develop params,
/// dust strokes, and IR removal so the thumbnail matches the viewport.
#[tauri::command]
pub fn thumbnail(id: String, params: InvertParams, view: ThumbView, session: State<Session>) -> Result<String, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;

    // Geometry: orient (lossless) → straighten → persistent crop. No view crop —
    // the whole frame is shown, scaled to fit THUMB_EDGE.
    let oriented = orient(&dev.working, view.rot90, view.flip_h, view.flip_v);
    let straightened = rotate(&oriented, view.angle);
    let base_img = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, ix, iy, iw, ih)
        }
        None => straightened,
    };
    let small = proxy(&base_img, THUMB_EDGE);
    let (ow, oh) = (small.width as u32, small.height as u32);
    let ip = resolve_params(&params, &dev.thumb, effective_base(&params, dev.base));
    let mut inv = invert_image(&small, &ip, mode_from(&params.mode));
    let stamps = view_stamps(
        &view.dust, base_img.width, base_img.height,
        0, 0, base_img.width, base_img.height, ow, oh,
    );
    dust::apply(&mut inv, &stamps);
    if view.ir_removal.enabled {
        if let Some(ir) = small.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, view.ir_removal.sensitivity);
        }
    }
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
    meta_override: Option<MetaOverride>,
    session: State<Session>,
) -> Result<(), String> {
    ensure_resident(&session, &id)?;
    let (path, base, thumb, metadata) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (img.path.clone(), dev.base, dev.thumb.clone(), img.metadata.clone())
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
    let ip = resolve_params(&params, &thumb, effective_base(&params, base));
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
    }?;

    // Best-effort EXIF embed. The pixel file is already written and valid; a
    // metadata failure is logged but never fails the export.
    let eff = effective_metadata(&metadata, meta_override.as_ref());
    if let Err(e) = crate::exif_write::write_exif(out, &eff) {
        eprintln!("[exif] embed failed for {out_path}: {e}");
    }
    Ok(())
}

/// Dims + resolved inversion uniforms handed back to the frontend so it can
/// upload the baked source and render invert+finish offscreen.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportPrep {
    pub w: u32,
    pub h: u32,
    pub uniforms: ResolvedInversion,
}

/// Decode full-res, bake geometry + pre-invert heal, stash the half-float bytes,
/// and return the dims + resolved inversion uniforms. The frontend then renders
/// the GPU invert+finish offscreen and calls export_finish with the readback.
#[tauri::command]
pub fn export_begin(id: String, params: InvertParams, spec: BakeSpec, session: State<Session>) -> Result<ExportPrep, String> {
    ensure_resident(&session, &id)?;
    let (path, base) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (img.path.clone(), dev.base)
    };
    let full = decode_any(Path::new(&path))?;
    let baked = bake_working(&full, &spec);           // geometry + pre-invert heal, full-res
    let (w, h, bytes) = pack_rgba16f(&baked, u32::MAX); // no cap for export
    let uniforms = resolve_to_uniforms(&params, effective_base(&params, base));
    *session.pending_export.lock().unwrap() = Some(PreparedExport { w, h, bytes });
    Ok(ExportPrep { w, h, uniforms })
}

/// Return the stashed half-float bytes for upload (consumes nothing; kept until export_finish).
#[tauri::command]
pub fn export_pixels(session: State<Session>) -> Result<tauri::ipc::Response, String> {
    let guard = session.pending_export.lock().unwrap();
    let prep = guard.as_ref().ok_or("no prepared export")?;
    Ok(tauri::ipc::Response::new(prep.bytes.clone()))
}

/// `bit16 = true` → `data` is f32 RGBA (4 floats/px); else RGBA8 (4 bytes/px).
#[derive(Debug, Clone, Deserialize)]
pub struct ExportReadback {
    pub w: u32,
    pub h: u32,
    pub bit16: bool,
}

/// Build an Image from the GPU readback and encode it with the chosen format.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn export_finish(
    id: String,
    out_path: String,
    readback: ExportReadback,
    data: Vec<u8>,
    format: ExportFormat,
    meta_override: Option<MetaOverride>,
    session: State<Session>,
) -> Result<(), String> {
    let img = if readback.bit16 {
        // data is little-endian f32 RGBA
        let floats: Vec<f32> = data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        image_from_rgba_f32(readback.w, readback.h, &floats)
    } else {
        image_from_rgba8(readback.w, readback.h, &data)
    };
    let out = Path::new(&out_path);
    match format.kind.as_str() {
        "tiff" => {
            if format.bit_depth == 16 {
                film_core::export::write_tiff16(&img, out).map_err(|e| format!("{e}"))
            } else {
                write_tiff8(&img, out)
            }
        }
        "png" => write_png(&img, out, format.bit_depth),
        "jpeg" => write_jpeg(&img, out, format.quality, format.max_bytes),
        other => Err(format!("unknown export format: {other}")),
    }?;

    // Best-effort EXIF embed, mirroring export_image. The pixel file is already
    // written and valid; a metadata failure is logged but never fails the export.
    let metadata = {
        let images = session.images.lock().unwrap();
        images.get(&id).map(|i| i.metadata.clone())
    };
    if let Some(md) = metadata {
        let eff = effective_metadata(&md, meta_override.as_ref());
        if let Err(e) = crate::exif_write::write_exif(out, &eff) {
            eprintln!("[exif] embed failed for {out_path}: {e}");
        }
    }
    *session.pending_export.lock().unwrap() = None; // release the buffer
    Ok(())
}

/// Estimated as-shot white point for the developed image, as (Kelvin, tint).
/// The UI seeds the Temp/Tint sliders with this when an image becomes active.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AsShotWb { pub temp: f32, pub tint: f32 }

#[tauri::command]
pub fn as_shot_wb(id: String, params: InvertParams, session: State<Session>) -> Result<AsShotWb, String> {
    ensure_resident(&session, &id)?;
    let (base, thumb) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (dev.base, dev.thumb.clone())
    };
    // Lock released — the inversion + gray-world estimate run unlocked.
    // Estimate WB against the user's ACTUAL stock/mode so the gains neutralise the
    // colour space the image is actually rendered in. `build_params` leaves `wb` at
    // [1,1,1], so the estimate is independent of any temp/tint already on the sliders.
    let ip = build_params(&params, effective_base(&params, base));
    let first = invert_image(&thumb, &ip, mode_from(&params.mode));
    let gains = auto_wb_gains(&first);
    let (temp, tint) = gains_to_cct(gains);
    Ok(AsShotWb { temp, tint: tint * 150.0 }) // back to UI −150..150
}

/// Load the whole catalog at launch: return the snapshot to the frontend AND
/// repopulate the in-memory Session with lightweight (undeveloped) records so
/// `develop_image`/`render_view` can find each image by id.
#[tauri::command]
pub fn load_catalog(
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<crate::catalog::CatalogSnapshot, String> {
    let mut snap = catalog
        .snapshot(&|p| Path::new(p).exists())
        .map_err(|e| e.to_string())?;

    // Annotate each image with cache presence (cheap stat) before sending to frontend.
    for ci in &mut snap.images {
        let cache_path = session.cache_path(&ci.id);
        ci.developed = cache_path.exists();
        ci.has_ir = if ci.developed {
            crate::cache::read_has_ir(&cache_path).unwrap_or(false)
        } else {
            false
        };
    }

    let mut imgs = session.images.lock().unwrap();
    imgs.clear();
    for ci in &snap.images {
        let metadata = serde_json::from_value(ci.metadata.clone()).unwrap_or_default();
        imgs.insert(
            ci.id.clone(),
            CachedImage {
                path: ci.path.clone(),
                file_name: ci.file_name.clone(),
                metadata,
                thumbnail: ci.thumbnail.clone(),
                developed: None, // lazy: ensure_resident loads on first view
            },
        );
    }
    Ok(snap)
}

#[tauri::command]
pub fn save_edits(
    id: String,
    params_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_params(&id, &params_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_crop(
    id: String,
    crop_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_crop(&id, &crop_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_dust(
    id: String,
    dust_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_dust(&id, &dust_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_meta(
    id: String,
    meta_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_meta(&id, &meta_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_pref(
    key: String,
    value: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_pref(&key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_app_state(
    key: String,
    value: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_app_state(&key, &value).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkingInfo {
    /// Capped dimensions of the float texture working_pixels will return.
    pub w: u32,
    pub h: u32,
}

/// Dimensions of the GPU float texture for this image (after the MAX_GPU_EDGE cap).
#[tauri::command]
pub fn working_info(id: String, session: State<Session>) -> Result<WorkingInfo, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (w, h) = capped_dims(&dev.working, MAX_GPU_EDGE);
    Ok(WorkingInfo { w, h })
}

/// Raw half-float RGBA bytes of the linear working image (pre-inversion), for a
/// one-shot WebGL2 RGBA16F upload. Returned as raw IPC bytes (no base64/JPEG).
#[tauri::command]
pub fn working_pixels(id: String, session: State<Session>) -> Result<tauri::ipc::Response, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (_, _, bytes) = pack_rgba16f(&dev.working, MAX_GPU_EDGE);
    Ok(tauri::ipc::Response::new(bytes))
}

/// Capped dims of the BAKED (geometry + heal) working texture.
#[tauri::command]
pub fn working_baked_info(id: String, spec: BakeSpec, session: State<Session>) -> Result<WorkingInfo, String> {
    ensure_resident(&session, &id)?;
    let working = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        img.developed.as_ref().ok_or("not developed")?.working.clone()
    };
    let geom = bake_geometry(&working, &spec); // geometry only — exact dims, no Telea heal
    let (w, h) = capped_dims(&geom, MAX_GPU_EDGE);
    Ok(WorkingInfo { w, h })
}

/// Half-float RGBA bytes of the BAKED working buffer (geometry applied, dust/IR
/// healed pre-invert), for a one-shot RGBA16F upload. GPU then inverts with
/// IDENTITY geometry.
#[tauri::command]
pub fn working_baked_pixels(id: String, spec: BakeSpec, session: State<Session>) -> Result<tauri::ipc::Response, String> {
    ensure_resident(&session, &id)?;
    let working = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        img.developed.as_ref().ok_or("not developed")?.working.clone()
    };
    let baked = bake_working(&working, &spec);
    let (_, _, bytes) = pack_rgba16f(&baked, MAX_GPU_EDGE);
    Ok(tauri::ipc::Response::new(bytes))
}

/// Resolve inversion params (+ this image's sampled base) into GPU uniforms.
#[tauri::command]
pub fn resolved_inversion(
    id: String, params: InvertParams, session: State<Session>,
) -> Result<ResolvedInversion, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    Ok(resolve_to_uniforms(&params, effective_base(&params, dev.base)))
}

/// Sample the orange-mask base from a normalized rect [x,y,w,h] (0..1) over the
/// resident working image. Used by the base-picker tool; cheap, no re-decode.
#[tauri::command]
pub fn sample_base_at(
    id: String, rect: [f64; 4], session: State<Session>,
) -> Result<[f32; 3], String> {
    use film_core::calibrate::Rect;
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (x, y, w, h) = crop_px(rect, dev.working.width, dev.working.height);
    Ok(sample_base(&dev.working, Some(Rect { x, y, w, h })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_base_prefers_override_then_dev_base() {
        let mut p = crate::commands_test_support::sample_invert_params();
        p.base_override = None;
        assert_eq!(effective_base(&p, [0.8, 0.6, 0.4]), [0.8, 0.6, 0.4], "None -> dev base");
        p.base_override = Some([0.1, 0.2, 0.3]);
        assert_eq!(effective_base(&p, [0.8, 0.6, 0.4]), [0.1, 0.2, 0.3], "Some -> override");
    }

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
    fn sample_base_at_maps_normalized_rect_to_region() {
        use film_core::calibrate::{sample_base, Rect};
        // 4x4 image: left half bright [0.9,...], right half dark [0.1,...].
        let mut pixels = vec![[0.1f32; 3]; 16];
        for y in 0..4 { for x in 0..2 { pixels[y * 4 + x] = [0.9, 0.9, 0.9]; } }
        let img = film_core::Image { width: 4, height: 4, pixels, ir: None };
        // Normalized rect over the left half -> bright base.
        let (x, y, w, h) = crop_px([0.0, 0.0, 0.5, 1.0], img.width, img.height);
        let base = sample_base(&img, Some(Rect { x, y, w, h }));
        assert!(base[0] >= 0.85, "left-half base should be bright, got {base:?}");
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
