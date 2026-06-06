//! Tethered watch-folder: watch a directory, emit an event per fully-written scan.

use std::path::Path;
use std::time::{Duration, Instant};

/// Block until `path`'s size is unchanged across two consecutive reads spaced by
/// `poll`, or until `max_wait` elapses. Returns true once stable, false on
/// timeout or if the file never becomes readable. Cheap: just stats the file.
pub fn wait_until_stable(path: &Path, poll: Duration, max_wait: Duration) -> bool {
    let deadline = Instant::now() + max_wait;
    let mut last: Option<u64> = None;
    loop {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
        let size = std::fs::metadata(path).map(|m| m.len()).ok();
        match (last, size) {
            (Some(prev), Some(cur)) if prev == cur && cur > 0 => return true,
            _ => {}
        }
        last = size;
    }
}

/// File extensions we treat as scans, lowercase, no dot. Mirrors the import
/// dialog filter in `panels/Source.svelte`.
const SCAN_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "dng", "tif", "tiff", "raf", "rw2", "nef", "arw", "cr3", "3fr", "raw",
];

/// True if `file_name` is a scan we should auto-develop: a known image extension,
/// not a hidden dotfile, not an editor/OS temp, not an XMP sidecar.
pub fn is_accepted_scan(file_name: &str) -> bool {
    // Hidden dotfiles and tilde temp files are never scans.
    if file_name.starts_with('.') || file_name.starts_with('~') {
        return false;
    }
    let lower = file_name.to_ascii_lowercase();
    // Reject common in-progress/temp suffixes that wrap a real name.
    if lower.ends_with(".tmp") || lower.ends_with(".part") || lower.ends_with(".xmp") {
        return false;
    }
    match lower.rsplit_once('.') {
        Some((_, ext)) => SCAN_EXTS.contains(&ext),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn accepts_known_raw_and_image_extensions() {
        assert!(is_accepted_scan("DSCF1234.RAF"));
        assert!(is_accepted_scan("IMG_0001.dng"));
        assert!(is_accepted_scan("scan.tiff"));
        assert!(is_accepted_scan("frame.JPG"));
    }

    #[test]
    fn rejects_unknown_extensions_and_no_extension() {
        assert!(!is_accepted_scan("notes.txt"));
        assert!(!is_accepted_scan("Makefile"));
        assert!(!is_accepted_scan("movie.mov"));
    }

    #[test]
    fn rejects_sidecars_hidden_and_temp_files() {
        assert!(!is_accepted_scan("DSCF1234.xmp"));
        assert!(!is_accepted_scan(".DS_Store"));
        assert!(!is_accepted_scan(".hidden.dng"));
        assert!(!is_accepted_scan("DSCF1234.dng.tmp"));
        assert!(!is_accepted_scan("~temp.dng"));
    }

    #[test]
    fn stable_returns_true_for_a_complete_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("done.dng");
        std::fs::write(&p, b"already fully written").unwrap();
        // Short cadence so the test is fast; file is already stable.
        assert!(wait_until_stable(&p, Duration::from_millis(10), Duration::from_secs(2)));
    }

    #[test]
    fn stable_returns_false_for_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.dng");
        assert!(!wait_until_stable(&p, Duration::from_millis(10), Duration::from_millis(80)));
    }

    #[test]
    fn stable_waits_out_a_growing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("growing.dng");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"chunk1").unwrap();
        f.flush().unwrap();
        let p2 = p.clone();
        // Append once more after a beat, then stop growing.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut g = std::fs::OpenOptions::new().append(true).open(&p2).unwrap();
            g.write_all(b"chunk2").unwrap();
            g.flush().unwrap();
        });
        assert!(wait_until_stable(&p, Duration::from_millis(20), Duration::from_secs(2)));
        // Final size reflects both chunks (gate didn't fire mid-write).
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 12);
    }
}
