//! Per-image decoded-working cache: write/read a zstd-compressed binary sidecar
//! (`.oecache`) holding `base + working + thumb` so images come back developed on
//! relaunch without re-decoding the original RAW.
//!
//! File layout:
//!   [has_ir: u8]  (uncompressed, 0 or 1; mirrors the working image's IR presence)
//!   [zstd payload…]
//!
//! Payload (zstd-compressed):
//!   base: 3 × f32 LE
//!   working: Image serialised as width u32 LE, height u32 LE, has_ir u8,
//!            pixels (w*h*3 f32 LE), [ir w*h f32 LE if has_ir]
//!   thumb: same Image encoding

use film_core::Image;
use std::io::{self, Read, Write};
use std::path::Path;

const ZSTD_LEVEL: i32 = 3;

// ---------------------------------------------------------------------------
// Serialise / deserialise a single Image into/from a byte buffer.
// ---------------------------------------------------------------------------

fn encode_image(img: &Image, buf: &mut Vec<u8>) {
    buf.extend_from_slice(&(img.width as u32).to_le_bytes());
    buf.extend_from_slice(&(img.height as u32).to_le_bytes());
    let has_ir: u8 = if img.ir.is_some() { 1 } else { 0 };
    buf.push(has_ir);
    for px in &img.pixels {
        for &ch in px.iter() {
            buf.extend_from_slice(&ch.to_le_bytes());
        }
    }
    if let Some(ir) = &img.ir {
        for &v in ir.iter() {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
}

fn decode_image(cur: &mut io::Cursor<&[u8]>) -> io::Result<Image> {
    let width = read_u32_le(cur)? as usize;
    let height = read_u32_le(cur)? as usize;
    let has_ir = read_u8(cur)?;
    let n = width
        .checked_mul(height)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "cache dims overflow"))?;
    let remaining = (cur.get_ref().len()).saturating_sub(cur.position() as usize);
    let per_pixel: usize = if has_ir == 1 { 16 } else { 12 }; // 3×f32 RGB (+ 1×f32 IR)
    let needed = n
        .checked_mul(per_pixel)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "cache size overflow"))?;
    if needed > remaining {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "cache truncated/corrupt"));
    }
    let mut pixels = Vec::with_capacity(n);
    for _ in 0..n {
        let r = read_f32_le(cur)?;
        let g = read_f32_le(cur)?;
        let b = read_f32_le(cur)?;
        pixels.push([r, g, b]);
    }
    let ir = if has_ir == 1 {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(read_f32_le(cur)?);
        }
        Some(v)
    } else {
        None
    };
    Ok(Image { width, height, pixels, ir })
}

// ---------------------------------------------------------------------------
// Low-level read helpers over an io::Cursor<&[u8]>
// ---------------------------------------------------------------------------

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32_le(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_f32_le(r: &mut impl Read) -> io::Result<f32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(f32::from_le_bytes(b))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Write a cache file at `path` encoding `(base, working, thumb)`.
/// The working image's `has_ir` flag is stored uncompressed as the first byte.
pub fn write(path: &Path, base: [f32; 3], working: &Image, thumb: &Image) -> io::Result<()> {
    let has_ir_byte: u8 = if working.ir.is_some() { 1 } else { 0 };

    // Build the payload to compress.
    let mut payload: Vec<u8> = Vec::new();
    for &v in base.iter() {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    encode_image(working, &mut payload);
    encode_image(thumb, &mut payload);

    let compressed = zstd::encode_all(payload.as_slice(), ZSTD_LEVEL)
        .map_err(io::Error::other)?;

    let mut file = std::fs::File::create(path)?;
    file.write_all(&[has_ir_byte])?;
    file.write_all(&compressed)?;
    Ok(())
}

/// Read a cache file and return `(base, working, thumb)`.
pub fn read(path: &Path) -> io::Result<([f32; 3], Image, Image)> {
    let raw = std::fs::read(path)?;
    if raw.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty cache file"));
    }
    // Skip the leading has_ir byte; the full image encoding already encodes this.
    let compressed = &raw[1..];
    let payload = zstd::decode_all(compressed)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut cur = io::Cursor::new(payload.as_slice());
    let base = [read_f32_le(&mut cur)?, read_f32_le(&mut cur)?, read_f32_le(&mut cur)?];
    let working = decode_image(&mut cur)?;
    let thumb = decode_image(&mut cur)?;
    Ok((base, working, thumb))
}

/// Fast path: open the file and read only the first byte to check IR presence.
/// Returns `false` if the file is missing or unreadable.
pub fn read_has_ir(path: &Path) -> io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    Ok(byte[0] == 1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_image_no_ir(w: usize, h: usize) -> Image {
        let mut pixels = Vec::with_capacity(w * h);
        for i in 0..(w * h) {
            pixels.push([i as f32 * 0.01, i as f32 * 0.02, i as f32 * 0.03]);
        }
        Image { width: w, height: h, pixels, ir: None }
    }

    fn make_image_with_ir(w: usize, h: usize) -> Image {
        let mut img = make_image_no_ir(w, h);
        img.ir = Some((0..(w * h)).map(|i| i as f32 * 0.001).collect());
        img
    }

    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "oe-cache-test-{}-{}.oecache",
            std::process::id(),
            suffix
        ))
    }

    #[test]
    fn roundtrip_without_ir() {
        let path = tmp_path("no-ir");
        let working = make_image_no_ir(3, 2);
        let thumb = make_image_no_ir(2, 1);
        let base: [f32; 3] = [0.1, 0.2, 0.3];

        write(&path, base, &working, &thumb).expect("write ok");
        let (rb, rw, rt) = read(&path).expect("read ok");

        assert_eq!(rb, base);
        assert_eq!(rw.width, working.width);
        assert_eq!(rw.height, working.height);
        assert_eq!(rw.pixels, working.pixels);
        assert!(rw.ir.is_none());
        assert_eq!(rt.width, thumb.width);
        assert_eq!(rt.height, thumb.height);
        assert_eq!(rt.pixels, thumb.pixels);
        assert!(rt.ir.is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn roundtrip_with_ir() {
        let path = tmp_path("with-ir");
        let working = make_image_with_ir(3, 2);
        let thumb = make_image_no_ir(2, 1);
        let base: [f32; 3] = [0.5, 0.6, 0.7];

        write(&path, base, &working, &thumb).expect("write ok");
        let (rb, rw, rt) = read(&path).expect("read ok");

        assert_eq!(rb, base);
        assert_eq!(rw.width, working.width);
        assert_eq!(rw.height, working.height);
        assert_eq!(rw.pixels, working.pixels);
        let ir_out = rw.ir.expect("ir should be present");
        let ir_in = working.ir.as_ref().unwrap();
        assert_eq!(ir_out.len(), ir_in.len());
        for (a, b) in ir_out.iter().zip(ir_in.iter()) {
            assert_eq!(a.to_bits(), b.to_bits(), "ir values must be bit-exact");
        }
        assert_eq!(rt.pixels, thumb.pixels);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_has_ir_reflects_working_ir_presence() {
        let path_with = tmp_path("has-ir-check-yes");
        let path_without = tmp_path("has-ir-check-no");

        let with_ir = make_image_with_ir(3, 2);
        let without_ir = make_image_no_ir(3, 2);
        let thumb = make_image_no_ir(2, 1);
        let base: [f32; 3] = [0.1, 0.2, 0.3];

        write(&path_with, base, &with_ir, &thumb).expect("write with-ir ok");
        write(&path_without, base, &without_ir, &thumb).expect("write without-ir ok");

        assert!(read_has_ir(&path_with).expect("read_has_ir with-ir"), "should be true when working has IR");
        assert!(!read_has_ir(&path_without).expect("read_has_ir without-ir"), "should be false when working has no IR");

        let _ = std::fs::remove_file(&path_with);
        let _ = std::fs::remove_file(&path_without);
    }

    #[test]
    fn read_rejects_corrupt_dims_without_panicking() {
        // Round-trip a tiny image, then overwrite the file so the stored
        // dimensions claim far more pixels than the payload contains.
        let path = std::env::temp_dir().join(format!(
            "oe-cache-corrupt-{}.oecache",
            std::process::id()
        ));
        let img = make_image_no_ir(2, 2);
        let thumb = make_image_no_ir(1, 1);
        write(&path, [0.0; 3], &img, &thumb).unwrap();

        // Build a payload with a valid header but no pixel data (truncated):
        // layout matches write(): base 3×f32, then working: width u32, height u32, has_ir u8,
        // (no pixels), then thumb would follow — but we stop here to simulate truncation.
        let mut payload: Vec<u8> = Vec::new();
        for &v in [0.0f32, 0.0, 0.0].iter() {
            payload.extend_from_slice(&v.to_le_bytes()); // base r, g, b
        }
        payload.extend_from_slice(&9999u32.to_le_bytes()); // working width
        payload.extend_from_slice(&9999u32.to_le_bytes()); // working height
        payload.push(0u8);                                 // working has_ir = false
        // (no pixel data — truncated)
        let comp = zstd::encode_all(&payload[..], 3).unwrap();
        let mut bytes = vec![0u8]; // leading has_ir byte (uncompressed)
        bytes.extend_from_slice(&comp);
        std::fs::write(&path, &bytes).unwrap();

        let res = read(&path);
        let _ = std::fs::remove_file(&path);
        assert!(res.is_err(), "corrupt cache must return Err, not panic/abort");
    }

    #[test]
    fn pixels_are_bit_exact_after_roundtrip() {
        let path = tmp_path("bit-exact");
        // Use specific bit patterns to ensure no float rounding.
        let px = [
            [0.25f32, 0.5f32, 0.75f32],
            [1.0f32, 0.0f32, 0.125f32],
            [f32::MIN_POSITIVE, f32::MAX, 0.333_333_34f32],
        ];
        let working = Image { width: 3, height: 1, pixels: px.to_vec(), ir: None };
        let thumb = Image { width: 1, height: 1, pixels: vec![[0.1, 0.2, 0.3]], ir: None };
        let base = [0.0f32, 0.5f32, 1.0f32];

        write(&path, base, &working, &thumb).expect("write");
        let (_, rw, _) = read(&path).expect("read");

        for (a, b) in rw.pixels.iter().zip(px.iter()) {
            for c in 0..3 {
                assert_eq!(a[c].to_bits(), b[c].to_bits(), "pixel channel {c} must be bit-exact");
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}
