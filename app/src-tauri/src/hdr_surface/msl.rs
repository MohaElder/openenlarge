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
/// `constant HdrUniforms& u [[buffer(HDR_UNIFORMS_BUFFER_INDEX)]]` at. The
/// invert stage (`INVERT_FRAG_MSL`) binds the uniforms here; `macos.rs` sets it
/// via `setFragmentBytes:length:atIndex:` (inline constant, < 4KB).
pub const HDR_UNIFORMS_BUFFER_INDEX: u64 = 0;

/// MSL invert stage — a port of `INVERT_FRAG` (`app/src/lib/viewport/gl/
/// shaders.ts`), which itself mirrors `crates/film-core/src/engine.rs`'s invert
/// math. Concatenated AFTER a preamble (`#include`, `using namespace metal`, the
/// `VOut` struct + `edr_vertex`) and `HDR_UNIFORMS_STRUCT_MSL`, so it can
/// reference `VOut` and `constant HdrUniforms&`.
///
/// The fragment samples the raw linear negative (RGBA16F), applies geometry
/// (crop → un-straighten → un-orient, from the geometry uniform fields) as an
/// output-UV → source-UV transform, then runs the per-channel log-density
/// inversion for the four modes. Output is UNCLAMPED for the active Faithful
/// Mode-D body (super-white preserved for the finish stage); the Filmic and
/// B/C/Naive branches clamp exactly as the GLSL/engine do.
///
/// Constants here MUST equal `engine.rs` / the GLSL twin: `EXPO_K=0.14`,
/// `FAITHFUL_EXPO_K=1.0`, `CMY_STRENGTH=1.6`, `FAITHFUL_GAMMA=1.590`,
/// `FAITHFUL_SCALE=1/0.700`, `FILMIC_K=5.0`, `FILMIC_PIVOT=0.44`,
/// `FILMIC_WHITE_T=1.05`. (Constants only used by the *finish* stage —
/// `FAITHFUL_KNEE`, `LOOK_K`, `REC_*_GAIN` — are intentionally omitted; they
/// belong to Task 4.)
///
/// One deliberate deviation from the GLSL twin: no `uv.y = 1 - uv.y` flip. The
/// Metal fullscreen triangle already yields image-space (y-down, uv.y=0 = top)
/// coordinates matching the source texture's row order, whereas WebGL's `v_uv`
/// is y-up and must be flipped. Net orientation is identical.
pub const INVERT_FRAG_MSL: &str = r#"
// ---- invert stage: port of INVERT_FRAG (shaders.ts) / engine.rs invert ----
constant float INV_EPS = 1e-5;
constant float LOG10 = 0.30102999566;         // log10(x) = log2(x) * LOG10
constant float EXPO_K = 0.14;                  // MUST equal engine.rs EXPO_K
constant float FAITHFUL_EXPO_K = 1.0;          // MUST equal engine.rs FAITHFUL_EXPO_K
constant float CMY_STRENGTH = 1.6;             // MUST equal engine.rs CMY_STRENGTH
constant float FAITHFUL_GAMMA = 1.590;         // MUST equal engine.rs
constant float FAITHFUL_SCALE = 1.0 / 0.700;   // MUST equal engine.rs (1/recommended_d_max)
constant float FILMIC_K = 5.0;                 // MUST equal engine.rs FILMIC_K
constant float FILMIC_PIVOT = 0.44;            // MUST equal engine.rs FILMIC_PIVOT
constant float FILMIC_WHITE_T = 1.05;          // MUST equal engine.rs FILMIC_WHITE_T

// Faithful gamma BODY only (no shoulder/look/clamp) — super-white preserved.
float gammaBody(float x) { return pow(max(x, 0.0), 1.0 / FAITHFUL_GAMMA); }
float filmicL(float x) { return 1.0 / (1.0 + exp(-FILMIC_K * (x - FILMIC_PIVOT))); }
float filmicSraw(float t) {
    float l0 = filmicL(0.0);
    float lw = filmicL(FILMIC_WHITE_T);
    return (filmicL(t) - l0) / (lw - l0);
}
float filmicS(float t) { return clamp(filmicSraw(t), 0.0, 1.0); }
float filmicInv(float y) {
    float l0 = filmicL(0.0);
    float lw = filmicL(FILMIC_WHITE_T);
    float big = clamp(y * (lw - l0) + l0, 1e-6, 1.0 - 1e-6);
    return FILMIC_PIVOT + log(big / (1.0 - big)) / FILMIC_K;
}
float tone(float v, float gain, float exposure, float black, float gamma) {
    v = max(v * exposure * gain - black, 0.0);
    return pow(v, gamma);
}

float3 hdr_invert(float3 rgbIn, constant HdrUniforms& u) {
    float3 r = clamp(float3(
        rgbIn.r / max(u.base.r, INV_EPS),
        rgbIn.g / max(u.base.g, INV_EPS),
        rgbIn.b / max(u.base.b, INV_EPS)), INV_EPS, 1.0);
    if (u.mode == 3) {                 // Mode D: Cineon negadoctor. Mirrors engine.rs invert_d.
        const float THRESH = 2.3283064e-10;
        float3 clamped = max(rgbIn, float3(THRESH));
        float3 dmin = max(u.base, float3(INV_EPS));
        float3 d = max(log2(dmin / clamped) * LOG10, float3(0.0));   // log10(dmin/clamped)
        float ev = log2(max(u.print_exposure, INV_EPS));
        float expo_gain = exp2(EXPO_K * ev);
        float3 t = d / max(u.d_max, INV_EPS);                        // d == d_max -> t == 1 (white)
        float3 v;
        if (u.tone_mode == 1) {
            // Faithful: gamma body (unclamped); finish stage applies shoulder + look.
            float3 lScene = max(pow(float3(10.0), d * u.cam_balance) - 1.0, 0.0);
            float3 lit = lScene * exp2(FAITHFUL_EXPO_K * ev);
            float3 te = log2(lit + 1.0) * LOG10 * FAITHFUL_SCALE;
            if (u.wb_mode == 1) {
                float3 s = pow(max(u.wb, float3(INV_EPS)), float3(CMY_STRENGTH));
                v = float3(gammaBody(te.r * s.r), gammaBody(te.g * s.g), gammaBody(te.b * s.b));
            } else {
                v = float3(gammaBody(te.r) * u.wb.r, gammaBody(te.g) * u.wb.g, gammaBody(te.b) * u.wb.b);
            }
            return v;   // super-white body, UNCLAMPED — finish stage finalizes.
        } else {
            // Filmic: logistic S-curve on WB-neutralised log-density.
            if (u.wb_mode == 1) {
                float3 s = pow(max(u.wb, float3(INV_EPS)), float3(CMY_STRENGTH));
                v = float3(
                    filmicS(t.r * s.r * expo_gain),
                    filmicS(t.g * s.g * expo_gain),
                    filmicS(t.b * s.b * expo_gain));
            } else {
                float3 y = float3(filmicSraw(t.r), filmicSraw(t.g), filmicSraw(t.b)) * u.wb;
                v = float3(
                    filmicS(filmicInv(y.r) * expo_gain),
                    filmicS(filmicInv(y.g) * expo_gain),
                    filmicS(filmicInv(y.b) * expo_gain));
            }
        }
        return clamp(v, 0.0, 1.0);
    }
    if (u.mode == 2) {                 // Naive: 1 - clamp(I/base, 0, 1). engine.rs invert_naive.
        float3 n = clamp(float3(
            rgbIn.r / max(u.base.r, INV_EPS),
            rgbIn.g / max(u.base.g, INV_EPS),
            rgbIn.b / max(u.base.b, INV_EPS)), 0.0, 1.0);
        return 1.0 - n;
    }
    if (u.mode == 1) {                 // Mode C: per-channel log density.
        float3 dens = -float3(log2(r.r), log2(r.g), log2(r.b)) * LOG10;
        return float3(
            tone(dens.r, u.wb.r, u.exposure, u.black, u.gamma),
            tone(dens.g, u.wb.g, u.exposure, u.black, u.gamma),
            tone(dens.b, u.wb.b, u.exposure, u.black, u.gamma));
    }
    // Mode B: M_post * (-log10(M_pre * r)) then tone.
    float3 mixed = u.m_pre * r;
    float3 dens = -float3(
        log2(max(mixed.r, INV_EPS)), log2(max(mixed.g, INV_EPS)), log2(max(mixed.b, INV_EPS))) * LOG10;
    float3 unmixed = u.m_post * dens;
    return float3(
        tone(unmixed.r, u.wb.r, u.exposure, u.black, u.gamma),
        tone(unmixed.g, u.wb.g, u.exposure, u.black, u.gamma),
        tone(unmixed.b, u.wb.b, u.exposure, u.black, u.gamma));
}

// Map output UV -> source UV through crop + un-straighten + un-orient. The
// output is the crop sub-rect of the (straightened) oriented image, so this
// inverts the backend's source->output order (orient -> straighten -> crop).
// No y-flip (see the INVERT_FRAG_MSL doc comment).
float2 hdr_source_uv(float2 uv, constant HdrUniforms& u) {
    uv = u.view_off + uv * u.view_scale;                 // deep-zoom window (identity = off 0, scale 1)
    float2 c = u.crop_off + uv * u.crop_scale - 0.5;     // into the (straightened) oriented frame, centred
    float s = sin(u.angle), co = cos(u.angle);           // un-straighten in oriented PIXEL space
    c = float2x2(float2(co, -s / u.aspect), float2(s * u.aspect, co)) * c;
    c = u.orient * c;                                    // un-orient (rot90/flip) into source UV
    return c + 0.5;
}

fragment float4 invert_frag(VOut in [[stage_in]],
                            texture2d<float> src [[texture(0)]],
                            constant HdrUniforms& u [[buffer(0)]]) {
    constexpr sampler smp(filter::linear, address::clamp_to_edge);
    float2 suv = hdr_source_uv(in.uv, u);
    if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
        return float4(0.0, 0.0, 0.0, 1.0);               // outside source (straighten corners) = black
    }
    float3 rgb = src.sample(smp, suv).rgb;
    if (u.raw != 0) {                                    // output the scan (display gamma), no inversion
        return float4(pow(clamp(rgb, 0.0, 1.0), float3(1.0 / 2.2)), 1.0);
    }
    if (u.positive != 0) {                               // positive passthrough (scan + exposure/WB)
        return float4(pow(max(rgb * u.print_exposure * u.wb, 0.0), float3(1.0 / 2.2)), 1.0);
    }
    return float4(hdr_invert(rgb, u), 1.0);
}
"#;
