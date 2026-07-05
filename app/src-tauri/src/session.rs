//! In-memory session: lightweight image records (path + thumbnail + metadata),
//! with decoded working data filled in lazily by `develop_image`.

use crate::metadata::Metadata;
use film_core::Image;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// One Point Color sample: a picked target color + per-sample adjustments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointColorSample {
    pub hue: f32,       // 0..360
    pub sat: f32,       // 0..1
    pub lum: f32,       // 0..1
    pub hue_shift: f32, // −100..100
    pub sat_shift: f32,
    pub lum_shift: f32,
    pub variance: f32, // −100..100
    pub range: f32,    // 0..100
}

/// Knobs the UI sends for an inversion (mirrors the engine's exposed controls).
#[derive(Debug, Clone, Deserialize)]
pub struct InvertParams {
    pub mode: String,
    /// Deserialized from the UI but not consumed by the current engine path; kept on
    /// the wire contract (the frontend's params blob carries it).
    #[allow(dead_code)]
    pub stock: String,
    /// Per-image film-base override. When set, used verbatim as the orange-mask
    /// base; when None, the develop-time auto base (`Developed.base`) is used.
    #[serde(default)]
    pub base_override: Option<[f32; 3]>,
    /// Per-image Cineon `D_max` (density-range / black-point) override. When set,
    /// used verbatim; when None, the engine default (2.0) is used. Set by the
    /// `analyze` command (sampled inside the image-area crop).
    #[serde(default)]
    pub d_max_override: Option<f32>,
    /// Exposure in EV stops (−5..5); converted to a multiplier (2^ev) downstream.
    pub exposure: f32,
    /// black/gamma: legacy Cineon knobs still sent by the UI but not consumed by the
    /// current engine path; kept on the wire contract.
    #[allow(dead_code)]
    pub black: f32,
    #[allow(dead_code)]
    pub gamma: f32,
    /// Vestigial: WB is now absolute (Kelvin); the UI "Auto" button reseeds via
    /// the `as_shot_wb` command instead. Kept in the wire contract for now.
    #[allow(dead_code)]
    pub auto_wb: bool,
    /// Kelvin (e.g. 5500) and green↔magenta tint (−150..150).
    pub temp: f32,
    pub tint: f32,
    /// Hidden auto-WB baseline gains (per-image). The auto estimate populates this
    /// instead of writing temp/tint, so the visible sliders re-zero to neutral with
    /// symmetric headroom. Final WB = wb_baseline × wb_from_kelvin(temp, tint).
    /// Defaults to identity so old edits (no key) render unchanged.
    #[serde(default = "default_wb_baseline")]
    pub wb_baseline: [f32; 3],
    /// True once the user has deliberately set WB (gray-point pick): the auto-WB
    /// reseed (which fires on base/profile changes) must not clobber it. The Auto
    /// button clears it. Backend never reads it; carried for persistence.
    #[serde(default)]
    #[allow(dead_code)]
    pub wb_manual: bool,
    /// HDR preview toggle (per image). Frontend trigger for the gain-map overlay +
    /// encode_hdr; the live render stays SDR regardless.
    #[serde(default)]
    #[allow(dead_code)]
    pub hdr: bool,
    /// Positive passthrough (slide/print): skip inversion, render the scan with
    /// exposure + WB only. Seeded by the develop-time classifier; user-overridable.
    #[serde(default)]
    pub positive: bool,
    /// Spoke/border metering mode for auto-exposure, D_max, and WB: "auto"
    /// (confidence-gated detection), "exclude" (force), or "include" (meter the
    /// full crop — today's behavior). Defaults to "auto" for edits saved before
    /// this key existed.
    #[serde(default = "default_meter_border")]
    pub meter_border: String,
    /// Auto-exposure anchor: "highlight" (bright content — 90th-pct luminance
    /// driven to 0.80) or "midgray" (median luminance driven to 18% gray,
    /// ≈0.46 display) — a fixed-luminance exposure normalization so frames shot
    /// at different exposures land on a comparable brightness (issue #29).
    /// Defaults to "highlight" for edits saved before this key existed.
    #[serde(default = "default_meter_anchor")]
    pub meter_anchor: String,
    // Creative finishing (UI −100..100; 0 = identity).
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub vibrance: f32,
    pub saturation: f32,
    /// Brightness/density (UI −100..100; 0 = identity). Sits between Exposure and
    /// Contrast in the panel. Unlike Exposure (which redistributes via eff_d_max),
    /// this is a plain multiplicative gain on the finished image, mapped through a
    /// log curve so equal slider steps = equal density (log-luminance) steps —
    /// `gain = 10^(b/100 · BRIGHTNESS_DENSITY_RANGE)`. Applied in finish.rs /
    /// shaders.ts before the tone curve. `#[serde(default)]` so pre-existing saved
    /// edits (no `brightness` key) load as 0 = identity.
    #[serde(default)]
    pub brightness: f32,

    // Tone Curve: region sliders (−100..100) + point curves (0..1 control points).
    #[serde(default)]
    pub tc_highlights: f32,
    #[serde(default)]
    pub tc_lights: f32,
    #[serde(default)]
    pub tc_darks: f32,
    #[serde(default)]
    pub tc_shadows: f32,
    #[serde(default = "identity_curve")]
    pub tc_curve: Vec<[f32; 2]>,
    #[serde(default = "identity_curve")]
    pub tc_red: Vec<[f32; 2]>,
    #[serde(default = "identity_curve")]
    pub tc_green: Vec<[f32; 2]>,
    #[serde(default = "identity_curve")]
    pub tc_blue: Vec<[f32; 2]>,

    // Color Grading: hue 0..360, sat 0..100, lum −100..100 per region.
    #[serde(default)]
    pub cg_sh_hue: f32,
    #[serde(default)]
    pub cg_sh_sat: f32,
    #[serde(default)]
    pub cg_sh_lum: f32,
    #[serde(default)]
    pub cg_mid_hue: f32,
    #[serde(default)]
    pub cg_mid_sat: f32,
    #[serde(default)]
    pub cg_mid_lum: f32,
    #[serde(default)]
    pub cg_hi_hue: f32,
    #[serde(default)]
    pub cg_hi_sat: f32,
    #[serde(default)]
    pub cg_hi_lum: f32,
    #[serde(default)]
    pub cg_glob_hue: f32,
    #[serde(default)]
    pub cg_glob_sat: f32,
    #[serde(default)]
    pub cg_glob_lum: f32,
    #[serde(default = "default_blending")]
    pub cg_blending: f32,
    #[serde(default)]
    pub cg_balance: f32,

    // Color Mixer (HSL): 8 bands × hue/sat/lum, each −100..100.
    #[serde(default)]
    pub cm_red_hue: f32,
    #[serde(default)]
    pub cm_red_sat: f32,
    #[serde(default)]
    pub cm_red_lum: f32,
    #[serde(default)]
    pub cm_orange_hue: f32,
    #[serde(default)]
    pub cm_orange_sat: f32,
    #[serde(default)]
    pub cm_orange_lum: f32,
    #[serde(default)]
    pub cm_yellow_hue: f32,
    #[serde(default)]
    pub cm_yellow_sat: f32,
    #[serde(default)]
    pub cm_yellow_lum: f32,
    #[serde(default)]
    pub cm_green_hue: f32,
    #[serde(default)]
    pub cm_green_sat: f32,
    #[serde(default)]
    pub cm_green_lum: f32,
    #[serde(default)]
    pub cm_aqua_hue: f32,
    #[serde(default)]
    pub cm_aqua_sat: f32,
    #[serde(default)]
    pub cm_aqua_lum: f32,
    #[serde(default)]
    pub cm_blue_hue: f32,
    #[serde(default)]
    pub cm_blue_sat: f32,
    #[serde(default)]
    pub cm_blue_lum: f32,
    #[serde(default)]
    pub cm_purple_hue: f32,
    #[serde(default)]
    pub cm_purple_sat: f32,
    #[serde(default)]
    pub cm_purple_lum: f32,
    #[serde(default)]
    pub cm_magenta_hue: f32,
    #[serde(default)]
    pub cm_magenta_sat: f32,
    #[serde(default)]
    pub cm_magenta_lum: f32,
    // Point Color: up to 8 samples.
    #[serde(default)]
    pub pc_samples: Vec<PointColorSample>,
    /// White-balance application mode: "gain" (post-curve display gain, legacy) or
    /// "subtractive" (pre-curve density multiply, optical color-head look). Serde
    /// default "gain" so pre-existing saved edits load exactly as before.
    #[serde(default = "wb_mode_gain")]
    pub wb_mode: String,
    /// Display tone path: "filmic" (legacy S-curve, default) or "faithful" (gamma+shoulder).
    /// Serde default "filmic" so pre-existing saved edits load exactly as before.
    /// Deserialized for the wire/persistence contract; the GPU path reads its own
    /// numeric tone_mode (ResolvedInversion), not this string.
    #[serde(default = "tone_mode_filmic")]
    #[allow(dead_code)]
    pub tone_mode: String,

    // Per-zone white-balance neutralizer. Gains are stored already damped by pz_strength
    // (applied at estimate time in the per_zone_wb command, Task 9). Identity defaults
    // so old session JSON loads as a no-op.
    #[serde(default = "pz_default_enabled")]
    pub pz_enabled: bool,
    #[serde(default = "pz_default_strength")]
    pub pz_strength: f32,
    #[serde(default = "default_wb_baseline")]
    pub pz_sh: [f32; 3],
    #[serde(default = "default_wb_baseline")]
    pub pz_mid: [f32; 3],
    #[serde(default = "default_wb_baseline")]
    pub pz_hi: [f32; 3],
}

/// Default identity tone curve: a straight 0→0, 1→1 line.
pub fn identity_curve() -> Vec<[f32; 2]> {
    vec![[0.0, 0.0], [1.0, 1.0]]
}
fn default_wb_baseline() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn default_meter_border() -> String {
    "auto".to_string()
}
fn default_meter_anchor() -> String {
    "highlight".to_string()
}
fn pz_default_enabled() -> bool {
    true
}
fn pz_default_strength() -> f32 {
    0.7
}
fn default_blending() -> f32 {
    50.0
}
fn wb_mode_gain() -> String {
    "gain".to_string()
}
fn tone_mode_filmic() -> String {
    "filmic".to_string()
}

/// Import-time auto-detected lightbox crop, normalized 0..1 on the source image.
/// Present only on freshly imported entries that had a confident lightbox border.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AutoCropRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// What the frontend gets per image.
#[derive(Debug, Clone, Serialize)]
pub struct ImageEntry {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub thumbnail: String,
    pub metadata: Metadata,
    pub developed: bool,
    pub has_ir: bool,
    /// True when the referenced file is missing on disk (restored from catalog).
    #[serde(default)]
    pub offline: bool,
    /// Develop-time negative/positive classification (true = positive).
    #[serde(default)]
    pub positive: bool,
    /// True when the baked `thumbnail` predates the current engine version (grid
    /// lazily regenerates). A freshly rendered entry is never stale.
    #[serde(default)]
    pub thumb_stale: bool,
    /// Import-time auto lightbox crop, normalized 0..1. None when no confident
    /// border was found. Only set on fresh imports; the frontend applies it once.
    #[serde(default)]
    pub auto_crop: Option<AutoCropRect>,
}

/// Decoded working data, present once an image is developed.
pub struct Developed {
    pub working: Image,
    pub thumb: Image,
    pub base: [f32; 3],
    /// Detector confidence (0..1) for the auto base; low → UI suggests a repoint.
    pub base_confidence: f32,
    /// Develop-time auto Cineon D_max (density range); per-image, stored so the
    /// inversion never recomputes it on every view. Override: InvertParams.d_max_override.
    pub d_max: f32,
    /// Develop-time classification: true = positive scan (no inversion).
    pub positive: bool,
    /// Classifier confidence 0..1 (diagnostic; not currently surfaced in UI).
    #[allow(dead_code)]
    pub positive_confidence: f32,
    /// Per-channel density-neutralisation factors for camera-matrix mode
    /// (calibrate::sample_channel_balance). `[1,1,1]` in the normal path (identity).
    pub channel_balance: [f32; 3],
}

/// A session image: always has path/metadata/thumbnail; `developed` is lazy.
pub struct CachedImage {
    pub path: String,
    pub file_name: String,
    pub metadata: Metadata,
    pub thumbnail: String,
    pub developed: Option<Developed>,
    /// Monotonic LRU tick of the last access (stamped via `Session::touch`); 0 = never
    /// touched. Drives `evict_lru` when the resident `developed` count exceeds the cap.
    pub last_access: u64,
}

/// A full-res baked (geometry + pre-invert heal) raw-negative buffer awaiting GPU
/// export, plus the dims the frontend uploads at. Held between export_begin and
/// export_pixels so the file is decoded+baked exactly once per export.
pub struct PreparedExport {
    /// Source dims at bake time (diagnostic); the frontend uploads at its own dims.
    #[allow(dead_code)]
    pub w: u32,
    #[allow(dead_code)]
    pub h: u32,
    pub bytes: Vec<u8>, // half-float RGBA, full-res
}

/// A finished, upscaled full-res image awaiting save (held between `upscale_image`
/// and `save_upscaled`), plus the metadata to embed as EXIF on save.
pub struct PendingUpscale {
    pub image: Image,
    pub metadata: Metadata,
}

/// One slot of the export decode cache: the full-res decoded source for `path` as
/// it stood at `mtime`. Shared via `Arc` so concurrent export workers borrow one
/// buffer instead of each re-decoding the file.
pub struct DecodeCacheEntry {
    pub path: std::path::PathBuf,
    pub mtime: SystemTime,
    pub image: Arc<Image>,
}

#[derive(Default)]
pub struct Session {
    pub images: Mutex<HashMap<String, CachedImage>>,
    pub cache_dir: Mutex<std::path::PathBuf>,
    /// Single-slot high-res (≤ MAX_GPU_EDGE) decoded raw-negative for the currently
    /// deep-zoomed image, pre-bake. Replaced when a different image zooms. Lets zoom
    /// re-bake on param/dust tweaks without re-decoding. Bounded to ~1 buffer.
    pub zoom_src: Mutex<Option<(String, Image)>>,
    pub pending_export: Mutex<HashMap<String, PreparedExport>>,
    pub pending_upscale: Mutex<Option<PendingUpscale>>,
    /// Single-slot full-res decode cache for export. The resident `developed.working`
    /// buffer is only a ≤2560 proxy, so every export must re-decode the original at
    /// full res; this slot lets repeat exports of one frame (different format/size)
    /// and the GPU→CPU fallback reuse a single decode instead of re-running the
    /// (often multi-second) RAW demosaic. Single-slot by design: a 60MP `Image` is
    /// ~720MB, so a multi-entry cache under CONCURRENCY=4 could exhaust memory. An
    /// `Arc` so it can be cloned into `export_begin`'s `spawn_blocking`.
    pub decode_cache: Arc<Mutex<Option<DecodeCacheEntry>>>,
    /// Cached AI-dust probability map per image id (`(w, h, w*h f32 in [0,1])`).
    /// The detector runs once; the sensitivity slider only re-thresholds + refills.
    pub autodust_prob: Mutex<HashMap<String, (usize, usize, Vec<f32>)>>,
    /// Cached auto-dust-healed baked buffer per image id, keyed by a heal signature
    /// (sensitivity + exclusions). Lets a fine-tune brush stroke re-heal only its own
    /// region instead of re-inpainting every global dust spot. Proxy (fit) tier only;
    /// dropped when the detector prob map is recomputed (geometry/content change) or
    /// on LRU eviction.
    pub autodust_healed: Mutex<HashMap<String, (String, Image)>>,
    /// Monotonic counter for LRU access ticks (see `CachedImage::last_access`).
    pub access_tick: AtomicU64,
}

/// Max resident decoded `developed` buffers (LRU-evicted beyond this). Proxies are
/// ≤2560 (~17 MB), so ~24 ≈ ~400 MB worst case. The lightweight `CachedImage` record
/// (path/metadata/thumbnail) is never evicted; evicted buffers re-hydrate from cache.
const MAX_RESIDENT_DEVELOPED: usize = 24;

impl Session {
    /// Return the path for a given image id's cache sidecar file.
    pub fn cache_path(&self, id: &str) -> std::path::PathBuf {
        self.cache_dir.lock().unwrap().join(format!("{id}.oecache"))
    }

    /// Next monotonic LRU tick. Stamp it into `CachedImage::last_access` while holding
    /// the `images` lock (this method itself takes no lock — safe to call under it).
    pub fn next_tick(&self) -> u64 {
        self.access_tick.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// LRU-evict resident `developed` buffers beyond `MAX_RESIDENT_DEVELOPED`, never
    /// evicting `keep_id` (the just-inserted/just-touched image). Drops each evicted
    /// image's `developed` and its `autodust_prob` map. Returns the evicted ids.
    /// Call only after a new `developed` is inserted (the resident set just grew).
    pub fn evict_lru(&self, keep_id: &str) -> Vec<String> {
        let evicted: Vec<String> = {
            let mut images = self.images.lock().unwrap();
            let mut resident: Vec<(String, u64)> = images
                .iter()
                .filter(|(_, c)| c.developed.is_some())
                .map(|(id, c)| (id.clone(), c.last_access))
                .collect();
            if resident.len() <= MAX_RESIDENT_DEVELOPED {
                return Vec::new();
            }
            resident.sort_by_key(|(_, tick)| *tick); // oldest first
            let overflow = resident.len() - MAX_RESIDENT_DEVELOPED;
            let ids: Vec<String> = resident
                .into_iter()
                .filter(|(id, _)| id != keep_id)
                .take(overflow)
                .map(|(id, _)| id)
                .collect();
            for id in &ids {
                if let Some(c) = images.get_mut(id) {
                    c.developed = None;
                }
            }
            ids
        }; // images lock released here
        if !evicted.is_empty() {
            let mut probs = self.autodust_prob.lock().unwrap();
            let mut healed = self.autodust_healed.lock().unwrap();
            for id in &evicted {
                probs.remove(id);
                healed.remove(id);
            }
        }
        evicted
    }

    /// Insert a cached image under an explicit (catalog-assigned) id.
    pub fn insert_with_id(&self, id: String, img: CachedImage) -> ImageEntry {
        let entry = ImageEntry {
            id: id.clone(),
            path: img.path.clone(),
            file_name: img.file_name.clone(),
            thumbnail: img.thumbnail.clone(),
            metadata: img.metadata.clone(),
            developed: img.developed.is_some(),
            has_ir: img
                .developed
                .as_ref()
                .map(|d| d.working.ir.is_some())
                .unwrap_or(false),
            offline: false,
            positive: false,
            thumb_stale: false,
            auto_crop: None,
        };
        self.images.lock().unwrap().insert(id, img);
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evict_lru_drops_oldest_beyond_cap() {
        let s = Session::default();
        let dummy = || Developed {
            working: Image::new(1, 1),
            thumb: Image::new(1, 1),
            base: [0.0; 3],
            base_confidence: 0.0,
            d_max: 0.0,
            channel_balance: [1.0; 3],
            positive: false,
            positive_confidence: 0.0,
        };
        let total = MAX_RESIDENT_DEVELOPED + 3;
        {
            let mut images = s.images.lock().unwrap();
            for i in 0..total {
                images.insert(
                    format!("img{i}"),
                    CachedImage {
                        path: format!("/x/{i}"),
                        file_name: format!("{i}"),
                        metadata: Metadata::default(),
                        thumbnail: "data:,".into(),
                        developed: Some(dummy()),
                        last_access: i as u64, // ascending: img0 oldest … last newest
                    },
                );
            }
        }
        let keep = format!("img{}", total - 1); // newest
        let evicted = s.evict_lru(&keep);
        assert_eq!(evicted.len(), 3);
        for i in 0..3 {
            assert!(evicted.contains(&format!("img{i}")), "img{i} should be evicted");
        }
        let images = s.images.lock().unwrap();
        let resident = images.values().filter(|c| c.developed.is_some()).count();
        assert_eq!(resident, MAX_RESIDENT_DEVELOPED);
        assert!(images.get(&keep).unwrap().developed.is_some(), "keep_id survives");
        assert!(images.get("img0").unwrap().developed.is_none(), "oldest evicted");
        // The evicted record itself remains (only `developed` was dropped).
        assert!(images.contains_key("img0"));
    }

    #[test]
    fn insert_reports_undeveloped() {
        let s = Session::default();
        let img = CachedImage {
            path: "/x/a.dng".into(),
            file_name: "a.dng".into(),
            metadata: Metadata::default(),
            thumbnail: "data:,".into(),
            developed: None,
            last_access: 0,
        };
        let e = s.insert_with_id("abc".into(), img);
        assert_eq!(e.id, "abc");
        assert!(!e.developed);
        assert!(!e.offline);
    }

    #[test]
    fn insert_reports_has_ir_false_when_undeveloped() {
        let s = Session::default();
        let img = CachedImage {
            path: "/x/a.tif".into(),
            file_name: "a.tif".into(),
            metadata: Metadata::default(),
            thumbnail: "data:,".into(),
            developed: None,
            last_access: 0,
        };
        let e = s.insert_with_id("xyz".into(), img);
        assert!(!e.has_ir);
    }

    #[test]
    fn invert_params_backfills_missing_fields_via_serde_default() {
        // An "old" catalog blob saved before color-grading/tone-curve fields existed.
        let old = r#"{
            "mode":"b","stock":"none","base_rect":null,
            "exposure":0.0,"black":0.0,"gamma":0.4545,"auto_wb":true,
            "temp":5500.0,"tint":0.0,"contrast":0.0,"highlights":0.0,
            "shadows":0.0,"whites":0.0,"blacks":0.0,"texture":0.0,
            "vibrance":0.0,"saturation":0.0
        }"#;
        let p: InvertParams = serde_json::from_str(old).unwrap();
        assert_eq!(p.cg_blending, 50.0); // defaulted
        assert_eq!(p.tc_curve, super::identity_curve()); // defaulted
    }

    #[test]
    fn invert_params_defaults_wb_baseline_to_identity_from_old_json() {
        // Old session JSON has no `wb_baseline` key. Serde must fill the identity
        // default so existing edits render with WB == wb_from_kelvin(temp,tint).
        let json = r#"{
            "mode":"d","stock":"none","exposure":0.0,"black":0.0,"gamma":0.4545,
            "auto_wb":true,"temp":5500.0,"tint":0.0,"wb_manual":false,
            "wb_mode":"gain","tone_mode":"faithful","contrast":0.0,"highlights":0.0,
            "shadows":0.0,"whites":0.0,"blacks":0.0,"texture":0.0,
            "vibrance":0.0,"saturation":0.0
        }"#;
        let p: InvertParams = serde_json::from_str(json).expect("must deserialize old JSON");
        assert_eq!(p.wb_baseline, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn per_zone_params_default_from_old_json() {
        // Old session JSON without any pz_* keys — serde must fill identity defaults.
        let json = r#"{
            "mode":"d","stock":"none","exposure":0.0,"black":0.0,"gamma":0.4545,
            "auto_wb":true,"temp":5500.0,"tint":0.0,"wb_manual":false,
            "wb_mode":"gain","tone_mode":"faithful",
            "contrast":0.0,"highlights":0.0,"shadows":0.0,"whites":0.0,"blacks":0.0,
            "texture":0.0,"vibrance":0.0,"saturation":0.0
        }"#;
        let p: InvertParams = serde_json::from_str(json).unwrap();
        assert!(p.pz_enabled);
        assert_eq!(p.pz_sh, [1.0, 1.0, 1.0]);
        assert_eq!(p.pz_mid, [1.0, 1.0, 1.0]);
        assert_eq!(p.pz_hi, [1.0, 1.0, 1.0]);
        assert!((p.pz_strength - 0.7).abs() < 1e-6);
    }

    #[test]
    fn meter_border_defaults_to_auto() {
        // An old saved edit with no meter_border key must load as "auto".
        let json = r#"{"mode":"c","stock":"none","exposure":0.0,"black":0.0,"gamma":1.0,
            "auto_wb":false,"temp":5500.0,"tint":0.0,"contrast":0.0,"highlights":0.0,
            "shadows":0.0,"whites":0.0,"blacks":0.0,"texture":0.0,"vibrance":0.0,
            "saturation":0.0}"#;
        let p: InvertParams = serde_json::from_str(json).expect("parse");
        assert_eq!(p.meter_border, "auto");
    }

    #[test]
    fn meter_anchor_defaults_to_highlight() {
        // An old saved edit with no meter_anchor key must load as "highlight".
        let json = r#"{"mode":"c","stock":"none","exposure":0.0,"black":0.0,"gamma":1.0,
            "auto_wb":false,"temp":5500.0,"tint":0.0,"contrast":0.0,"highlights":0.0,
            "shadows":0.0,"whites":0.0,"blacks":0.0,"texture":0.0,"vibrance":0.0,
            "saturation":0.0}"#;
        let p: InvertParams = serde_json::from_str(json).expect("parse");
        assert_eq!(p.meter_anchor, "highlight");
    }
}
