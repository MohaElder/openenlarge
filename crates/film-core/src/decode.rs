//! Decode scan files into a linear-RGB Image.
//!
//! `decode_tiff` handles plain 8/16-bit RGB TIFF and scanner *linear* DNGs that
//! the `tiff` crate can read directly. `decode_raw` handles Bayer RAF/DNG via
//! rawler (demosaiced, linear light, no white-balance, no gamma).

use crate::Image;
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::ColorType;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("tiff error: {0}")]
    Tiff(#[from] tiff::TiffError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported color type: {0:?}")]
    UnsupportedColor(ColorType),
    #[error("raw decode error: {0}")]
    Raw(String),
    #[error("image decode error: {0}")]
    Image(#[from] ::image::ImageError),
}

/// Turn rawler's terse decode error into an actionable `DecodeError`.
///
/// The one case worth special-casing: Nikon's "High Efficiency" (HE) and
/// "High Efficiency★" (HE*) NEF compression. These use intoPIX's patented
/// TicoRAW codec, which rawler — and every open-source raw library — cannot
/// decode (the bitstream is proprietary and licensing it is incompatible with
/// an open-source decoder). rawler reports this as e.g.
/// `NEF compression Some(HighEfficencyStar) is not supported` (note rawler's
/// "Efficency" spelling), which reads like a corrupt-file error. Rewrite it
/// into the only two things the user can actually do about it.
fn map_decode_err(raw: String) -> DecodeError {
    if raw.contains("HighEfficency") || raw.contains("HighEfficencyStar") {
        return DecodeError::Raw(
            "This is a Nikon High Efficiency (HE/HE★) RAW, which uses a \
             proprietary, licensed codec that open-source decoders can't read. \
             To use this scan, either set your camera's RAW recording to \
             \"Lossless compressed\", or convert the file to DNG with Adobe \
             DNG Converter (free) and import the .dng."
                .to_string(),
        );
    }
    DecodeError::Raw(raw)
}

/// sRGB electro-optical transfer function: gamma-encoded sRGB → linear light.
/// Input and output are normalized to [0, 1].
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Decode an 8- or 16-bit RGB(A) TIFF / linear DNG into a normalized f32 Image.
/// A 4th (alpha/IR) channel, if present, is captured into `ir`.
pub fn decode_tiff(path: &Path) -> Result<Image, DecodeError> {
    let file = std::fs::File::open(path)?;
    // Lift the crate's default buffer caps (intermediate_buffer_size = 128 MiB,
    // decoding_buffer_size = 256 MiB). Scanner TIFFs are routinely uncompressed,
    // 16-bit, and stored as a SINGLE strip (RowsPerStrip == ImageLength), so one
    // chunk can exceed 128 MiB — e.g. a 5958×3945 Nikon Coolscan scan is ~134 MiB
    // in one strip, which trips `LimitsExceeded`. These are local, user-selected
    // files (no untrusted-input DoS surface), so removing the cap is safe.
    let mut dec = Decoder::new(file)?.with_limits(Limits::unlimited());
    let (w, h) = dec.dimensions()?;
    let color = dec.colortype()?;
    let (channels, max) = match color {
        ColorType::RGB(8) => (3usize, 255.0f32),
        ColorType::RGB(16) => (3, 65535.0),
        ColorType::RGBA(8) => (4, 255.0),
        ColorType::RGBA(16) => (4, 65535.0),
        other => return Err(DecodeError::UnsupportedColor(other)),
    };
    let result = dec.read_image()?;
    let floats: Vec<f32> = match result {
        DecodingResult::U8(v) => v.into_iter().map(|x| x as f32 / max).collect(),
        DecodingResult::U16(v) => v.into_iter().map(|x| x as f32 / max).collect(),
        _ => return Err(DecodeError::UnsupportedColor(color)),
    };
    let n = (w as usize) * (h as usize);
    let mut pixels = Vec::with_capacity(n);
    let mut ir: Option<Vec<f32>> = if channels == 4 {
        Some(Vec::with_capacity(n))
    } else {
        None
    };
    for i in 0..n {
        let base = i * channels;
        pixels.push([floats[base], floats[base + 1], floats[base + 2]]);
        if let Some(ir) = ir.as_mut() {
            ir.push(floats[base + 3]);
        }
    }
    Ok(Image {
        width: w as usize,
        height: h as usize,
        pixels,
        ir,
    })
}

/// Rewrite TIFF `IFD` (type 13) entries to `LONG` (type 4) in an in-memory
/// TIFF/DNG buffer so rawler can follow the file's SubIFD pointers.
///
/// # Why this exists
/// Some DNG writers — notably **Capture One on Windows** — encode the `SubIFDs`
/// pointer (tag 330) using TIFF data type **13 (IFD)** rather than the far more
/// common type **4 (LONG)**. The two are byte-for-byte identical on the wire:
/// both are 32-bit unsigned offsets. But rawler 0.7's IFD parser only follows a
/// SubIFD pointer whose entry decodes to `Long`/`Unknown`/`Undefined`; a type-13
/// entry falls through to a 1-byte `Unknown`, so the 32-bit offset is truncated
/// to its low byte, the real (full-resolution, uncompressed) raw IFD is never
/// parsed, and decoding fails with the misleading `Unsupported DNG compression`
/// (the only IFD rawler then sees is the reduced-resolution preview).
///
/// Rewriting the 2-byte type field of every type-13 entry to type 4 fixes this
/// losslessly. It is a no-op for files that don't use type 13, and the leading
/// `II`/`MM` + magic-42 guard makes it a safe no-op for non-TIFF containers
/// (e.g. Fujifilm `.raf`, which is not a bare TIFF at offset 0).
fn normalize_ifd_pointer_types(buf: &mut [u8]) {
    #[inline]
    fn rd_u16(b: &[u8], o: usize, le: bool) -> u16 {
        let v = [b[o], b[o + 1]];
        if le {
            u16::from_le_bytes(v)
        } else {
            u16::from_be_bytes(v)
        }
    }
    #[inline]
    fn rd_u32(b: &[u8], o: usize, le: bool) -> u32 {
        let v = [b[o], b[o + 1], b[o + 2], b[o + 3]];
        if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    }

    if buf.len() < 8 {
        return;
    }
    let le = match &buf[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return, // not a classic TIFF container
    };
    if rd_u16(buf, 2, le) != 42 {
        return; // BigTIFF (43) or non-TIFF: different layout, leave untouched
    }

    // SubIFDs (330) and ExifIFD (34665) are the pointer tags rawler itself
    // recurses into; following them is enough to reach every raw IFD.
    const SUB_IFDS: u16 = 330;
    const EXIF_IFD: u16 = 34665;

    let mut stack: Vec<usize> = vec![rd_u32(buf, 4, le) as usize];
    let mut visited = std::collections::HashSet::new();
    let mut budget = 512usize; // bound work on malformed/looping files

    while let Some(mut off) = stack.pop() {
        // Walk this IFD and any chained IFDs (the trailing next-IFD pointer).
        while off != 0 {
            if budget == 0 || !visited.insert(off) || off + 2 > buf.len() {
                break;
            }
            budget -= 1;
            let n = rd_u16(buf, off, le) as usize;
            let end = off + 2 + n * 12; // entries, followed by next-IFD u32
            if end + 4 > buf.len() {
                break;
            }
            for i in 0..n {
                let e = off + 2 + i * 12;
                let tag = rd_u16(buf, e, le);
                if rd_u16(buf, e + 2, le) == 13 {
                    // IFD (13) -> LONG (4); identical 4-byte unsigned offset.
                    let four = if le {
                        4u16.to_le_bytes()
                    } else {
                        4u16.to_be_bytes()
                    };
                    buf[e + 2] = four[0];
                    buf[e + 3] = four[1];
                }
                if tag == SUB_IFDS || tag == EXIF_IFD {
                    let count = rd_u32(buf, e + 4, le) as usize;
                    if count <= 1 {
                        let v = rd_u32(buf, e + 8, le) as usize;
                        if v != 0 {
                            stack.push(v);
                        }
                    } else {
                        // >1 offsets are stored out-of-line at the value offset.
                        let vo = rd_u32(buf, e + 8, le) as usize;
                        for k in 0..count {
                            if vo + k * 4 + 4 <= buf.len() {
                                let v = rd_u32(buf, vo + k * 4, le) as usize;
                                if v != 0 {
                                    stack.push(v);
                                }
                            }
                        }
                    }
                }
            }
            off = rd_u32(buf, end, le) as usize;
        }
    }
}

/// Decode a camera RAW file (Fujifilm `.raf`, `.dng`, or any rawler-supported
/// format) into a demosaiced, linear-light RGB `Image`.
///
/// # Processing pipeline
/// We run rawler's `RawDevelop` with only the steps needed for a clean linear
/// decode:
///   - `Rescale` — applies black/white level correction, scaling raw u16 data
///     to f32 in [0, 1] before demosaic.
///   - `Demosaic` — PPG demosaic for standard RGB Bayer; bilinear for 4-channel
///     CFAs. Output remains in [0, 1] linear camera-native light.
///   - `CropActiveArea` — crops the optical black borders used during demosaic.
///   - `CropDefault` — applies the camera's default image crop.
///
/// Deliberately excluded:
///   - `WhiteBalance` — the inversion engine does its own channel balancing.
///   - `Calibrate` — skips the XYZ→camera color matrix; we want raw camera-
///     native values, not a rendering colorspace transform.
///   - `SRgb` — no gamma/tone curve; output stays linear.
///
/// # Normalization
/// `rawler`'s `Rescale` step subtracts per-channel black levels and divides by
/// (white_level − black_level), producing f32 values nominally in [0, 1].
/// After demosaic the values remain in that range (bilinear/PPG only
/// interpolate; they don't amplify). We clamp to [0, 1] as a safety net in
/// case of hot pixels or sensor artefacts slightly above white level.
/// Reconstruct a sensor-shift **pixel-shift high-res** CFA mosaic by 2×2 binning.
///
/// In an ordinary Bayer sensor the four sites of a 2×2 block sample four *adjacent*
/// scene points, so colour must be interpolated (demosaiced). A pixel-shift high-res
/// frame is different: the sensor is physically shifted by sub-pixel steps between
/// sub-exposures so that **every** site of a 2×2 block measures the *same* scene point
/// through a different colour filter. The correct reconstruction is therefore to
/// *bin* each block — average the two greens, take the lone red and blue — yielding a
/// true-colour pixel at quarter resolution. Running Bayer demosaic on such a frame
/// instead cross-contaminates the channels (on a colour-negative scan it collapses
/// R≈B, destroying the colour the inversion relies on).
///
/// `cfa.color_at(row, col)` maps a mosaic site to a colour index (0=R, 1=G, 2=B);
/// `black`/`white` are per-RGB-channel levels in raw code values. Output values are
/// `(v − black) / (white − black)`, clamped to `[0, 1]`, matching [`decode_raw`].
fn bin_cfa_2x2(
    data: &[u16],
    width: usize,
    height: usize,
    cfa: &rawler::CFA,
    black: [f32; 3],
    white: [f32; 3],
) -> (usize, usize, Vec<[f32; 3]>) {
    let bw = width / 2;
    let bh = height / 2;
    let scale = [
        1.0 / (white[0] - black[0]).max(1.0),
        1.0 / (white[1] - black[1]).max(1.0),
        1.0 / (white[2] - black[2]).max(1.0),
    ];
    let mut out = vec![[0.0f32; 3]; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let (y, x) = (by * 2, bx * 2);
            let mut sum = [0.0f32; 3];
            let mut cnt = [0u32; 3];
            for dy in 0..2 {
                for dx in 0..2 {
                    let c = cfa.color_at(y + dy, x + dx);
                    if c < 3 {
                        sum[c] += data[(y + dy) * width + (x + dx)] as f32;
                        cnt[c] += 1;
                    }
                }
            }
            let mut px = [0.0f32; 3];
            for c in 0..3 {
                let v = if cnt[c] > 0 {
                    sum[c] / cnt[c] as f32
                } else {
                    0.0
                };
                px[c] = ((v - black[c]) * scale[c]).clamp(0.0, 1.0);
            }
            out[by * bw + bx] = px;
        }
    }
    (bw, bh, out)
}

/// Extract the `cw × ch` sub-rectangle at `(cx, cy)` from a row-major RGB image,
/// clamping the requested size to the available bounds (never panics on overflow).
fn crop_image(
    px: &[[f32; 3]],
    w: usize,
    h: usize,
    cx: usize,
    cy: usize,
    cw: usize,
    ch: usize,
) -> (usize, usize, Vec<[f32; 3]>) {
    let cw = cw.min(w.saturating_sub(cx));
    let ch = ch.min(h.saturating_sub(cy));
    let mut out = Vec::with_capacity(cw * ch);
    for y in 0..ch {
        let start = (cy + y) * w + cx;
        out.extend_from_slice(&px[start..start + cw]);
    }
    (cw, ch, out)
}

pub fn decode_raw(path: &Path, apply_camera_matrix: bool) -> Result<Image, DecodeError> {
    use rawler::imgop::develop::Intermediate;
    use rawler::imgop::develop::{ProcessingStep, RawDevelop};
    use std::sync::Arc;

    // Step 1: decode the raw file into a mosaic RawImage (integer u16 data,
    // not yet demosaiced).
    //
    // We read the bytes ourselves and run `normalize_ifd_pointer_types` before
    // handing them to rawler (see that function for why). This is the same data
    // `rawler::decode_file` would mmap; routing through an in-memory buffer lets
    // us patch a TIFF-encoding quirk that otherwise makes some DNGs undecodable.
    let mut bytes = std::fs::read(path)?;
    normalize_ifd_pointer_types(&mut bytes);
    let source = rawler::rawsource::RawSource::new_from_shared_vec(Arc::new(bytes)).with_path(path);
    let raw = rawler::decode(&source, &rawler::decoders::RawDecodeParams::default())
        .map_err(|e| map_decode_err(e.to_string()))?;

    // Step 1.5: sensor-shift HIGH-RES frames need binning, not demosaic.
    //
    // Pixel-shift high-res modes (Olympus / OM "High Res Shot", etc.) store a frame
    // whose 2×2 CFA blocks each sample the SAME scene point through R/G/G/B filters,
    // not four adjacent points. rawler surfaces these via a "highres" camera mode but
    // still tags them as an ordinary Bayer CFA, so its demosaic cross-contaminates the
    // channels — on a colour-negative scan that collapses R≈B into a near-monochrome
    // inversion. Reconstruct by 2×2 binning instead (see `bin_cfa_2x2`).
    if raw.camera.mode == "highres" && raw.cpp == 1 {
        if let rawler::RawImageData::Integer(ref data) = raw.data {
            let cfa = &raw.camera.cfa;
            // Black level is a positional 2×2 repeat pattern; average per colour via
            // the CFA. White level is colour-indexed (RGBE order), single value when
            // uniform.
            let bl = raw.blacklevel.as_bayer_array();
            let mut bsum = [0.0f32; 3];
            let mut bcnt = [0u32; 3];
            for (i, (r, c)) in [(0, 0), (0, 1), (1, 0), (1, 1)].iter().enumerate() {
                let col = cfa.color_at(*r, *c);
                if col < 3 {
                    bsum[col] += bl[i];
                    bcnt[col] += 1;
                }
            }
            let black = [
                if bcnt[0] > 0 {
                    bsum[0] / bcnt[0] as f32
                } else {
                    0.0
                },
                if bcnt[1] > 0 {
                    bsum[1] / bcnt[1] as f32
                } else {
                    0.0
                },
                if bcnt[2] > 0 {
                    bsum[2] / bcnt[2] as f32
                } else {
                    0.0
                },
            ];
            let wl = &raw.whitelevel.0;
            let wl_at =
                |c: usize| -> f32 { wl.get(c).or_else(|| wl.first()).copied().unwrap_or(0) as f32 };
            let white = [wl_at(0), wl_at(1), wl_at(2)];

            let (bw, bh, binned) = bin_cfa_2x2(data, raw.width, raw.height, cfa, black, white);

            // Apply the camera's recommended crop (mosaic coords → halved for the
            // binned grid) so framing matches the normal demosaic path.
            let (width, height, pixels) = match raw.crop_area {
                Some(rect) => crop_image(
                    &binned,
                    bw,
                    bh,
                    rect.p.x / 2,
                    rect.p.y / 2,
                    rect.d.w / 2,
                    rect.d.h / 2,
                ),
                None => (bw, bh, binned),
            };
            return Ok(Image {
                width,
                height,
                pixels,
                ir: None,
            });
        }
    }

    // Step 2: develop with only linear steps — always camera-native (no WB; no rawler
    // `Calibrate`, which is sRGB-only; no gamma). The optional camera matrix is applied
    // MANUALLY below (Adobe RGB target). See fn docs.
    let develop = RawDevelop {
        steps: vec![
            ProcessingStep::Rescale,
            ProcessingStep::Demosaic,
            ProcessingStep::CropActiveArea,
            ProcessingStep::CropDefault,
        ],
    };
    let intermediate = develop
        .develop_intermediate(&raw)
        .map_err(|e| DecodeError::Raw(e.to_string()))?;

    // Camera-matrix mode: a manual camera-native → Adobe-RGB-linear 3×3 (see `cam_to_adobe`).
    // Adobe RGB is a wide working space, so the matrix is gentle (no sRGB over-saturation);
    // NO white balance is baked in — the inversion engine's per-channel neutralisation
    // balances the channels. `None` when off / no usable matrix → camera-native passthrough.
    let cam2adobe: Option<[[f32; 3]; 3]> = if apply_camera_matrix {
        cam_to_adobe(&raw)
    } else {
        None
    };
    let px = |r: f32, g: f32, b: f32| -> [f32; 3] {
        match &cam2adobe {
            Some(m) => [
                (m[0][0] * r + m[0][1] * g + m[0][2] * b).clamp(0.0, 1.0),
                (m[1][0] * r + m[1][1] * g + m[1][2] * b).clamp(0.0, 1.0),
                (m[2][0] * r + m[2][1] * g + m[2][2] * b).clamp(0.0, 1.0),
            ],
            None => [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)],
        }
    };

    // Step 3: extract the three-channel f32 pixel data (applying the matrix if present).
    // After Rescale the data is in [0,1]; after Demosaic it stays in [0,1].
    let (width, height, pixels) = match intermediate {
        Intermediate::ThreeColor(color2d) => {
            let w = color2d.width;
            let h = color2d.height;
            // color2d.data is Vec<[f32;3]> — exactly our Image::pixels type.
            let clamped: Vec<[f32; 3]> = color2d
                .data
                .into_iter()
                .map(|[r, g, b]| px(r, g, b))
                .collect();
            (w, h, clamped)
        }
        Intermediate::FourColor(color2d) => {
            // Some exotic CFAs produce a 4-channel intermediate; collapse to RGB
            // by dropping the 4th channel (which is typically a second green or
            // near-IR channel — not meaningful for film inversion).
            let w = color2d.width;
            let h = color2d.height;
            let clamped: Vec<[f32; 3]> = color2d
                .data
                .into_iter()
                .map(|[r, g, b, _]| px(r, g, b))
                .collect();
            (w, h, clamped)
        }
        Intermediate::Monochrome(pix_data) => {
            // Monochrome sensor: replicate the single channel into R=G=B (no matrix).
            let w = pix_data.width;
            let h = pix_data.height;
            let clamped: Vec<[f32; 3]> = pix_data
                .data
                .into_iter()
                .map(|v| {
                    let c = v.clamp(0.0, 1.0);
                    [c, c, c]
                })
                .collect();
            (w, h, clamped)
        }
    };

    Ok(Image {
        width,
        height,
        pixels,
        ir: None,
    })
}

/// Build a camera-native → Adobe-RGB-linear 3×3 from the RAW's embedded color matrix.
///
/// Targets Adobe RGB (1998) D65 — a WIDE working space, so the matrix is gentle (unlike a
/// camera→sRGB matrix, which has large negative terms to fit the camera gamut into narrow
/// sRGB and over-saturates). NO white balance is applied; the inversion engine's per-channel
/// neutralisation balances the channels downstream. Each row is normalised so a neutral Adobe
/// white maps to the camera neutral. Returns `None` when the RAW exposes no finite color
/// matrix → the caller falls back to camera-native.
fn cam_to_adobe(raw: &rawler::RawImage) -> Option<[[f32; 3]; 3]> {
    use nalgebra::Matrix3;
    // `xyz_to_cam` rows = camera channels, cols = XYZ. Use the first 3 rows.
    let x2c = &raw.xyz_to_cam;
    let xyz2cam = Matrix3::new(
        x2c[0][0], x2c[0][1], x2c[0][2], x2c[1][0], x2c[1][1], x2c[1][2], x2c[2][0], x2c[2][1],
        x2c[2][2],
    );
    if !xyz2cam.iter().all(|v| v.is_finite()) || xyz2cam.iter().all(|&v| v == 0.0) {
        return None;
    }
    // Adobe RGB (1998), D65 linear → XYZ (D65).
    let adobe2xyz = Matrix3::new(
        0.576_730_9,
        0.185_554,
        0.188_185_2,
        0.297_376_9,
        0.627_349_1,
        0.075_274_1,
        0.027_034_3,
        0.070_687_2,
        0.991_108_5,
    );
    // Adobe-linear → camera, then normalise each row to sum 1 (neutral white → camera neutral).
    let mut rgb2cam = xyz2cam * adobe2xyz;
    for i in 0..3 {
        let s = rgb2cam[(i, 0)] + rgb2cam[(i, 1)] + rgb2cam[(i, 2)];
        if s.abs() > 1e-6 {
            for j in 0..3 {
                rgb2cam[(i, j)] /= s;
            }
        }
    }
    let m = rgb2cam.try_inverse()?; // camera → Adobe-linear
    Some([
        [m[(0, 0)], m[(0, 1)], m[(0, 2)]],
        [m[(1, 0)], m[(1, 1)], m[(1, 2)]],
        [m[(2, 0)], m[(2, 1)], m[(2, 2)]],
    ])
}

/// Extract a camera RAW file's EMBEDDED preview as a linear-light RGB `Image`,
/// downscaled so its long edge is ≤ `max_edge`, without a full decode/demosaic.
///
/// # Why
/// The LIGHT import path needs a cheap thumbnail so a freshly-imported, not-yet-
/// developed RAW shows a real picture in the Library grid instead of a black
/// placeholder. Most camera/scanner RAWs embed a JPEG preview; we ask rawler for
/// it via `full_image` (the most widely-implemented across formats), then
/// `thumbnail_image`. This is best-effort: some decoders (notably Olympus `.orf`)
/// expose no embedded preview at all, in which case the caller falls back to
/// `decode_tiff` or a full `decode_raw`.
///
/// # Color
/// Only 8-bit previews are accepted — those are reliably sRGB-encoded JPEGs, so —
/// exactly like [`decode_ldr`] — we apply the sRGB EOTF to land in the pipeline's
/// linear-light domain. (A rare 16-bit / float preview SubIFD has an ambiguous
/// transfer function `srgb_to_linear` would misread, so we reject it and let the
/// caller fall back.) The caller re-applies display gamma when encoding the
/// thumbnail (`to_png_b64(.., true)`); the encoder's 1/2.2 curve is a close — not
/// exact — inverse of the sRGB EOTF, the same near-neutral round-trip
/// [`decode_ldr`] uses for JPEG/PNG thumbnails.
///
/// # Robustness
/// We read + `normalize_ifd_pointer_types` the bytes ourselves (same as
/// [`decode_raw`]) so Capture One DNGs — whose preview lives behind a type-13
/// SubIFD pointer — still resolve. rawler can *panic* on malformed input (the
/// app's release profile deliberately avoids `panic = "abort"` for this), so the
/// decode runs inside `catch_unwind`. Returns `Err` when there is no usable
/// embedded preview.
///
/// EXIF orientation is not applied (consistent with [`decode_raw`]), so a portrait
/// frame's preview may appear rotated until it is developed.
pub fn decode_raw_preview(path: &Path, max_edge: u32) -> Result<Image, DecodeError> {
    use rawler::decoders::RawDecodeParams;
    use rawler::rawsource::RawSource;
    use std::sync::Arc;

    let mut bytes = std::fs::read(path)?;
    normalize_ifd_pointer_types(&mut bytes);
    let pb = path.to_path_buf();

    // rawler decoders can panic on malformed files — contain it so a single bad
    // file can't crash import. Constructing the source/decoder inside the closure
    // keeps the `UnwindSafe` bound on captures (owned `bytes` + `pb`) only.
    let dynimg = std::panic::catch_unwind(move || -> Option<::image::DynamicImage> {
        let source = RawSource::new_from_shared_vec(Arc::new(bytes)).with_path(&pb);
        let decoder = rawler::get_decoder(&source).ok()?;
        let params = RawDecodeParams::default();
        decoder
            .full_image(&source, &params)
            .ok()
            .flatten()
            // `preview_image` has no concrete override in rawler 0.7.2 (only the
            // default `Ok(None)` impl), so this branch is a no-op today — kept for
            // forward-compat if a future version implements it per-decoder.
            .or_else(|| decoder.preview_image(&source, &params).ok().flatten())
            .or_else(|| decoder.thumbnail_image(&source, &params).ok().flatten())
    })
    .map_err(|_| DecodeError::Raw("decoder panicked extracting preview".into()))?
    .ok_or_else(|| DecodeError::Raw("no embedded preview".into()))?;

    // Accept only 8-bit previews (reliably sRGB JPEGs). 16-bit / float preview
    // SubIFDs have an ambiguous transfer function; defer them to the caller.
    match &dynimg {
        ::image::DynamicImage::ImageRgb8(_)
        | ::image::DynamicImage::ImageRgba8(_)
        | ::image::DynamicImage::ImageLuma8(_)
        | ::image::DynamicImage::ImageLumaA8(_) => {}
        _ => return Err(DecodeError::Raw("non-8-bit embedded preview".into())),
    }

    // Downscale BEFORE linearizing so the per-pixel sRGB EOTF (a `powf`) runs over
    // the small thumbnail, not the full-res (multi-megapixel) embedded JPEG.
    let small = dynimg.thumbnail(max_edge, max_edge);
    let rgb = small.to_rgb32f(); // normalized [0,1] f32, alpha dropped
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let pixels: Vec<[f32; 3]> = rgb
        .pixels()
        .map(|p| {
            [
                srgb_to_linear(p[0]),
                srgb_to_linear(p[1]),
                srgb_to_linear(p[2]),
            ]
        })
        .collect();
    Ok(Image {
        width: w,
        height: h,
        pixels,
        ir: None,
    })
}

/// Decode a gamma-encoded LDR image (JPEG / PNG) into a linear-light RGB `Image`.
///
/// Unlike camera RAW and scanner TIFFs — which the pipeline treats as already
/// linear — JPEG/PNG are almost always **sRGB gamma-encoded**. We apply the sRGB
/// EOTF (`srgb_to_linear`) so the decoded values land in the same linear-light
/// domain the inversion engine expects. Any alpha channel is dropped; 16-bit PNGs
/// are supported (decoded at full precision before normalizing).
///
/// Note: 8-bit JPEG is lossy and low-bit-depth, so density-domain inversion has
/// less headroom than with a 16-bit RAW/TIFF scan — quality will be lower.
pub fn decode_ldr(path: &Path) -> Result<Image, DecodeError> {
    let img = ::image::open(path)?;
    let rgb = img.to_rgb32f(); // normalized [0,1] f32, alpha dropped
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let pixels: Vec<[f32; 3]> = rgb
        .pixels()
        .map(|p| {
            [
                srgb_to_linear(p[0]),
                srgb_to_linear(p[1]),
                srgb_to_linear(p[2]),
            ]
        })
        .collect();
    Ok(Image {
        width: w,
        height: h,
        pixels,
        ir: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_cfa_2x2_separates_grbg_channels() {
        // A pixel-shift high-res frame samples the same scene point through all
        // four CFA sites, so 2x2 binning a GRBG block must recover true RGB.
        // GRBG row-major: [G@(0,0), R@(0,1), B@(1,0), G@(1,1)].
        // black=0, white=4000 → R=3000/4000=0.75, G=2000/4000=0.5, B=1000/4000=0.25.
        let cfa = rawler::CFA::new("GRBG");
        let data = vec![2000u16, 3000, 1000, 2000];
        let (w, h, px) = bin_cfa_2x2(&data, 2, 2, &cfa, [0.0; 3], [4000.0; 3]);
        assert_eq!((w, h), (1, 1));
        let p = px[0];
        assert!((p[0] - 0.75).abs() < 1e-4, "R = {}", p[0]);
        assert!((p[1] - 0.50).abs() < 1e-4, "G = {}", p[1]);
        assert!((p[2] - 0.25).abs() < 1e-4, "B = {}", p[2]);
    }

    #[test]
    fn bin_cfa_2x2_applies_per_channel_black_and_white() {
        // RGGB block with per-channel black/white levels.
        // R@(0,0)=1100 (black100,white1100 → 1.0), G@(0,1)=G@(1,0)=600 (black100,white600 → 1.0),
        // B@(1,1)=350 (black100,white600 → 0.5).
        let cfa = rawler::CFA::new("RGGB");
        let data = vec![1100u16, 600, 600, 350];
        let (_, _, px) = bin_cfa_2x2(
            &data,
            2,
            2,
            &cfa,
            [100.0, 100.0, 100.0],
            [1100.0, 600.0, 600.0],
        );
        let p = px[0];
        assert!((p[0] - 1.0).abs() < 1e-4, "R = {}", p[0]);
        assert!((p[1] - 1.0).abs() < 1e-4, "G = {}", p[1]);
        assert!((p[2] - 0.5).abs() < 1e-4, "B = {}", p[2]);
    }

    #[test]
    fn crop_image_extracts_subrect() {
        // 3x2 image; R channel encodes the linear pixel index 0..6.
        let px: Vec<[f32; 3]> = (0..6).map(|i| [i as f32, 0.0, 0.0]).collect();
        // crop x=1,y=0,w=2,h=2 → indices (row0)1,2 (row1)4,5.
        let (w, h, out) = crop_image(&px, 3, 2, 1, 0, 2, 2);
        assert_eq!((w, h), (2, 2));
        let got: Vec<i32> = out.iter().map(|p| p[0] as i32).collect();
        assert_eq!(got, vec![1, 2, 4, 5]);
    }

    #[test]
    fn crop_image_clamps_out_of_bounds() {
        // Requesting a region past the right/bottom edge must clamp, not panic.
        let px: Vec<[f32; 3]> = (0..6).map(|i| [i as f32, 0.0, 0.0]).collect();
        let (w, h, out) = crop_image(&px, 3, 2, 2, 1, 5, 5);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out[0][0] as i32, 5);
    }

    #[test]
    fn srgb_to_linear_endpoints_and_midtone() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-6);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        // 128/255 sRGB ≈ 0.50196 encodes to ≈ 0.2159 linear.
        let mid = srgb_to_linear(128.0 / 255.0);
        assert!((mid - 0.2159).abs() < 1e-3, "got {mid}");
    }

    #[test]
    fn normalize_rewrites_type13_subifds_to_long() {
        // Minimal little-endian TIFF: header + one IFD with a single SubIFDs
        // entry (tag 330) encoded as TIFF type 13 (IFD) — the Capture One quirk.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"II"); // little-endian
        buf.extend_from_slice(&42u16.to_le_bytes()); // magic
        buf.extend_from_slice(&8u32.to_le_bytes()); // first IFD at offset 8
                                                    // IFD @ 8: 1 entry
        buf.extend_from_slice(&1u16.to_le_bytes()); // entry count
        buf.extend_from_slice(&330u16.to_le_bytes()); // tag = SubIFDs
        buf.extend_from_slice(&13u16.to_le_bytes()); // type = 13 (IFD)
        buf.extend_from_slice(&1u32.to_le_bytes()); // count = 1
        buf.extend_from_slice(&0u32.to_le_bytes()); // value/offset (0 = no recurse)
        buf.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0

        let type_field = 8 + 2 + 2; // header(8) + count(2) + tag(2)
        assert_eq!(
            u16::from_le_bytes([buf[type_field], buf[type_field + 1]]),
            13
        );
        normalize_ifd_pointer_types(&mut buf);
        assert_eq!(
            u16::from_le_bytes([buf[type_field], buf[type_field + 1]]),
            4,
            "type-13 SubIFDs entry should be rewritten to type-4 LONG"
        );
    }

    #[test]
    fn normalize_is_noop_on_non_tiff() {
        // A non-TIFF magic (e.g. Fujifilm RAF) must be left untouched.
        let mut buf = b"FUJIFILMCCD-RAW \x00\x01\x02\x03".to_vec();
        let before = buf.clone();
        normalize_ifd_pointer_types(&mut buf);
        assert_eq!(buf, before);
    }

    #[test]
    fn decode_ldr_png_linearizes() {
        // 2x1 PNG: black, white. Decoded linear values must be 0.0 and 1.0.
        let mut buf: ::image::RgbImage = ::image::ImageBuffer::new(2, 1);
        buf.put_pixel(0, 0, ::image::Rgb([0, 0, 0]));
        buf.put_pixel(1, 0, ::image::Rgb([255, 255, 255]));
        let dir = std::env::temp_dir();
        let path = dir.join("filmrev_decode_ldr_test.png");
        buf.save(&path).unwrap();

        let img = decode_ldr(&path).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert!(img.ir.is_none());
        assert!((img.pixels[0][0] - 0.0).abs() < 1e-6);
        assert!((img.pixels[1][0] - 1.0).abs() < 1e-6);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decode_raw_preview_errs_gracefully_on_non_raw() {
        // A plain PNG is not a camera RAW with an embedded preview. The extractor
        // must return Err (so import falls back to decode_tiff / placeholder) and
        // never panic out of `catch_unwind`.
        let mut buf: ::image::RgbImage = ::image::ImageBuffer::new(2, 2);
        buf.put_pixel(0, 0, ::image::Rgb([10, 20, 30]));
        let dir = std::env::temp_dir();
        let path = dir.join("filmrev_decode_raw_preview_test.png");
        buf.save(&path).unwrap();

        assert!(decode_raw_preview(&path, 320).is_err());

        let _ = std::fs::remove_file(&path);
    }
}
