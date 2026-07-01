//! `HdrUniforms`: the packed uniforms-buffer contract shared between this Rust
//! side and the MSL `constant HdrUniforms` struct declared in `msl.rs`. The two
//! MUST stay byte-for-byte identical — `hdr_uniforms_layout_matches_msl` below
//! pins the Rust layout so it can't silently drift from the shader struct.
//!
//! MSL `constant`-address-space layout rules used throughout (Apple's Metal
//! Shading Language Specification, "Metal and the C++ Standard Library" ch.):
//!   - scalar (`float`/`int`; GLSL `bool` uniforms are wired as ints via
//!     `uniform1i`, so they're `int` here too): 4 bytes, align 4.
//!   - `float2`: 8 bytes, align 8.
//!   - `float3` / `float4`: 16 bytes, align 16 (`float3` wastes 4 bytes and has
//!     the SAME size/alignment as `float4` — the single most common MSL layout
//!     gotcha, and the reason plain `[f32; 3]` can't be used directly).
//!   - `float3x3`: 3 columns of `float3` (16 bytes each) = 48 bytes, align 16.
//!   - `float2x2`: 2 columns of `float2` (8 bytes each) = 16 bytes, align 8.
//!   - an array of SCALAR elements is packed tightly at the scalar's own
//!     alignment (no extra per-element padding) — this differs from an array
//!     of VECTOR elements, where each element pads up to its own alignment
//!     (e.g. an array of `float3` would have stride 16). Not a concern here:
//!     every array in this struct (`cm_*`/`pc_*`) is `float[8]`, not
//!     `float3[8]`.
//!   - the whole struct is aligned to its largest member (16, from the
//!     `float3`/`float3x3` fields) and its size is padded up to a multiple of
//!     that alignment.
//!
//! Rust's `#[repr(C)]` inserts inter-field padding exactly like C (and MSL) do,
//! PROVIDED each field's Rust type reports the alignment its MSL counterpart
//! needs. Plain `[f32; 2]` / `[f32; 3]` do NOT (Rust gives bare arrays align 4),
//! so this module defines explicit `#[repr(C, align(N))]` wrapper types
//! (`Vec2`, `Vec3`, `Mat2`, `Mat3`) for every `float2`/`float3`/matrix field
//! instead of using bare arrays, and relies on `repr(C)`'s normal
//! padding-insertion behavior (no manual inter-field pad fields needed) for
//! everything else.
//!
//! Nothing in this module is wired into a Tauri command yet (that's a later
//! task's `set_uniforms` / HDR-surface-render command) — `#![allow(dead_code)]`
//! keeps the build warning-free in the meantime.
#![allow(dead_code)]

use crate::commands::{finish_from, ViewSpec};
use crate::gpu_upload::resolve_to_uniforms;
use crate::session::InvertParams;

/// MSL `float2` mirror: 8 bytes, 8-byte aligned.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}
impl From<[f32; 2]> for Vec2 {
    fn from(v: [f32; 2]) -> Self {
        Vec2 { x: v[0], y: v[1] }
    }
}

/// MSL `float3` mirror: 16 bytes (12 data + 4 pad), 16-byte aligned — MSL gives
/// `float3` the same size/alignment as `float4`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub _pad: f32,
}
impl From<[f32; 3]> for Vec3 {
    fn from(v: [f32; 3]) -> Self {
        Vec3 { x: v[0], y: v[1], z: v[2], _pad: 0.0 }
    }
}

/// MSL `float2x2` mirror: 2 columns of `float2`, 16 bytes total, 8-byte aligned.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Mat2 {
    pub c0: Vec2,
    pub c1: Vec2,
}
impl From<[f32; 4]> for Mat2 {
    /// `v` is column-major, matching GL's `uniformMatrix2fv`/MSL conventions:
    /// `[c0.x, c0.y, c1.x, c1.y]`.
    fn from(v: [f32; 4]) -> Self {
        Mat2 { c0: Vec2 { x: v[0], y: v[1] }, c1: Vec2 { x: v[2], y: v[3] } }
    }
}

/// MSL `float3x3` mirror: 3 columns of `float3`, 48 bytes total, 16-byte aligned.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Mat3 {
    pub c0: Vec3,
    pub c1: Vec3,
    pub c2: Vec3,
}
impl From<[f32; 9]> for Mat3 {
    /// `v` is column-major 9-vec, matching `ResolvedInversion::m_pre`/`m_post`
    /// and GL's `uniformMatrix3fv`: `[c0(3), c1(3), c2(3)]`.
    fn from(v: [f32; 9]) -> Self {
        Mat3 {
            c0: [v[0], v[1], v[2]].into(),
            c1: [v[3], v[4], v[5]].into(),
            c2: [v[6], v[7], v[8]].into(),
        }
    }
}

/// Clip-warning overlay toggle state (mirrors `clipUniforms` /
/// `ClipUniforms` in `app/src/lib/viewport/gl/clip.ts`). Kept as its own tiny
/// struct — not part of `InvertParams` — because it's transient UI view state
/// (a toolbar toggle), not a persisted edit param.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClipState {
    pub high: bool,
    pub low: bool,
    pub strict: bool,
}

/// Packed uniforms buffer for the fused invert+finish HDR/EDR Metal shader.
/// Every field mirrors a GLSL uniform declared in `FRAG` or `INVERT_FRAG`
/// (`app/src/lib/viewport/gl/shaders.ts`); see the MSL twin in `msl.rs`.
/// `#[repr(C, align(16))]` matches MSL's top-level `constant` struct alignment
/// (16, driven by the `float3`/`float3x3` members) and its size-rounds-to-16
/// rule (`repr(C)` pads the struct's end to a multiple of its alignment, same
/// as MSL/C).
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HdrUniforms {
    // --- finish (finish.rs::FinishParams / shaders.ts FRAG, minus the LUT
    // texture + the tone-curve regions, which stay a sampled LUT, not uniforms) ---
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub brightness: f32,

    /// `u_texel`: 1/out_w, 1/out_h of the finishing viewport.
    pub texel: Vec2,

    // Color grading (finish.rs::ColorGrade / colorGrade()).
    pub cg_sh_off: Vec3,
    pub cg_mid_off: Vec3,
    pub cg_hi_off: Vec3,
    pub cg_glob_off: Vec3,
    pub cg_sh_lum: f32,
    pub cg_mid_lum: f32,
    pub cg_hi_lum: f32,
    pub cg_glob_lum: f32,
    pub cg_sh_edge: f32,
    pub cg_hi_edge: f32,
    pub cg_soft: f32,

    // Color Mixer: 8-band HSL (finish.rs::ColorMix / colorMix()).
    pub cm_hue: [f32; 8],
    pub cm_sat: [f32; 8],
    pub cm_lum: [f32; 8],

    // Point Color: up to 8 samples.
    pub pc_count: i32,
    pub pc_hue: [f32; 8],
    pub pc_sat: [f32; 8],
    pub pc_lum: [f32; 8],
    pub pc_hue_shift: [f32; 8],
    pub pc_sat_shift: [f32; 8],
    pub pc_lum_shift: [f32; 8],
    pub pc_variance: [f32; 8],
    pub pc_range: [f32; 8],

    // Per-zone WB neutralizer (finish.rs::PerZoneWb / perZoneWb()).
    pub pz_enabled: i32,
    pub pz_sh: Vec3,
    pub pz_mid: Vec3,
    pub pz_hi: Vec3,

    // Clip-warning overlay (B1) — enables + the shared soft-clip knee.
    pub clip_high_on: f32,
    pub clip_low_on: f32,
    pub clip_strict: f32,
    pub soft_clip: f32, // shared with invert: InversionParams::soft_clip

    /// 0 = present (with clip overlay); 1 = write the plain finished color to
    /// an FBO for a separate texture/USM pass. The fused single-pass Metal
    /// shader has no separate USM pass yet, so `from_params` always sets 0.
    pub finish_mode: i32,
    /// 1.0 = Faithful-SDR-negative body (finalize at end of tone()); 0.0 =
    /// already display-referred (positive passthrough / HDR rendition).
    pub finalize_body: f32,

    // --- invert (engine.rs::InversionParams / shaders.ts INVERT_FRAG) ---
    pub base: Vec3,
    pub wb: Vec3,
    pub m_pre: Mat3,
    pub m_post: Mat3,
    pub exposure: f32,
    pub black: f32,
    pub gamma: f32,
    pub d_max: f32,
    pub print_exposure: f32,
    pub paper_black: f32,
    pub paper_grade: f32,
    /// 0=B 1=C 2=Naive 3=D (always 3 today — one engine, Cineon negadoctor).
    pub mode: i32,
    /// GLSL `uniform bool u_raw`, wired via `uniform1i` — kept `int` here for
    /// natural 4-byte layout (Metal `bool` is 1 byte, which would misalign
    /// neighboring scalar fields).
    pub raw: i32,
    /// GLSL `uniform bool u_positive` — see `raw` above.
    pub positive: i32,
    /// 0 = gain (post-curve), 1 = subtractive (pre-curve, color-head look).
    pub wb_mode: i32,
    /// 0 = filmic, 1 = faithful (always 1 today — Faithful is the sole path).
    pub tone_mode: i32,
    pub hi_recovery: f32,
    pub lo_recovery: f32,
    pub cam_balance: Vec3,

    // --- geometry: output UV → source UV mapping ---
    pub crop_off: Vec2,
    pub crop_scale: Vec2,
    pub angle: f32,
    pub aspect: f32,
    pub orient: Mat2,
    pub view_off: Vec2,
    pub view_scale: Vec2,
}

/// Rust port of `orientUVMatrix` (`app/src/lib/crop/transforms.ts`): the
/// column-major 2×2 mapping oriented UV → source UV that undoes rot90/flip.
/// MUST stay numerically identical to the TS twin (same trig, same column
/// order) since both feed the same `u_orient` uniform semantics.
fn orient_uv_matrix(rot90: u8, flip_h: bool, flip_v: bool) -> [f32; 4] {
    let ang = (rot90 % 4) as f32 * (std::f32::consts::PI / 2.0);
    let (s, c) = ang.sin_cos();
    let fx = if flip_h { -1.0 } else { 1.0 };
    let fy = if flip_v { -1.0 } else { 1.0 };
    [c * fx, -s * fy, s * fx, c * fy]
}

impl HdrUniforms {
    /// Build the packed uniforms buffer from the UI's edit params, the current
    /// view, and the clip-warning toggle state. Reuses the exact same
    /// finish/invert param resolution the live WebGL2 renderer drives
    /// (`commands::finish_from`, `gpu_upload::resolve_to_uniforms`) so the
    /// fused Metal shader (Tasks 2-4) stays numerically identical to the
    /// GL path for every field it CAN resolve from `InvertParams` alone.
    ///
    /// Documented gap (left for the consuming command, later tasks): `base`,
    /// `d_max`, and `cam_balance` normally come from the per-image
    /// `Developed` record (the develop-time auto film-base / D_max /
    /// channel-balance) — data this pure-`InvertParams` builder can't see.
    /// Only the explicit `base_override`/`d_max_override` (if the user set
    /// one) are honored here, falling back to the exact same defaults
    /// `commands::build_params` uses ([1,1,1] base, D_max 1.5, identity
    /// channel balance). The `set_uniforms` command should overwrite those
    /// three fields from the session's `Developed`, mirroring how
    /// `resolved_inversion` overwrites `dev.base`/`dev.d_max`/
    /// `dev.channel_balance` after calling the same `resolve_to_uniforms`.
    /// Geometry is mostly resolvable here: `crop_off`/`crop_scale` come
    /// straight from `ViewSpec.image_crop` (the normalized `[x,y,w,h]`
    /// persistent crop, mapped directly like `Viewport.svelte`'s `imageCrop ??
    /// [0,0,1,1]` — no pixel math), and `orient`/`angle` derive from
    /// `ViewSpec`'s rot90/flip/angle. Only `aspect` (needs the oriented
    /// working-image pixel dimensions) and the deep-zoom `view_off`/
    /// `view_scale` (need the visible-window state) are genuinely
    /// deferred-to-consumer — they default to identity here and the
    /// `set_uniforms` command should patch them from the same
    /// working-image/window state, alongside `base`/`d_max`/`cam_balance`.
    pub fn from_params(params: &InvertParams, view: &ViewSpec, clip: &ClipState) -> HdrUniforms {
        let base = params.base_override.unwrap_or([1.0, 1.0, 1.0]);
        let ri = resolve_to_uniforms(params, base);
        let fp = finish_from(params);

        let pc_count = fp.cm.samples.len().min(8);
        let mut pc_hue = [0.0f32; 8];
        let mut pc_sat = [0.0f32; 8];
        let mut pc_lum = [0.0f32; 8];
        let mut pc_hue_shift = [0.0f32; 8];
        let mut pc_sat_shift = [0.0f32; 8];
        let mut pc_lum_shift = [0.0f32; 8];
        let mut pc_variance = [0.0f32; 8];
        let mut pc_range = [0.0f32; 8];
        for (i, s) in fp.cm.samples.iter().take(8).enumerate() {
            pc_hue[i] = s.hue;
            pc_sat[i] = s.sat;
            pc_lum[i] = s.lum;
            pc_hue_shift[i] = s.hue_shift;
            pc_sat_shift[i] = s.sat_shift;
            pc_lum_shift[i] = s.lum_shift;
            pc_variance[i] = s.variance;
            pc_range[i] = s.range;
        }

        let out_w = (view.out_w.max(1)) as f32;
        let out_h = (view.out_h.max(1)) as f32;

        // Persistent crop: ViewSpec.image_crop carries the normalized [x,y,w,h]
        // directly (mirrors Viewport.svelte's `imageCrop ?? [0,0,1,1]` → crop
        // off/scale — no pixel math). None = whole image (identity).
        let ic = view.image_crop.unwrap_or([0.0, 0.0, 1.0, 1.0]);

        HdrUniforms {
            contrast: fp.contrast,
            highlights: fp.highlights,
            shadows: fp.shadows,
            whites: fp.whites,
            blacks: fp.blacks,
            texture: fp.texture,
            vibrance: fp.vibrance,
            saturation: fp.saturation,
            brightness: fp.brightness,

            texel: [1.0 / out_w, 1.0 / out_h].into(),

            cg_sh_off: fp.cg.sh_off.into(),
            cg_mid_off: fp.cg.mid_off.into(),
            cg_hi_off: fp.cg.hi_off.into(),
            cg_glob_off: fp.cg.glob_off.into(),
            cg_sh_lum: fp.cg.sh_lum,
            cg_mid_lum: fp.cg.mid_lum,
            cg_hi_lum: fp.cg.hi_lum,
            cg_glob_lum: fp.cg.glob_lum,
            cg_sh_edge: fp.cg.sh_edge,
            cg_hi_edge: fp.cg.hi_edge,
            cg_soft: fp.cg.softness,

            cm_hue: fp.cm.cm_hue,
            cm_sat: fp.cm.cm_sat,
            cm_lum: fp.cm.cm_lum,

            pc_count: pc_count as i32,
            pc_hue,
            pc_sat,
            pc_lum,
            pc_hue_shift,
            pc_sat_shift,
            pc_lum_shift,
            pc_variance,
            pc_range,

            pz_enabled: fp.per_zone.enabled as i32,
            pz_sh: fp.per_zone.sh.into(),
            pz_mid: fp.per_zone.mid.into(),
            pz_hi: fp.per_zone.hi.into(),

            clip_high_on: if clip.high { 1.0 } else { 0.0 },
            clip_low_on: if clip.low { 1.0 } else { 0.0 },
            clip_strict: if clip.strict { 1.0 } else { 0.0 },
            soft_clip: ri.soft_clip,

            finish_mode: 0,
            finalize_body: if fp.finalize_body { 1.0 } else { 0.0 },

            base: ri.base.into(),
            wb: ri.wb.into(),
            m_pre: ri.m_pre.into(),
            m_post: ri.m_post.into(),
            exposure: ri.exposure,
            black: ri.black,
            gamma: ri.gamma,
            d_max: ri.d_max,
            print_exposure: ri.print_exposure,
            paper_black: ri.paper_black,
            paper_grade: ri.paper_grade,
            mode: ri.mode as i32,
            raw: view.raw as i32,
            positive: ri.positive as i32,
            wb_mode: ri.wb_mode as i32,
            tone_mode: ri.tone_mode as i32,
            hi_recovery: ri.hi_recovery,
            lo_recovery: ri.lo_recovery,
            cam_balance: ri.channel_balance.into(),

            crop_off: [ic[0] as f32, ic[1] as f32].into(),
            crop_scale: [ic[2] as f32, ic[3] as f32].into(),
            angle: view.angle.to_radians(),
            aspect: 1.0,
            orient: orient_uv_matrix(view.rot90, view.flip_h, view.flip_v).into(),
            view_off: [0.0, 0.0].into(),
            view_scale: [1.0, 1.0].into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{AutoDust, DustStroke, IrRemoval};
    use crate::commands_test_support::sample_invert_params;

    fn sample_view_spec() -> ViewSpec {
        ViewSpec {
            crop: [0.0, 0.0, 1.0, 1.0],
            out_w: 1024,
            out_h: 768,
            raw: false,
            finish: true,
            image_crop: None,
            rot90: 0,
            flip_h: false,
            flip_v: false,
            angle: 0.0,
            dust: Vec::<DustStroke>::new(),
            ir_removal: IrRemoval::default(),
            auto_dust: AutoDust::default(),
        }
    }

    #[test]
    fn hdr_uniforms_layout_matches_msl() {
        // MSL `constant HdrUniforms` packs float=4 (align 4), float2=8 (align
        // 8), float3=16 (align 16, 4 bytes wasted vs the 12 data bytes),
        // float3x3 = 3 columns of float3 = 48 (align 16), float2x2 = 2 columns
        // of float2 = 16 (align 8). Scalar arrays (cm_*/pc_*) are packed
        // tightly at the scalar's own 4-byte alignment — no vector-array
        // padding applies, since every array here is `float[8]`, not
        // `float3[8]`. See the module doc comment for the full rule set and
        // `msl.rs` for the MSL-side twin.
        assert_eq!(std::mem::align_of::<HdrUniforms>(), 16);
        // size must be a multiple of 16 (MSL constant-buffer rule).
        assert_eq!(std::mem::size_of::<HdrUniforms>() % 16, 0);

        // Field order (documented so a future edit can recompute these by
        // hand): contrast..brightness (9x f32) | texel (Vec2, 4B lead-in pad)
        // | cg_*_off x4 (Vec3) | cg_*_lum x4 (f32) | cg_sh_edge/cg_hi_edge/
        // cg_soft (f32x3) | cm_hue/sat/lum (f32x8 x3) | pc_count (i32) | pc_*
        // x8 arrays (f32x8 x8) | pz_enabled (i32) | pz_sh/mid/hi (Vec3x3, 12B
        // lead-in pad) | clip_high_on/low_on/strict/soft_clip (f32x4) |
        // finish_mode (i32) | finalize_body (f32) | base/wb (Vec3x2) |
        // m_pre/m_post (Mat3x2) | exposure..paper_grade (f32x7) |
        // mode/raw/positive/wb_mode/tone_mode (i32x5) |
        // hi_recovery/lo_recovery (f32x2) | cam_balance (Vec3, 8B lead-in pad)
        // | crop_off/crop_scale (Vec2x2) | angle/aspect (f32x2) | orient
        // (Mat2) | view_off/view_scale (Vec2x2).
        assert_eq!(std::mem::offset_of!(HdrUniforms, contrast), 0);
        assert_eq!(std::mem::offset_of!(HdrUniforms, brightness), 32);
        assert_eq!(std::mem::offset_of!(HdrUniforms, texel), 40);
        assert_eq!(std::mem::offset_of!(HdrUniforms, cg_sh_off), 48);
        assert_eq!(std::mem::offset_of!(HdrUniforms, cm_hue), 140);
        assert_eq!(std::mem::offset_of!(HdrUniforms, pc_count), 236);
        assert_eq!(std::mem::offset_of!(HdrUniforms, pz_enabled), 496);
        assert_eq!(std::mem::offset_of!(HdrUniforms, pz_sh), 512);
        assert_eq!(std::mem::offset_of!(HdrUniforms, finish_mode), 576);
        assert_eq!(std::mem::offset_of!(HdrUniforms, base), 592);
        assert_eq!(std::mem::offset_of!(HdrUniforms, m_pre), 624);
        assert_eq!(std::mem::offset_of!(HdrUniforms, m_post), 672);
        assert_eq!(std::mem::offset_of!(HdrUniforms, mode), 748);
        assert_eq!(std::mem::offset_of!(HdrUniforms, cam_balance), 784);
        assert_eq!(std::mem::offset_of!(HdrUniforms, crop_off), 800);
        assert_eq!(std::mem::offset_of!(HdrUniforms, orient), 824);
        assert_eq!(std::mem::offset_of!(HdrUniforms, view_scale), 848);
    }

    #[test]
    fn from_params_maps_clip_state() {
        let p = sample_invert_params();
        let v = sample_view_spec();
        let u = HdrUniforms::from_params(&p, &v, &ClipState { high: true, low: false, strict: true });
        assert_eq!(u.clip_high_on, 1.0);
        assert_eq!(u.clip_low_on, 0.0);
        assert_eq!(u.clip_strict, 1.0);
    }

    #[test]
    fn from_params_maps_finish_sliders() {
        let mut p = sample_invert_params();
        p.contrast = 50.0;
        p.highlights = -40.0; // negative half suppressed, mirrors finish_from
        let v = sample_view_spec();
        let u = HdrUniforms::from_params(&p, &v, &ClipState::default());
        assert!((u.contrast - 0.5).abs() < 1e-6);
        assert_eq!(u.highlights, 0.0);
    }

    #[test]
    fn from_params_derives_texel_from_view() {
        let p = sample_invert_params();
        let mut v = sample_view_spec();
        v.out_w = 2000;
        v.out_h = 1000;
        let u = HdrUniforms::from_params(&p, &v, &ClipState::default());
        assert!((u.texel.x - 1.0 / 2000.0).abs() < 1e-9);
        assert!((u.texel.y - 1.0 / 1000.0).abs() < 1e-9);
    }

    #[test]
    fn from_params_maps_crop_from_image_crop() {
        let p = sample_invert_params();
        let mut v = sample_view_spec();
        v.image_crop = Some([0.1, 0.2, 0.5, 0.6]);
        let u = HdrUniforms::from_params(&p, &v, &ClipState::default());
        assert!((u.crop_off.x - 0.1).abs() < 1e-6);
        assert!((u.crop_off.y - 0.2).abs() < 1e-6);
        assert!((u.crop_scale.x - 0.5).abs() < 1e-6);
        assert!((u.crop_scale.y - 0.6).abs() < 1e-6);
        // None → whole-image identity.
        v.image_crop = None;
        let u2 = HdrUniforms::from_params(&p, &v, &ClipState::default());
        assert_eq!(u2.crop_off, Vec2 { x: 0.0, y: 0.0 });
        assert_eq!(u2.crop_scale, Vec2 { x: 1.0, y: 1.0 });
    }
}
