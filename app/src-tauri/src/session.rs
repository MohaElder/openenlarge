//! In-memory session: lightweight image records (path + thumbnail + metadata),
//! with decoded working data filled in lazily by `develop_image`.

use crate::metadata::Metadata;
use film_core::Image;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Preview render quality: caps the decoded working-image resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Performance,
    Quality,
}

impl Quality {
    /// Max long-edge (px) for the working image. Quality = no cap.
    pub fn cap(self) -> u32 {
        match self {
            Quality::Performance => 4096,
            Quality::Quality => u32::MAX,
        }
    }
}

#[allow(clippy::derivable_impls)]
impl Default for Quality {
    fn default() -> Self {
        Quality::Performance
    }
}

/// One Point Color sample: a picked target color + per-sample adjustments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointColorSample {
    pub hue: f32,        // 0..360
    pub sat: f32,        // 0..1
    pub lum: f32,        // 0..1
    pub hue_shift: f32,  // −100..100
    pub sat_shift: f32,
    pub lum_shift: f32,
    pub variance: f32,   // −100..100
    pub range: f32,      // 0..100
}

/// Knobs the UI sends for an inversion (mirrors the engine's exposed controls).
#[derive(Debug, Clone, Deserialize)]
pub struct InvertParams {
    pub mode: String,
    pub stock: String,
    /// Per-image film-base override. When set, used verbatim as the orange-mask
    /// base; when None, the develop-time auto base (`Developed.base`) is used.
    #[serde(default)]
    pub base_override: Option<[f32; 3]>,
    /// Exposure in EV stops (−5..5); converted to a multiplier (2^ev) downstream.
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    /// Vestigial: WB is now absolute (Kelvin); the UI "Auto" button reseeds via
    /// the `as_shot_wb` command instead. Kept in the wire contract for now.
    #[allow(dead_code)]
    pub auto_wb: bool,
    /// Kelvin (e.g. 5500) and green↔magenta tint (−150..150).
    pub temp: f32,
    pub tint: f32,
    // Creative finishing (UI −100..100; 0 = identity).
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub vibrance: f32,
    pub saturation: f32,

    // Tone Curve: region sliders (−100..100) + point curves (0..1 control points).
    #[serde(default)] pub tc_highlights: f32,
    #[serde(default)] pub tc_lights: f32,
    #[serde(default)] pub tc_darks: f32,
    #[serde(default)] pub tc_shadows: f32,
    #[serde(default = "identity_curve")] pub tc_curve: Vec<[f32; 2]>,
    #[serde(default = "identity_curve")] pub tc_red: Vec<[f32; 2]>,
    #[serde(default = "identity_curve")] pub tc_green: Vec<[f32; 2]>,
    #[serde(default = "identity_curve")] pub tc_blue: Vec<[f32; 2]>,

    // Color Grading: hue 0..360, sat 0..100, lum −100..100 per region.
    #[serde(default)] pub cg_sh_hue: f32,
    #[serde(default)] pub cg_sh_sat: f32,
    #[serde(default)] pub cg_sh_lum: f32,
    #[serde(default)] pub cg_mid_hue: f32,
    #[serde(default)] pub cg_mid_sat: f32,
    #[serde(default)] pub cg_mid_lum: f32,
    #[serde(default)] pub cg_hi_hue: f32,
    #[serde(default)] pub cg_hi_sat: f32,
    #[serde(default)] pub cg_hi_lum: f32,
    #[serde(default)] pub cg_glob_hue: f32,
    #[serde(default)] pub cg_glob_sat: f32,
    #[serde(default)] pub cg_glob_lum: f32,
    #[serde(default = "default_blending")] pub cg_blending: f32,
    #[serde(default)] pub cg_balance: f32,

    // Color Mixer (HSL): 8 bands × hue/sat/lum, each −100..100.
    #[serde(default)] pub cm_red_hue: f32, #[serde(default)] pub cm_red_sat: f32, #[serde(default)] pub cm_red_lum: f32,
    #[serde(default)] pub cm_orange_hue: f32, #[serde(default)] pub cm_orange_sat: f32, #[serde(default)] pub cm_orange_lum: f32,
    #[serde(default)] pub cm_yellow_hue: f32, #[serde(default)] pub cm_yellow_sat: f32, #[serde(default)] pub cm_yellow_lum: f32,
    #[serde(default)] pub cm_green_hue: f32, #[serde(default)] pub cm_green_sat: f32, #[serde(default)] pub cm_green_lum: f32,
    #[serde(default)] pub cm_aqua_hue: f32, #[serde(default)] pub cm_aqua_sat: f32, #[serde(default)] pub cm_aqua_lum: f32,
    #[serde(default)] pub cm_blue_hue: f32, #[serde(default)] pub cm_blue_sat: f32, #[serde(default)] pub cm_blue_lum: f32,
    #[serde(default)] pub cm_purple_hue: f32, #[serde(default)] pub cm_purple_sat: f32, #[serde(default)] pub cm_purple_lum: f32,
    #[serde(default)] pub cm_magenta_hue: f32, #[serde(default)] pub cm_magenta_sat: f32, #[serde(default)] pub cm_magenta_lum: f32,
    // Point Color: up to 8 samples.
    #[serde(default)] pub pc_samples: Vec<PointColorSample>,
}

/// Default identity tone curve: a straight 0→0, 1→1 line.
pub fn identity_curve() -> Vec<[f32; 2]> {
    vec![[0.0, 0.0], [1.0, 1.0]]
}
fn default_blending() -> f32 { 50.0 }

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
}

/// Decoded working data, present once an image is developed.
pub struct Developed {
    pub working: Image,
    pub thumb: Image,
    pub base: [f32; 3],
}

/// A session image: always has path/metadata/thumbnail; `developed` is lazy.
pub struct CachedImage {
    pub path: String,
    pub file_name: String,
    pub metadata: Metadata,
    pub thumbnail: String,
    pub developed: Option<Developed>,
}

/// A full-res baked (geometry + pre-invert heal) raw-negative buffer awaiting GPU
/// export, plus the dims the frontend uploads at. Held between export_begin and
/// export_pixels so the file is decoded+baked exactly once per export.
pub struct PreparedExport {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>, // half-float RGBA, full-res
}

#[derive(Default)]
pub struct Session {
    pub images: Mutex<HashMap<String, CachedImage>>,
    pub quality: Mutex<Quality>,
    pub cache_dir: Mutex<std::path::PathBuf>,
    pub pending_export: Mutex<Option<PreparedExport>>,
}

impl Session {
    /// Return the path for a given image id's cache sidecar file.
    pub fn cache_path(&self, id: &str) -> std::path::PathBuf {
        self.cache_dir.lock().unwrap().join(format!("{id}.oecache"))
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
            has_ir: img.developed.as_ref().map(|d| d.working.ir.is_some()).unwrap_or(false),
            offline: false,
        };
        self.images.lock().unwrap().insert(id, img);
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_cap_values() {
        assert_eq!(Quality::Performance.cap(), 4096);
        assert_eq!(Quality::Quality.cap(), u32::MAX);
        assert_eq!(Quality::default(), Quality::Performance);
    }

    #[test]
    fn quality_deserializes_from_lowercase() {
        let p: Quality = serde_json::from_str("\"performance\"").unwrap();
        let q: Quality = serde_json::from_str("\"quality\"").unwrap();
        assert_eq!(p, Quality::Performance);
        assert_eq!(q, Quality::Quality);
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
            path: "/x/a.tif".into(), file_name: "a.tif".into(),
            metadata: Metadata::default(), thumbnail: "data:,".into(), developed: None,
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
}
