//! In-memory session: decoded images (full-res + proxy) keyed by id, plus serde
//! types shared with the frontend.

use crate::metadata::Metadata;
use film_core::Image;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Deserialize)]
pub struct InvertParams {
    pub mode: String,            // "b" | "c"
    pub stock: String,           // "none" | "portra400" | "fujic200"
    pub base_rect: Option<[usize; 4]>,
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    pub auto_wb: bool,
    pub temp: f32,
    pub tint: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageEntry {
    pub id: String,
    pub file_name: String,
    pub thumbnail: String,
    pub metadata: Metadata,
}

pub struct CachedImage {
    pub full_res: Image,
    pub proxy: Image,
    pub thumb_img: Image,
    /// Film base (orange mask), sampled once at import — avoids re-sorting the
    /// whole proxy on every render.
    pub base: [f32; 3],
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
        CachedImage { full_res: img.clone(), proxy: img.clone(), thumb_img: img, base: [1.0; 3],
            file_name: name.to_string(), metadata: Metadata::default(), thumbnail: "data:,".to_string() }
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
