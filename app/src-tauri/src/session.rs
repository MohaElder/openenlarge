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

/// Knobs the UI sends for an inversion (mirrors the engine's exposed controls).
#[derive(Debug, Clone, Deserialize)]
pub struct InvertParams {
    pub mode: String,
    pub stock: String,
    /// Reserved for manual base-rect sampling from the UI; not yet consumed by
    /// the develop-on-working-image flow.
    #[allow(dead_code)]
    pub base_rect: Option<[usize; 4]>,
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    pub auto_wb: bool,
    pub temp: f32,
    pub tint: f32,
}

/// What the frontend gets per image.
#[derive(Debug, Clone, Serialize)]
pub struct ImageEntry {
    pub id: String,
    pub file_name: String,
    pub thumbnail: String,
    pub metadata: Metadata,
    pub developed: bool,
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

#[derive(Default)]
pub struct Session {
    pub images: Mutex<HashMap<String, CachedImage>>,
    pub next_id: Mutex<u64>,
    pub quality: Mutex<Quality>,
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
            developed: img.developed.is_some(),
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
    fn insert_reports_undeveloped_then_assigns_ids() {
        let s = Session::default();
        let img = CachedImage {
            path: "/x/a.dng".into(),
            file_name: "a.dng".into(),
            metadata: Metadata::default(),
            thumbnail: "data:,".into(),
            developed: None,
        };
        let e = s.insert(img);
        assert_eq!(e.id, "img0");
        assert!(!e.developed);
    }
}
