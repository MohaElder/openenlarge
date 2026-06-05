//! Pack a linear-RGB working image into half-float RGBA bytes for a one-shot
//! WebGL2 `RGBA16F` texture upload, and resolve inversion params into a flat,
//! serialisable uniform set the GPU shader consumes.

use crate::commands::{export_stamps, DustStroke, IrRemoval};
use crate::convert::proxy;
use crate::convert::{crop, orient, rotate};
use film_core::Image;
use half::f16;

/// Geometry + dust/IR for baking a heal-ready working buffer (raw negative).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BakeSpec {
    pub rot90: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    pub angle: f32,
    pub image_crop: Option<[f64; 4]>,
    pub dust: Vec<DustStroke>,
    pub ir_removal: IrRemoval,
}

/// Geometry only (orient → straighten → persistent crop) on the raw negative.
/// This determines the baked dimensions; cheap relative to the Telea heal.
pub fn bake_geometry(working: &Image, spec: &BakeSpec) -> Image {
    let oriented = orient(working, spec.rot90, spec.flip_h, spec.flip_v);
    let straightened = rotate(&oriented, spec.angle);
    match spec.image_crop {
        Some(nc) => {
            let (x, y, w, h) = crate::commands::crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, x, y, w, h)
        }
        None => straightened,
    }
}

/// Apply geometry (orient → straighten → persistent crop) to the raw negative,
/// then heal dust strokes + IR defects IN THE RAW (pre-invert) DOMAIN. Returns the
/// baked raw-negative image; the GPU then inverts+finishes it with identity geometry.
pub fn bake_working(working: &Image, spec: &BakeSpec) -> Image {
    let mut img = bake_geometry(working, spec);
    // Strokes are normalized to this (post-geometry) image — same space export_stamps maps into.
    let stamps = export_stamps(&spec.dust, img.width, img.height);
    film_core::dust::apply(&mut img, &stamps);
    if spec.ir_removal.enabled {
        if let Some(ir) = img.ir.clone() {
            film_core::dust::apply_ir(&mut img, &ir, spec.ir_removal.sensitivity);
        }
    }
    img
}

/// Max GPU texture long-edge we will upload. WebGL2 guarantees at least 2048,
/// real GPUs >= 16384; 8192 is a safe, ample bound for the live proxy.
pub const MAX_GPU_EDGE: u32 = 8192;

/// The capped texture dimensions for `cap` long-edge, WITHOUT allocating pixels.
/// Mirrors `proxy`'s aspect-preserving downscale + rounding.
pub fn capped_dims(img: &Image, cap: u32) -> (u32, u32) {
    let long = img.width.max(img.height) as u32;
    if long <= cap {
        return (img.width as u32, img.height as u32);
    }
    let scale = cap as f32 / long as f32;
    let w = (img.width as f32 * scale).round().max(1.0) as u32;
    let h = (img.height as f32 * scale).round().max(1.0) as u32;
    (w, h)
}

/// Downscale (if needed) so the long edge <= `cap`, then pack the linear-RGB
/// pixels as little-endian half-float RGBA (alpha = 1.0). Returns the (possibly
/// reduced) dimensions and the byte buffer ready for `texImage2D(RGBA16F)`.
pub fn pack_rgba16f(img: &Image, cap: u32) -> (u32, u32, Vec<u8>) {
    let capped = proxy(img, cap); // no-op if already within cap
    let one = f16::from_f32(1.0).to_le_bytes();
    let mut bytes = Vec::with_capacity(capped.pixels.len() * 8);
    for px in &capped.pixels {
        bytes.extend_from_slice(&f16::from_f32(px[0]).to_le_bytes());
        bytes.extend_from_slice(&f16::from_f32(px[1]).to_le_bytes());
        bytes.extend_from_slice(&f16::from_f32(px[2]).to_le_bytes());
        bytes.extend_from_slice(&one);
    }
    (capped.width as u32, capped.height as u32, bytes)
}

use crate::commands::{build_params, mode_from, wb_from_params};
use crate::session::InvertParams;
use film_core::engine::Mode;
use serde::Serialize;

/// Flat, JS-friendly inversion uniforms. Matrices are column-major 9-vecs to
/// match GLSL `mat3` constructor/`uniformMatrix3fv` layout.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedInversion {
    pub base: [f32; 3],
    pub wb: [f32; 3],
    pub m_pre: [f32; 9],
    pub m_post: [f32; 9],
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    /// 0 = Mode B (density matrix), 1 = Mode C (per-channel). (2 = Naive exists in the shader but the app never emits it.)
    pub mode: u8,
}

/// Resolve the UI params (+ sampled film base) into GPU uniforms, reusing the
/// exact same param construction the CPU path uses (build_params + wb).
pub fn resolve_to_uniforms(p: &InvertParams, base: [f32; 3]) -> ResolvedInversion {
    let mut ip = build_params(p, base);
    ip.wb = wb_from_params(p.temp, p.tint);
    let mode = match mode_from(&p.mode) {
        Mode::B => 0u8,
        Mode::C => 1,
        Mode::Naive => 2,
    };
    let m_pre: [f32; 9] = ip.m_pre.as_slice().try_into().expect("mat3 has 9 elements");
    let m_post: [f32; 9] = ip.m_post.as_slice().try_into().expect("mat3 has 9 elements");
    ResolvedInversion {
        base: ip.base,
        wb: ip.wb,
        m_pre,
        m_post,
        exposure: ip.exposure,
        black: ip.black,
        gamma: ip.gamma,
        mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{DustStroke, IrRemoval};
    use crate::commands_test_support::sample_invert_params;
    use film_core::Image;
    use half::f16;

    #[test]
    fn bake_working_applies_geometry_then_heals() {
        // 4x4 solid grey with one bright speck; a dust stroke over the speck should
        // inpaint it toward the surrounding grey (pre-invert raw domain).
        let mut pixels = vec![[0.5_f32, 0.5, 0.5]; 16];
        pixels[5] = [0.9, 0.9, 0.9]; // speck at (x=1,y=1)
        let img = Image { width: 4, height: 4, pixels, ir: None };
        let spec = BakeSpec {
            rot90: 0, flip_h: false, flip_v: false, angle: 0.0, image_crop: None,
            dust: vec![DustStroke { points: vec![[0.25, 0.25]], r: 0.5 }], // centered on the speck
            ir_removal: IrRemoval { enabled: false, sensitivity: 0.0 },
        };
        let out = bake_working(&img, &spec);
        assert_eq!((out.width, out.height), (4, 4));
        // The speck should be healed toward grey, not still 0.9.
        assert!((out.pixels[5][0] - 0.5).abs() < 0.35, "speck healed: {}", out.pixels[5][0]);
    }

    #[test]
    fn bake_working_crop_changes_dims() {
        let img = Image { width: 10, height: 8, pixels: vec![[0.3, 0.3, 0.3]; 80], ir: None };
        let spec = BakeSpec {
            rot90: 0, flip_h: false, flip_v: false, angle: 0.0,
            image_crop: Some([0.0, 0.0, 0.5, 0.5]), // top-left quarter
            dust: vec![], ir_removal: IrRemoval { enabled: false, sensitivity: 0.0 },
        };
        let out = bake_working(&img, &spec);
        assert_eq!((out.width, out.height), (5, 4));
    }

    #[test]
    fn bake_geometry_dims_match_baked_pixels() {
        let img = Image { width: 10, height: 8, pixels: vec![[0.3, 0.3, 0.3]; 80], ir: None };
        let spec = BakeSpec {
            rot90: 1, flip_h: false, flip_v: true, angle: 0.0,
            image_crop: Some([0.1, 0.1, 0.5, 0.5]),
            dust: vec![], ir_removal: IrRemoval { enabled: false, sensitivity: 0.0 },
        };
        let geom = bake_geometry(&img, &spec);
        let baked = bake_working(&img, &spec);
        assert_eq!((geom.width, geom.height), (baked.width, baked.height));
        assert_eq!(capped_dims(&geom, MAX_GPU_EDGE), capped_dims(&baked, MAX_GPU_EDGE));
    }

    #[test]
    fn pack_rgba16f_one_pixel_round_trips_with_alpha_one() {
        let img = Image { width: 1, height: 1, pixels: vec![[0.25, 0.5, 0.75]], ir: None };
        let (w, h, bytes) = pack_rgba16f(&img, 8192);
        assert_eq!((w, h), (1, 1));
        assert_eq!(bytes.len(), 1 * 1 * 4 * 2, "RGBA, 2 bytes per channel");
        // Decode the 4 channels back from little-endian u16 half-floats.
        let chan = |i: usize| f16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]).to_f32();
        assert!((chan(0) - 0.25).abs() < 1e-3, "r");
        assert!((chan(1) - 0.50).abs() < 1e-3, "g");
        assert!((chan(2) - 0.75).abs() < 1e-3, "b");
        assert!((chan(3) - 1.0).abs() < 1e-3, "a defaults to 1.0");
    }

    #[test]
    fn pack_rgba16f_caps_long_edge() {
        // 10x4 image, cap 5 → downscaled so long edge <= 5, bytes match the capped dims.
        let img = Image { width: 10, height: 4, pixels: vec![[0.1, 0.2, 0.3]; 40], ir: None };
        let (w, h, bytes) = pack_rgba16f(&img, 5);
        assert!(w <= 5 && h <= 5, "long edge capped: {w}x{h}");
        assert_eq!(bytes.len(), (w * h * 4 * 2) as usize);
    }

    #[test]
    fn uniforms_none_stock_mode_c_is_identity_matrices_mode_1() {
        let mut p = sample_invert_params();
        p.stock = "none".into();
        p.mode = "c".into();
        p.exposure = 1.0; // 1 EV → 2.0x
        let u = resolve_to_uniforms(&p, [0.8, 0.6, 0.4]);
        assert_eq!(u.mode, 1, "c → 1");
        assert_eq!(u.base, [0.8, 0.6, 0.4]);
        // identity m_pre/m_post (column-major 9-vec)
        assert_eq!(u.m_pre, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(u.m_post, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert!((u.exposure - 2.0).abs() < 1e-5, "2^1");
    }

    #[test]
    fn capped_dims_matches_pack_dims() {
        let small = Image { width: 3, height: 2, pixels: vec![[0.0; 3]; 6], ir: None };
        let (pw, ph, _) = pack_rgba16f(&small, 8192);
        assert_eq!(capped_dims(&small, 8192), (pw, ph));
        let big = Image { width: 10, height: 4, pixels: vec![[0.0; 3]; 40], ir: None };
        let (bw, bh, _) = pack_rgba16f(&big, 5);
        assert_eq!(capped_dims(&big, 5), (bw, bh));
    }

    #[test]
    fn uniforms_portra_mode_b_fits_nonidentity_mpost_mode_0() {
        let mut p = sample_invert_params();
        p.stock = "portra400".into();
        p.mode = "b".into();
        let u = resolve_to_uniforms(&p, [0.8, 0.6, 0.4]);
        assert_eq!(u.mode, 0, "b → 0");
        // m_post from fit_m_post is NOT identity for a real stock
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_ne!(u.m_post, identity, "stock fit produces a real matrix");
    }
}
