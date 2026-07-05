//! Detect the photographic FRAME on a 35mm film-strip scan and return its
//! rectangle, so import can crop away the sprocket-hole rebate (top/bottom) and
//! the dark scan margins (sides). Runs on the import-time thumbnail proxy.
//!
//! Signal (see the design doc): on a strip scan the rebate bands that carry the
//! sprocket holes read as rows/cols with a high BRIGHT fraction — clear film
//! punches bright lightbox through the holes — while the dark scan border beside
//! the film reads as near-black rows/cols. The frame is the central region
//! between them. Conservative: only fires when one axis shows a sprocket band on
//! BOTH opposite edges (the unmistakable film-strip fingerprint); otherwise
//! returns `None` and the image is left full-frame.

use crate::image::Image;

// A pixel is "bright" (sprocket hole / clear film over the lightbox) above this luma.
const BRIGHT_T: f32 = 0.5;
// A row/col is part of a sprocket rebate band when at least this fraction is bright.
const BF_SPROCKET: f32 = 0.45;
// A row/col is a dark scan margin when its mean luma is at or below this.
const DARK_T: f32 = 0.06;
// The rebate is always near the edge: only look this deep (fraction of the axis)
// from each side for the sprocket band, so bright FRAME content mid-image is not
// mistaken for rebate.
const REBATE_ZONE: f32 = 0.35;
// A sprocket band must be at least this thick (fraction of the axis) to count as
// the strip fingerprint.
const MIN_REBATE_FRAC: f32 = 0.03;
// Each side may trim at most this fraction of its axis.
const MAX_TRIM: f32 = 0.40;
// The kept frame must retain at least this fraction of the total area.
const MIN_KEEP_AREA: f32 = 0.25;

fn luma(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Per-line bright fraction and mean luma along one axis. `lines` is the number
/// of lines (rows or cols); `along` is the length of each line. `at(line, k)`
/// returns the pixel at position `k` along `line`.
fn line_stats(
    lines: usize,
    along: usize,
    at: impl Fn(usize, usize) -> [f32; 3],
) -> (Vec<f32>, Vec<f32>) {
    let mut bf = vec![0.0f32; lines];
    let mut mean = vec![0.0f32; lines];
    for l in 0..lines {
        let mut bright = 0u32;
        let mut sum = 0.0f32;
        for k in 0..along {
            let v = luma(at(l, k));
            if v > BRIGHT_T {
                bright += 1;
            }
            sum += v;
        }
        bf[l] = bright as f32 / along as f32;
        mean[l] = sum / along as f32;
    }
    (bf, mean)
}

/// Sprocket-band inner edges along one axis. Returns
/// `(lo_inner, lo_count, hi_inner, hi_count)`: `lo_inner` is just past the
/// deepest sprocket line in the outer (low-side) zone; `hi_inner` is the
/// shallowest sprocket line in the outer (high-side) zone; the counts are how
/// many sprocket lines were seen in each zone. `hi_inner` is exclusive.
fn sprocket_edges(n: usize, bf: &[f32]) -> (usize, usize, usize, usize) {
    let zone = ((n as f32 * REBATE_ZONE) as usize).max(1).min(n);
    let mut lo_inner = 0usize;
    let mut lo_count = 0usize;
    for (i, &v) in bf.iter().enumerate().take(zone) {
        if v >= BF_SPROCKET {
            lo_inner = i + 1;
            lo_count += 1;
        }
    }
    let mut hi_inner = n;
    let mut hi_count = 0usize;
    for i in (n - zone..n).rev() {
        if bf[i] >= BF_SPROCKET {
            hi_inner = i;
            hi_count += 1;
        }
    }
    (lo_inner, lo_count, hi_inner, hi_count)
}

/// Depth of the contiguous near-black margin from each edge along one axis.
/// Returns `(lo, hi)` where `[0, lo)` and `[hi, n)` are dark; `hi` exclusive.
fn dark_edges(n: usize, mean: &[f32]) -> (usize, usize) {
    let mut lo = 0usize;
    while lo < n && mean[lo] <= DARK_T {
        lo += 1;
    }
    let mut hi = n;
    while hi > 0 && mean[hi - 1] <= DARK_T {
        hi -= 1;
    }
    (lo, hi)
}

/// Detect the photographic frame on a film-strip scan. Returns `[x, y, w, h]`
/// normalized 0..1, or `None` when the image is not a recognizable strip scan.
pub fn detect_film_frame_crop(img: &Image) -> Option<[f32; 4]> {
    let (w, h) = (img.width, img.height);
    if w < 8 || h < 8 {
        return None;
    }
    let px = |x: usize, y: usize| img.pixels[y * w + x];

    let (row_bf, row_mean) = line_stats(h, w, |y, x| px(x, y));
    let (col_bf, col_mean) = line_stats(w, h, px);

    // Sprocket bands per axis, and the strip fingerprint: a sprocket band on BOTH
    // ends of an axis. Only an axis that passes the fingerprint is allowed to
    // trim by sprockets — otherwise bright FRAME content near an edge of the
    // CROSS axis (e.g. a blown sky on one side) would be mistaken for a rebate.
    let min_rebate = (h.max(w) as f32 * MIN_REBATE_FRAC).ceil() as usize;
    let (v_lo, v_lo_c, v_hi, v_hi_c) = sprocket_edges(h, &row_bf);
    let (hz_lo, hz_lo_c, hz_hi, hz_hi_c) = sprocket_edges(w, &col_bf);
    let v_strip = v_lo_c >= min_rebate && v_hi_c >= min_rebate;
    let h_strip = hz_lo_c >= min_rebate && hz_hi_c >= min_rebate;
    if !(v_strip || h_strip) {
        return None;
    }

    // Each side trims the deeper of its dark margin and (only on the strip axis)
    // its sprocket band.
    let (dark_top, dark_bottom) = dark_edges(h, &row_mean);
    let (dark_left, dark_right) = dark_edges(w, &col_mean);
    let top = if v_strip {
        v_lo.max(dark_top)
    } else {
        dark_top
    };
    let bottom = if v_strip {
        v_hi.min(dark_bottom)
    } else {
        dark_bottom
    };
    let left = if h_strip {
        hz_lo.max(dark_left)
    } else {
        dark_left
    };
    let right = if h_strip {
        hz_hi.min(dark_right)
    } else {
        dark_right
    };

    // Non-degenerate frame.
    if top + 1 >= bottom || left + 1 >= right {
        return None;
    }

    // Trim sanity: no single side eats too much; keep enough area.
    let trim_top = top as f32 / h as f32;
    let trim_bottom = (h - bottom) as f32 / h as f32;
    let trim_left = left as f32 / w as f32;
    let trim_right = (w - right) as f32 / w as f32;
    if trim_top >= MAX_TRIM
        || trim_bottom >= MAX_TRIM
        || trim_left >= MAX_TRIM
        || trim_right >= MAX_TRIM
    {
        return None;
    }
    let kept = (right - left) as f32 / w as f32 * (bottom - top) as f32 / h as f32;
    if kept < MIN_KEEP_AREA {
        return None;
    }

    Some([
        left as f32 / w as f32,
        top as f32 / h as f32,
        (right - left) as f32 / w as f32,
        (bottom - top) as f32 / h as f32,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Image;

    /// Build a synthetic horizontal film-strip scan:
    /// - dark scan margins on the left/right (`margin` px each),
    /// - top & bottom sprocket rebate bands (`rebate` px each): dark film base with
    ///   periodic bright "holes",
    /// - a mid-gray photographic frame in the middle.
    fn strip(w: usize, h: usize, rebate: usize, margin: usize) -> Image {
        let mut img = Image::new(w, h);
        let dark = [0.02, 0.02, 0.02];
        let frame = [0.30, 0.25, 0.22];
        let hole = [0.95, 0.95, 0.95];
        for y in 0..h {
            let in_rebate = y < rebate || y >= h - rebate;
            for x in 0..w {
                let p = if x < margin || x >= w - margin {
                    dark
                } else if in_rebate {
                    // periodic holes: ~50% of the rebate width is hole.
                    if (x / 12) % 2 == 0 {
                        hole
                    } else {
                        dark
                    }
                } else {
                    frame
                };
                img.pixels[y * w + x] = p;
            }
        }
        img
    }

    #[test]
    fn detects_frame_between_sprocket_bands() {
        // 320x214, 24px rebate top/bottom, 20px dark margin each side.
        let img = strip(320, 214, 24, 20);
        let [x, y, rw, rh] = detect_film_frame_crop(&img).expect("should detect frame");
        // top/bottom trimmed to ~24px → y≈0.112, h≈0.776; sides ~20px → x≈0.0625, w≈0.875.
        assert!((y - 24.0 / 214.0).abs() < 0.04, "y={y}");
        assert!((rh - (214.0 - 48.0) / 214.0).abs() < 0.06, "rh={rh}");
        assert!((x - 20.0 / 320.0).abs() < 0.03, "x={x}");
        assert!((rw - (320.0 - 40.0) / 320.0).abs() < 0.04, "rw={rw}");
    }

    #[test]
    fn plain_photo_is_not_a_strip() {
        // Uniform mid-gray: no sprocket bands, no dark margins → None.
        let mut img = Image::new(320, 214);
        for p in img.pixels.iter_mut() {
            *p = [0.3, 0.3, 0.3];
        }
        assert_eq!(detect_film_frame_crop(&img), None);
    }

    #[test]
    fn one_bright_edge_is_not_a_strip() {
        // Bright band only along the top (like a blown sky), gray elsewhere. Only
        // one end has a sprocket-like band → fails the both-ends fingerprint → None.
        let mut img = Image::new(320, 214);
        for y in 0..214 {
            for x in 0..320 {
                img.pixels[y * 320 + x] = if y < 24 {
                    [0.95, 0.95, 0.95]
                } else {
                    [0.3, 0.3, 0.3]
                };
            }
        }
        assert_eq!(detect_film_frame_crop(&img), None);
    }

    #[test]
    fn bright_frame_content_does_not_overcrop() {
        // Strip with a bright horizontal band in the MIDDLE of the frame (a window/
        // sky in the photo). The rebate-zone cap means it is not mistaken for a
        // sprocket band, so the crop stays at the real rebate edges.
        let mut img = strip(320, 214, 24, 20);
        for y in 100..114 {
            for x in 20..300 {
                img.pixels[y * 320 + x] = [0.9, 0.9, 0.9];
            }
        }
        let [_, y, _, rh] = detect_film_frame_crop(&img).expect("should still detect");
        assert!(
            (y - 24.0 / 214.0).abs() < 0.05,
            "y={y} (should crop at rebate, not the mid band)"
        );
        assert!(rh > 0.6, "rh={rh} (frame not eaten by the mid bright band)");
    }

    #[test]
    fn bright_content_on_one_side_does_not_overcrop_width() {
        // Horizontal strip (sprockets top/bottom) with a blown-bright region
        // filling the RIGHT third of the frame (a sky). The horizontal axis has no
        // sprocket fingerprint, so those bright columns must NOT pull the right
        // edge in — width stays full to the dark side margins.
        let mut img = strip(320, 214, 24, 20);
        for y in 24..(214 - 24) {
            for x in 210..300 {
                img.pixels[y * 320 + x] = [0.95, 0.95, 0.95];
            }
        }
        let [x, _, rw, _] = detect_film_frame_crop(&img).expect("should detect");
        // Right edge stays at the ~20px dark margin (x≈0.0625, w≈0.875), NOT pulled
        // in to the bright region at x≈210/320≈0.656.
        assert!((x - 20.0 / 320.0).abs() < 0.03, "x={x}");
        assert!(
            rw > 0.82,
            "rw={rw} (right edge wrongly pulled in by bright content)"
        );
    }

    #[test]
    fn rejects_tiny_images() {
        assert_eq!(detect_film_frame_crop(&Image::new(4, 4)), None);
    }
}
