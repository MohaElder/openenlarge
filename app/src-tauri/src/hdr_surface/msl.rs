//! MSL-side `HdrUniforms` struct declaration — the Metal mirror of
//! `hdr_surface::uniforms::HdrUniforms`. Field order and padding MUST stay
//! byte-for-byte identical between the two; `uniforms::tests::
//! hdr_uniforms_layout_matches_msl` pins the Rust side so it can't drift.
//!
//! Kept as a plain source string (like `EDR_SHADER_SRC` in `macos.rs`) rather
//! than a `.metal` file, since Tauri's HDR surface compiles shaders at runtime
//! via `MTLDevice::newLibraryWithSource_options_error`. Only the struct
//! declaration + the buffer-binding index are defined here; the actual
//! fragment/kernel FUNCTION bodies that take `constant HdrUniforms&` as a
//! parameter are added by later tasks (invert-pass, finish-pass, and
//! composition), which concatenate their own source onto
//! `HDR_UNIFORMS_STRUCT_MSL`.
//!
//! This file is plain data (a `&str` constant) with no Metal/objc bindings, so
//! it compiles and is exported on every target, not just macOS.
//!
//! Neither constant is read yet (Tasks 2-4 do that) — `#![allow(dead_code)]`
//! keeps the build warning-free in the meantime.
#![allow(dead_code)]

/// Metal `constant`-buffer struct mirroring `uniforms::HdrUniforms` field for
/// field. See that module's doc comment for the MSL layout rules (`float3`
/// padded to 16 bytes, `float3x3`/`float2x2` as column arrays, scalar arrays
/// packed tightly, whole-struct 16-byte alignment).
pub const HDR_UNIFORMS_STRUCT_MSL: &str = r#"
struct HdrUniforms {
    // --- finish (finish.rs::FinishParams / shaders.ts FRAG) ---
    float contrast;
    float highlights;
    float shadows;
    float whites;
    float blacks;
    float texture;
    float vibrance;
    float saturation;
    float brightness;

    float2 texel;                // 1/out_w, 1/out_h

    // Color grading (finish.rs::ColorGrade / colorGrade()).
    float3 cg_sh_off;
    float3 cg_mid_off;
    float3 cg_hi_off;
    float3 cg_glob_off;
    float cg_sh_lum;
    float cg_mid_lum;
    float cg_hi_lum;
    float cg_glob_lum;
    float cg_sh_edge;
    float cg_hi_edge;
    float cg_soft;

    // Color Mixer: 8-band HSL (finish.rs::ColorMix / colorMix()).
    float cm_hue[8];
    float cm_sat[8];
    float cm_lum[8];

    // Point Color: up to 8 samples.
    int pc_count;
    float pc_hue[8];
    float pc_sat[8];
    float pc_lum[8];
    float pc_hue_shift[8];
    float pc_sat_shift[8];
    float pc_lum_shift[8];
    float pc_variance[8];
    float pc_range[8];

    // Per-zone WB neutralizer (finish.rs::PerZoneWb / perZoneWb()).
    int pz_enabled;
    float3 pz_sh;
    float3 pz_mid;
    float3 pz_hi;

    // Clip-warning overlay (B1) — enables + the shared soft-clip knee.
    float clip_high_on;
    float clip_low_on;
    float clip_strict;
    float soft_clip;             // shared with invert (InversionParams::soft_clip)

    int finish_mode;             // 0 = present w/ clip overlay, 1 = plain FBO write
    float finalize_body;         // 1.0 = Faithful body finalize, 0.0 = already display-referred

    // --- invert (engine.rs::InversionParams / shaders.ts INVERT_FRAG) ---
    float3 base;
    float3 wb;
    float3x3 m_pre;
    float3x3 m_post;
    float exposure;
    float black;
    float gamma;
    float d_max;
    float print_exposure;
    float paper_black;
    float paper_grade;
    int mode;                    // 0=B 1=C 2=Naive 3=D (always 3 today)
    int raw;                     // GLSL `uniform bool`, wired via uniform1i — kept int
    int positive;                // GLSL `uniform bool`, wired via uniform1i — kept int
    int wb_mode;                 // 0 = gain (post-curve), 1 = subtractive (pre-curve)
    int tone_mode;                // 0 = filmic, 1 = faithful (always 1 today)
    float hi_recovery;
    float lo_recovery;
    float3 cam_balance;

    // --- geometry: output UV -> source UV mapping ---
    float2 crop_off;
    float2 crop_scale;
    float angle;
    float aspect;
    float2x2 orient;
    float2 view_off;
    float2 view_scale;
};
"#;

/// Argument-table buffer index the fused invert+finish shader binds
/// `constant HdrUniforms& u [[buffer(HDR_UNIFORMS_BUFFER_INDEX)]]` at. Not
/// referenced by any compiled function yet — Tasks 2-4 append fragment/kernel
/// bodies (which use this binding) onto `HDR_UNIFORMS_STRUCT_MSL`.
pub const HDR_UNIFORMS_BUFFER_INDEX: u64 = 0;
