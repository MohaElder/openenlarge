//! Best-effort EXIF embedding for exported files (JPEG/TIFF/PNG) via little_exif.
//!
//! The exporter writes pixels first, then this pass injects the (possibly
//! user-edited) metadata. Every step is non-fatal: a parse miss skips one tag and
//! a write failure is reported but never aborts the export — the pixel file is
//! already on disk and valid.

use crate::metadata::Metadata;
use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata as ExifMetadata;
use little_exif::rational::uR64;
use std::path::Path;

/// Embed `meta` into the file at `path`. Returns Err only so the caller can log;
/// it intentionally does not propagate as an export failure.
pub fn write_exif(path: &Path, meta: &Metadata) -> Result<(), String> {
    // Start from the file's existing EXIF when present. This matters for TIFF,
    // whose EXIF IFD is the image's own IFD: little_exif requires its structural
    // tags (ImageWidth, …) to already be there. JPEG/PNG we just wrote have no
    // EXIF yet, so fall back to a fresh block.
    let mut exif = ExifMetadata::new_from_path(path).unwrap_or_else(|_| ExifMetadata::new());

    if let Some(camera) = nonempty(&meta.camera) {
        exif.set_tag(ExifTag::Model(camera.to_string()));
    }
    if let Some(lens) = nonempty(&meta.lens) {
        exif.set_tag(ExifTag::LensModel(lens.to_string()));
    }
    if let Some(iso) = nonempty(&meta.iso).and_then(parse_iso) {
        exif.set_tag(ExifTag::ISO(vec![iso]));
    }
    if let Some(r) = nonempty(&meta.shutter).and_then(parse_shutter) {
        exif.set_tag(ExifTag::ExposureTime(vec![r]));
    }
    if let Some(r) = nonempty(&meta.aperture).and_then(parse_aperture) {
        exif.set_tag(ExifTag::FNumber(vec![r]));
    }
    if let Some(date) = nonempty(&meta.date) {
        exif.set_tag(ExifTag::DateTimeOriginal(to_exif_datetime(date)));
    }
    if let Some(note) = nonempty(&meta.note) {
        exif.set_tag(ExifTag::ImageDescription(note.to_string()));
    }

    exif.write_to_file(path).map_err(|e| format!("exif write: {e}"))
}

/// Trim and drop empty strings → `None`.
fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// ISO digits → u16 (EXIF `ISO` is INT16U). Out-of-range/garbage → None.
fn parse_iso(s: &str) -> Option<u16> {
    s.trim().parse::<u32>().ok().and_then(|v| u16::try_from(v).ok())
}

/// Shutter "1/250" → 1/250; a decimal like "0.004" → 4/1000. None on garbage.
fn parse_shutter(s: &str) -> Option<uR64> {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let n: u32 = n.trim().parse().ok()?;
        let d: u32 = d.trim().parse().ok()?;
        if d == 0 { return None; }
        return Some(uR64 { nominator: n, denominator: d });
    }
    let v: f32 = s.parse().ok()?;
    decimal_to_rational(v)
}

/// Aperture "f/2.8" / "2.8" → 28/10. None on garbage.
fn parse_aperture(s: &str) -> Option<uR64> {
    let v: f32 = s.trim().trim_start_matches(['f', 'F', '/']).trim().parse().ok()?;
    decimal_to_rational(v)
}

/// A non-negative decimal → a /100 (or finer) rational. None for negatives/NaN.
fn decimal_to_rational(v: f32) -> Option<uR64> {
    if !v.is_finite() || v < 0.0 { return None; }
    Some(uR64 { nominator: (v * 100.0).round() as u32, denominator: 100 })
}

/// Normalize a date to EXIF form `YYYY:MM:DD HH:MM:SS`. Accepts both the EXIF
/// form (passthrough) and the HTML `datetime-local` form `YYYY-MM-DDTHH:MM`.
fn to_exif_datetime(s: &str) -> String {
    let s = s.trim().replace('T', " ");
    let (date, time) = match s.split_once(' ') {
        Some((d, t)) => (d.to_string(), t.to_string()),
        None => (s.clone(), String::new()),
    };
    // YYYY-MM-DD → YYYY:MM:DD (EXIF dates already use ':', leaving them untouched).
    let date = date.replacen('-', ":", 2);
    let time = if time.is_empty() {
        "00:00:00".to_string()
    } else if time.matches(':').count() == 1 {
        format!("{time}:00") // HH:MM → HH:MM:00
    } else {
        time
    };
    format!("{date} {time}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fractional_shutter() {
        let r = parse_shutter("1/250").unwrap();
        assert_eq!((r.nominator, r.denominator), (1, 250));
    }

    #[test]
    fn parses_decimal_shutter() {
        let r = parse_shutter("0.5").unwrap();
        assert_eq!((r.nominator, r.denominator), (50, 100));
    }

    #[test]
    fn rejects_zero_denominator_and_garbage() {
        assert!(parse_shutter("1/0").is_none());
        assert!(parse_shutter("abc").is_none());
    }

    #[test]
    fn parses_aperture_with_and_without_prefix() {
        assert_eq!(parse_aperture("f/2.8").unwrap().nominator, 280);
        assert_eq!(parse_aperture("2.8").unwrap().nominator, 280);
    }

    #[test]
    fn parses_iso_in_range_only() {
        assert_eq!(parse_iso("400"), Some(400));
        assert_eq!(parse_iso("100000"), None); // exceeds u16
        assert_eq!(parse_iso("x"), None);
    }

    #[test]
    fn datetime_local_to_exif() {
        assert_eq!(to_exif_datetime("2024-05-01T13:45"), "2024:05:01 13:45:00");
    }

    #[test]
    fn exif_datetime_passthrough() {
        assert_eq!(to_exif_datetime("2024:05:01 13:45:30"), "2024:05:01 13:45:30");
    }

    #[test]
    fn date_only_gets_midnight() {
        assert_eq!(to_exif_datetime("2024-05-01"), "2024:05:01 00:00:00");
    }

    #[test]
    fn nonempty_trims_and_drops_blank() {
        assert_eq!(nonempty(&Some("  hi ".into())), Some("hi"));
        assert_eq!(nonempty(&Some("   ".into())), None);
        assert_eq!(nonempty(&None), None);
    }

    fn sample_meta() -> Metadata {
        Metadata {
            camera: Some("Leica M6".into()),
            lens: Some("Summicron 50".into()),
            iso: Some("400".into()),
            shutter: Some("1/250".into()),
            aperture: Some("f/2.8".into()),
            date: Some("2024-05-01T13:45".into()),
            note: Some("roll 12, sunny 16".into()),
            ..Default::default()
        }
    }

    // Write a tiny RGB file, embed EXIF, read the tags back. Asserts the export
    // actually carries metadata for the given container format.
    fn round_trip(ext: &str) {
        let path = std::env::temp_dir()
            .join(format!("oe-exif-{}-{}.{ext}", std::process::id(), ext));
        let _ = std::fs::remove_file(&path);
        let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
            image::ImageBuffer::from_pixel(4, 4, image::Rgb([128, 64, 32]));
        img.save(&path).expect("write base image");

        write_exif(&path, &sample_meta()).expect("embed exif");

        let read = ExifMetadata::new_from_path(&path).expect("read exif back");
        let tags: Vec<ExifTag> = (&read).into_iter().cloned().collect();
        let clean = |s: &str| s.trim_matches(char::from(0)).trim().to_string();

        let model = tags.iter().find_map(|t| match t {
            ExifTag::Model(s) => Some(clean(s)),
            _ => None,
        });
        assert_eq!(model.as_deref(), Some("Leica M6"), "{ext}: Model");

        let note = tags.iter().find_map(|t| match t {
            ExifTag::ImageDescription(s) => Some(clean(s)),
            _ => None,
        });
        assert_eq!(note.as_deref(), Some("roll 12, sunny 16"), "{ext}: ImageDescription");

        let date = tags.iter().find_map(|t| match t {
            ExifTag::DateTimeOriginal(s) => Some(clean(s)),
            _ => None,
        });
        assert_eq!(date.as_deref(), Some("2024:05:01 13:45:00"), "{ext}: DateTimeOriginal");

        let shutter = tags.iter().find_map(|t| match t {
            ExifTag::ExposureTime(v) => v.first().map(|r| (r.nominator, r.denominator)),
            _ => None,
        });
        assert_eq!(shutter, Some((1, 250)), "{ext}: ExposureTime");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn jpeg_round_trips_exif() { round_trip("jpg"); }

    #[test]
    fn png_round_trips_exif() { round_trip("png"); }

    #[test]
    fn tiff_round_trips_exif() { round_trip("tiff"); }
}
