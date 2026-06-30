//! Tauri commands orchestrating film-core for the OpenEnlarge UI.

use crate::convert::{crop, orient, orient_dims, proxy, resize_to, rotate};
use crate::encode::{to_jpeg_b64, to_png_b64, write_jpeg, write_png, write_tiff8};
use crate::gpu_upload::{
    bake_geometry, bake_working, capped_dims, image_from_rgba8, image_from_rgba_f32_le,
    pack_rgba16f, resolve_to_uniforms, BakeSpec, ResolvedInversion, MAX_GPU_EDGE,
};
use crate::metadata::extract;
use crate::session::{
    CachedImage, DecodeCacheEntry, Developed, ImageEntry, InvertParams, PreparedExport, Session,
};
use film_core::calibrate::auto_wb_gains_masked;
use film_core::decode::{decode_ldr, decode_raw, decode_raw_preview, decode_tiff};
use film_core::dust::{self, Stamp};
use film_core::engine::{invert_image, invert_image_core, InversionParams, Mode};
use film_core::finish::{finish_image, tone_luts, ColorGrade, ColorMix, FinishParams, PcSample};
use film_core::wb::{gains_to_cct, wb_from_kelvin};
use base64::Engine;
use serde::Deserialize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::State;

fn default_bits() -> u8 {
    16
}
fn default_quality() -> u8 {
    85
}

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
    #[serde(default)]
    pub resize_long_edge: Option<u32>, // downscale cap on the long edge; None/0 = full res
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
/// Display-sized proxy cap (long edge) for the resident working buffer + fit-view GPU
/// upload. Replaces the old Quality cap: crisp at fit on hi-DPI, small + fast to decode
/// and upload. Deep zoom loads a higher-res source on demand (see `zoom_source`).
const PROXY_EDGE: u32 = 2560;

pub(crate) fn default_invert_params() -> InvertParams {
    InvertParams {
        mode: "d".into(),
        stock: "none".into(),
        base_override: None,
        d_max_override: None,
        exposure: 0.0,
        black: 0.0,
        gamma: 0.4545,
        auto_wb: true,
        temp: 5500.0,
        tint: 0.0,
        wb_baseline: [1.0, 1.0, 1.0],
        wb_manual: false,
        hdr: false,
        positive: false,
        meter_border: "auto".to_string(),
        contrast: 0.0,
        highlights: 0.0,
        shadows: 0.0,
        whites: 0.0,
        blacks: 0.0,
        texture: 0.0,
        vibrance: 0.0,
        saturation: 0.0,
        brightness: 0.0,
        tc_highlights: 0.0,
        tc_lights: 0.0,
        tc_darks: 0.0,
        tc_shadows: 0.0,
        tc_curve: crate::session::identity_curve(),
        tc_red: crate::session::identity_curve(),
        tc_green: crate::session::identity_curve(),
        tc_blue: crate::session::identity_curve(),
        cg_sh_hue: 0.0,
        cg_sh_sat: 0.0,
        cg_sh_lum: 0.0,
        cg_mid_hue: 0.0,
        cg_mid_sat: 0.0,
        cg_mid_lum: 0.0,
        cg_hi_hue: 0.0,
        cg_hi_sat: 0.0,
        cg_hi_lum: 0.0,
        cg_glob_hue: 0.0,
        cg_glob_sat: 0.0,
        cg_glob_lum: 0.0,
        cg_blending: 50.0,
        cg_balance: 0.0,
        cm_red_hue: 0.0,
        cm_red_sat: 0.0,
        cm_red_lum: 0.0,
        cm_orange_hue: 0.0,
        cm_orange_sat: 0.0,
        cm_orange_lum: 0.0,
        cm_yellow_hue: 0.0,
        cm_yellow_sat: 0.0,
        cm_yellow_lum: 0.0,
        cm_green_hue: 0.0,
        cm_green_sat: 0.0,
        cm_green_lum: 0.0,
        cm_aqua_hue: 0.0,
        cm_aqua_sat: 0.0,
        cm_aqua_lum: 0.0,
        cm_blue_hue: 0.0,
        cm_blue_sat: 0.0,
        cm_blue_lum: 0.0,
        cm_purple_hue: 0.0,
        cm_purple_sat: 0.0,
        cm_purple_lum: 0.0,
        cm_magenta_hue: 0.0,
        cm_magenta_sat: 0.0,
        cm_magenta_lum: 0.0,
        pc_samples: Vec::new(),
        wb_mode: "gain".to_string(),
        tone_mode: "filmic".to_string(),
        pz_enabled: true,
        pz_strength: 0.7,
        pz_sh: [1.0, 1.0, 1.0],
        pz_mid: [1.0, 1.0, 1.0],
        pz_hi: [1.0, 1.0, 1.0],
    }
}

fn metadata_to_json(m: &crate::metadata::Metadata) -> Result<String, String> {
    serde_json::to_string(m).map_err(|e| e.to_string())
}

fn decode_any(path: &Path) -> Result<film_core::Image, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "tif" | "tiff" => decode_tiff(path).map_err(|e| format!("{e}")),
        "jpg" | "jpeg" | "png" => decode_ldr(path).map_err(|e| format!("{e}")),
        _ => decode_raw(path, true).map_err(|e| format!("{e}")),
    }
}

/// `decode_any` with a single-slot, mtime-validated cache (see `Session::decode_cache`).
/// All three export paths re-decode the full-res original (the resident buffer is only
/// a proxy); this returns a shared `Arc` so a repeat export of the same frame — or the
/// GPU→CPU fallback decoding the same file a second time — reuses one decode instead of
/// re-running the (often multi-second) RAW demosaic. A changed file (new mtime) misses
/// and re-decodes, so an edited source is never served stale. The decode itself runs
/// WITHOUT the lock held, so concurrent workers decoding different frames don't serialize.
fn decode_cached(
    cache: &Mutex<Option<DecodeCacheEntry>>,
    path: &Path,
) -> Result<Arc<film_core::Image>, String> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some(mt) = mtime {
        let guard = cache.lock().unwrap();
        if let Some(e) = guard.as_ref() {
            if e.path == path && e.mtime == mt {
                return Ok(e.image.clone());
            }
        }
    }
    // Miss (or unknown mtime): decode off-lock, then publish to the slot.
    let img = Arc::new(decode_any(path)?);
    if let Some(mt) = mtime {
        *cache.lock().unwrap() = Some(DecodeCacheEntry {
            path: path.to_path_buf(),
            mtime: mt,
            image: img.clone(),
        });
    }
    Ok(img)
}

pub(crate) fn mode_from(_s: &str) -> Mode {
    // One engine. The `mode` wire field is vestigial; always Cineon.
    Mode::D
}

/// Parse the wire `wb_mode` string into the engine enum. Unknown values fall back
/// to `Gain` (the safe legacy behavior).
pub(crate) fn wb_mode_from(s: &str) -> film_core::WbMode {
    match s {
        "subtractive" => film_core::WbMode::Subtractive,
        _ => film_core::WbMode::Gain,
    }
}

/// Parse the wire `tone_mode` string into the engine enum. Unknown values fall back
/// to `Filmic` (the safe legacy behavior).
// Kept for tests and back-compat; the production chokepoints now hard-code Faithful.
#[allow(dead_code)]
pub(crate) fn tone_mode_from(s: &str) -> film_core::ToneMode {
    match s {
        "faithful" => film_core::ToneMode::Faithful,
        _ => film_core::ToneMode::Filmic,
    }
}

pub(crate) fn build_params(p: &InvertParams, base: [f32; 3]) -> InversionParams {
    // One engine: Kodak Cineon (negadoctor). The exposure slider drives print
    // exposure; d_max/paper_* come from InversionParams::Default; WB is set by the
    // caller (resolve_params / resolve_to_uniforms). `stock`/`mode`/`black`/`gamma`
    // are vestigial — kept in the wire contract for back-compat, no longer read.
    InversionParams {
        base,
        print_exposure: 2f32.powf(p.exposure), // EV stops → linear print exposure
        d_max: p.d_max_override.unwrap_or(1.5),
        positive: p.positive,
        wb_mode: wb_mode_from(&p.wb_mode),
        // Faithful is the sole develop/tune path; ignore any stored `tone_mode` (Filmic
        // is retired from the app). See docs/superpowers/specs/2026-06-21-faithful-look-layer-sole-path-design.md.
        tone_mode: film_core::ToneMode::Faithful,
        // Highlight/shadow recovery is retired — the finish tone tools recover detail
        // on the super-white body instead (see headroom-tone-recovery design). Keep the
        // fields (wire contract) but always zero.
        hi_recovery: 0.0,
        lo_recovery: 0.0,
        ..Default::default()
    }
}

/// The base to invert with: the per-image override if set, else the develop-time
/// auto base sampled at `develop_image` time.
pub(crate) fn effective_base(p: &InvertParams, dev_base: [f32; 3]) -> [f32; 3] {
    p.base_override.unwrap_or(dev_base)
}

/// Effective Cineon D_max for an inversion: the per-image override if set, else the
/// develop-time auto value stored on `Developed`.
pub(crate) fn effective_dmax(p: &InvertParams, dev_dmax: f32) -> f32 {
    p.d_max_override.unwrap_or(dev_dmax)
}

/// Decide the d_max to apply after a crop re-analysis (B3): a crop that lacks real
/// blacks (density `spread` below `MIN_SPREAD`) gives an unreliable, range-destroying
/// estimate, so keep `prior`; otherwise take the fresh `estimate`. Tuned so normal
/// frames clear the bar and a sky-/highlight-only crop does not.
pub(crate) fn guard_dmax(estimate: f32, spread: f32, prior: f32) -> f32 {
    const MIN_SPREAD: f32 = 0.35;
    if spread < MIN_SPREAD { prior } else { estimate }
}

/// Pick the film base for a freshly-developed working image. Returns (base, confidence).
///
/// 1. Confident edge-detected rebate → use it.
/// 2. Otherwise the brightest-cluster fallback — UNLESS that fallback is non-orange
///    (B ≥ R) while the edge detector did find an orange candidate. A C-41 mask is
///    always orange, so a blue fallback means a bright blue scene outscored a dim
///    rebate (the mixed-roll "Phoenix" bug); prefer the orange edge even at low
///    confidence (the UI flags it for an optional repoint).
/// 3. Else the brightest-cluster fallback (already orange-ish).
pub(crate) fn auto_base(working: &film_core::Image) -> ([f32; 3], f32) {
    use film_core::calibrate::{
        detect_rebate_base, sample_base_coherent, BASE_BAND_AUTO, REBATE_CONFIDENCE,
    };
    let det = detect_rebate_base(working);
    if det.confidence >= REBATE_CONFIDENCE {
        return (det.base, det.confidence);
    }
    let (lo, hi) = BASE_BAND_AUTO;
    let fb = sample_base_coherent(working, None, lo, hi);
    // Anti-blue: orange edge beats a non-orange (blue) fallback — but only if the
    // edge has real brightness. A near-black noise patch can satisfy R>B ordering
    // yet make a useless dmin; the luma floor rejects it (a dim real rebate is
    // ~0.12–0.24 luma, well above 0.06), leaving the fallback for true rebate-less frames.
    let det_luma = (det.base[0] + det.base[1] + det.base[2]) / 3.0;
    if fb[2] >= fb[0] && det.base[0] > det.base[2] && det_luma > 0.06 {
        return (det.base, det.confidence);
    }
    (fb, det.confidence)
}

pub(crate) fn wb_from_params(temp: f32, tint: f32) -> [f32; 3] {
    wb_from_kelvin(temp, tint / 150.0)
}

/// Baseline gains for a (temp, tint) estimate — exactly the WB the render applies.
pub(crate) fn as_shot_gains(temp: f32, tint: f32) -> [f32; 3] {
    wb_from_params(temp, tint)
}

pub(crate) fn resolve_params(
    p: &InvertParams,
    _autowb_src: &film_core::Image,
    base: [f32; 3],
) -> InversionParams {
    let mut ip = build_params(p, base);
    // Final WB = hidden auto baseline × visible slider trim (both per-channel gains).
    let slider = wb_from_params(p.temp, p.tint);
    ip.wb = std::array::from_fn(|c| p.wb_baseline[c] * slider[c]);
    ip
}

fn finish_default() -> bool {
    true
}

/// Map a normalized crop rect [x,y,w,h] (0..1) to integer pixels on a w×h image,
/// clamped to bounds with a 1px minimum.
pub(crate) fn crop_px(norm: [f64; 4], w: usize, h: usize) -> (usize, usize, usize, usize) {
    let x = (norm[0] * w as f64).round().clamp(0.0, (w - 1) as f64) as usize;
    let y = (norm[1] * h as f64).round().clamp(0.0, (h - 1) as f64) as usize;
    let cw = (norm[2] * w as f64).round().clamp(1.0, (w - x) as f64) as usize;
    let ch = (norm[3] * h as f64).round().clamp(1.0, (h - y) as f64) as usize;
    (x, y, cw, ch)
}

/// Apply the same lossless geometry the render/export pipeline applies *before* the
/// persistent crop (orient → straighten), so a normalized crop expressed in oriented
/// image space samples the region the user actually sees. Returns a borrow when the
/// geometry is identity to skip copying the whole buffer.
fn geom_base(
    img: &film_core::Image,
    rot90: u8,
    flip_h: bool,
    flip_v: bool,
    angle: f32,
) -> std::borrow::Cow<'_, film_core::Image> {
    use std::borrow::Cow;
    let oriented: Cow<film_core::Image> = if rot90 % 4 == 0 && !flip_h && !flip_v {
        Cow::Borrowed(img)
    } else {
        Cow::Owned(orient(img, rot90, flip_h, flip_v))
    };
    if angle.abs() < 1e-4 {
        oriented
    } else {
        Cow::Owned(rotate(&oriented, angle))
    }
}

/// Geometry-aware `sample_dmax`: orient/straighten the working buffer to match the UI
/// before mapping the normalized crop, so flipped/rotated frames analyze the correct
/// image area (a crop in oriented space applied to the un-oriented buffer samples the
/// wrong region — washing out brightness on horizontally-flipped images).
///
/// `mode` / `positive` are forwarded to `meter_mask` so spoke/rebate borders are
/// excluded from the D_max estimate when the user has chosen "exclude" metering.
#[allow(clippy::too_many_arguments)]
fn sample_dmax_oriented(
    working: &film_core::Image,
    base: [f32; 3],
    crop: Option<[f64; 4]>,
    rot90: u8,
    flip_h: bool,
    flip_v: bool,
    angle: f32,
    mode: &str,
    positive: bool,
) -> (f32, f32) {
    use film_core::calibrate::{sample_dmax_spread, Rect};
    let geom = geom_base(working, rot90, flip_h, flip_v, angle);
    let rect = crop.map(|nc| {
        let (x, y, w, h) = crop_px(nc, geom.width, geom.height);
        Rect { x, y, w, h }
    });
    // Build the mask on the same downscaled grid sample_dmax_spread reduces on.
    // We detect over the cropped region so the mask indexes match the rect sampling.
    let mask = match rect {
        Some(r) => {
            let cropped = crate::convert::crop(&geom, r.x, r.y, r.w, r.h);
            meter_mask(&cropped, base, positive, mode)
        }
        None => meter_mask(&*geom, base, positive, mode),
    };
    match mask {
        // Masked: sample the cropped+downscaled grid directly (rect = None because
        // it's already cropped), excluding spoke/rebate borders.
        Some((small, keep)) => sample_dmax_spread(&small, base, None, Some(&keep)),
        None => sample_dmax_spread(&geom, base, rect, None),
    }
}

/// Map normalized strokes → `Stamp`s in OUTPUT pixel space.
/// `base_w/base_h` are the WORKING image dims BEFORE the view crop is applied
/// (oriented + straightened + persistent image_crop, but NOT the per-render view
/// crop) — this is the image the UI normalizes stroke coords against. `(cx,cy,cw,ch)`
/// is the view-crop window within that base; `out_w/out_h` is the rendered output size.
#[allow(clippy::too_many_arguments)]
fn view_stamps(
    dust: &[DustStroke],
    base_w: usize,
    base_h: usize,
    cx: usize,
    cy: usize,
    cw: usize,
    ch: usize,
    out_w: u32,
    out_h: u32,
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
            out.push(Stamp {
                cx: (pt[0] * w as f64) as f32,
                cy: (pt[1] * h as f64) as f32,
                r,
            });
        }
    }
    out
}

fn color_mix_from(p: &crate::session::InvertParams) -> ColorMix {
    let cm_hue = [
        p.cm_red_hue,
        p.cm_orange_hue,
        p.cm_yellow_hue,
        p.cm_green_hue,
        p.cm_aqua_hue,
        p.cm_blue_hue,
        p.cm_purple_hue,
        p.cm_magenta_hue,
    ];
    let cm_sat = [
        p.cm_red_sat,
        p.cm_orange_sat,
        p.cm_yellow_sat,
        p.cm_green_sat,
        p.cm_aqua_sat,
        p.cm_blue_sat,
        p.cm_purple_sat,
        p.cm_magenta_sat,
    ];
    let cm_lum = [
        p.cm_red_lum,
        p.cm_orange_lum,
        p.cm_yellow_lum,
        p.cm_green_lum,
        p.cm_aqua_lum,
        p.cm_blue_lum,
        p.cm_purple_lum,
        p.cm_magenta_lum,
    ];
    let samples = p
        .pc_samples
        .iter()
        .map(|s| PcSample {
            hue: s.hue,
            sat: s.sat,
            lum: s.lum,
            hue_shift: s.hue_shift / 100.0,
            sat_shift: s.sat_shift / 100.0,
            lum_shift: s.lum_shift / 100.0,
            variance: s.variance,
            range: s.range,
        })
        .collect();
    ColorMix {
        cm_hue: cm_hue.map(|v| v / 100.0),
        cm_sat: cm_sat.map(|v| v / 100.0),
        cm_lum: cm_lum.map(|v| v / 100.0),
        samples,
    }
}

pub(crate) fn finish_from(p: &InvertParams) -> FinishParams {
    // Region sliders ordered [shadows, darks, lights, highlights] to match the engine.
    let regions = [
        p.tc_shadows / 100.0,
        p.tc_darks / 100.0,
        p.tc_lights / 100.0,
        p.tc_highlights / 100.0,
    ];
    let (lut_r, lut_g, lut_b) = tone_luts(regions, &p.tc_curve, &p.tc_red, &p.tc_green, &p.tc_blue);
    let cg = ColorGrade::new(
        ([p.cg_sh_hue, p.cg_sh_sat / 100.0], p.cg_sh_lum / 100.0),
        ([p.cg_mid_hue, p.cg_mid_sat / 100.0], p.cg_mid_lum / 100.0),
        ([p.cg_hi_hue, p.cg_hi_sat / 100.0], p.cg_hi_lum / 100.0),
        (
            [p.cg_glob_hue, p.cg_glob_sat / 100.0],
            p.cg_glob_lum / 100.0,
        ),
        p.cg_blending / 100.0,
        p.cg_balance / 100.0,
    );
    FinishParams {
        contrast: p.contrast / 100.0,
        // Negative half is engine recovery (build_params); finish only sees the
        // positive half so the slider never darkens AND recovers at once.
        highlights: (p.highlights / 100.0).max(0.0),
        shadows: (p.shadows / 100.0).max(0.0),
        whites: p.whites / 100.0,
        blacks: p.blacks / 100.0,
        texture: p.texture / 100.0,
        vibrance: p.vibrance / 100.0,
        saturation: p.saturation / 100.0,
        brightness: p.brightness / 100.0,
        lut_r,
        lut_g,
        lut_b,
        cg,
        cm: color_mix_from(p),
        per_zone: film_core::finish::PerZoneWb {
            enabled: p.pz_enabled,
            sh: p.pz_sh,
            mid: p.pz_mid,
            hi: p.pz_hi,
        },
        // Positive scans short-circuit invert_d_core to develop_positive_px (a
        // display value) — do NOT apply display_finalize a second time. Negatives
        // carry the Faithful gamma BODY (may exceed 1.0) and need finalization.
        finalize_body: !p.positive,
    }
}

/// LIGHT import: thumbnail (embedded preview if available) + metadata + stored
/// path. Previewable formats skip the demosaic — the heavy develop work happens in
/// `develop_image`. (Formats rawler can't preview — e.g. Olympus `.orf` — do run a
/// one-time full decode here so the grid never shows a black cell.)
#[tauri::command]
pub async fn import_image(
    path: String,
    session: State<'_, Session>,
    catalog: State<'_, crate::catalog::Catalog>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<ImageEntry, String> {
    let __log = log.inner().clone();
    crate::time_op!(__log, "import_image", {
        // The preview decode + metadata extraction run for tens of ms (RAW embedded
        // preview) up to seconds (ORF full decode), and imports are driven serially per
        // file from the frontend. Keep it off the main thread — which the WKWebView
        // shares on macOS — like `develop_image`, so the UI stays live and thumbnails
        // populate progressively. No `Session`/`Catalog` borrow crosses the boundary;
        // the catalog upsert + session insert happen back here on the async side.
        let path_for_compute = path.clone();
        let (thumbnail, file_name, metadata, metadata_json, auto_crop) =
            tauri::async_runtime::spawn_blocking(move || import_compute(path_for_compute))
                .await
                .map_err(|e| e.to_string())??;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let id = catalog
            .upsert_image(&path, &file_name, &metadata_json, &thumbnail, now)
            .map_err(|e| e.to_string())?;
        let cached = CachedImage {
            path,
            file_name,
            metadata,
            thumbnail,
            developed: None,
            last_access: 0,
        };
        let mut entry = session.insert_with_id(id, cached);
        entry.auto_crop = auto_crop.map(|[x, y, w, h]| crate::session::AutoCropRect { x, y, w, h });
        Ok(entry)
    })
}

/// CPU-bound half of `import_image`, run on `spawn_blocking`: build the light
/// thumbnail and extract metadata. Owned in / owned out so no `Session`/`Catalog`
/// borrow crosses the thread boundary. Returns `(thumbnail, file_name, metadata,
/// metadata_json, auto_crop)`.
fn import_compute(
    path: String,
) -> Result<
    (
        String,
        String,
        crate::metadata::Metadata,
        String,
        Option<[f32; 4]>,
    ),
    String,
> {
    let p = Path::new(&path);
    // Light thumbnail so the Library grid shows a real picture the instant a file is
    // imported, before develop:
    //   • JPEG/PNG/TIFF decode cheaply → render the real image directly.
    //   • RAW (dng/raf/nef/cr2/cr3/arw/rw2/3fr/…) → pull the camera's EMBEDDED
    //     preview JPEG via rawler. Without this a fresh RAW renders as a black
    //     placeholder until it's developed.
    //   • RAW with no embedded preview (Olympus .orf, scanner "linear DNG", bare
    //     .raw) → fall back to the tiff crate, then a one-time full demosaic
    //     (`decode_raw`). Heavier, but it only runs for these few formats and we're
    //     off the UI thread here.
    // Anything still undecodable falls back to the 1x1 placeholder.
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let preview: Option<film_core::Image> = match ext.as_str() {
        "jpg" | "jpeg" | "png" => decode_ldr(p).ok(),
        "tif" | "tiff" => decode_tiff(p).ok(),
        _ => decode_raw_preview(p, THUMB_EDGE)
            .ok()
            .or_else(|| decode_tiff(p).ok())
            .or_else(|| {
                // decode_raw can panic on malformed RAW; guard it so one bad file
                // degrades to the placeholder instead of failing the whole import.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    decode_raw(p, true).ok()
                }))
                    .ok()
                    .flatten()
            }),
    };
    // Detect the film-strip frame on the SAME small THUMB_EDGE proxy we build for
    // the thumbnail — coordinates are normalized so the result matches the full
    // image, and we avoid scanning a full-resolution preview (LDR/TIFF can be 24MP).
    let mut auto_crop = None;
    let thumbnail = match preview {
        Some(prev) => {
            let small = proxy(&prev, THUMB_EDGE);
            auto_crop = film_core::autocrop::detect_film_frame_crop(&small);
            to_png_b64(&small, true)?
        }
        None => "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
    };
    let metadata = extract(p, 0, 0);
    let file_name = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    let metadata_json = metadata_to_json(&metadata)?;
    Ok((thumbnail, file_name, metadata, metadata_json, auto_crop))
}

/// List the absolute paths of regular files under `dir`, recursing into every
/// subfolder. Extension filtering is left to the frontend so `IMPORT_EXTENSIONS`
/// stays the single source of truth. Used by the "Import Folder…" picker.
#[tauri::command]
pub fn list_dir_files(dir: String) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    collect_files(Path::new(&dir), &mut paths).map_err(|e| format!("{e}"))?;
    paths.sort();
    Ok(paths)
}

/// Depth-first walk: push every regular file's path, descending into subdirectories.
/// Unreadable subdirectories are skipped rather than aborting the whole scan.
fn collect_files(dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let ty = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ty.is_dir() {
            let _ = collect_files(&entry.path(), out); // skip folders we can't read
        } else if ty.is_file() {
            if let Some(p) = entry.path().to_str() {
                out.push(p.to_string());
            }
        }
    }
    Ok(())
}

/// HEAVY step: decode the file, build the working image at the quality cap, a
/// small auto-WB thumb, and sample the base. Drops full_res. Returns the updated
/// entry (real dimensions + developed=true).
#[tauri::command]
pub async fn develop_image(
    id: String,
    session: State<'_, Session>,
    catalog: State<'_, crate::catalog::Catalog>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<ImageEntry, String> {
    let __log = log.inner().clone();
    crate::time_op!(__log, "develop_image", {
        develop_heavy(id, &session, &catalog).await
    })
}

/// Owned result of the CPU-heavy decode/analysis, computed off the UI thread and
/// then folded into the session + catalog by `develop_heavy`.
struct DevelopComputed {
    working: film_core::Image,
    thumb: film_core::Image,
    base: [f32; 3],
    base_confidence: f32,
    positive: bool,
    positive_confidence: f32,
    d_max: f32,
    channel_balance: [f32; 3],
    has_ir: bool,
    w: u32,
    h: u32,
    thumbnail: String,
    cache_working: film_core::Image,
    cache_thumb: film_core::Image,
}

/// Per-channel density-neutralisation factors for the current decode: the camera-matrix
/// auto-equaliser. RAW decode always applies the camera color matrix now, so this is
/// computed unconditionally. Derived from an already-decoded working buffer so the
/// cache-rehydrate path can recompute it without re-decoding.
fn channel_balance_for(working: &film_core::Image, base: [f32; 3]) -> [f32; 3] {
    film_core::calibrate::sample_channel_balance(working, base)
}

/// Pure CPU work: decode the RAW, build the quality-capped working image + auto-WB
/// thumb, sample the base, and render the grid thumbnail. Owned in/owned out so it
/// can run inside `spawn_blocking` (no `Session` borrow crosses the thread boundary).
fn develop_compute(path: String) -> Result<DevelopComputed, String> {
    let full = decode_any(Path::new(&path))?;
    let working = proxy(&full, PROXY_EDGE);
    let has_ir = working.ir.is_some();
    let thumb = proxy(&full, AUTOWB_EDGE);
    let (base, base_confidence) = auto_base(&working);
    let (positive, positive_confidence) = film_core::classify::classify_positive(&working);
    let d_max = film_core::calibrate::sample_dmax(&working, base, None);
    let channel_balance = channel_balance_for(&working, base);
    let (w, h) = (full.width as u32, full.height as u32);
    drop(full);

    let small = proxy(&working, THUMB_EDGE);
    // Bake the per-image auto-WB seed (estimated on the 256px `thumb`, matching the
    // frontend `as_shot_wb`) into the develop-time grid thumbnail, so the contact
    // sheet / grid / filmstrip show the correct look immediately on import instead
    // of a neutral-WB render that only snaps right once you open Develop.
    let thumbnail = render_grid_thumbnail(&small, &thumb, base, d_max, channel_balance, None, positive)?;

    // Build a cache-bounded copy of working (≤CACHE_WORKING_CAP long edge) for the sidecar.
    // Clone thumb too before the move into Developed.
    let cache_working = if working.width.max(working.height) > CACHE_WORKING_CAP as usize {
        crate::convert::proxy(&working, CACHE_WORKING_CAP)
    } else {
        working.clone()
    };
    let cache_thumb = thumb.clone();

    Ok(DevelopComputed {
        working,
        thumb,
        base,
        base_confidence,
        positive,
        positive_confidence,
        d_max,
        channel_balance,
        has_ir,
        w,
        h,
        thumbnail,
        cache_working,
        cache_thumb,
    })
}

/// Decode the RAW (off the UI thread), then fold the result into the session,
/// refresh the thumbnail/catalog, and write the cache sidecar. The expensive path
/// shared by `develop_image` and `ensure_developed`.
async fn develop_heavy(
    id: String,
    session: &Session,
    catalog: &crate::catalog::Catalog,
) -> Result<ImageEntry, String> {
    let path = {
        let images = session.images.lock().unwrap();
        images.get(&id).ok_or("unknown image id")?.path.clone()
    };
    // The RAW decode + base/d_max analysis runs for seconds on large files; keep it
    // off the main thread (which the WKWebView shares on macOS) so the UI stays live.
    let c = tauri::async_runtime::spawn_blocking(move || develop_compute(path))
        .await
        .map_err(|e| e.to_string())??;

    // Mutate session state and build the entry inside the lock, then release
    // the guard before the expensive cache write (tens of MB, zstd + file IO).
    let (entry, metadata_json) = {
        let mut images = session.images.lock().unwrap();
        let img = images.get_mut(&id).ok_or("unknown image id")?;
        img.metadata.width = c.w;
        img.metadata.height = c.h;
        img.thumbnail = c.thumbnail.clone();
        img.developed = Some(Developed {
            working: c.working,
            thumb: c.thumb,
            base: c.base,
            base_confidence: c.base_confidence,
            d_max: c.d_max,
            positive: c.positive,
            positive_confidence: c.positive_confidence,
            channel_balance: c.channel_balance,
        });
        img.last_access = session.next_tick();
        let metadata_json = metadata_to_json(&img.metadata)?;
        let entry = ImageEntry {
            id: id.clone(),
            path: img.path.clone(),
            file_name: img.file_name.clone(),
            thumbnail: c.thumbnail,
            metadata: img.metadata.clone(),
            developed: true,
            has_ir: c.has_ir,
            offline: false,
            positive: c.positive,
            thumb_stale: false,
            auto_crop: None,
        };
        (entry, metadata_json)
    }; // lock released here
    session.evict_lru(&id); // a freshly developed buffer just became resident

    if let Err(e) = catalog.update_image_render(&id, &entry.thumbnail, &metadata_json) {
        eprintln!("[catalog] update_image_render failed for {id}: {e}");
    }

    // Write cache sidecar (best-effort; never fails the develop command).
    {
        let cp = session.cache_path(&id);
        if let Err(e) = crate::cache::write(&cp, c.base, &c.cache_working, &c.cache_thumb) {
            eprintln!("[cache] write failed for {id}: {e}; retrying once");
            if let Err(e2) = crate::cache::write(&cp, c.base, &c.cache_working, &c.cache_thumb) {
                eprintln!("[cache] write RETRY also failed for {id}: {e2}");
            }
        }
    }

    Ok(entry)
}

/// Idempotent, cache-aware develop. Loads the cached proxy buffer if not resident,
/// otherwise decodes + develops once. A resident developed image is always adequate
/// now that the working buffer is a fixed-size proxy.
#[tauri::command]
pub async fn ensure_developed(
    id: String,
    session: State<'_, Session>,
    catalog: State<'_, crate::catalog::Catalog>,
) -> Result<ImageEntry, String> {
    // Best-effort cache rehydration; ignore "not developed" — we full-develop below.
    let _ = ensure_resident(&session, &id);

    // A resident developed image is always adequate (proxy is a fixed cap, and old
    // oversized caches are re-proxied on load in `ensure_resident`).
    let resident_entry = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        img.developed.as_ref().map(|dev| ImageEntry {
            id: id.clone(),
            path: img.path.clone(),
            file_name: img.file_name.clone(),
            thumbnail: img.thumbnail.clone(),
            metadata: img.metadata.clone(),
            developed: true,
            has_ir: dev.working.ir.is_some(),
            offline: false,
            positive: dev.positive,
            thumb_stale: false,
            auto_crop: None,
        })
    };

    if let Some(entry) = resident_entry {
        return Ok(entry);
    }
    develop_heavy(id, &session, &catalog).await
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

/// Total bytes used by the render cache (`cache/*.oecache`).
#[tauri::command]
pub fn cache_size(session: State<Session>) -> u64 {
    let dir = session.cache_dir.lock().unwrap().clone();
    crate::cache::oecache_bytes(&dir)
}

/// Delete the render cache (`cache/*.oecache`) and drop the in-memory developed
/// buffers so the grid re-hydrates (images re-develop on next open). Catalog,
/// edits, and folders are untouched. Returns bytes freed.
#[tauri::command]
pub fn clear_image_cache(session: State<Session>) -> u64 {
    let dir = session.cache_dir.lock().unwrap().clone();
    let freed = crate::cache::clear_oecache(&dir);
    // Drop the heavy `developed` buffers (keep the lightweight image records);
    // they would otherwise mask the now-deleted cache until relaunch.
    for ci in session.images.lock().unwrap().values_mut() {
        ci.developed = None;
    }
    session.autodust_prob.lock().unwrap().clear();
    session.autodust_healed.lock().unwrap().clear();
    freed
}

/// Wipe all catalog content (images, edits, folder structure, app state) and the
/// render cache, but preserve preferences. The frontend relaunches the app after
/// this returns to drop all remaining in-memory state.
#[tauri::command]
pub fn reset_all_data(
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.reset_content().map_err(|e| format!("{e}"))?;
    let dir = session.cache_dir.lock().unwrap().clone();
    let _ = crate::cache::clear_oecache(&dir);
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

/// AI (learned-model) auto dust/hair removal settings from the UI. When
/// `enabled`, the bake path inverts the working buffer, runs the cached detector,
/// thresholds at `sensitivity`, and MI-GAN-heals the defect mask (unioned with
/// brush strokes) — see `working_baked_pixels` / `bake_for_view_from_baked`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AutoDust {
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
    #[serde(default)]
    pub rot90: u8,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub dust: Vec<DustStroke>,
    #[serde(default)]
    pub ir_removal: IrRemoval,
    /// Frontend sends auto-dust settings on the view spec, but the live view/preview
    /// render does not apply them (auto-dust is baked separately into the heal cache).
    /// Deserialized for the wire contract; never read on this path.
    #[serde(default)]
    #[allow(dead_code)]
    pub auto_dust: AutoDust,
}

/// Ensure the image's decoded `Developed` is resident in the Session: if absent
/// but a cache file exists, load it. Drops the lock during file IO.
fn ensure_resident(session: &Session, id: &str) -> Result<(), String> {
    {
        let mut images = session.images.lock().unwrap();
        match images.get_mut(id) {
            // Already resident: stamp LRU access and we're done.
            Some(c) if c.developed.is_some() => {
                c.last_access = session.next_tick();
                return Ok(());
            }
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
    // Re-proxy old oversized caches (pre-redesign sidecars held up to 4096) down to
    // the proxy cap so resident memory + fit-view upload stay consistent.
    let working = if working.width.max(working.height) as u32 > PROXY_EDGE {
        crate::convert::proxy(&working, PROXY_EDGE)
    } else {
        working
    };
    let base_confidence = film_core::calibrate::detect_rebate_base(&working).confidence;
    let (positive, positive_confidence) = film_core::classify::classify_positive(&working);
    let d_max = film_core::calibrate::sample_dmax(&working, base, None);
    let channel_balance = channel_balance_for(&working, base);
    {
        let mut images = session.images.lock().unwrap();
        if let Some(c) = images.get_mut(id) {
            if c.developed.is_none() {
                c.developed = Some(Developed {
                    working,
                    thumb,
                    base,
                    base_confidence,
                    d_max,
                    positive,
                    positive_confidence,
                    channel_balance,
                });
                c.last_access = session.next_tick();
            }
        }
    } // images lock released before evict (which re-locks)
    session.evict_lru(id); // a new buffer just became resident
    Ok(())
}

/// Ensure the single-slot high-res zoom source for `id` is cached, decoding from the
/// source file (capped at `MAX_GPU_EDGE`) on a miss or id change. Callers then read
/// dims under the lock, or clone it out for off-thread packing/baking. The heavy
/// `decode_any` + `proxy` runs on `spawn_blocking` so it never stalls the Tauri async
/// runtime worker (which on macOS shares fate with UI-serving IPC); only the brief
/// lock/path lookups touch the calling thread. `async` — callers must `.await`.
async fn ensure_zoom_src(session: &Session, id: &str) -> Result<(), String> {
    {
        let g = session.zoom_src.lock().unwrap();
        if matches!(g.as_ref(), Some((cid, _)) if cid == id) {
            return Ok(());
        }
    }
    let path = {
        let images = session.images.lock().unwrap();
        images.get(id).ok_or("unknown image id")?.path.clone()
    };
    // Decode + downscale off-thread: a RAW/TIFF decode is 100s ms–s and would
    // otherwise block the runtime worker for the whole gesture.
    let hi = tauri::async_runtime::spawn_blocking(move || -> Result<film_core::Image, String> {
        let full = decode_any(Path::new(&path))?;
        // Match the live proxy's noise floor before the GPU inverts this near-native
        // buffer: the convex inversion turns full-res per-pixel noise into a colour cast
        // (pink) the downscaled proxy never shows. See convert::match_proxy_noise.
        Ok(crate::convert::match_proxy_noise(
            &proxy(&full, MAX_GPU_EDGE),
            PROXY_EDGE,
        ))
    })
    .await
    .map_err(|e| e.to_string())??;
    *session.zoom_src.lock().unwrap() = Some((id.to_string(), hi));
    Ok(())
}

/// Clone the cached high-res zoom source for `id` (for off-thread packing/baking).
async fn zoom_src_clone(session: &Session, id: &str) -> Result<film_core::Image, String> {
    ensure_zoom_src(session, id).await?;
    let g = session.zoom_src.lock().unwrap();
    g.as_ref()
        .filter(|(cid, _)| cid == id)
        .map(|(_, img)| img.clone())
        .ok_or_else(|| "no zoom source".to_string())
}

#[tauri::command]
pub async fn render_view(
    id: String,
    params: InvertParams,
    view: ViewSpec,
    session: State<'_, Session>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<String, String> {
    let __log = log.inner().clone();
    ensure_resident(&session, &id)?;
    // Snapshot the owned inputs the CPU render needs, then drop the lock so the
    // pipeline (geometry → resize → invert → dust → finish → JPEG) can run off the
    // UI thread. The working clone is the cost of getting it off the main thread.
    let (working, thumb, base, d_max, channel_balance, meta_w, meta_h) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (
            dev.working.clone(),
            dev.thumb.clone(),
            dev.base,
            dev.d_max,
            dev.channel_balance,
            img.metadata.width,
            img.metadata.height,
        )
    };
    crate::time_op!(__log, "render_view", {
        tauri::async_runtime::spawn_blocking(move || {
            render_view_compute(&working, &thumb, base, d_max, channel_balance, meta_w, meta_h, &params, &view)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r)
    })
}

/// Pure CPU render of one preview frame: geometry (orient → straighten → persistent
/// crop → view crop) → resize → invert → dust/IR → finish → JPEG. Owned inputs so it
/// runs in `spawn_blocking` without borrowing the `Session`.
#[allow(clippy::too_many_arguments)]
fn render_view_compute(
    working: &film_core::Image,
    thumb: &film_core::Image,
    base: [f32; 3],
    d_max: f32,
    channel_balance: [f32; 3],
    meta_w: u32,
    meta_h: u32,
    params: &InvertParams,
    view: &ViewSpec,
) -> Result<String, String> {
    // Each stage borrows the previous buffer when it would be a no-op, so an
    // identity/full-frame view (e.g. the film-base picker: no rot/flip/straighten/crop)
    // skips re-copying the whole working image — the dominant cost on large buffers.
    let oriented_owned;
    let oriented: &film_core::Image = if view.rot90 == 0 && !view.flip_h && !view.flip_v {
        working
    } else {
        oriented_owned = orient(working, view.rot90, view.flip_h, view.flip_v);
        &oriented_owned
    };
    let straightened_owned;
    let straightened: &film_core::Image = if view.angle.abs() < 1e-4 {
        oriented
    } else {
        straightened_owned = rotate(oriented, view.angle);
        &straightened_owned
    };
    let base_img_owned;
    let base_img: &film_core::Image = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            base_img_owned = crop(straightened, ix, iy, iw, ih);
            &base_img_owned
        }
        None => straightened,
    };
    // The view crop is in oriented full-res coords → map to working px via the
    // oriented metadata width (orientation is lossless, so the ratio is preserved).
    let (ometa_w, _) = orient_dims(meta_w as usize, meta_h as usize, view.rot90);
    let s_scale = oriented.width as f64 / ometa_w.max(1) as f64;
    let cx = (view.crop[0] * s_scale).max(0.0).round() as usize;
    let cy = (view.crop[1] * s_scale).max(0.0).round() as usize;
    let cw = (view.crop[2] * s_scale).round().max(1.0) as usize;
    let ch = (view.crop[3] * s_scale).round().max(1.0) as usize;
    // Whole-frame view crop (the common case — zoom/pan and pickers pass [0,0,w,h])
    // borrows base_img instead of copying it.
    let cropped_owned;
    let cropped: &film_core::Image =
        if cx == 0 && cy == 0 && cw >= base_img.width && ch >= base_img.height {
            base_img
        } else {
            cropped_owned = crop(base_img, cx, cy, cw, ch);
            &cropped_owned
        };
    if cropped.pixels.is_empty() {
        return Err("empty crop".into());
    }
    let (cw_px, ch_px) = (cropped.width, cropped.height);
    let scaled = resize_to(cropped, view.out_w.max(1), view.out_h.max(1));

    if view.raw {
        return to_jpeg_b64(&scaled, true, PREVIEW_JPEG_QUALITY);
    }
    let mut ip = resolve_params(params, thumb, effective_base(params, base));
    ip.d_max = effective_dmax(params, d_max);
    ip.channel_balance = channel_balance;
    let mut inv = invert_image_core(&scaled, &ip, mode_from(&params.mode));
    let stamps = view_stamps(
        &view.dust,
        base_img.width,
        base_img.height,
        cx,
        cy,
        cw_px,
        ch_px,
        view.out_w.max(1),
        view.out_h.max(1),
    );
    dust::apply(&mut inv, &stamps);
    if view.ir_removal.enabled {
        if let Some(ir) = scaled.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, view.ir_removal.sensitivity);
        }
    }
    let out = if view.finish {
        finish_image(&inv, &finish_from(params))
    } else {
        inv
    };
    to_jpeg_b64(&out, false, PREVIEW_JPEG_QUALITY)
}

/// Run one prepared image through invert → dust → IR-retouch → finish, as the
/// HDR rendition: `hdr=true` (the invert-side highlight expansion above the
/// filmic knee, up to `HDR_HEADROOM`) + `finalize_body=false` (the finish-side
/// legacy clamped tone curve, applied to an already-display-referred input).
/// Shared by the gain-map HDR encoder (`render_and_encode_hdr`) and the raw
/// rgba16f buffer command (`encode_hdr_raw`) so neither duplicates the
/// invert→dust→ir→finish block.
///
/// `finish_image`'s tone-curve / saturation / LUT / color-grade / color-mix
/// stages all assume (and internally clamp to) an SDR `[0,1]` domain — they
/// were written for the plain SDR finish path and have no headroom concept.
/// Routing an HDR-expanded body (which can reach up to `HDR_HEADROOM`, e.g.
/// 2.5) straight through `finish_image` would therefore silently clip every
/// super-white highlight back down to `1.0`, defeating the point of an HDR
/// rendition. To avoid that without reaching into `finish_image`'s internals,
/// each channel is split into its `[0,1]`-clamped SDR body (which gets the
/// full, ordinary finish treatment — identical tone tools/LUT/grade a user
/// dials in) and the raw super-white *excess* above `1.0`; the excess is
/// added back, unprocessed, after finish. Below the knee (the vast majority of
/// a typical frame) this is a no-op (excess is 0, so behavior is byte-identical
/// to the plain SDR finish); only literal super-white highlights carry through
/// as real linear headroom.
fn render_hdr_image(
    src: &film_core::Image,
    ip: &InversionParams,
    mode: Mode,
    finish: &FinishParams,
    stamps: &[Stamp],
    ir_removal: &IrRemoval,
) -> film_core::Image {
    let mut ip_hdr = ip.clone();
    ip_hdr.hdr = true;
    let mut finish_hdr = finish.clone();
    finish_hdr.finalize_body = false; // HDR rendition is already display-referred

    let mut inv = invert_image_core(src, &ip_hdr, mode);
    dust::apply(&mut inv, stamps);
    if ir_removal.enabled {
        if let Some(ir) = src.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, ir_removal.sensitivity);
        }
    }

    let excess: Vec<[f32; 3]> = inv
        .pixels
        .iter()
        .map(|px| std::array::from_fn(|c| (px[c] - 1.0).max(0.0)))
        .collect();
    for px in inv.pixels.iter_mut() {
        for c in px.iter_mut() {
            *c = c.min(1.0);
        }
    }
    let mut out = finish_image(&inv, &finish_hdr);
    for (px, ex) in out.pixels.iter_mut().zip(excess.iter()) {
        for c in 0..3 {
            px[c] += ex[c];
        }
    }
    out
}

/// Pack a finished `film_core::Image` (display-referred sRGB, highlights may
/// exceed 1.0) into a row-major RGBA half-float buffer in linear extended-P3:
/// each channel is linearized via `srgb_to_linear_ext` (the same EOTF-with-
/// linear-continuation `encode_gain_map_jpeg` uses), so super-white highlights
/// stay > 1.0 instead of being clipped. Alpha is always opaque (1.0).
fn hdr_image_to_rgba16f(img: &film_core::Image) -> crate::hdr_surface::HdrBuffer {
    let (w, h) = (img.width as u32, img.height as u32);
    let mut out = Vec::with_capacity(img.pixels.len() * 4);
    for px in &img.pixels {
        out.push(half::f16::from_f32(crate::hdr::srgb_to_linear_ext(px[0])).to_bits());
        out.push(half::f16::from_f32(crate::hdr::srgb_to_linear_ext(px[1])).to_bits());
        out.push(half::f16::from_f32(crate::hdr::srgb_to_linear_ext(px[2])).to_bits());
        out.push(half::f16::from_f32(1.0).to_bits());
    }
    crate::hdr_surface::HdrBuffer { width: w, height: h, rgba16f: out }
}

/// Dual-render one prepared image (invert → dust → IR → finish) as SDR and HDR,
/// then mux into a gain-map JPEG. Shared by the HDR preview command and HDR export.
/// `src` carries the optional IR plane used when `ir_removal.enabled`.
fn render_and_encode_hdr(
    src: &film_core::Image,
    ip: &InversionParams,
    mode: Mode,
    finish: &FinishParams,
    stamps: &[Stamp],
    ir_removal: &IrRemoval,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let mut inv = invert_image_core(src, ip, mode);
    dust::apply(&mut inv, stamps);
    if ir_removal.enabled {
        if let Some(ir) = src.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, ir_removal.sensitivity);
        }
    }
    let sdr = finish_image(&inv, finish); // Faithful SDR body → finalize (as given)
    let hdr = render_hdr_image(src, ip, mode, finish, stamps, ir_removal);
    crate::hdr::encode_gain_map_jpeg(&sdr, &hdr, quality)
}

/// Render the developed image (geometry + crop + develop params) as an SDR base
/// AND an HDR rendition, encode a gain-map JPEG, and return it as a data URL for
/// an `<img src>`. The SDR rendition matches `render_view`'s finished output
/// exactly (same geometry, params, dust/IR retouch, and finish); the HDR
/// rendition re-runs the same pipeline with `hdr` highlight expansion enabled.
#[tauri::command]
pub fn encode_hdr(
    id: String,
    params: InvertParams,
    view: ViewSpec,
    session: State<Session>,
) -> Result<String, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;

    // Geometry: orient (lossless) → straighten → persistent crop, then the view
    // crop — identical to `render_view`.
    let oriented = orient(&dev.working, view.rot90, view.flip_h, view.flip_v);
    let straightened = rotate(&oriented, view.angle);
    let base_img = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, ix, iy, iw, ih)
        }
        None => straightened,
    };
    let (ometa_w, _) = orient_dims(
        img.metadata.width as usize,
        img.metadata.height as usize,
        view.rot90,
    );
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

    // Develop params identical to `render_view`'s construction.
    let mut ip = resolve_params(&params, &dev.thumb, effective_base(&params, dev.base));
    ip.d_max = effective_dmax(&params, dev.d_max);
    ip.channel_balance = dev.channel_balance;
    let mode = mode_from(&params.mode);
    let finish = finish_from(&params);
    let stamps = view_stamps(
        &view.dust,
        base_img.width,
        base_img.height,
        cx,
        cy,
        cw_px,
        ch_px,
        view.out_w.max(1),
        view.out_h.max(1),
    );

    let jpeg = render_and_encode_hdr(&scaled, &ip, mode, &finish, &stamps, &view.ir_removal, PREVIEW_JPEG_QUALITY)?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    Ok(format!("data:image/jpeg;base64,{b64}"))
}

/// Render the developed image (same geometry + crop + develop-param resolution
/// as `encode_hdr`) into the raw linearized rgba16f HDR buffer that the native
/// EDR surface draws directly. Shared by the `encode_hdr_raw` command (which
/// returns the buffer to JS — kept for the future Windows/WebGPU path) and the
/// fused `hdr_surface_render_show` command (macOS, which uploads it to the
/// native surface in-process with no IPC pixel round-trip).
pub(crate) fn render_hdr_buffer(
    id: String,
    params: InvertParams,
    view: ViewSpec,
    session: &Session,
) -> Result<crate::hdr_surface::HdrBuffer, String> {
    ensure_resident(session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;

    // Geometry: orient (lossless) → straighten → persistent crop, then the view
    // crop — identical to `render_view` / `encode_hdr`.
    let oriented = orient(&dev.working, view.rot90, view.flip_h, view.flip_v);
    let straightened = rotate(&oriented, view.angle);
    let base_img = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, ix, iy, iw, ih)
        }
        None => straightened,
    };
    let (ometa_w, _) = orient_dims(
        img.metadata.width as usize,
        img.metadata.height as usize,
        view.rot90,
    );
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

    // Develop params identical to `render_view` / `encode_hdr`'s construction.
    let mut ip = resolve_params(&params, &dev.thumb, effective_base(&params, dev.base));
    ip.d_max = effective_dmax(&params, dev.d_max);
    ip.channel_balance = dev.channel_balance;
    let mode = mode_from(&params.mode);
    let finish = finish_from(&params);
    let stamps = view_stamps(
        &view.dust,
        base_img.width,
        base_img.height,
        cx,
        cy,
        cw_px,
        ch_px,
        view.out_w.max(1),
        view.out_h.max(1),
    );

    let hdr = render_hdr_image(&scaled, &ip, mode, &finish, &stamps, &view.ir_removal);
    Ok(hdr_image_to_rgba16f(&hdr))
}

/// Render the developed image and return the raw linearized rgba16f HDR buffer
/// instead of a gain-map data URL — for the native EDR compositing surface,
/// which draws real super-white pixels directly rather than decoding a gain map.
/// Kept (alongside the fused `hdr_surface_render_show`) for the future
/// Windows/WebGPU path that uploads the buffer JS-side.
#[tauri::command]
pub fn encode_hdr_raw(
    id: String,
    params: InvertParams,
    view: ViewSpec,
    session: State<Session>,
) -> Result<crate::hdr_surface::HdrBuffer, String> {
    render_hdr_buffer(id, params, view, &session)
}

/// Fused render + upload for the macOS native EDR surface: render the HDR buffer
/// in Rust and hand it straight to the main-thread surface-show path, so only
/// `params`/`view`/`rect` cross IPC — never the ~tens-of-MB pixel buffer. This
/// avoids the two IPC serializations of the two-step (`encode_hdr_raw` → JS →
/// `hdr_surface_show`) approach. On non-macOS it is a no-op.
#[tauri::command]
pub fn hdr_surface_render_show(
    id: String,
    params: InvertParams,
    view: ViewSpec,
    rect: crate::hdr_surface::ViewportRect,
    session: State<Session>,
    state: State<crate::hdr_surface::HdrSurfaceState>,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let buf = render_hdr_buffer(id, params, view, &session)?;
        crate::hdr_surface::show_buffer(&window, &state, buf.rgba16f, buf.width, buf.height, rect)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, params, view, rect, &session, &state, &window);
        Ok(())
    }
}

/// The persistent per-image edits that shape a thumbnail's geometry and retouching.
/// Mirrors the relevant `ViewSpec` fields but without the zoom/view crop — a
/// thumbnail always shows the whole (cropped) frame. All fields default so the
/// grid can request a plain develop-only thumbnail with `{}`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThumbView {
    #[serde(default)]
    pub image_crop: Option<[f64; 4]>,
    #[serde(default)]
    pub rot90: u8,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub dust: Vec<DustStroke>,
    #[serde(default)]
    pub ir_removal: IrRemoval,
    /// Long-edge cap for the rendered preview. Defaults to `THUMB_EDGE` (320);
    /// the Library grid passes a larger value (e.g. 1080) when zoomed to big cells.
    #[serde(default)]
    pub edge: Option<u32>,
}

/// Render a small (~320px) inverted JPEG of the developed image at the given
/// params and persistent edits — used to live-refresh the Library grid cell and
/// filmstrip while editing. Applies orientation/straighten/crop, develop params,
/// dust strokes, and IR removal so the thumbnail matches the viewport.
///
/// `async` + `spawn_blocking` (like `render_view`): the CPU pipeline runs off the
/// Tauri runtime worker so it never blocks the UI thread. This is what lets the
/// roll contact sheet's render pool actually run frames concurrently — a sync
/// command would serialize every frame on the main thread (the ~5s adjust lag).
#[tauri::command]
pub async fn thumbnail(
    id: String,
    params: InvertParams,
    view: ThumbView,
    session: State<'_, Session>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<String, String> {
    let __log = log.inner().clone();
    ensure_resident(&session, &id)?;
    // Snapshot the owned inputs under the lock, then drop it so the render runs
    // off-thread. Cloning `working` is the cost of getting it off the UI thread
    // (mirrors render_view); the borrow-skips in the compute fn avoid a *second*
    // copy when geometry is identity (the common contact-sheet case).
    let (working, thumb, base, d_max, channel_balance) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (dev.working.clone(), dev.thumb.clone(), dev.base, dev.d_max, dev.channel_balance)
    };
    crate::time_op!(__log, "thumbnail", {
        tauri::async_runtime::spawn_blocking(move || {
            thumbnail_compute(&working, &thumb, base, d_max, channel_balance, &params, &view)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r)
    })
}

/// Pure CPU render of one thumbnail: geometry (orient → straighten → persistent
/// crop) → downscale → invert → dust/IR → finish → JPEG. Owned inputs so it runs
/// in `spawn_blocking` without borrowing the `Session`. Each geometry stage borrows
/// the previous buffer when it would be a no-op, so identity geometry (no
/// rot/flip/straighten/crop — the usual roll case) skips re-copying the whole
/// working image before the downscale.
#[allow(clippy::too_many_arguments)]
fn thumbnail_compute(
    working: &film_core::Image,
    thumb: &film_core::Image,
    base: [f32; 3],
    d_max: f32,
    channel_balance: [f32; 3],
    params: &InvertParams,
    view: &ThumbView,
) -> Result<String, String> {
    let oriented_owned;
    let oriented: &film_core::Image = if view.rot90 == 0 && !view.flip_h && !view.flip_v {
        working
    } else {
        oriented_owned = orient(working, view.rot90, view.flip_h, view.flip_v);
        &oriented_owned
    };
    let straightened_owned;
    let straightened: &film_core::Image = if view.angle.abs() < 1e-4 {
        oriented
    } else {
        straightened_owned = rotate(oriented, view.angle);
        &straightened_owned
    };
    let base_img_owned;
    let base_img: &film_core::Image = match view.image_crop {
        Some(nc) => {
            let (ix, iy, iw, ih) = crop_px(nc, straightened.width, straightened.height);
            base_img_owned = crop(straightened, ix, iy, iw, ih);
            &base_img_owned
        }
        None => straightened,
    };
    let small = proxy(base_img, view.edge.unwrap_or(THUMB_EDGE));
    let (ow, oh) = (small.width as u32, small.height as u32);
    let mut ip = resolve_params(params, thumb, effective_base(params, base));
    ip.d_max = effective_dmax(params, d_max);
    ip.channel_balance = channel_balance;
    let mut inv = invert_image_core(&small, &ip, mode_from(&params.mode));
    let stamps = view_stamps(
        &view.dust,
        base_img.width,
        base_img.height,
        0,
        0,
        base_img.width,
        base_img.height,
        ow,
        oh,
    );
    dust::apply(&mut inv, &stamps);
    if view.ir_removal.enabled {
        if let Some(ir) = small.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, view.ir_removal.sensitivity);
        }
    }
    let fin = finish_image(&inv, &finish_from(params));
    to_jpeg_b64(&fin, false, 82)
}

/// Persist an image's edited-look thumbnail (the data URL the frontend rendered via
/// `thumbnail`) to the session and catalog, so the filmstrip shows the user's edits
/// after relaunch instead of reverting to the develop-time default-params render.
#[tauri::command]
pub fn save_thumbnail(
    id: String,
    thumbnail: String,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    if let Some(img) = session.images.lock().unwrap().get_mut(&id) {
        img.thumbnail = thumbnail.clone();
    }
    catalog.update_thumbnail(&id, &thumbnail).map_err(|e| format!("{e}"))
}

/// Stamp the given images' thumbnails as stale (thumb_version below the engine
/// version) so the frontend regen worker rebakes them. Called when a look-change
/// (apply-to-roll, roll commit) touches frames the active view won't itself rebake.
#[tauri::command]
pub fn invalidate_thumbnails(
    ids: Vec<String>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.invalidate_thumbnails(&ids).map_err(|e| format!("{e}"))
}

/// Decode the full-res file, apply geometry + inversion + dust/IR + finishing, and
/// return the finished image and its source metadata. Shared by `export_image` and
/// the upscaler so both produce identical pixels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_full_res(
    id: &str,
    params: &InvertParams,
    image_crop: Option<[f64; 4]>,
    rot90: u8,
    flip_h: bool,
    flip_v: bool,
    angle: f32,
    dust: &[DustStroke],
    ir_removal: &IrRemoval,
    session: &Session,
) -> Result<(film_core::Image, crate::metadata::Metadata), String> {
    ensure_resident(session, id)?;
    let (path, base, thumb, metadata, dev_dmax, dev_balance) = {
        let images = session.images.lock().unwrap();
        let img = images.get(id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (
            img.path.clone(),
            dev.base,
            dev.thumb.clone(),
            img.metadata.clone(),
            dev.d_max,
            dev.channel_balance,
        )
    };
    let full = decode_cached(&session.decode_cache, Path::new(&path))?;
    // Match the live proxy's noise floor before inverting at full res, so export
    // reproduces the tuned preview instead of a noise-induced pink cast (the convex
    // inversion amplifies per-pixel noise into colour). See convert::match_proxy_noise.
    let full = crate::convert::match_proxy_noise(&full, PROXY_EDGE);
    let full = orient(&full, rot90, flip_h, flip_v);
    let full = rotate(&full, angle);
    let full = match image_crop {
        Some(nc) => {
            let (x, y, w, h) = crop_px(nc, full.width, full.height);
            crop(&full, x, y, w, h)
        }
        None => full,
    };
    let mut ip = resolve_params(params, &thumb, effective_base(params, base));
    ip.d_max = effective_dmax(params, dev_dmax);
    ip.channel_balance = dev_balance;
    let mut inv = invert_image_core(&full, &ip, mode_from(&params.mode));
    let stamps = export_stamps(dust, inv.width, inv.height);
    dust::apply(&mut inv, &stamps);
    if ir_removal.enabled {
        if let Some(ir) = full.ir.as_ref() {
            dust::apply_ir(&mut inv, ir, ir_removal.sensitivity);
        }
    }
    let fin = finish_image(&inv, &finish_from(params));
    Ok((fin, metadata))
}

/// Downscale a finished image so its long edge is at most `long_edge` px. Never
/// upscales; `None`/0 leaves the image untouched. Shared by the CPU, GPU, and HDR
/// export paths so the Export modal's resolution selector applies uniformly.
fn downscale_long_edge(img: film_core::Image, long_edge: Option<u32>) -> film_core::Image {
    let le = match long_edge {
        Some(le) if le > 0 => le,
        _ => return img,
    };
    let cur = img.width.max(img.height) as u32;
    if cur <= le {
        return img;
    }
    let scale = le as f64 / cur as f64;
    let w = ((img.width as f64 * scale).round() as u32).max(1);
    let h = ((img.height as f64 * scale).round() as u32).max(1);
    resize_to(&img, w, h)
}

/// Re-decode the file at full resolution and export it in the chosen format.
#[allow(clippy::too_many_arguments)] // Tauri command: flat args mirror the JS invoke contract
#[tauri::command]
pub async fn export_image(
    id: String,
    params: InvertParams,
    out_path: String,
    image_crop: Option<[f64; 4]>,
    rot90: u8,
    flip_h: bool,
    flip_v: bool,
    angle: f32,
    dust: Vec<DustStroke>,
    ir_removal: IrRemoval,
    format: ExportFormat,
    meta_override: Option<MetaOverride>,
    session: State<'_, Session>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<(), String> {
    let __log = log.inner().clone();
    crate::time_op!(__log, "export_image", {
        let (fin, metadata) = finish_full_res(
            &id, &params, image_crop, rot90, flip_h, flip_v, angle, &dust, &ir_removal, &session,
        )?;
        let fin = downscale_long_edge(fin, format.resize_long_edge);
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
    })
}

/// Export a single developed image as a gain-map HDR JPEG. Mirrors `export_image`
/// (decode full-res → orient/rotate/crop → invert+dust+IR+finish) but renders the
/// SDR base and the HDR rendition and muxes them. JPEG-only by construction; the
/// frontend only calls this when the chosen format is JPEG and the image's HDR
/// toggle is on. `format.quality` drives JPEG quality; `format.max_bytes` is not
/// applied (the gain-map encoder has no size target).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn export_image_hdr(
    id: String,
    params: InvertParams,
    out_path: String,
    image_crop: Option<[f64; 4]>,
    rot90: u8,
    flip_h: bool,
    flip_v: bool,
    angle: f32,
    dust: Vec<DustStroke>,
    ir_removal: IrRemoval,
    format: ExportFormat,
    meta_override: Option<MetaOverride>,
    session: State<'_, Session>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<(), String> {
    let __log = log.inner().clone();
    crate::time_op!(__log, "export_image_hdr", {
        ensure_resident(&session, &id)?;
        let (path, base, thumb, metadata, dev_dmax, dev_balance) = {
            let images = session.images.lock().unwrap();
            let img = images.get(&id).ok_or("unknown image id")?;
            let dev = img.developed.as_ref().ok_or("not developed")?;
            (
                img.path.clone(),
                dev.base,
                dev.thumb.clone(),
                img.metadata.clone(),
                dev.d_max,
                dev.channel_balance,
            )
        };
        let full = decode_cached(&session.decode_cache, Path::new(&path))?;
        // Match the live proxy's noise floor before inverting at full res, so export
        // reproduces the tuned preview instead of a noise-induced pink cast (the convex
        // inversion amplifies per-pixel noise into colour). See convert::match_proxy_noise.
        let full = crate::convert::match_proxy_noise(&full, PROXY_EDGE);
        let full = orient(&full, rot90, flip_h, flip_v);
        let full = rotate(&full, angle);
        let full = match image_crop {
            Some(nc) => {
                let (x, y, w, h) = crop_px(nc, full.width, full.height);
                crop(&full, x, y, w, h)
            }
            None => full,
        };
        let full = downscale_long_edge(full, format.resize_long_edge);

        let mut ip = resolve_params(&params, &thumb, effective_base(&params, base));
        ip.d_max = effective_dmax(&params, dev_dmax);
        ip.channel_balance = dev_balance;
        let stamps = export_stamps(&dust, full.width, full.height);
        let finish = finish_from(&params);

        let bytes = render_and_encode_hdr(
            &full,
            &ip,
            mode_from(&params.mode),
            &finish,
            &stamps,
            &ir_removal,
            format.quality,
        )?;

        std::fs::write(&out_path, &bytes).map_err(|e| format!("write {out_path}: {e}"))?;

        // Best-effort EXIF embed, identical policy to export_image (never fails export).
        let eff = effective_metadata(&metadata, meta_override.as_ref());
        if let Err(e) = crate::exif_write::write_exif(Path::new(&out_path), &eff) {
            eprintln!("[exif] embed failed for {out_path}: {e}");
        }
        Ok(())
    })
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
pub async fn export_begin(
    id: String,
    params: InvertParams,
    spec: BakeSpec,
    max_edge: u32,
    session: State<'_, Session>,
) -> Result<ExportPrep, String> {
    ensure_resident(&session, &id)?;
    let (path, base, dev_dmax, dev_balance) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (img.path.clone(), dev.base, dev.d_max, dev.channel_balance)
    };
    // Decode + bake (full-res) off the UI thread so batch export can overlap this
    // with other images' GPU render + encode. When the baked image exceeds the GPU
    // texture cap, skip the (large) f16 pack and stash nothing: the frontend falls
    // back to CPU export and re-decodes, so there's no buffer to leak in the map.
    let cache = session.decode_cache.clone();
    let (w, h, bytes) = tauri::async_runtime::spawn_blocking(move || {
        let full = decode_cached(&cache, Path::new(&path))?;
        // Match the live proxy's noise floor before the GPU inverts this full-res buffer,
        // so export matches the tuned preview rather than a noise-induced pink cast (the
        // convex inversion amplifies per-pixel noise into colour). See match_proxy_noise.
        let full = crate::convert::match_proxy_noise(&full, PROXY_EDGE);
        let baked = bake_working(&full, &spec); // geometry + pre-invert heal, full-res
        if baked.width.max(baked.height) as u32 > max_edge {
            return Ok::<(u32, u32, Option<Vec<u8>>), String>((
                baked.width as u32,
                baked.height as u32,
                None,
            ));
        }
        let (w, h, bytes) = pack_rgba16f(&baked, u32::MAX); // no cap for export
        Ok((w, h, Some(bytes)))
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut uniforms = resolve_to_uniforms(&params, effective_base(&params, base));
    uniforms.d_max = effective_dmax(&params, dev_dmax);
    uniforms.channel_balance = dev_balance;
    if let Some(bytes) = bytes {
        session
            .pending_export
            .lock()
            .unwrap()
            .insert(id, PreparedExport { w, h, bytes });
    }
    Ok(ExportPrep { w, h, uniforms })
}

/// Return the stashed half-float bytes for upload (consumes nothing; kept until export_finish).
#[tauri::command]
pub fn export_pixels(id: String, session: State<Session>) -> Result<tauri::ipc::Response, String> {
    // Move the stashed buffer out of the map (frees it here, not at export_finish).
    let prep = session
        .pending_export
        .lock()
        .unwrap()
        .remove(&id)
        .ok_or("no prepared export")?;
    Ok(tauri::ipc::Response::new(prep.bytes))
}

/// `bit16 = true` → `data` is f32 RGBA (4 floats/px); else RGBA8 (4 bytes/px).
#[derive(Debug, Clone, Deserialize)]
pub struct ExportReadback {
    pub w: u32,
    pub h: u32,
    pub bit16: bool,
}

/// Metadata for `export_finish`, carried in the base64 `x-meta` header so the
/// (potentially hundreds-of-MB) pixel buffer can cross the IPC as a raw byte
/// body instead of a JSON number array. Mirrors the JS object.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFinishMeta {
    pub id: String,
    pub out_path: String,
    pub readback: ExportReadback,
    pub format: ExportFormat,
    #[serde(default)]
    pub meta_override: Option<MetaOverride>,
}

/// Build an Image from the GPU readback and encode it with the chosen format.
///
/// The pixel readback arrives as the **raw request body** (not a JSON array): a
/// 16-bit export is f32 RGBA = 16 bytes/px, so a 24 MP frame is ~384 MB — sending
/// that as a `number[]` JSON-serialized the whole buffer (multi-GB transient,
/// thrashing the batch). Metadata rides in the base64 `x-meta` header instead.
#[tauri::command]
pub async fn export_finish(
    request: tauri::ipc::Request<'_>,
    session: State<'_, Session>,
) -> Result<(), String> {
    // Pull the raw pixel body + decode the header metadata, then drop the borrowed
    // request before the await so the encode future stays Send.
    let (data, id, out_path, readback, format, meta_override) = {
        let data = match request.body() {
            tauri::ipc::InvokeBody::Raw(bytes) => bytes.clone(),
            _ => return Err("export_finish expects a raw byte body".into()),
        };
        let b64 = request
            .headers()
            .get("x-meta")
            .ok_or("export_finish missing x-meta header")?
            .to_str()
            .map_err(|e| e.to_string())?;
        let json = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("x-meta base64: {e}"))?;
        let m: ExportFinishMeta = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
        (data, m.id, m.out_path, m.readback, m.format, m.meta_override)
    };
    drop(request);

    // Snapshot the metadata before the lock-free encode so batch export can overlap
    // this encode/write with other images' decode/bake. (The stashed input buffer
    // was already freed by export_pixels.)
    let metadata = {
        let images = session.images.lock().unwrap();
        images.get(&id).map(|i| i.metadata.clone())
    };
    tauri::async_runtime::spawn_blocking(move || {
        let n = (readback.w as usize) * (readback.h as usize);
        let img = if readback.bit16 {
            // data is little-endian f32 RGBA (16 B/px). Reconstruct the Image in ONE
            // parallel pass straight from the bytes — no intermediate Vec<f32> collect.
            if data.len() < n * 16 {
                return Err(format!(
                    "export_finish: truncated f32 readback ({} < {} bytes)",
                    data.len(),
                    n * 16
                ));
            }
            image_from_rgba_f32_le(readback.w, readback.h, &data)
        } else {
            if data.len() < n * 4 {
                return Err(format!(
                    "export_finish: truncated rgba8 readback ({} < {} bytes)",
                    data.len(),
                    n * 4
                ));
            }
            image_from_rgba8(readback.w, readback.h, &data)
        };
        let img = downscale_long_edge(img, format.resize_long_edge);
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
        if let Some(md) = metadata {
            let eff = effective_metadata(&md, meta_override.as_ref());
            if let Err(e) = crate::exif_write::write_exif(out, &eff) {
                eprintln!("[exif] embed failed for {out_path}: {e}");
            }
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// For each output path, whether a file already exists there. The export dialog
/// calls this up front to warn before any image overwrites an existing file.
#[tauri::command]
pub fn paths_exist(paths: Vec<String>) -> Vec<bool> {
    paths.iter().map(|p| Path::new(p).exists()).collect()
}

/// Return an output path that does not collide with an existing file: the input
/// unchanged if it is free, otherwise the first free `"<stem> (n).<ext>"` (n = 1, 2, …).
/// Backs the export dialog's "Keep both" choice. Falls back to the original path if no
/// free name is found within a sane bound.
#[tauri::command]
pub fn unique_path(path: String) -> String {
    let p = Path::new(&path);
    if !p.exists() {
        return path;
    }
    let parent = p.parent();
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = p.extension().and_then(|s| s.to_str());
    for n in 1..=9999u32 {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let cand = match parent {
            Some(par) => par.join(name),
            None => std::path::PathBuf::from(name),
        };
        if !cand.exists() {
            return cand.to_string_lossy().into_owned();
        }
    }
    path
}

/// Estimated as-shot white point for the developed image, as (Kelvin, tint).
/// The UI seeds the Temp/Tint sliders with this when an image becomes active.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AsShotWb {
    pub temp: f32,
    pub tint: f32,
    /// Baseline gains the frontend stores in `wb_baseline`; equals the WB today's
    /// render applies for (temp,tint), so storing it + neutral sliders is identical.
    pub gains: [f32; 3],
}

#[tauri::command]
pub fn as_shot_wb(
    id: String,
    params: InvertParams,
    crop: Option<[f64; 4]>,
    rot90: Option<u8>,
    flip_h: Option<bool>,
    flip_v: Option<bool>,
    angle: Option<f32>,
    session: State<Session>,
) -> Result<AsShotWb, String> {
    ensure_resident(&session, &id)?;
    let (base, thumb, dev_dmax) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (dev.base, dev.thumb.clone(), dev.d_max)
    };
    // Restrict the estimate to the image area so borders/rebate don't bias WB. The
    // crop is in oriented space, so orient/straighten the thumb first to match the
    // render pipeline (otherwise flipped frames estimate WB from the wrong region).
    let thumb = match crop {
        Some(nc) => {
            let geom = geom_base(
                &thumb,
                rot90.unwrap_or(0),
                flip_h.unwrap_or(false),
                flip_v.unwrap_or(false),
                angle.unwrap_or(0.0),
            );
            let (x, y, w, h) = crop_px(nc, geom.width, geom.height);
            crate::convert::crop(&geom, x, y, w, h)
        }
        None => thumb,
    };
    // Estimate WB against the user's ACTUAL stock/mode so the gains neutralise the
    // colour space the image is actually rendered in. `build_params` leaves `wb` at
    // [1,1,1], so the estimate is independent of any temp/tint already on the sliders.
    // Route through meter_mask so spoke/border regions don't bias the gray-world.
    let eff_base = effective_base(&params, base);
    let (temp, tint) = match meter_mask(&thumb, eff_base, params.positive, &params.meter_border) {
        Some((small, keep)) => auto_seed_wb(&small, &params, base, dev_dmax, Some(&keep)),
        None => auto_seed_wb(&thumb, &params, base, dev_dmax, None),
    };
    Ok(AsShotWb { temp, tint, gains: as_shot_gains(temp, tint) })
}

/// Per-image auto white balance as `(Kelvin, UI-tint −150..150)` — the gray-world
/// estimate `as_shot_wb` returns, factored out so the develop-time grid thumbnail
/// and the on-load regeneration bake the SAME WB the frontend seed applies when
/// you open Develop. `build_params` leaves `wb` at [1,1,1], so the estimate is
/// independent of any temp/tint already on `params`.
pub(crate) fn auto_seed_wb(
    src: &film_core::Image,
    params: &InvertParams,
    base: [f32; 3],
    dev_dmax: f32,
    mask: Option<&[bool]>,
) -> (f32, f32) {
    let mut ip = build_params(params, effective_base(params, base));
    ip.d_max = effective_dmax(params, dev_dmax);
    let first = invert_image(src, &ip, mode_from(&params.mode));
    let gains = auto_wb_gains_masked(&first, mask);
    let (temp, tint) = gains_to_cct(gains);
    (temp, tint * 150.0) // tint back to UI −150..150
}

/// Per-zone WB seed: run `film_core::calibrate::per_zone_wb_gains` on `src`
/// (a developed positive) to estimate residual per-zone gray-world gains.
/// Factored out like `auto_seed_wb` so it can be tested independently.
pub(crate) fn per_zone_seed(src: &film_core::Image, strength: f32, mask: Option<&[bool]>) -> [[f32; 3]; 3] {
    film_core::calibrate::per_zone_wb_gains(src, strength, mask)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PerZoneWbResult {
    pub sh: [f32; 3],
    pub mid: [f32; 3],
    pub hi: [f32; 3],
}

/// Estimate residual per-zone white balance on the developed image.
/// Inverts the thumb with the CURRENT resolved WB so the gains measure the
/// RESIDUAL cast on top of global WB. Returns shadow/mid/highlight zone gains
/// the frontend stores into `pz_sh`/`pz_mid`/`pz_hi`.
#[tauri::command]
pub fn per_zone_wb(
    id: String,
    params: InvertParams,
    crop: Option<[f64; 4]>,
    rot90: Option<u8>,
    flip_h: Option<bool>,
    flip_v: Option<bool>,
    angle: Option<f32>,
    session: State<Session>,
) -> Result<PerZoneWbResult, String> {
    ensure_resident(&session, &id)?;
    let (base, thumb, dev_dmax) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (dev.base, dev.thumb.clone(), dev.d_max)
    };
    // Orient/crop exactly like as_shot_wb so the estimate matches the rendered region.
    let thumb = match crop {
        Some(nc) => {
            let geom = geom_base(
                &thumb,
                rot90.unwrap_or(0),
                flip_h.unwrap_or(false),
                flip_v.unwrap_or(false),
                angle.unwrap_or(0.0),
            );
            let (x, y, w, h) = crop_px(nc, geom.width, geom.height);
            crate::convert::crop(&geom, x, y, w, h)
        }
        None => thumb,
    };
    // Invert with the CURRENT resolved WB so per-zone measures the RESIDUAL cast.
    // Route through meter_mask so spoke/border regions don't bias the per-zone estimate.
    let eff_base = effective_base(&params, base);
    let z = match meter_mask(&thumb, eff_base, params.positive, &params.meter_border) {
        Some((small, keep)) => {
            let mut ip = resolve_params(&params, &small, eff_base);
            ip.d_max = effective_dmax(&params, dev_dmax);
            let positive = invert_image(&small, &ip, mode_from(&params.mode));
            per_zone_seed(&positive, params.pz_strength, Some(&keep))
        }
        None => {
            let mut ip = resolve_params(&params, &thumb, eff_base);
            ip.d_max = effective_dmax(&params, dev_dmax);
            let positive = invert_image(&thumb, &ip, mode_from(&params.mode));
            per_zone_seed(&positive, params.pz_strength, None)
        }
    };
    Ok(PerZoneWbResult { sh: z[0], mid: z[1], hi: z[2] })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoBrightness {
    /// Solved exposure in EV stops. Drives the highlight-preserving filmic exposure
    /// (NOT the linear `brightness` gain), so it brightens via the display curve's
    /// shoulder without clipping — a nondestructive, curve-based lift.
    pub exposure: f32,
}

/// Build the spoke-exclusion keep-mask for a cropped analysis thumb, honoring the
/// `meter_border` mode + confidence gate. Returns the downscaled measurement image
/// plus an aligned keep-mask when masking applies, or `None` to meter `cropped`
/// directly (today's behavior). `base` is ignored on the positive path.
fn meter_mask(
    cropped: &film_core::Image,
    base: [f32; 3],
    positive: bool,
    mode: &str,
) -> Option<(film_core::Image, Vec<bool>)> {
    use film_core::calibrate::{detect_photo_mask, gate_photo_mask, MeterBorder};
    let mode = MeterBorder::from_str_lenient(mode);
    if mode == MeterBorder::Include {
        return None;
    }
    let pm = detect_photo_mask(cropped, base, positive);
    // detect_photo_mask downscales internally to SAMPLE_CAP; reproduce that grid so
    // the returned mask aligns to the image we hand the sampler.
    // Compute detect_grid lazily inside the map so we skip the downscale when the gate rejects.
    gate_photo_mask(pm, mode).map(|keep| (film_core::calibrate::detect_grid(cropped), keep))
}

/// Auto-brightness for one image: solve the EXPOSURE (EV) that lands the image's
/// bright-content luminance on a target, measured on the FINISHED display positive.
/// Per-image (each frame measures its own), crop/orient-aware like `as_shot_wb`.
#[tauri::command]
pub fn auto_brightness(
    id: String,
    params: InvertParams,
    crop: Option<[f64; 4]>,
    rot90: Option<u8>,
    flip_h: Option<bool>,
    flip_v: Option<bool>,
    angle: Option<f32>,
    session: State<Session>,
) -> Result<AutoBrightness, String> {
    ensure_resident(&session, &id)?;
    let (base, thumb, dev_dmax) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        (dev.base, dev.thumb.clone(), dev.d_max)
    };
    // Measure only the visible image area so rebate/borders don't bias the estimate
    // (the crop is in oriented space — orient/straighten first, as `as_shot_wb` does).
    let thumb = match crop {
        Some(nc) => {
            let geom = geom_base(
                &thumb,
                rot90.unwrap_or(0),
                flip_h.unwrap_or(false),
                flip_v.unwrap_or(false),
                angle.unwrap_or(0.0),
            );
            let (x, y, w, h) = crop_px(nc, geom.width, geom.height);
            crate::convert::crop(&geom, x, y, w, h)
        }
        None => thumb,
    };
    let eff_base = effective_base(&params, base);
    let exposure = match meter_mask(&thumb, eff_base, params.positive, &params.meter_border) {
        Some((small, keep)) => auto_brightness_value(&small, &params, eff_base, dev_dmax, Some(&keep)),
        None => auto_brightness_value(&thumb, &params, eff_base, dev_dmax, None),
    };
    Ok(AutoBrightness { exposure })
}

/// Solve the exposure (EV) that maps the `AUTO_PCT`-th luminance percentile of the
/// FINISHED display positive (the user's full look — contrast, tone curve, brightness)
/// to `AUTO_TARGET`, so the auto value suits the final image. The metric is monotonic
/// non-decreasing in EV and saturates at 1.0 once a strong Contrast clamps the brights,
/// so we BISECT for the target rather than fit a secant: the secant's derivative step
/// stalls in that flat clamped region and froze at its seed (a fixed ~-0.32 EV) when
/// contrast was high. A few cheap invert+finish passes on the small proxy.
pub(crate) fn auto_brightness_value(
    src: &film_core::Image,
    params: &InvertParams,
    base: [f32; 3],
    dev_dmax: f32,
    mask: Option<&[bool]>,
) -> f32 {
    const AUTO_TARGET: f32 = 0.80; // display [0,1] anchor for bright content ("balanced")
    const AUTO_PCT: f32 = 0.90; // 90th-pct luminance — below speculars / rebate bleed
    const TOL: f32 = 0.01;
    const EV_CLAMP: f32 = 3.0;

    // Finished-positive luminance percentile at a candidate exposure (the FULL look,
    // so the target reflects what the user actually sees).
    let measure = |ev: f32| -> f32 {
        let mut p = params.clone();
        p.exposure = ev;
        let mut ip = resolve_params(&p, src, base); // honors temp/tint WB; print_exposure = 2^ev
        ip.d_max = effective_dmax(params, dev_dmax);
        let inv = invert_image_core(src, &ip, mode_from(&params.mode));
        let pos = finish_image(&inv, &finish_from(params)); // current contrast/curve/brightness
        percentile_luma(&pos, AUTO_PCT, mask)
    };

    // Bracket the achievable range at the EV clamp. `measure` is monotonic in EV, so
    // these bound the percentile; the target lives between them in the normal case.
    let y_lo = measure(-EV_CLAMP); // darkest
    if y_lo >= AUTO_TARGET {
        return -EV_CLAMP; // even the darkest allowed exposure is already too bright
    }
    let y_hi = measure(EV_CLAMP); // brightest
    if y_hi < 1e-4 {
        return 0.0; // degenerate / near-black frame — don't crank exposure on noise
    }
    if y_hi <= AUTO_TARGET {
        return EV_CLAMP; // can't reach the target even at the brightest allowed exposure
    }

    // Bisection: the first midpoint is EV 0, so an already-balanced frame returns ~0.
    // Monotonic metric ⇒ the target stays bracketed by [lo, hi] throughout. ~14 steps
    // resolve well below the 0.01-EV slider step over the ±3 EV range.
    let mut lo = -EV_CLAMP;
    let mut hi = EV_CLAMP;
    let mut e = 0.0f32;
    for _ in 0..14 {
        e = 0.5 * (lo + hi);
        let y = measure(e);
        if (y - AUTO_TARGET).abs() <= TOL {
            break;
        }
        if y < AUTO_TARGET {
            lo = e;
        } else {
            hi = e;
        }
    }
    (e * 100.0).round() / 100.0 // exposure slider step is 0.01
}

/// The `pct` (0..1) percentile of BT.709 luminance over an image's pixels.
/// If `mask` is supplied, only pixels whose index has `true` in the mask are counted.
fn percentile_luma(img: &film_core::Image, pct: f32, mask: Option<&[bool]>) -> f32 {
    let mut ys: Vec<f32> = img
        .pixels
        .iter()
        .enumerate()
        .filter(|(i, _)| mask.map_or(true, |m| m.get(*i).copied().unwrap_or(true)))
        .map(|(_, p)| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
        .collect();
    if ys.is_empty() {
        return 0.0;
    }
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (((ys.len() - 1) as f32) * pct.clamp(0.0, 1.0)).round() as usize;
    ys[idx.min(ys.len() - 1)]
}

/// Render the catalog/grid display thumbnail (JPEG b64) from a linear proxy.
/// `render_src` is the higher-res proxy the JPEG is drawn from; `seed_src` is the
/// (typically smaller) proxy the auto-WB estimate runs on — matching `as_shot_wb`
/// so a freshly baked thumbnail does not "snap" when Develop is first opened.
///
/// With `saved` edits present, those params drive the render (the user's look).
/// Without them (never opened), the per-image auto-WB seed is baked in so the
/// thumbnail still matches what opening Develop would show. Shared by
/// `develop_compute` (fresh develop) and the on-load thumbnail regeneration.
#[allow(clippy::too_many_arguments)]
fn render_grid_thumbnail(
    render_src: &film_core::Image,
    seed_src: &film_core::Image,
    base: [f32; 3],
    d_max: f32,
    channel_balance: [f32; 3],
    saved: Option<&InvertParams>,
    positive: bool,
) -> Result<String, String> {
    let params = match saved {
        Some(p) => p.clone(),
        None => {
            let mut p = InvertParams { positive, ..default_invert_params() };
            let (temp, tint) = auto_seed_wb(seed_src, &p, base, d_max, None);
            p.temp = temp;
            p.tint = tint;
            // Bake per-zone WB the same way Develop seeds it, so the thumbnail matches.
            let mut ip_seed = resolve_params(&p, seed_src, effective_base(&p, base));
            ip_seed.d_max = effective_dmax(&p, d_max);
            let positive_seed = invert_image(seed_src, &ip_seed, mode_from(&p.mode));
            let z = per_zone_seed(&positive_seed, p.pz_strength, None);
            p.pz_sh = z[0]; p.pz_mid = z[1]; p.pz_hi = z[2];
            p
        }
    };
    let mut ip = resolve_params(&params, seed_src, base);
    ip.d_max = effective_dmax(&params, d_max);
    ip.channel_balance = channel_balance;
    let inv = invert_image_core(render_src, &ip, mode_from(&params.mode));
    let inv = finish_image(&inv, &finish_from(&params));
    to_jpeg_b64(&inv, false, 82)
}

/// (Kelvin, gains_to_cct tint) that makes a sampled display pixel `rgb` render neutral.
/// `rgb` is the displayed positive at the clicked point, encoded by the active mode's
/// output power. We undo that encode and divide out the current WB to recover the
/// WB-neutral linear inverted value `P` at the point, then take gray-world gains of `P`
/// — the same convention as `as_shot_wb`, so a gray-point pick is consistent with Auto.
/// Unlike Auto it is UNDAMPED: a deliberate gray click should land the point exactly
/// neutral. The result is absolute — the current Temp/Tint cancels (it is baked into the
/// sampled pixel and divided back out), so clicking always means "make this point gray".
fn gray_point_temp_tint(params: &InvertParams, rgb: [f32; 3]) -> (f32, f32) {
    let ip = build_params(params, [1.0, 1.0, 1.0]);
    let wb_old = wb_from_params(params.temp, params.tint);
    // Recover the WB-neutral positive `P` at the clicked pixel. Mode D's filmic curve
    // applies WB as a plain gain on its OUTPUT (no trailing power): displayed = P·wb
    // ⇒ P = d / wb_old. Legacy modes still encode `(P·wb)^gamma` (WB inside the
    // power) ⇒ P = d^(1/γ) / wb_old. (paper_grade is inert under the filmic engine.)
    let p: [f32; 3] = if params.mode == "d" {
        std::array::from_fn(|c| rgb[c].max(1e-5) / wb_old[c].max(1e-5))
    } else {
        let e = ip.gamma.max(1e-3);
        std::array::from_fn(|c| rgb[c].max(1e-5).powf(1.0 / e) / wb_old[c].max(1e-5))
    };
    let gray = (p[0] + p[1] + p[2]) / 3.0;
    let gains = [
        gray / p[0].max(1e-5),
        gray / p[1].max(1e-5),
        gray / p[2].max(1e-5),
    ];
    gains_to_cct(gains)
}

/// Temp/Tint that neutralises a clicked gray point. `rgb` is the displayed positive
/// sampled at the click. See [`gray_point_temp_tint`].
#[tauri::command]
pub fn gray_point_wb(params: InvertParams, rgb: [f32; 3]) -> AsShotWb {
    // `gray_point_temp_tint` already divides the current WB back out, so (gtemp, gtint)
    // is the ABSOLUTE white point that makes the clicked pixel gray — not a correction
    // relative to where the sliders sit. Return it directly (matching `as_shot_wb`).
    //
    // Two earlier bugs made the picker creep instead of converge: `gtint` is in the
    // −1..1 convention but was fed to `wb_from_params` (which divides tint by 150 again),
    // so the tint correction landed at ~1/150 strength; and the current slider was
    // multiplied back in, double-counting it. Both are gone — `wb_from_params(_, ·150)`
    // round-trips `gtint` cleanly, and there is no slider multiply, so re-picking the
    // same point is idempotent.
    let (gtemp, gtint) = gray_point_temp_tint(&params, rgb); // gtint in −1..1
    let gains = wb_from_params(gtemp, gtint * 150.0);
    AsShotWb {
        temp: gtemp,
        tint: gtint * 150.0,
        gains,
    }
}

/// Load the whole catalog at launch: return the snapshot to the frontend AND
/// repopulate the in-memory Session with lightweight (undeveloped) records so
/// `develop_image`/`render_view` can find each image by id.
#[tauri::command]
pub fn load_catalog(
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<crate::catalog::CatalogSnapshot, String> {
    let __log = log.inner().clone();
    crate::time_op!(__log, "load_catalog", {
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
                    last_access: 0,
                },
            );
        }
        Ok(snap)
    })
}

#[tauri::command]
pub fn save_edits(
    id: String,
    params_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog
        .save_params(&id, &params_json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_crop(
    id: String,
    crop_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog
        .save_crop(&id, &crop_json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_dust(
    id: String,
    dust_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog
        .save_dust(&id, &dust_json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_meta(
    id: String,
    meta_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog
        .save_meta(&id, &meta_json)
        .map_err(|e| e.to_string())
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
    catalog
        .save_app_state(&key, &value)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkingInfo {
    /// Capped dimensions of the float texture working_pixels will return.
    pub w: u32,
    pub h: u32,
}

/// Dimensions of the GPU float texture for this image (after the MAX_GPU_EDGE cap).
#[tauri::command]
pub async fn working_info(
    id: String,
    hires: bool,
    session: State<'_, Session>,
) -> Result<WorkingInfo, String> {
    ensure_resident(&session, &id)?;
    if hires {
        ensure_zoom_src(&session, &id).await?;
        let g = session.zoom_src.lock().unwrap();
        return match g.as_ref() {
            Some((cid, img)) if cid == &id => {
                let (w, h) = capped_dims(img, MAX_GPU_EDGE);
                Ok(WorkingInfo { w, h })
            }
            _ => Err("no zoom source".into()),
        };
    }
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (w, h) = capped_dims(&dev.working, MAX_GPU_EDGE);
    Ok(WorkingInfo { w, h })
}

/// Raw half-float RGBA bytes of the linear working image (pre-inversion), for a
/// one-shot WebGL2 RGBA16F upload. Returned as raw IPC bytes (no base64/JPEG).
#[tauri::command]
pub async fn working_pixels(
    id: String,
    hires: bool,
    session: State<'_, Session>,
) -> Result<tauri::ipc::Response, String> {
    ensure_resident(&session, &id)?;
    // hires (deep zoom): pack the high-res decode; else the resident proxy.
    let working = if hires {
        zoom_src_clone(&session, &id).await?
    } else {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        img.developed.as_ref().ok_or("not developed")?.working.clone()
    };
    // pack_rgba16f is O(pixels); offload so a full-res working buffer doesn't stall
    // the UI on image switch.
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let (_, _, bytes) = pack_rgba16f(&working, MAX_GPU_EDGE);
        bytes
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Capped dims of the BAKED (geometry + heal) working texture.
#[tauri::command]
pub async fn working_baked_info(
    id: String,
    spec: BakeSpec,
    hires: bool,
    session: State<'_, Session>,
) -> Result<WorkingInfo, String> {
    ensure_resident(&session, &id)?;
    let working = if hires {
        zoom_src_clone(&session, &id).await?
    } else {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        img.developed
            .as_ref()
            .ok_or("not developed")?
            .working
            .clone()
    };
    let geom = bake_geometry(&working, &spec); // geometry only — exact dims, no Telea heal
    let (w, h) = capped_dims(&geom, MAX_GPU_EDGE);
    Ok(WorkingInfo { w, h })
}

/// Full-frame defect `Mask` (x0=y0=0) covering the brush `stamps` — used to feed
/// the MI-GAN inpainter, which expects a whole-frame mask.
fn full_mask_from_stamps(w: usize, h: usize, stamps: &[Stamp]) -> film_core::dust::Mask {
    let m = film_core::dust::rasterize(w, h, stamps, film_core::dust::GROW, 0);
    if m.w == 0 || m.h == 0 {
        return film_core::dust::Mask { x0: 0, y0: 0, w: 0, h: 0, bits: Vec::new() };
    }
    let mut bits = vec![false; w * h];
    for yy in 0..m.h {
        for xx in 0..m.w {
            if m.bits[yy * m.w + xx] {
                bits[(m.y0 + yy) * w + (m.x0 + xx)] = true;
            }
        }
    }
    film_core::dust::Mask { x0: 0, y0: 0, w, h, bits }
}

/// Build the auto-dust defect mask for a baked NEGATIVE working image: invert to
/// a positive, run the detector (reusing `cached` prob if its dims match, else
/// run once), and threshold at `sensitivity`. Returns the whole-frame mask plus
/// the prob map to cache when freshly computed. Detector failure → empty mask.
fn auto_dust_mask(
    app_data: &Path,
    baked: &film_core::Image,
    ip: &InversionParams,
    mode: Mode,
    sensitivity: f32,
    exclusions: &[[f64; 2]],
    cached: Option<(usize, usize, Vec<f32>)>,
) -> (film_core::dust::Mask, Option<(usize, usize, Vec<f32>)>) {
    let (w, h) = (baked.width, baked.height);
    let empty = film_core::dust::Mask { x0: 0, y0: 0, w: 0, h: 0, bits: Vec::new() };
    let positive = invert_image(baked, ip, mode);
    let (prob, fresh) = match cached {
        Some((cw, ch, p)) if (cw, ch) == (w, h) && p.len() == w * h => (p, None),
        _ => match crate::autodust::engine::detect(app_data, &positive) {
            Ok(p) => (p.clone(), Some((w, h, p))),
            Err(_) => return (empty, None),
        },
    };
    let max_blob = (crate::autodust::MAX_BLOB * w.max(h) / 2000).max(1);
    let mut mask = film_core::dust::prob_defect_mask(w, h, &prob, sensitivity, max_blob);
    drop_excluded(&mut mask, exclusions);
    (mask, fresh)
}

/// Connected components (4-neighbour) of the set pixels in a whole-frame mask,
/// returning each blob's centroid normalized to [0,1] (x by width, y by height).
/// These are the distinct dust/defect spots surfaced to the UI as heal markers.
fn blob_centroids(mask: &film_core::dust::Mask) -> Vec<[f64; 2]> {
    let (w, h) = (mask.w, mask.h);
    let mut out = Vec::new();
    if w == 0 || h == 0 {
        return out;
    }
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..w * h {
        if !mask.bits[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut sx, mut sy, mut n) = (0usize, 0usize, 0usize);
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            sx += x;
            sy += y;
            n += 1;
            let push = |q: usize, seen: &mut Vec<bool>, stack: &mut Vec<usize>| {
                if mask.bits[q] && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            };
            if x > 0 { push(p - 1, &mut seen, &mut stack); }
            if x + 1 < w { push(p + 1, &mut seen, &mut stack); }
            if y > 0 { push(p - w, &mut seen, &mut stack); }
            if y + 1 < h { push(p + w, &mut seen, &mut stack); }
        }
        if n > 0 {
            out.push([(sx as f64 / n as f64) / w as f64, (sy as f64 / n as f64) / h as f64]);
        }
    }
    out
}

/// Remove from `mask` any connected blob touched by an excluded seed point
/// (normalized [0,1]) — i.e. global dust the user chose to KEEP. A small search
/// window tolerates sub-pixel drift between the stored centroid and the
/// re-thresholded mask. Searches nearest pixels first to avoid latching onto a
/// distant blob. Mutates the mask in place.
fn drop_excluded(mask: &mut film_core::dust::Mask, exclusions: &[[f64; 2]]) {
    let (w, h) = (mask.w, mask.h);
    if w == 0 || h == 0 || exclusions.is_empty() {
        return;
    }
    const WIN: i32 = 6; // px radius to snap a seed onto its blob
    for ex in exclusions {
        let cx = ((ex[0] * w as f64).round() as i32).clamp(0, w as i32 - 1);
        let cy = ((ex[1] * h as f64).round() as i32).clamp(0, h as i32 - 1);
        let mut seed: Option<usize> = None;
        // Search center-first (by Chebyshev distance) so we latch onto the
        // closest blob, not the topmost one in scan order.
        'find: for r in 0..=WIN {
            for dy in -r..=r {
                for dx in -r..=r {
                    // Only visit the ring at this radius (Chebyshev distance == r).
                    if dy.abs() != r && dx.abs() != r {
                        continue;
                    }
                    let x = cx + dx;
                    let y = cy + dy;
                    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                        continue;
                    }
                    let i = y as usize * w + x as usize;
                    if mask.bits[i] {
                        seed = Some(i);
                        break 'find;
                    }
                }
            }
        }
        let Some(s) = seed else { continue };
        let mut stack = vec![s];
        mask.bits[s] = false;
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            let clear = |q: usize, bits: &mut Vec<bool>, stack: &mut Vec<usize>| {
                if bits[q] {
                    bits[q] = false;
                    stack.push(q);
                }
            };
            if x > 0 { clear(p - 1, &mut mask.bits, &mut stack); }
            if x + 1 < w { clear(p + 1, &mut mask.bits, &mut stack); }
            if y > 0 { clear(p - w, &mut mask.bits, &mut stack); }
            if y + 1 < h { clear(p + w, &mut mask.bits, &mut stack); }
        }
    }
}

/// MI-GAN-heal ONLY the auto-dust defect mask onto a geometry-baked buffer (Stage A).
/// Returns the healed buffer; the caller then heals brush strokes on top (Stage B).
/// No-op when the mask is empty or the model isn't installed.
fn autodust_heal(
    app_data: &Path,
    mut img: film_core::Image,
    mask: &film_core::dust::Mask,
    base: [f32; 3],
) -> film_core::Image {
    if mask.bits.iter().any(|&b| b) && crate::autodust::assets::installed(app_data) {
        let _ = crate::autodust::engine::inpaint(app_data, &mut img, mask, base);
    }
    img
}

/// Cache signature for the Stage-A auto-dust-healed buffer: sensitivity + the set of
/// kept-dust exclusions. Geometry is NOT included — a geometry change recomputes the
/// detector prob map (returns `fresh`), which the caller uses to drop this entry.
fn autodust_heal_key(sensitivity: f32, exclusions: &[[f64; 2]]) -> String {
    let mut s = format!("{:.2}", sensitivity);
    for e in exclusions {
        s.push_str(&format!("|{:.4},{:.4}", e[0], e[1]));
    }
    s
}

/// Heal an already-(geometry + auto-dust)-baked working buffer: brush dust strokes per
/// the spec's mode (classic Telea, MI-GAN, or skipped for the AI-mask overlay), then
/// IR. Global auto-dust is healed separately in Stage A (`autodust_heal`), so this no
/// longer touches the auto-dust mask — a fine-tune stroke only inpaints its own region.
fn bake_for_view_from_baked(
    app_data: &Path,
    mut img: film_core::Image,
    spec: &BakeSpec,
    base: [f32; 3],
) -> film_core::Image {
    let stamps = export_stamps(&spec.dust, img.width, img.height);
    let want_migan = spec.migan && crate::autodust::assets::installed(app_data);
    if want_migan {
        if !spec.skip_dust_heal {
            let mask = full_mask_from_stamps(img.width, img.height, &stamps);
            if mask.bits.iter().any(|&b| b) {
                let _ = crate::autodust::engine::inpaint(app_data, &mut img, &mask, base);
            }
        }
    } else if !spec.skip_dust_heal {
        film_core::dust::apply(&mut img, &stamps);
    }
    if spec.ir_removal.enabled {
        if let Some(ir) = img.ir.clone() {
            film_core::dust::apply_ir(&mut img, &ir, spec.ir_removal.sensitivity);
        }
    }
    img
}

/// Half-float RGBA bytes of the BAKED working buffer (geometry applied, dust/IR
/// healed pre-invert), for a one-shot RGBA16F upload. GPU then inverts with
/// IDENTITY geometry.
#[tauri::command]
pub async fn working_baked_pixels(
    app: tauri::AppHandle,
    id: String,
    params: InvertParams,
    spec: BakeSpec,
    hires: bool,
    session: State<'_, Session>,
) -> Result<tauri::ipc::Response, String> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    ensure_resident(&session, &id)?;
    // Inversion params/base come from the resident developed image; the pixel SOURCE
    // is the resident proxy (fit) or the high-res decode (deep zoom).
    let (ip, mode, base) = {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        let dev = img.developed.as_ref().ok_or("not developed")?;
        // The SAME base the inversion divides by — neutralizing it before MI-GAN
        // keeps the heal's per-channel error balanced (no colored halo on sky).
        let base = effective_base(&params, dev.base);
        let mut ip = resolve_params(&params, &dev.thumb, base);
        ip.d_max = effective_dmax(&params, dev.d_max);
        (ip, mode_from(&params.mode), base)
    };
    let working = if hires {
        zoom_src_clone(&session, &id).await?
    } else {
        let images = session.images.lock().unwrap();
        let img = images.get(&id).ok_or("unknown image id")?;
        img.developed.as_ref().ok_or("not developed")?.working.clone()
    };
    let cached_prob = if spec.auto_dust.enabled {
        session.autodust_prob.lock().unwrap().get(&id).cloned()
    } else {
        None
    };
    // Reuse the auto-dust-healed buffer only on the proxy tier (deep-zoom recomputes).
    let want_cache = spec.auto_dust.enabled && !hires;
    let cached_healed = if want_cache {
        session.autodust_healed.lock().unwrap().get(&id).cloned()
    } else {
        None
    };
    let do_auto = spec.auto_dust.enabled;
    let sens = spec.auto_dust.sensitivity;
    let exclusions = spec.auto_dust_exclusions.clone();
    // The heal can run the detector + MI-GAN (seconds) — keep it off the main thread so
    // the UI stays responsive. Returns any freshly computed prob map + a freshly healed
    // buffer to cache, and the active spot centroids to surface as UI markers.
    let (bytes, fresh, store_healed, spots) = tauri::async_runtime::spawn_blocking(move || {
        let baked = bake_geometry(&working, &spec);
        let mut fresh_prob = None;
        let mut store_healed: Option<(String, film_core::Image)> = None;
        let mut spots: Option<Vec<[f64; 2]>> = None;
        // Stage A: auto-dust heal (cached by sensitivity + exclusions).
        let stage_a = if do_auto && crate::autodust::assets::installed(&app_data) {
            let (mask, fr) = auto_dust_mask(&app_data, &baked, &ip, mode, sens, &exclusions, cached_prob);
            fresh_prob = fr;
            spots = Some(blob_centroids(&mask));
            let key = autodust_heal_key(sens, &exclusions);
            // A fresh prob means the detector re-ran (geometry/content changed) → any
            // cached heal is stale even if the key matches.
            match cached_healed {
                Some((ck, img)) if want_cache && ck == key && fresh_prob.is_none() => img,
                _ => {
                    let healed = autodust_heal(&app_data, baked, &mask, base);
                    if want_cache {
                        store_healed = Some((key, healed.clone()));
                    }
                    healed
                }
            }
        } else {
            baked
        };
        // Stage B: brush strokes + IR on top of the (cached) auto-dust-healed buffer.
        let healed = bake_for_view_from_baked(&app_data, stage_a, &spec, base);
        let (_, _, bytes) = pack_rgba16f(&healed, MAX_GPU_EDGE);
        (bytes, fresh_prob, store_healed, spots)
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(p) = fresh {
        session.autodust_prob.lock().unwrap().insert(id.clone(), p);
    }
    if let Some(entry) = store_healed {
        session.autodust_healed.lock().unwrap().insert(id.clone(), entry);
    }
    if let Some(sp) = spots {
        use tauri::Emitter;
        let _ = app.emit(
            "autodust://result",
            serde_json::json!({ "id": id, "count": sp.len(), "spots": sp }),
        );
    }
    Ok(tauri::ipc::Response::new(bytes))
}

/// Resolve inversion params (+ this image's sampled base) into GPU uniforms.
#[tauri::command]
pub fn resolved_inversion(
    id: String,
    params: InvertParams,
    session: State<Session>,
) -> Result<ResolvedInversion, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let mut u = resolve_to_uniforms(&params, effective_base(&params, dev.base));
    u.d_max = effective_dmax(&params, dev.d_max);
    u.channel_balance = dev.channel_balance;
    Ok(u)
}

/// Sample the orange-mask base from a normalized rect [x,y,w,h] (0..1) over the
/// resident working image. Used by the base-picker tool; cheap, no re-decode.
#[tauri::command]
pub fn sample_base_at(
    id: String,
    rect: [f64; 4],
    session: State<Session>,
) -> Result<[f32; 3], String> {
    use film_core::calibrate::Rect;
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let (x, y, w, h) = crop_px(rect, dev.working.width, dev.working.height);
    use film_core::calibrate::{sample_base_coherent_fullres, BASE_BAND_REBATE};
    let (lo, hi) = BASE_BAND_REBATE;
    // Full-res (no 512-proxy bleed): the picker rect is a small, deliberately-aimed
    // patch, so the loupe-aimed color is what gets sampled even on a thin rebate.
    Ok(sample_base_coherent_fullres(
        &dev.working,
        Rect { x, y, w, h },
        lo,
        hi,
    ))
}

/// The active per-image AUTO base (the develop-time detected/fallback film base) and
/// its detector confidence — so the UI can show what's in use and flag low confidence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoBaseInfo {
    pub base: [f32; 3],
    pub confidence: f32,
}

#[tauri::command]
pub fn auto_base_info(id: String, session: State<Session>) -> Result<AutoBaseInfo, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    Ok(AutoBaseInfo { base: dev.base, confidence: dev.base_confidence })
}

/// Aggregate a roll's per-frame auto bases into one robust roll base: the
/// per-channel median of the frames whose rebate detector cleared
/// `REBATE_CONFIDENCE`. Returns `None` if no frame is confident (rebate-less roll
/// → caller falls back to per-image auto base). Per-channel (not vector) median is
/// deliberate: it rejects a single pink/blue outlier channel independently.
pub(crate) fn aggregate_roll_base(samples: &[([f32; 3], f32)]) -> Option<[f32; 3]> {
    use film_core::calibrate::REBATE_CONFIDENCE;
    let confident: Vec<[f32; 3]> = samples
        .iter()
        .filter(|(_, c)| *c >= REBATE_CONFIDENCE)
        .map(|(b, _)| *b)
        .collect();
    if confident.is_empty() {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (ch, slot) in out.iter_mut().enumerate() {
        let mut col: Vec<f32> = confident.iter().map(|b| b[ch]).collect();
        col.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = col.len();
        *slot = if n % 2 == 1 {
            col[n / 2]
        } else {
            (col[n / 2 - 1] + col[n / 2]) / 2.0
        };
    }
    Some(out)
}

/// The aggregated roll base + how many confident frames fed it. `None` from
/// `roll_base` (serialized as JSON `null`) means no confident rebate in the roll.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RollBase {
    pub base: [f32; 3],
    pub frames_used: u32,
}

/// Compute one robust base for a whole roll by aggregating the already-stored
/// per-frame `dev.base`/`dev.base_confidence` (no re-decode/re-sample). Frames that
/// can't be made resident are skipped. Returns `null` when no frame is confident.
#[tauri::command]
pub fn roll_base(ids: Vec<String>, session: State<Session>) -> Result<Option<RollBase>, String> {
    let mut samples: Vec<([f32; 3], f32)> = Vec::with_capacity(ids.len());
    for id in &ids {
        if ensure_resident(&session, id).is_err() {
            continue;
        }
        let images = session.images.lock().unwrap();
        if let Some(dev) = images.get(id).and_then(|img| img.developed.as_ref()) {
            samples.push((dev.base, dev.base_confidence));
        }
    }
    let frames_used = samples
        .iter()
        .filter(|(_, c)| *c >= film_core::calibrate::REBATE_CONFIDENCE)
        .count() as u32;
    Ok(aggregate_roll_base(&samples).map(|base| RollBase { base, frames_used }))
}

/// Result of `analyze`: the auto-derived Cineon black point for the image area.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Analysis {
    pub d_max: f32,
}

/// Auto-derive `D_max` from the IMAGE AREA (the persistent crop, normalized
/// [x,y,w,h] 0..1 in working space). Excluding the borders is the whole point —
/// black surround / rebate would otherwise inflate the density range and wash the
/// image out (GitHub issue #1). `crop = None` analyzes the whole frame.
#[tauri::command]
pub fn analyze(
    id: String,
    params: InvertParams,
    crop: Option<[f64; 4]>,
    rot90: Option<u8>,
    flip_h: Option<bool>,
    flip_v: Option<bool>,
    angle: Option<f32>,
    session: State<Session>,
) -> Result<Analysis, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let base = effective_base(&params, dev.base);
    // The crop is in oriented (post orient/straighten) space — match the render
    // pipeline's geometry before sampling so flipped/rotated frames analyze the
    // correct image area.
    let prior = effective_dmax(&params, dev.d_max);
    let (estimate, spread) = sample_dmax_oriented(
        &dev.working,
        base,
        crop,
        rot90.unwrap_or(0),
        flip_h.unwrap_or(false),
        flip_v.unwrap_or(false),
        angle.unwrap_or(0.0),
        &params.meter_border,
        params.positive,
    );
    Ok(Analysis {
        d_max: guard_dmax(estimate, spread, prior),
    })
}

/// Anchor D_max to a measured white-point: sample the exposed leader from a
/// normalized rect [x,y,w,h] (0..1) over the resident working image and return the
/// scalar D_max. The frontend stores this in `d_max_override` (same as `analyze`),
/// so it overrides the per-frame scene estimate for this image.
#[tauri::command]
pub fn analyze_white_point(
    id: String,
    params: InvertParams,
    rect: [f64; 4],
    session: State<Session>,
) -> Result<Analysis, String> {
    use film_core::calibrate::{dmax_from_white_point, Rect};
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let base = effective_base(&params, dev.base);
    let (x, y, w, h) = crop_px(rect, dev.working.width, dev.working.height);
    Ok(Analysis {
        d_max: dmax_from_white_point(&dev.working, base, Some(Rect { x, y, w, h })),
    })
}

// ---------------------------------------------------------------------------
// Debug-log commands (called from api.ts in the frontend debug panel)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct DebugLine {
    pub level: String,
    pub msg: String,
}

/// Enable or disable the backend debug log. Enabling installs the panic hook and
/// starts the memory sampler; disabling flushes and closes the writer.
#[tauri::command]
pub fn debug_set(enabled: bool, app: tauri::AppHandle) {
    use tauri::Manager;
    let log = app.state::<crate::debug_log::DebugLog>().inner().clone();
    if enabled {
        let was_off = !log.is_on();
        log.enable();
        if was_off {
            crate::debug_log::install_panic_hook(log.clone());
            let app2 = app.clone();
            crate::debug_log::start_mem_sampler(log.clone(), move || {
                crate::cache::oecache_bytes(
                    &app2.state::<crate::session::Session>().cache_dir.lock()
                        .map(|d| d.clone()).unwrap_or_default(),
                )
            });
        }
        crate::dlog!(log, "INFO", "debug mode enabled");
    } else {
        crate::dlog!(log, "INFO", "debug mode disabled");
        log.disable();
    }
}

/// Append frontend log lines (batched from the JS side) into the shared log file.
#[tauri::command]
pub fn debug_log_append(lines: Vec<DebugLine>, log: tauri::State<crate::debug_log::DebugLog>) {
    for l in &lines {
        log.write("FE", &l.level, &l.msg);
    }
}

/// Truncate the debug log file (while keeping the writer open if enabled).
#[tauri::command]
pub fn debug_clear(log: tauri::State<crate::debug_log::DebugLog>) {
    log.clear();
}

/// Write the debug log with a summary header to `out_path` (chosen by the user).
#[tauri::command]
pub fn save_log(out_path: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let log = app.state::<crate::debug_log::DebugLog>();
    let body = std::fs::read_to_string(log.path()).unwrap_or_default();
    let version = app.package_info().version.to_string();
    let summary = crate::debug_log::build_summary(&body, &version, std::env::consts::OS);
    std::fs::write(&out_path, format!("{summary}\n{body}")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use half::f16;

    /// A 2x1 synthetic "developed" (pre-invert) image: pixel 0 is a dense
    /// negative patch (scan value far below `base`, the highest-density /
    /// brightest-after-inversion highlight a Cineon negative encodes); pixel 1
    /// sits at the film base (zero density / scene black). Mirrors the small
    /// inline fixtures in `engine.rs`'s `#[cfg(test)]` module.
    fn test_developed_image_2x1_bright() -> film_core::Image {
        let mut img = film_core::Image::new(2, 1);
        img.pixels[0] = [1e-4, 1e-4, 1e-4]; // far below base=1.0 → dense → bright highlight
        img.pixels[1] = [1.0, 1.0, 1.0]; // == base → zero density → scene black
        img
    }

    fn test_inversion_params() -> InversionParams {
        InversionParams {
            base: [1.0, 1.0, 1.0],
            d_max: 1.5,
            ..Default::default()
        }
    }

    fn test_finish_params() -> FinishParams {
        FinishParams::default()
    }

    #[test]
    fn hdr_raw_buffer_has_dims_and_superwhite() {
        let src = test_developed_image_2x1_bright();
        let ip = test_inversion_params();
        let finish = test_finish_params();
        let img = render_hdr_image(&src, &ip, Mode::D, &finish, &[], &IrRemoval::default());
        let buf = hdr_image_to_rgba16f(&img);
        assert_eq!(buf.width, 2);
        assert_eq!(buf.height, 1);
        assert_eq!(buf.rgba16f.len(), 2 * 1 * 4);
        // Bright pixel must exceed SDR white (1.0) in linear space after expansion+linearize.
        let r0 = f16::from_bits(buf.rgba16f[0]).to_f32();
        assert!(r0 > 1.0, "expected super-white highlight, got {r0}");
        // Alpha channel is 1.0.
        assert_eq!(f16::from_bits(buf.rgba16f[3]).to_f32(), 1.0);
    }

    #[test]
    fn blob_centroids_finds_one_blob_center() {
        // 4x4 frame, a single 2x2 set block at (x=2..3, y=0..1) → centroid (2.5,0.5).
        let mut bits = vec![false; 16];
        for &i in &[2usize, 3, 6, 7] {
            bits[i] = true;
        }
        let mask = film_core::dust::Mask { x0: 0, y0: 0, w: 4, h: 4, bits };
        let c = blob_centroids(&mask);
        assert_eq!(c.len(), 1);
        assert!((c[0][0] - 2.5 / 4.0).abs() < 1e-6, "cx {}", c[0][0]);
        assert!((c[0][1] - 0.5 / 4.0).abs() < 1e-6, "cy {}", c[0][1]);
    }

    #[test]
    fn decode_cached_reuses_one_decode_then_evicts_single_slot() {
        // Write two distinct TIFFs.
        let dir = std::env::temp_dir();
        let pa = dir.join(format!("oe-decode-cache-a-{}.tiff", std::process::id()));
        let pb = dir.join(format!("oe-decode-cache-b-{}.tiff", std::process::id()));
        let mut a = film_core::Image::new(3, 2);
        a.pixels[0] = [0.25, 0.5, 0.75];
        let b = film_core::Image::new(5, 4); // different dims → distinguishable
        film_core::export::write_tiff16(&a, &pa).unwrap();
        film_core::export::write_tiff16(&b, &pb).unwrap();

        let cache: Mutex<Option<DecodeCacheEntry>> = Mutex::new(None);

        // Same path twice in a row → second call is a cache HIT (same Arc allocation).
        let arc1 = decode_cached(&cache, &pa).unwrap();
        let arc2 = decode_cached(&cache, &pa).unwrap();
        assert!(Arc::ptr_eq(&arc1, &arc2), "repeat decode of same file must hit the cache");
        assert_eq!((arc1.width, arc1.height), (3, 2));

        // A different path replaces the single slot (and decodes fresh).
        let arc3 = decode_cached(&cache, &pb).unwrap();
        assert!(!Arc::ptr_eq(&arc1, &arc3));
        assert_eq!((arc3.width, arc3.height), (5, 4));

        // Back to A: the slot now holds B, so A misses and re-decodes (single-slot).
        let arc4 = decode_cached(&cache, &pa).unwrap();
        assert!(!Arc::ptr_eq(&arc1, &arc4), "single-slot cache evicts the prior entry");
        assert_eq!((arc4.width, arc4.height), (3, 2));

        let _ = std::fs::remove_file(&pa);
        let _ = std::fs::remove_file(&pb);
    }

    #[test]
    fn drop_excluded_clears_only_the_seeded_blob() {
        // Two separate single-pixel blobs: keep one via an exclusion seed on it.
        let mut bits = vec![false; 16];
        bits[0] = true; // blob A at (0,0)
        bits[10] = true; // blob B at (2,2)
        let mut mask = film_core::dust::Mask { x0: 0, y0: 0, w: 4, h: 4, bits };
        // Seed on blob B (normalized center of pixel (2,2)).
        drop_excluded(&mut mask, &[[2.0 / 4.0, 2.0 / 4.0]]);
        assert!(mask.bits[0], "blob A kept");
        assert!(!mask.bits[10], "blob B cleared");
        assert_eq!(blob_centroids(&mask).len(), 1);
    }

    #[test]
    fn white_point_dmax_matches_engine() {
        use film_core::calibrate::dmax_from_white_point;
        let white = [0.08f32, 0.008, 0.08];
        let img = film_core::Image { width: 4, height: 4, pixels: vec![white; 16], ir: None };
        let d = dmax_from_white_point(&img, [0.8, 0.8, 0.8], None);
        assert!((d - 2.0).abs() < 1e-3);
    }

    #[test]
    fn auto_seed_wb_neutralizes_cast() {
        // A negative with a denser GREEN channel inverts (neutral WB) to a green-cast
        // positive. The develop-time / regenerated grid thumbnail must bake the same
        // auto-WB the frontend seed applies, so a seeded render is MORE neutral than
        // the un-seeded one. (This is the per-image WB that used to only appear once
        // you opened Develop.)
        use film_core::Image;
        let neg = Image { width: 8, height: 8, pixels: vec![[0.30, 0.15, 0.30]; 64], ir: None };
        let base = [1.0, 1.0, 1.0];
        let dmax = 1.5;
        let (temp, tint) = auto_seed_wb(&neg, &default_invert_params(), base, dmax, None);
        let render = |p: &InvertParams| {
            let mut ip = resolve_params(p, &neg, base);
            ip.d_max = dmax;
            invert_image(&neg, &ip, Mode::D).pixels[0]
        };
        let mut seeded = default_invert_params();
        seeded.temp = temp;
        seeded.tint = tint;
        let imbalance =
            |px: [f32; 3]| (px[0] - px[1]).abs() + (px[1] - px[2]).abs() + (px[0] - px[2]).abs();
        let n = imbalance(render(&default_invert_params()));
        let s = imbalance(render(&seeded));
        assert!(s < n, "seeded imbalance {s} must be < neutral {n}");
    }

    #[test]
    fn auto_brightness_hits_target_under_high_contrast() {
        // Auto-exposure must leave the FINAL image (the user's full look, including a
        // strong Contrast) well-exposed — its 90th-pct luminance near the 0.80 target,
        // not blown. The bug: the secant measured that same clamped metric and froze at
        // its seed (~-0.32 EV) regardless of the image, leaving highlights pinned at 1.0
        // ("super bright"). Bisection over the monotonic metric finds the real exposure.
        use film_core::Image;
        let w = 256usize;
        let pixels: Vec<[f32; 3]> = (0..w)
            .map(|i| {
                let v = 0.10 + 0.45 * (i as f32 / (w as f32 - 1.0)); // negative density ramp
                [v, v, v]
            })
            .collect();
        let neg = Image { width: w, height: 1, pixels, ir: None };
        let base = [0.7f32, 0.7, 0.7];
        let dmax = 2.0f32;

        let mut pc = default_invert_params();
        pc.contrast = 100.0; // maximum contrast
        let evc = auto_brightness_value(&neg, &pc, base, dmax, None);

        // Render the full finish (with the contrast) at the solved EV and check the
        // 90th-pct lands near target — i.e. neither frozen-bright nor crushed-dark.
        pc.exposure = evc;
        let mut ip = resolve_params(&pc, &neg, effective_base(&pc, base));
        ip.d_max = effective_dmax(&pc, dmax);
        let inv = invert_image_core(&neg, &ip, mode_from(&pc.mode));
        let pos = finish_image(&inv, &finish_from(&pc));
        let p90 = percentile_luma(&pos, 0.90, None);
        assert!(
            (p90 - 0.80).abs() < 0.05,
            "final 90th-pct {p90} should sit near the 0.80 target (evc={evc})"
        );
        // And the solve must not be the frozen seed value.
        assert!(
            (evc - (-0.32)).abs() > 0.02,
            "evc {evc} looks like the old frozen secant seed"
        );
    }

    #[test]
    fn per_zone_seed_uniform_cast_matches_global_direction() {
        // Invert a synthetic uniformly-cast neutral ramp; the per-zone seed gains should
        // all push the same direction (faithfulness: agrees with a global gray-world).
        use film_core::Image;
        let mut pixels = Vec::new();
        for i in 0..300 {
            let s = 0.1 + 0.8 * (i as f32 / 299.0);
            pixels.push([0.6 * s, 0.5 * s, 0.45 * s]);
        }
        let src = Image { width: 300, height: 1, pixels, ir: None };
        let z = per_zone_seed(&src, 1.0, None);
        // Blue is the dimmest channel of the cast → blue gain > 1 in every populated zone.
        for zone in &z {
            if *zone != [1.0, 1.0, 1.0] {
                assert!(zone[2] >= zone[0], "blue should be boosted: {zone:?}");
            }
        }
    }

    #[test]
    fn render_grid_thumbnail_no_saved_bakes_per_zone_from_developed_positive() {
        // Verify that the None branch of render_grid_thumbnail (as patched) runs
        // per_zone_seed on the developed positive and stores the result into p.pz_*.
        // We use a mild red cast (sat < 0.25 to pass the saturation gate) so at least
        // one populated zone deviates from identity, confirming the code path ran.
        use film_core::Image;
        let (w, h) = (300usize, 4usize);
        // Mild red cast: sat ≈ (0.62−0.52)/0.62 ≈ 0.16 < 0.25 → passes gate.
        let pixels: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let s = 0.1 + 0.8 * ((i % w) as f32 / (w as f32 - 1.0));
                [0.62 * s, 0.56 * s, 0.52 * s]
            })
            .collect();
        let src = Image { width: w, height: h, pixels, ir: None };
        let base = [0.6f32; 3];
        let d_max = 2.0f32;
        // Replicate the None branch of render_grid_thumbnail exactly as patched.
        let mut p = InvertParams { positive: true, ..default_invert_params() };
        let (temp, tint) = auto_seed_wb(&src, &p, base, d_max, None);
        p.temp = temp;
        p.tint = tint;
        let mut ip_seed = resolve_params(&p, &src, effective_base(&p, base));
        ip_seed.d_max = effective_dmax(&p, d_max);
        let positive_seed = invert_image(&src, &ip_seed, mode_from(&p.mode));
        let z = per_zone_seed(&positive_seed, p.pz_strength, None);
        p.pz_sh = z[0]; p.pz_mid = z[1]; p.pz_hi = z[2];
        // At least one zone must deviate from identity — proves per-zone was baked.
        let identity = [[1.0f32; 3]; 3];
        assert_ne!(
            [p.pz_sh, p.pz_mid, p.pz_hi], identity,
            "cast image: None branch must produce non-identity pz gains; sh={:?} mid={:?} hi={:?}",
            p.pz_sh, p.pz_mid, p.pz_hi
        );
    }

    #[test]
    fn guard_dmax_keeps_prior_when_crop_is_flat() {
        // Flat crop (spread below MIN) → keep prior d_max, never destroy range (B3).
        assert_eq!(guard_dmax(1.05, 0.05, 2.4), 2.4, "flat crop must keep prior");
        // Real density range → accept the fresh estimate.
        assert_eq!(guard_dmax(1.8, 1.20, 2.4), 1.8, "ranged crop applies estimate");
    }

    #[test]
    fn analyze_sampling_follows_orientation() {
        // Left half bright (transmission 1.0 → density 0), right half dark
        // (transmission 1e-3 → density 3.0). A crop of the *oriented* left half must
        // sample the source LEFT half when un-flipped, but the source RIGHT half once
        // a horizontal flip is applied — otherwise flipped frames re-analyze the wrong
        // region and the D_max (hence brightness) is wrong.
        let (w, h) = (64usize, 8usize);
        let mut pixels = vec![[1.0f32; 3]; w * h];
        for y in 0..h {
            for x in (w / 2)..w {
                pixels[y * w + x] = [1e-3; 3];
            }
        }
        let img = film_core::Image { width: w, height: h, pixels, ir: None };
        let base = [1.0f32; 3];
        let left_half = Some([0.0, 0.0, 0.5, 1.0]);

        let unflipped = sample_dmax_oriented(&img, base, left_half, 0, false, false, 0.0, "include", false).0;
        assert!(unflipped < 1.5, "un-flipped left crop should be bright: {unflipped}");

        let flipped = sample_dmax_oriented(&img, base, left_half, 0, true, false, 0.0, "include", false).0;
        assert!(flipped > 2.5, "flipped left crop should sample the dark region: {flipped}");
    }

    #[test]
    fn sample_dmax_oriented_masks_border() {
        use film_core::Image;
        // Positive film: dark (near-black) sprocket holes around the border inflate
        // D_max when included. "exclude" mode detects them via positive_candidates
        // (luma ≤ POS_RAIL_MARGIN) and masks them out, lowering D_max to the true
        // image-content level.  The interior is mid-tone (not near either rail) so
        // it is never flagged as a spoke candidate and always stays in the mask.
        let base = [0.5, 0.4, 0.3];
        let spoke = [0.02, 0.02, 0.02]; // near-black → positive candidate → border
        let inner = [0.4, 0.4, 0.4]; // mid-tone → not a rail candidate → kept
        let mut img = Image::new(60, 60);
        for y in 0..60 {
            for x in 0..60 {
                let border = x < 8 || y < 8 || x >= 52 || y >= 52;
                img.pixels[y * 60 + x] = if border { spoke } else { inner };
            }
        }
        // No crop, no rotation. "include" = full; "exclude" = mask the dark spokes.
        let (d_full, _) = sample_dmax_oriented(&img, base, None, 0, false, false, 0.0, "include", true);
        let (d_excl, _) = sample_dmax_oriented(&img, base, None, 0, false, false, 0.0, "exclude", true);
        assert!(d_excl < d_full, "excl={d_excl} full={d_full}");
    }

    #[test]
    fn default_params_use_cineon_mode() {
        assert_eq!(default_invert_params().mode, "d");
    }

    #[test]
    fn mode_from_is_always_cineon() {
        use film_core::engine::Mode;
        for s in ["b", "c", "d", "naive", "anything"] {
            assert_eq!(mode_from(s), Mode::D, "mode {s} must resolve to Cineon");
        }
    }

    #[test]
    fn gray_point_wb_neutral_sample_is_neutral() {
        // A neutral display pixel with neutral current WB → ~neutral white point.
        let mut p = crate::commands_test_support::sample_invert_params();
        p.mode = "d".into();
        p.temp = 5500.0;
        p.tint = 0.0;
        let (temp, tint) = gray_point_temp_tint(&p, [0.5, 0.5, 0.5]);
        assert!((temp - 5500.0).abs() < 200.0, "neutral temp, got {temp}");
        assert!(tint.abs() < 0.05, "neutral tint, got {tint}");
    }

    #[test]
    fn gray_point_wb_warm_sample_cools_temp() {
        // A warm (red-heavy, blue-low) display pixel must drive Temp BELOW neutral so
        // the applied gains cool the point toward gray. Absolute: independent of where
        // Temp currently sits (start it warm and confirm the pick still cools).
        let mut p = crate::commands_test_support::sample_invert_params();
        p.mode = "d".into();
        p.temp = 7000.0;
        p.tint = 0.0;
        let (temp, _tint) = gray_point_temp_tint(&p, [0.6, 0.5, 0.4]);
        assert!(temp < 5500.0, "warm point should cool temp below neutral, got {temp}");
    }

    #[test]
    fn gray_point_wb_converges_no_drift() {
        // Repeatedly picking the SAME gray point must converge, not creep. This is the
        // user-reported bug: each pick under-applied tint (~1/150 scale) and re-multiplied
        // the current slider, so Temp/Tint drifted instead of neutralising the point.
        //
        // Model the mode-D render: displayed = underlying P · (wb_baseline · slider).
        // `underlying` is the wb-neutral scan value at the pixel; a green-heavy, r=b cast.
        let underlying = [0.50_f32, 0.66, 0.50];
        let render = |p: &crate::session::InvertParams| -> [f32; 3] {
            let wb = wb_from_params(p.temp, p.tint);
            std::array::from_fn(|c| underlying[c] * p.wb_baseline[c] * wb[c])
        };
        let chroma_dev = |rgb: [f32; 3]| -> f32 {
            let mean = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
            rgb.iter()
                .map(|c| (c - mean).abs() / mean)
                .fold(0.0_f32, f32::max)
        };

        // Start from a deliberately NON-neutral WB so the slider double-count shows.
        let mut p = crate::commands_test_support::sample_invert_params();
        p.mode = "d".into();
        p.temp = 6500.0;
        p.tint = 0.0;
        p.wb_baseline = [1.0, 1.0, 1.0];

        let dev0 = chroma_dev(render(&p));
        assert!(dev0 > 0.15, "test setup: starting point should be visibly tinted, got {dev0}");

        // First pick must SUBSTANTIALLY neutralise the point. Pre-fix the tint correction
        // landed at ~1/150 strength, so the point stayed ~as tinted as before (dev ~unchanged).
        let r1 = gray_point_wb(p.clone(), render(&p));
        p.temp = r1.temp;
        p.tint = r1.tint;
        p.wb_baseline = [1.0, 1.0, 1.0]; // frontend resets baseline (absolute-WB model)
        let dev1 = chroma_dev(render(&p));
        assert!(
            dev1 < dev0 * 0.5,
            "one pick must substantially neutralise the point: dev {dev0} -> {dev1}"
        );

        // ...and then the picker must be a FIXPOINT: re-picking the SAME (re-rendered)
        // point returns the same Temp/Tint, so it never creeps in either direction.
        // This is the reported bug — repeated picks used to drift instead of settling.
        let mut prev = r1;
        for i in 0..5 {
            let r = gray_point_wb(p.clone(), render(&p));
            assert!(
                (r.temp - prev.temp).abs() < 1.0 && (r.tint - prev.tint).abs() < 0.1,
                "re-pick #{i} must not drift: ({},{}) -> ({},{})",
                prev.temp,
                prev.tint,
                r.temp,
                r.tint
            );
            p.temp = r.temp;
            p.tint = r.tint;
            p.wb_baseline = [1.0, 1.0, 1.0];
            prev = r;
        }
    }

    #[test]
    fn effective_base_prefers_override_then_dev_base() {
        let mut p = crate::commands_test_support::sample_invert_params();
        p.base_override = None;
        assert_eq!(
            effective_base(&p, [0.8, 0.6, 0.4]),
            [0.8, 0.6, 0.4],
            "None -> dev base"
        );
        p.base_override = Some([0.1, 0.2, 0.3]);
        assert_eq!(
            effective_base(&p, [0.8, 0.6, 0.4]),
            [0.1, 0.2, 0.3],
            "Some -> override"
        );
    }

    #[test]
    fn auto_base_prefers_detected_rebate_else_fallback() {
        use film_core::Image;
        let mut img = Image::new(200, 150);
        let (bw, bh) = (20usize, 15usize);
        for y in 0..150 {
            for x in 0..200 {
                let edge = x < bw || x >= 200 - bw || y < bh || y >= 150 - bh;
                img.pixels[y * 200 + x] = if edge { [0.42, 0.19, 0.10] } else { [0.5, 0.5, 0.5] };
            }
        }
        let (base, conf) = auto_base(&img);
        assert!(conf >= film_core::calibrate::REBATE_CONFIDENCE, "should be confident: {conf}");
        assert!((base[0] - 0.42).abs() < 0.04 && base[0] > base[2], "detected orange: {base:?}");

        let grey = Image { width: 64, height: 64, pixels: vec![[0.5, 0.5, 0.5]; 64 * 64], ir: None };
        let (fb, fconf) = auto_base(&grey);
        assert!(fconf < film_core::calibrate::REBATE_CONFIDENCE, "fallback path: {fconf}");
        let (lo, hi) = film_core::calibrate::BASE_BAND_AUTO;
        let want = film_core::calibrate::sample_base_coherent(&grey, None, lo, hi);
        for c in 0..3 {
            assert!((fb[c] - want[c]).abs() < 1e-4, "ch {c}: {} vs {}", fb[c], want[c]);
        }
    }

    #[test]
    fn auto_base_anti_blue_prefers_dim_orange_edge_over_blue_fallback() {
        use film_core::Image;
        // The mixed-roll "Phoenix" case: a bright blue scene (the brightest-cluster
        // fallback would pick it, B>R) with only a DIM orange rebate at the edges
        // (low detector confidence). auto_base must still return an orange base.
        let (w, h) = (200usize, 150usize);
        let mut img = Image::new(w, h);
        let (bw, bh) = (20usize, 15usize);
        for y in 0..h {
            for x in 0..w {
                let edge = x < bw || x >= w - bw || y < bh || y >= h - bh;
                img.pixels[y * w + x] = if edge { [0.16, 0.11, 0.06] } else { [0.30, 0.22, 0.62] };
            }
        }
        // Precondition: the brightest-cluster fallback alone is blue (B>R).
        let (lo, hi) = film_core::calibrate::BASE_BAND_AUTO;
        let fb = film_core::calibrate::sample_base_coherent(&img, None, lo, hi);
        assert!(fb[2] > fb[0], "precondition: fallback is blue {fb:?}");
        // And the dim rebate is below the confident threshold (exercises anti-blue).
        let (base, conf) = auto_base(&img);
        assert!(conf < film_core::calibrate::REBATE_CONFIDENCE, "should be low-confidence: {conf}");
        assert!(base[0] > base[2], "anti-blue: must pick the orange edge, got {base:?}");
    }

    #[test]
    fn auto_base_anti_blue_ignores_near_black_orange_edge() {
        use film_core::Image;
        // No real rebate: a near-black (noise-level) orange-ordered edge over a bright
        // blue scene. The brightness floor must reject the near-black patch so we fall
        // back rather than invert against a garbage ~0.02 dmin.
        let (w, h) = (200usize, 150usize);
        let mut img = Image::new(w, h);
        let (bw, bh) = (20usize, 15usize);
        for y in 0..h {
            for x in 0..w {
                let edge = x < bw || x >= w - bw || y < bh || y >= h - bh;
                img.pixels[y * w + x] = if edge { [0.03, 0.02, 0.015] } else { [0.30, 0.22, 0.62] };
            }
        }
        let (base, _conf) = auto_base(&img);
        assert!(base[2] > base[0], "near-black edge must not be used as base: {base:?}");
    }

    #[test]
    fn build_params_is_always_cineon_regardless_of_mode_or_stock() {
        use crate::commands_test_support::sample_invert_params;
        // Even with the legacy Mode-B + Portra selection, we now build Cineon params:
        // identity matrices, d_max at the default, exposure → print_exposure (2^ev).
        let mut p = sample_invert_params();
        p.mode = "b".into();
        p.stock = "portra400".into();
        p.exposure = 1.0; // 1 EV → 2.0x print exposure
        let ip = build_params(&p, [0.8, 0.6, 0.4]);
        assert_eq!(ip.base, [0.8, 0.6, 0.4]);
        assert!((ip.print_exposure - 2.0).abs() < 1e-5, "exposure → print_exposure");
        assert!((ip.d_max - 1.5).abs() < 1e-6, "d_max default");
        assert!((ip.paper_grade - 0.95).abs() < 1e-6, "paper_grade default");
        // Identity m_post == no per-stock cross-channel matrix (nalgebra isn't a
        // direct dep of this crate, so compare against the engine default's identity).
        assert_eq!(ip.m_post, InversionParams::default().m_post, "no stock matrix");
    }

    #[test]
    fn build_params_honors_d_max_override() {
        use crate::commands_test_support::sample_invert_params;
        let mut p = sample_invert_params();
        p.d_max_override = None;
        assert!((build_params(&p, [0.8, 0.6, 0.4]).d_max - 1.5).abs() < 1e-6, "default 1.5");
        p.d_max_override = Some(2.7);
        assert!((build_params(&p, [0.8, 0.6, 0.4]).d_max - 2.7).abs() < 1e-6, "override used");
    }

    #[test]
    fn viewspec_finish_defaults_true_and_parses_false() {
        let d: ViewSpec =
            serde_json::from_str(r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#)
                .unwrap();
        assert!(d.finish, "finish should default to true when omitted");
        let f: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,"finish":false}"#,
        )
        .unwrap();
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
        assert!(
            green[0] < 1.0,
            "negative tint suppresses red relative to green"
        );
        assert!(
            green[2] < 1.0,
            "negative tint suppresses blue relative to green"
        );
    }

    #[test]
    fn viewspec_dust_defaults_empty_and_parses_points() {
        let d: ViewSpec =
            serde_json::from_str(r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#)
                .unwrap();
        assert!(d.dust.is_empty(), "dust defaults to empty when omitted");
        let p: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,
                "dust":[{"points":[[0.5,0.5],[0.6,0.5]],"r":0.02}]}"#,
        )
        .unwrap();
        assert_eq!(p.dust.len(), 1);
        assert_eq!(p.dust[0].points.len(), 2);
    }

    #[test]
    fn view_stamps_maps_normalized_points_to_output_pixels() {
        // base image 200x100; view crop = whole base; output 400x200 (2x).
        let dust = vec![DustStroke {
            points: vec![[0.5, 0.5]],
            r: 0.01,
        }];
        let s = view_stamps(&dust, 200, 100, 0, 0, 200, 100, 400, 200);
        assert_eq!(s.len(), 1);
        assert!((s[0].cx - 200.0).abs() < 0.5, "x: 0.5*200*2 = 200");
        assert!((s[0].cy - 100.0).abs() < 0.5, "y: 0.5*100*2 = 100");
        // r normalized to base width: 0.01*200 = 2 base px → *2 scale = 4 out px.
        assert!(
            (s[0].r - 4.0).abs() < 0.5,
            "r mapped to output px, got {}",
            s[0].r
        );
    }

    #[test]
    fn export_stamps_maps_normalized_points_to_full_res_pixels() {
        let dust = vec![DustStroke {
            points: vec![[0.25, 0.5]],
            r: 0.01,
        }];
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
        for y in 0..4 {
            for x in 0..2 {
                pixels[y * 4 + x] = [0.9, 0.9, 0.9];
            }
        }
        let img = film_core::Image {
            width: 4,
            height: 4,
            pixels,
            ir: None,
        };
        // Normalized rect over the left half -> bright base.
        let (x, y, w, h) = crop_px([0.0, 0.0, 0.5, 1.0], img.width, img.height);
        let base = sample_base(&img, Some(Rect { x, y, w, h }));
        assert!(
            base[0] >= 0.85,
            "left-half base should be bright, got {base:?}"
        );
    }

    #[test]
    fn viewspec_ir_removal_defaults_off_and_parses() {
        let d: ViewSpec =
            serde_json::from_str(r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false}"#)
                .unwrap();
        assert!(!d.ir_removal.enabled, "ir_removal defaults disabled");
        let p: ViewSpec = serde_json::from_str(
            r#"{"crop":[0,0,10,10],"out_w":10,"out_h":10,"raw":false,
                "ir_removal":{"enabled":true,"sensitivity":60}}"#,
        )
        .unwrap();
        assert!(p.ir_removal.enabled);
        assert!((p.ir_removal.sensitivity - 60.0).abs() < 1e-6);
    }

    #[test]
    fn render_and_encode_hdr_emits_gain_map() {
        use crate::commands_test_support::sample_invert_params;
        let src = film_core::Image {
            width: 8,
            height: 8,
            pixels: vec![[0.6, 0.4, 0.25]; 64],
            ir: None,
        };
        let params = sample_invert_params();
        let ip = resolve_params(&params, &src, effective_base(&params, [0.6, 0.4, 0.25]));
        let finish = finish_from(&params);
        let bytes = render_and_encode_hdr(
            &src,
            &ip,
            mode_from(&params.mode),
            &finish,
            &[],
            &IrRemoval { enabled: false, sensitivity: 50.0 },
            90,
        )
        .expect("encode");
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "not a JPEG");
        let iso = b"urn:iso";
        let apple = b"hdrgainmap";
        assert!(
            bytes.windows(iso.len()).any(|w| w == iso)
                || bytes.windows(apple.len()).any(|w| w == apple),
            "no gain map"
        );
    }

    #[test]
    fn autodust_heal_key_is_stable_and_distinguishes_inputs() {
        let a = autodust_heal_key(50.0, &[[0.1, 0.2]]);
        assert_eq!(a, autodust_heal_key(50.0, &[[0.1, 0.2]]), "same inputs → same key");
        assert_ne!(a, autodust_heal_key(60.0, &[[0.1, 0.2]]), "sensitivity changes key");
        assert_ne!(a, autodust_heal_key(50.0, &[[0.1, 0.2], [0.3, 0.4]]), "exclusions change key");
        assert_ne!(a, autodust_heal_key(50.0, &[]), "no exclusions differs");
    }

    #[test]
    fn bake_for_view_telea_heals_strokes_without_auto_mask() {
        // migan=false → classic Telea fill heals the stroke; no app_data needed.
        let mut pixels = vec![[0.5_f32, 0.5, 0.5]; 16];
        pixels[5] = [0.9, 0.9, 0.9]; // speck at (1,1)
        let img = film_core::Image { width: 4, height: 4, pixels, ir: None };
        let spec = BakeSpec {
            rot90: 0, flip_h: false, flip_v: false, angle: 0.0, image_crop: None,
            dust: vec![DustStroke { points: vec![[0.25, 0.25]], r: 0.5 }],
            ir_removal: IrRemoval { enabled: false, sensitivity: 0.0 },
            skip_dust_heal: false, migan: false,
            auto_dust: AutoDust::default(), auto_dust_exclusions: Vec::new(),
        };
        let out = bake_for_view_from_baked(Path::new("/nonexistent"), img, &spec, [0.0; 3]);
        assert!((out.pixels[5][0] - 0.5).abs() < 0.35, "speck healed: {}", out.pixels[5][0]);
    }

    #[test]
    fn build_params_defaults_wb_mode_gain() {
        let p = crate::commands_test_support::sample_invert_params();
        let ip = build_params(&p, [0.8, 0.6, 0.4]);
        assert_eq!(ip.wb_mode, film_core::WbMode::Gain);
    }

    #[test]
    fn build_params_reads_subtractive_wb_mode() {
        let mut p = crate::commands_test_support::sample_invert_params();
        p.wb_mode = "subtractive".to_string();
        let ip = build_params(&p, [0.8, 0.6, 0.4]);
        assert_eq!(ip.wb_mode, film_core::WbMode::Subtractive);
    }

    #[test]
    fn build_params_forces_faithful_tone_mode() {
        let mut p = crate::commands_test_support::sample_invert_params();
        p.tone_mode = "filmic".to_string();
        let ip = build_params(&p, [0.8, 0.6, 0.4]);
        assert!(matches!(ip.tone_mode, film_core::ToneMode::Faithful), "build_params must force Faithful");
    }

    #[test]
    fn build_params_derives_recovery_from_negative_sliders() {
        // Recovery wiring is retired: hi_recovery and lo_recovery are always 0.0
        // regardless of the Highlights/Shadows slider sign. The finish tone tools
        // operate on the super-white body instead (see headroom-tone-recovery design).
        let mut p = crate::commands_test_support::sample_invert_params();
        p.highlights = -100.0; p.shadows = -50.0;
        let ip = build_params(&p, [0.8, 0.6, 0.4]);
        assert_eq!(ip.hi_recovery, 0.0, "hi_recovery must be 0 regardless of highlights slider");
        assert_eq!(ip.lo_recovery, 0.0, "lo_recovery must be 0 regardless of shadows slider");

        p.highlights = 40.0; p.shadows = 0.0;
        let ip2 = build_params(&p, [0.8, 0.6, 0.4]);
        assert_eq!(ip2.hi_recovery, 0.0);
        assert_eq!(ip2.lo_recovery, 0.0);
    }

    #[test]
    fn finish_from_suppresses_negative_highlights_shadows() {
        // finish_from clamps Highlights/Shadows to the positive half (max(0, slider/100)).
        // Negative values are not a finish-pass move; only the positive half reaches the
        // tone curve. Engine-side recovery (hi_recovery/lo_recovery) is retired (zeroed
        // in build_params) — the finish tone tools operate on the super-white body instead.
        let mut p = crate::commands_test_support::sample_invert_params();
        p.highlights = -100.0; p.shadows = -100.0;
        let f = finish_from(&p);
        assert_eq!(f.highlights, 0.0, "negative Highlights must not reach finish as a tone move");
        assert_eq!(f.shadows, 0.0);

        p.highlights = 50.0; // positive half unchanged
        assert!((finish_from(&p).highlights - 0.5).abs() < 1e-6);
    }

    #[test]
    fn aggregate_roll_base_per_channel_median_of_confident() {
        let hi = film_core::calibrate::REBATE_CONFIDENCE + 0.1;
        let samples = [
            ([0.40, 0.22, 0.13], hi),
            ([0.44, 0.24, 0.15], hi),
            ([0.42, 0.23, 0.14], hi),
        ];
        let out = aggregate_roll_base(&samples).expect("confident frames present");
        assert!((out[0] - 0.42).abs() < 1e-6, "r median: {}", out[0]);
        assert!((out[1] - 0.23).abs() < 1e-6, "g median: {}", out[1]);
        assert!((out[2] - 0.14).abs() < 1e-6, "b median: {}", out[2]);
    }

    #[test]
    fn aggregate_roll_base_drops_sub_threshold_frames() {
        let hi = film_core::calibrate::REBATE_CONFIDENCE + 0.1;
        let lo = film_core::calibrate::REBATE_CONFIDENCE - 0.1;
        let samples = [
            ([0.40, 0.22, 0.13], hi),
            ([0.99, 0.10, 0.95], lo), // blue/pink outlier, low confidence → dropped
            ([0.42, 0.23, 0.14], hi),
        ];
        let out = aggregate_roll_base(&samples).expect("two confident frames");
        // Median of the two confident frames only; the low-confidence outlier never counts.
        assert!((out[0] - 0.41).abs() < 1e-6, "r: {}", out[0]);
        assert!((out[2] - 0.135).abs() < 1e-6, "b: {}", out[2]);
    }

    #[test]
    fn aggregate_roll_base_outlier_does_not_move_median() {
        let hi = film_core::calibrate::REBATE_CONFIDENCE + 0.1;
        // One confident-but-pink frame among good ones; median rejects it.
        let samples = [
            ([0.40, 0.22, 0.13], hi),
            ([0.41, 0.23, 0.14], hi),
            ([0.42, 0.24, 0.15], hi),
            ([0.90, 0.10, 0.80], hi), // pink outlier
            ([0.43, 0.25, 0.16], hi),
        ];
        let out = aggregate_roll_base(&samples).expect("confident frames");
        assert!((out[0] - 0.42).abs() < 1e-6, "r median unmoved: {}", out[0]);
        assert!(out[2] < 0.2, "b median not pulled high by outlier: {}", out[2]);
    }

    #[test]
    fn aggregate_roll_base_even_count_averages_middle_two() {
        let hi = film_core::calibrate::REBATE_CONFIDENCE + 0.1;
        let samples = [([0.40, 0.20, 0.10], hi), ([0.44, 0.24, 0.14], hi)];
        let out = aggregate_roll_base(&samples).expect("two frames");
        assert!((out[0] - 0.42).abs() < 1e-6, "r: {}", out[0]);
    }

    #[test]
    fn aggregate_roll_base_none_when_no_confident_frames() {
        let lo = film_core::calibrate::REBATE_CONFIDENCE - 0.1;
        let samples = [([0.40, 0.22, 0.13], lo), ([0.42, 0.23, 0.14], lo)];
        assert!(aggregate_roll_base(&samples).is_none());
    }

    #[test]
    fn aggregate_roll_base_single_confident_frame() {
        let hi = film_core::calibrate::REBATE_CONFIDENCE + 0.1;
        let samples = [([0.40, 0.22, 0.13], hi)];
        let out = aggregate_roll_base(&samples).expect("one frame");
        assert_eq!(out, [0.40, 0.22, 0.13]);
    }

    #[test]
    fn build_params_zeroes_retired_recovery() {
        // Recovery wiring is retired: hi_recovery and lo_recovery must always be 0.0
        // even when the Highlights/Shadows sliders are pushed negative (where the old
        // code derived recovery from them). The finish tone tools operate on the
        // super-white body instead (see headroom-tone-recovery design).
        let mut p = default_invert_params();
        p.highlights = -100.0; p.shadows = -100.0;
        let ip = build_params(&p, [0.18, 0.12, 0.08]);
        assert_eq!(ip.hi_recovery, 0.0);
        assert_eq!(ip.lo_recovery, 0.0);
    }

    #[test]
    fn headroom_pipeline_identity_at_zero_sliders() {
        // With all tone sliders at zero AND neutral WB (build_params defaults wb=[1,1,1]),
        // the new display pipeline (invert_image_core → finish_image, which applies
        // display_finalize as its last step) must produce the same output as the legacy
        // display path (invert_image, which already applies display_finalize internally).
        // This holds for all pixel densities because shoulder_only commutes with a
        // scalar wb of 1.0.  Use a 2×2 mid-key negative (scan < base).
        use film_core::Image;
        let neg = Image {
            width: 2,
            height: 2,
            pixels: vec![[0.09f32, 0.06, 0.04]; 4],
            ir: None,
        };
        let p = default_invert_params();
        let ip = build_params(&p, [0.18, 0.12, 0.08]);
        let legacy = invert_image(&neg, &ip, Mode::D);
        let headroom = finish_image(&invert_image_core(&neg, &ip, Mode::D), &finish_from(&p));
        for (a, b) in legacy.pixels.iter().zip(headroom.pixels.iter()) {
            for c in 0..3 {
                assert!(
                    (a[c] - b[c]).abs() < 1e-5,
                    "ch {c}: legacy={a:?} headroom={b:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod proxy_tests {
    use super::PROXY_EDGE;

    #[test]
    fn proxy_edge_caps_working_long_edge() {
        // A 4000x3000 buffer must be capped to PROXY_EDGE on the long edge.
        let img = film_core::Image::new(4000, 3000);
        let p = crate::convert::proxy(&img, PROXY_EDGE);
        assert_eq!(p.width.max(p.height) as u32, PROXY_EDGE);
        // An already-small buffer is untouched.
        let small = film_core::Image::new(1000, 800);
        let q = crate::convert::proxy(&small, PROXY_EDGE);
        assert_eq!((q.width, q.height), (1000, 800));
    }

    #[test]
    fn ensure_zoom_src_reuses_cached_same_id() {
        let s = crate::session::Session::default();
        *s.zoom_src.lock().unwrap() = Some(("abc".to_string(), film_core::Image::new(100, 80)));
        // A cache hit for the same id is a no-op (no file decode → cannot fail).
        assert!(tauri::async_runtime::block_on(super::ensure_zoom_src(&s, "abc")).is_ok());
        let g = s.zoom_src.lock().unwrap();
        let (cid, cached) = g.as_ref().unwrap();
        assert_eq!(cid, "abc");
        assert_eq!((cached.width, cached.height), (100, 80));
    }
}

/// Enhance the current developed preview via the configured AI provider.
/// `image_base64` is the preview JPEG payload WITHOUT the `data:` URL prefix.
/// Returns a PNG data URL on success, or a readable error string.
#[tauri::command]
pub async fn ai_enhance_image(
    image_base64: String,
    api_key: String,
    log: tauri::State<'_, crate::debug_log::DebugLog>,
) -> Result<String, String> {
    let __log = log.inner().clone();
    crate::time_op!(__log, "ai_enhance_image", {
        crate::ai_enhance::enhance(&image_base64, &api_key).await
    })
}

/// Match the current image's color toning to a reference image (fully local).
/// Returns the scoped develop params, blended by `strength` (0..100) from the
/// current params toward the optimized match. The frontend spreads these onto
/// the params store as a single undoable change.
#[tauri::command]
pub fn color_match_params(
    id: String,
    params: InvertParams,
    ref_path: String,
    strength: u8,
    session: State<Session>,
) -> Result<crate::color_match::MatchedParams, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    crate::color_match::match_to_reference(
        &params, &dev.thumb, dev.base, dev.d_max, &ref_path, strength,
    )
}

/// Decode a user-picked reference image and return a small base64 JPEG data URL
/// for the panel thumbnail (the app doesn't enable the `asset://` protocol).
#[tauri::command]
pub fn reference_thumb(path: String) -> Result<String, String> {
    crate::color_match::reference_thumb_data_url(&path)
}

/// Whether the AI-dust models (+ shared runtime) are installed, and download size.
#[tauri::command]
pub fn autodust_status(app: tauri::AppHandle) -> Result<crate::autodust::assets::Status, String> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(crate::autodust::assets::status(&app_data))
}

/// Download + verify the AI-dust assets, emitting `autodust://download-progress`.
#[tauri::command]
pub async fn download_autodust(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::autodust::assets::download(&app, &app_data).await
}

/// Whether the upscaler runtime+model are installed, and the download size.
#[tauri::command]
pub fn upscaler_status(app: tauri::AppHandle) -> Result<crate::upscale::assets::Status, String> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(crate::upscale::assets::status(&app_data))
}

/// Download + verify the upscaler assets, emitting `upscale://download-progress`.
#[tauri::command]
pub async fn download_upscaler(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::upscale::assets::download(&app, &app_data).await
}

/// Decode encoded image bytes (PNG/JPEG/etc.) into a film_core::Image with RGB
/// values in [0,1]. Used to upscale an externally-produced image (e.g. the
/// AI-enhanced PNG) that isn't one of our developed source files.
fn image_from_encoded(bytes: &[u8]) -> Result<film_core::Image, String> {
    let dynimg = image::load_from_memory(bytes).map_err(|e| format!("decode image: {e}"))?;
    let rgb = dynimg.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let pixels = rgb
        .pixels()
        .map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
        .collect();
    Ok(film_core::Image { width: w, height: h, pixels, ir: None })
}

/// Upscale the current developed image; stash full-res, return a preview.
/// Emits `upscale://progress` ({ done, total }) per tile.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn upscale_image(
    app: tauri::AppHandle,
    id: String,
    params: InvertParams,
    image_crop: Option<[f64; 4]>,
    rot90: u8,
    flip_h: bool,
    flip_v: bool,
    angle: f32,
    target_long: u32,
    dust: Vec<DustStroke>,
    ir_removal: IrRemoval,
    session: State<'_, Session>,
) -> Result<crate::upscale::UpscaleResult, String> {
    use tauri::{Emitter, Manager};
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let (fin, metadata) = finish_full_res(
        &id, &params, image_crop, rot90, flip_h, flip_v, angle, &dust, &ir_removal, &session,
    )?;
    let up = crate::upscale::run(&app_data, &fin, target_long, |done, total| {
        let _ = app.emit("upscale://progress", serde_json::json!({ "done": done, "total": total }));
    })?;
    let (out_w, out_h) = (up.width as u32, up.height as u32);
    let preview = crate::convert::proxy(&up, 1600);
    let jpeg = crate::encode::encode_jpeg_bytes(&preview, 85)?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    let preview_data_url = format!("data:image/jpeg;base64,{b64}");
    *session.pending_upscale.lock().unwrap() =
        Some(crate::session::PendingUpscale { image: up, metadata });
    Ok(crate::upscale::UpscaleResult { preview_data_url, out_w, out_h })
}

/// Upscale an externally-produced image (base64-encoded PNG/JPEG, e.g. the AI
/// Enhance result) to `target_long` on the longest side. Stashes the full-res
/// result for `save_upscaled`; returns a preview. Emits `upscale://progress`.
#[tauri::command]
pub async fn upscale_enhanced(
    app: tauri::AppHandle,
    image_base64: String,
    target_long: u32,
    session: State<'_, Session>,
) -> Result<crate::upscale::UpscaleResult, String> {
    use base64::Engine;
    use tauri::{Emitter, Manager};
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image_base64.trim())
        .map_err(|e| format!("decode base64: {e}"))?;
    let img = image_from_encoded(&bytes)?;
    let up = crate::upscale::run(&app_data, &img, target_long, |done, total| {
        let _ = app.emit("upscale://progress", serde_json::json!({ "done": done, "total": total }));
    })?;
    let (out_w, out_h) = (up.width as u32, up.height as u32);
    let preview = crate::convert::proxy(&up, 1600);
    let jpeg = crate::encode::encode_jpeg_bytes(&preview, 85)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    let preview_data_url = format!("data:image/jpeg;base64,{b64}");
    *session.pending_upscale.lock().unwrap() = Some(crate::session::PendingUpscale {
        image: up,
        metadata: crate::metadata::Metadata::default(),
    });
    Ok(crate::upscale::UpscaleResult { preview_data_url, out_w, out_h })
}

/// Save the stashed upscaled image to `out_path` in the chosen format, with EXIF.
#[tauri::command]
pub async fn save_upscaled(
    out_path: String,
    format: ExportFormat,
    meta_override: Option<MetaOverride>,
    session: State<'_, Session>,
) -> Result<(), String> {
    let guard = session.pending_upscale.lock().unwrap();
    let pending = guard.as_ref().ok_or("no upscaled image to save")?;
    let out = Path::new(&out_path);
    match format.kind.as_str() {
        "tiff" => {
            if format.bit_depth == 16 {
                film_core::export::write_tiff16(&pending.image, out).map_err(|e| format!("{e}"))
            } else {
                write_tiff8(&pending.image, out)
            }
        }
        "png" => write_png(&pending.image, out, format.bit_depth),
        "jpeg" => write_jpeg(&pending.image, out, format.quality, format.max_bytes),
        other => Err(format!("unknown export format: {other}")),
    }?;
    let eff = effective_metadata(&pending.metadata, meta_override.as_ref());
    if let Err(e) = crate::exif_write::write_exif(out, &eff) {
        eprintln!("[exif] embed failed for {out_path}: {e}");
    }
    Ok(())
}

/// Save an AI-enhanced result to `out_path` at its native (un-upscaled) resolution.
/// `image_base64` is the enhanced PNG payload WITHOUT the `data:` URL prefix; it is
/// decoded and re-encoded in the chosen format so the user can grab the enhanced image
/// directly instead of routing it through the upscaler.
#[tauri::command]
pub fn save_enhanced(
    out_path: String,
    image_base64: String,
    format: ExportFormat,
) -> Result<(), String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image_base64.trim())
        .map_err(|e| format!("decode base64: {e}"))?;
    let img = image_from_encoded(&bytes)?;
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
    }
}

#[cfg(test)]
mod wb_compose_tests {
    use super::*;

    fn base_params() -> InvertParams {
        default_invert_params()
    }

    #[test]
    fn resolve_composes_baseline_times_slider() {
        let mut p = base_params();
        p.wb_baseline = [1.2, 1.0, 0.8];
        // Neutral sliders → wb_from_params ≈ [1,1,1] → wb == baseline.
        let ip = resolve_params(&p, &film_core::Image::new(1, 1), [0.5, 0.5, 0.5]);
        for c in 0..3 {
            assert!((ip.wb[c] - p.wb_baseline[c]).abs() < 1e-3, "ch {c}: {:?}", ip.wb);
        }
    }

    #[test]
    fn neutral_baseline_equals_legacy_wb() {
        // Identity baseline must reproduce today's WB exactly (back-compat for old edits).
        let mut p = base_params();
        p.temp = 7200.0;
        p.tint = 20.0;
        p.wb_baseline = [1.0, 1.0, 1.0];
        let ip = resolve_params(&p, &film_core::Image::new(1, 1), [0.5, 0.5, 0.5]);
        let legacy = wb_from_params(p.temp, p.tint);
        for c in 0..3 {
            assert!((ip.wb[c] - legacy[c]).abs() < 1e-6, "ch {c}: {:?} vs {:?}", ip.wb, legacy);
        }
    }
}

#[cfg(test)]
mod as_shot_gains_tests {
    use super::*;

    #[test]
    fn as_shot_wb_gains_match_cct_projection() {
        // The returned gains must equal wb_from_params(temp,tint) so that storing
        // them as wb_baseline (with neutral sliders) reproduces today's applied WB.
        let temp = 6800.0_f32;
        let tint = 15.0_f32;
        let g = as_shot_gains(temp, tint);
        let want = wb_from_params(temp, tint);
        for c in 0..3 {
            assert!((g[c] - want[c]).abs() < 1e-6, "ch {c}: {g:?} vs {want:?}");
        }
    }
}

#[cfg(test)]
mod meter_tests {
    use super::*;

    #[test]
    fn percentile_luma_mask_skips_excluded() {
        let mut img = film_core::Image::new(10, 1); // 10 px row
        for i in 0..10 {
            let v = i as f32 / 9.0; // 0.0 .. 1.0
            img.pixels[i] = [v, v, v];
        }
        // Exclude the 5 brightest; the 90th pct of the remaining {0..0.444} is low.
        let keep: Vec<bool> = (0..10).map(|i| i < 5).collect();
        let full = percentile_luma(&img, 0.90, None);
        let masked = percentile_luma(&img, 0.90, Some(&keep));
        assert!(masked < full, "masked={masked} full={full}");
    }

    #[test]
    fn auto_brightness_value_mask_lowers_exposure_drop() {
        // Adding a clear (dark-inverting) border drags the percentile down and pushes
        // exposure up; masking the border must yield a LOWER exposure than including it.
        use film_core::Image;
        let base = [0.5, 0.4, 0.3];
        let clear = [0.95, 0.95, 0.95];
        let scene = [0.2, 0.2, 0.2];
        let mut img = Image::new(40, 40);
        for y in 0..40 {
            for x in 0..40 {
                let border = x < 6 || y < 6 || x >= 34 || y >= 34;
                img.pixels[y * 40 + x] = if border { clear } else { scene };
            }
        }
        let params = default_invert_params();
        let keep: Vec<bool> = (0..1600)
            .map(|i| { let (x, y) = (i % 40, i / 40); !(x < 6 || y < 6 || x >= 34 || y >= 34) })
            .collect();
        let ev_full = auto_brightness_value(&img, &params, base, 2.0, None);
        let ev_masked = auto_brightness_value(&img, &params, base, 2.0, Some(&keep));
        assert!(ev_masked <= ev_full, "masked={ev_masked} full={ev_full}");
    }
}
