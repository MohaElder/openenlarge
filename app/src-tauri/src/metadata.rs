//! Best-effort image metadata for the Library panel. Camera/lens/exposure come
//! from rawler when the file is a RAW/DNG; dimensions + file size are always set.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Metadata {
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<String>,
    pub shutter: Option<String>,
    pub aperture: Option<String>,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub date: Option<String>,
    /// Free-form note (EXIF ImageDescription). Not read from RAW EXIF — populated
    /// only by user edits via the metadata override. `#[serde(default)]` so catalog
    /// rows written before this field deserialize cleanly.
    #[serde(default)]
    pub note: Option<String>,
}

/// `width`/`height` come from the already-decoded image (authoritative); the rest
/// is best-effort from rawler EXIF and may be None. Never panics; rawler failure
/// is non-fatal.
pub fn extract(path: &Path, width: u32, height: u32) -> Metadata {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut md = Metadata { width, height, file_size, ..Default::default() };

    // rawler 0.7.2: EXIF is exposed via `RawMetadata` (obtained from a decoder),
    // NOT from `decode_file`'s `RawImage` (which has clean_make/clean_model but no
    // `exif` field). We build a RawSource, pick a decoder, and ask for metadata.
    // Any failure (non-RAW file, unsupported format) is non-fatal -> only
    // dims/size are set. We swallow panics too, since some decoders may panic on
    // malformed input.
    let meta = std::panic::catch_unwind(|| {
        let source = rawler::rawsource::RawSource::new(path).ok()?;
        let decoder = rawler::get_decoder(&source).ok()?;
        decoder
            .raw_metadata(&source, &rawler::decoders::RawDecodeParams::default())
            .ok()
    })
    .ok()
    .flatten();

    if let Some(meta) = meta {
        let make = meta.make.trim().to_string();
        let model = meta.model.trim().to_string();
        let cam = format!("{make} {model}").trim().to_string();
        if !cam.is_empty() {
            md.camera = Some(cam);
        }

        let exif = &meta.exif;
        // ISO: rawler has no single `iso` field; prefer iso_speed_ratings (u16),
        // fall back to iso_speed (u32).
        if let Some(iso) = exif.iso_speed_ratings {
            md.iso = Some(iso.to_string());
        } else if let Some(iso) = exif.iso_speed {
            md.iso = Some(iso.to_string());
        }
        // exposure_time / fnumber are `Rational { n: u32, d: u32 }`.
        if let Some(et) = exif.exposure_time {
            md.shutter = Some(format!("{}/{}", et.n, et.d));
        }
        if let Some(fnum) = exif.fnumber {
            md.aperture = Some(format!("f/{:.1}", fnum.n as f32 / fnum.d as f32));
        }
        if let Some(lens) = exif.lens_model.clone() {
            if !lens.is_empty() {
                md.lens = Some(lens);
            }
        }
        if let Some(d) = exif.date_time_original.clone() {
            if !d.is_empty() {
                md.date = Some(d);
            }
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dims_and_size_always_set_even_without_exif() {
        // This source file is not a RAW → rawler fails → only dims/size set.
        let p = Path::new(file!());
        let md = extract(p, 1234, 567);
        assert_eq!(md.width, 1234);
        assert_eq!(md.height, 567);
        assert!(md.file_size > 0);
        assert!(md.camera.is_none());
    }
}
