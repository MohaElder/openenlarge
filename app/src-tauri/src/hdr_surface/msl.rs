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

/// MSL finish stage — a port of `FRAG` (`app/src/lib/viewport/gl/shaders.ts:16-332`),
/// which mirrors `crates/film-core/src/finish.rs::finish_pixel`. Concatenated
/// AFTER the preamble + `HDR_UNIFORMS_STRUCT_MSL` + `INVERT_FRAG_MSL` (it reuses
/// `INV_EPS` from the invert stage and `VOut`/`HdrUniforms`). Reads the inverted
/// intermediate at `texture(0)`, the 256×1 composed tone LUT at `texture(1)`,
/// and `HdrUniforms` at `buffer(0)`.
///
/// Pipeline (identical order to `finish_pixel` / `finishAt`): per-zone WB →
/// brightness/density (`10^(b·0.5)`) → tone body (whites/blacks cubic,
/// highlights/shadows smoothstep, contrast pivot — NO leading clamp, NO
/// SDR display-finalize) → OKLab saturation → tone LUT → color grade → color
/// mix (8-band) → point color → **HDR finalize** → clipping overlay.
///
/// Constants MUST equal `finish.rs` / the GLSL twin: tone gains 0.20/0.20/0.18/
/// 0.18, `BRIGHTNESS_DENSITY_RANGE=0.5`, `SAT_C_REF=0.20`, `SAT_C_NEUTRAL=0.025`,
/// `SKIN_HUE=0.70`, `SKIN_WIDTH=0.55`, `SKIN_DAMP=0.5`, the OKLab matrices, the
/// color-mixer band centres + gains, the point-color tolerances, `HDR_KNEE=0.8`,
/// `HDR_HEADROOM=2.5` (`engine.rs`).
///
/// HDR-FINALIZE DESIGN (Task 4 + Task 6 fixes): the creative color ops
/// (saturation/LUT/grade/mix/point) run on the **display-finalized** body
/// `displayFinalize(toneBody)` — the Faithful shoulder + `lookS` contrast — EXACTLY
/// as `finish_pixel` / GLSL `finalize_body=true` do, giving the SDR finished color
/// `disp` in [0,1]. The HDR highlight is then a CHROMA-PRESERVING finalize —
/// mirror of `film-core::hdr_finish` (Sub-project C): below `HDR_KNEE` the output
/// is exactly `disp` (SDR parity); above, the highlight is the pre-shoulder body's
/// chromaticity `bodyU/mU` scaled to the HDR shoulder luminance
/// `hdr_finalize_scalar(mU)` (so a blown highlight keeps its hue into headroom
/// instead of graying), blended `mix(disp, highlight, smoothstep(HDR_KNEE,
/// HDR_W_HI, mU))` (`mU = max(bodyU)`).
/// One deliberate deviation from the GLSL, flagged in the report: the blacks term
/// uses `(1-v)^3` via multiplication (matching `finish.rs`'s `.powi(3)`), not
/// `pow(1-v,3.0)` which is NaN for the super-white `v>1` case.
pub const FINISH_FRAG_MSL: &str = r#"
// ---- finish stage: port of FRAG (shaders.ts) / finish.rs::finish_pixel ----
constant float FIN_PI = 3.14159265358979;
constant float FIN_BRIGHTNESS_RANGE = 0.5;      // MUST equal finish.rs BRIGHTNESS_DENSITY_RANGE
constant float HDR_KNEE = 0.8;                  // MUST equal engine.rs HDR_KNEE
constant float HDR_HEADROOM = 2.5;              // MUST equal engine.rs HDR_HEADROOM
constant float HDR_W_HI = 1.2;                  // MUST equal finish.rs HDR_W_HI (blend top)
// Faithful display-finalize (shoulder + lookS contrast) — MUST equal engine.rs
// display_finalize / shaders.ts. FAITHFUL_GAMMA is already declared by the invert stage.
constant float FAITHFUL_KNEE = 0.892;
constant float LOOK_K = 2.0;
// OKLab saturation constants (MUST equal finish.rs).
constant float SAT_C_REF = 0.20;
constant float SAT_C_NEUTRAL = 0.025;
constant float SKIN_HUE = 0.70;
constant float SKIN_WIDTH = 0.55;
constant float SKIN_DAMP = 0.5;
// Color-mixer / point-color constants (MUST equal shaders.ts).
constant float BAND_CENTERS[8] = { 0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0 };
constant float CM_FALLOFF_DEG = 50.0;
constant float CM_HUE_SHIFT_MAX = 30.0;
constant float CM_LUM_GAIN = 0.25;
constant float CM_SAT_GATE_LO = 0.05;
constant float CM_SAT_GATE_HI = 0.20;
constant float PC_RANGE_MIN_DEG = 5.0;
constant float PC_RANGE_MAX_DEG = 60.0;
constant float PC_SAT_TOL = 0.25;
constant float PC_LUM_TOL = 0.25;
constant float PC_VAR_SPAN = 2.0;
// Clipping-overlay thresholds (MUST equal shaders.ts).
constant float CLIP_LO = 2.0 / 255.0;
constant float CLIP_LO_STRICT = 8.0 / 255.0;
constant float CLIP_HI = 0.992;
constant float CLIP_HI_STRICT = 0.96;

// GLSL `mod` (result takes sign of the divisor), not C fmod — needed for the hue wraps.
float fin_mod(float x, float y) { return x - y * floor(x / y); }

float3 applyPerZoneWb(float3 rgb, constant HdrUniforms& u) {
    if (u.pz_enabled == 0) return rgb;
    float L = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    float wsh = 1.0 - smoothstep(0.08, 0.58, L);
    float whi = smoothstep(0.41, 0.91, L);
    float wmid = clamp(1.0 - wsh - whi, 0.0, 1.0);
    float3 gain = wsh * u.pz_sh + wmid * u.pz_mid + whi * u.pz_hi;
    return max(rgb * gain, float3(0.0));
}

// Tone body: tone sliders on the (possibly super-white) body. NO leading clamp,
// NO SDR display-finalize — the HDR finalize handles the shoulder. Blacks cube via
// multiply (matches finish.rs `.powi(3)`; `pow(neg,3.0)` is NaN for v>1).
float toneBody(float v, constant HdrUniforms& u) {
    v += u.whites * 0.20 * v * v * v;
    float omv = 1.0 - v;
    v += u.blacks * 0.20 * (omv * omv * omv);
    v += u.highlights * 0.18 * smoothstep(0.5, 1.0, v);
    v += u.shadows * 0.18 * (1.0 - smoothstep(0.0, 0.5, v));
    v = 0.5 + (v - 0.5) * (1.0 + u.contrast);
    return v;
}

// Faithful SDR display-finalize (shoulder roll-off to 1.0 + lookS contrast), the
// tail of the Faithful body. MUST equal engine.rs display_finalize /
// shaders.ts displayFinalize (recovery retired → hi=lo=0). The creative color ops
// run on THIS (so the HDR surface matches the SDR contrast below white).
float shoulderOnly(float raw, float ceil_val) {
    if (raw <= FAITHFUL_KNEE) return min(raw, ceil_val);
    float k = FAITHFUL_KNEE;
    float scale = (1.0 - k);                 // hi_recovery retired → 0
    return k + (ceil_val - k) * (1.0 - exp(-(raw - k) / scale));
}
float lookS(float v) {                       // lo_recovery retired → 0
    return clamp(0.5 + 0.5 * tanh(LOOK_K * (v - 0.5)) / tanh(LOOK_K * 0.5), 0.0, 1.0);
}
float displayFinalize(float v) { return lookS(shoulderOnly(v, 1.0)); }

float3 colorGrade(float3 rgb, constant HdrUniforms& u) {
    float L = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    float wsh = 1.0 - smoothstep(u.cg_sh_edge - u.cg_soft, u.cg_sh_edge + u.cg_soft, L);
    float whi = smoothstep(u.cg_hi_edge - u.cg_soft, u.cg_hi_edge + u.cg_soft, L);
    float wmid = clamp(1.0 - wsh - whi, 0.0, 1.0);
    float3 outc = rgb
        + wsh * (u.cg_sh_off + float3(u.cg_sh_lum))
        + wmid * (u.cg_mid_off + float3(u.cg_mid_lum))
        + whi * (u.cg_hi_off + float3(u.cg_hi_lum))
        + (u.cg_glob_off + float3(u.cg_glob_lum));
    return clamp(outc, 0.0, 1.0);
}

float3 rgb2hsl(float3 c) {
    float mx = max(max(c.r, c.g), c.b);
    float mn = min(min(c.r, c.g), c.b);
    float l = (mx + mn) * 0.5;
    if (mx - mn < 1e-7) return float3(0.0, 0.0, l);
    float d = mx - mn;
    float s = l > 0.5 ? d / (2.0 - mx - mn) : d / (mx + mn);
    float h;
    if (mx == c.r) h = (c.g - c.b) / d + (c.g < c.b ? 6.0 : 0.0);
    else if (mx == c.g) h = (c.b - c.r) / d + 2.0;
    else h = (c.r - c.g) / d + 4.0;
    return float3(h * 60.0, s, l);
}
float hue2rgb(float p, float q, float t) {
    t = fract(t);
    if (t < 1.0 / 6.0) return p + (q - p) * 6.0 * t;
    if (t < 0.5) return q;
    if (t < 2.0 / 3.0) return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    return p;
}
float3 hsl2rgb(float h, float s, float l) {
    if (s <= 0.0) return float3(l);
    float q = l < 0.5 ? l * (1.0 + s) : l + s - l * s;
    float p = 2.0 * l - q;
    float hk = h / 360.0;
    return float3(hue2rgb(p, q, hk + 1.0 / 3.0), hue2rgb(p, q, hk), hue2rgb(p, q, hk - 1.0 / 3.0));
}
float wrap180(float d) {
    float x = fin_mod(d + 180.0, 360.0) - 180.0;
    return x <= -180.0 ? x + 360.0 : x;
}
float bandWeight(float h, float center) {
    float d = abs(wrap180(h - center));
    return d >= CM_FALLOFF_DEG ? 0.0 : 0.5 * (1.0 + cos(FIN_PI * d / CM_FALLOFF_DEG));
}
float3 colorMixer(float3 rgb, constant HdrUniforms& u) {
    float3 hsl = rgb2hsl(rgb);
    float h = hsl.x, s = hsl.y, l = hsl.z;
    float gate = smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s);
    float hueDelta = 0.0, satFactor = 1.0, lumDelta = 0.0;
    for (int i = 0; i < 8; i++) {
        float w = bandWeight(h, BAND_CENTERS[i]);
        hueDelta += w * gate * u.cm_hue[i] * CM_HUE_SHIFT_MAX;
        satFactor += w * gate * u.cm_sat[i];
        lumDelta += w * u.cm_lum[i] * CM_LUM_GAIN;
    }
    return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}
float pcTol(float base, float variance) {
    return max(0.02, base * (1.0 + (variance / 100.0) * PC_VAR_SPAN));
}
float pcHueWeight(float h, float target, float range) {
    float hw = PC_RANGE_MIN_DEG + (range / 100.0) * (PC_RANGE_MAX_DEG - PC_RANGE_MIN_DEG);
    float d = abs(wrap180(h - target));
    return d >= hw ? 0.0 : 0.5 * (1.0 + cos(FIN_PI * d / hw));
}
float3 pointColor(float3 rgb, constant HdrUniforms& u) {
    if (u.pc_count <= 0) return rgb;
    float3 hsl = rgb2hsl(rgb);
    float h = hsl.x, s = hsl.y, l = hsl.z;
    float hueDelta = 0.0, satFactor = 1.0, lumDelta = 0.0;
    for (int k = 0; k < 8; k++) {
        if (k >= u.pc_count) break;
        float wh = pcHueWeight(h, u.pc_hue[k], u.pc_range[k]);
        if (wh <= 0.0) continue;
        float ws = clamp(1.0 - abs(s - u.pc_sat[k]) / pcTol(PC_SAT_TOL, u.pc_variance[k]), 0.0, 1.0);
        float wl = clamp(1.0 - abs(l - u.pc_lum[k]) / pcTol(PC_LUM_TOL, u.pc_variance[k]), 0.0, 1.0);
        float w = wh * ws * wl;
        hueDelta += w * u.pc_hue_shift[k] * CM_HUE_SHIFT_MAX;
        satFactor += w * u.pc_sat_shift[k];
        lumDelta += w * u.pc_lum_shift[k] * CM_LUM_GAIN;
    }
    return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}

// OKLab perceptual saturation (MUST equal finish.rs apply_saturation).
float srgbToLinear(float c) { return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4); }
float linearToSrgb(float c) { return c <= 0.0031308 ? 12.92 * c : 1.055 * pow(c, 1.0 / 2.4) - 0.055; }
float3 srgbToLinear3(float3 c) { return float3(srgbToLinear(c.r), srgbToLinear(c.g), srgbToLinear(c.b)); }
float3 linearToSrgb3(float3 c) { return float3(linearToSrgb(c.r), linearToSrgb(c.g), linearToSrgb(c.b)); }
float3 linearToOklab(float3 rgb) {
    float l = 0.4122214708 * rgb.r + 0.5363325363 * rgb.g + 0.0514459929 * rgb.b;
    float m = 0.2119034982 * rgb.r + 0.6806995451 * rgb.g + 0.1073969566 * rgb.b;
    float s = 0.0883024619 * rgb.r + 0.2817188376 * rgb.g + 0.6299787005 * rgb.b;
    float3 lms_ = pow(max(float3(l, m, s), float3(0.0)), float3(1.0 / 3.0));
    return float3(
        0.2104542553 * lms_.x + 0.7936177850 * lms_.y - 0.0040720468 * lms_.z,
        1.9779984951 * lms_.x - 2.4285922050 * lms_.y + 0.4505937099 * lms_.z,
        0.0259040371 * lms_.x + 0.7827717662 * lms_.y - 0.8086757660 * lms_.z);
}
float3 oklabToLinear(float3 lab) {
    float l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    float m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    float s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    float3 lms = float3(l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    return float3(
         4.0767416621 * lms.x - 3.3077115913 * lms.y + 0.2309699292 * lms.z,
        -1.2684380046 * lms.x + 2.6097574011 * lms.y - 0.3413193965 * lms.z,
        -0.0041960863 * lms.x - 0.7034186147 * lms.y + 1.7076147010 * lms.z);
}
float fin_hueDist(float a, float b) {
    float d = fin_mod(abs(a - b), 2.0 * FIN_PI);
    return d > FIN_PI ? 2.0 * FIN_PI - d : d;
}
float3 oklabSaturate(float3 rgb, constant HdrUniforms& u) {
    if (abs(u.saturation) < 1e-5 && abs(u.vibrance) < 1e-5) return rgb;
    float3 lab = linearToOklab(srgbToLinear3(rgb));
    float c = length(lab.yz);
    if (c < 1e-5) return rgb;
    float hh = atan2(lab.z, lab.y);
    float vibW = 1.0 - clamp(c / SAT_C_REF, 0.0, 1.0);
    float gain = u.saturation + u.vibrance * vibW;
    float neutral = smoothstep(0.0, SAT_C_NEUTRAL, c);
    float skin = 1.0 - SKIN_DAMP * smoothstep(SKIN_WIDTH, 0.0, fin_hueDist(hh, SKIN_HUE));
    gain *= neutral * skin;
    float scale = max(1.0 + gain, 0.0);
    float3 lab2 = float3(lab.x, lab.y * scale, lab.z * scale);
    float3 gray = oklabToLinear(float3(lab.x, 0.0, 0.0));
    float3 col = oklabToLinear(lab2);
    float tg = 1.0;
    for (int ch = 0; ch < 3; ch++) {
        float g0 = gray[ch]; float c0 = col[ch];
        if (c0 > 1.0) tg = min(tg, (1.0 - g0) / (c0 - g0));
        else if (c0 < 0.0) tg = min(tg, g0 / (g0 - c0));
    }
    tg = clamp(tg, 0.0, 1.0);
    float3 outLin = clamp(mix(gray, col, tg), 0.0, 1.0);
    return linearToSrgb3(outLin);
}

// Per-channel tone LUT sample (256x1 RGBA; R=ch0 G=ch1 B=ch2). clamp_to_edge +
// linear filter reproduce sample_lut's clamp + interpolation.
float lutSample(texture2d<float> lut, float x, int ch) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    float4 v = lut.sample(s, float2(x, 0.5));
    return ch == 0 ? v.r : (ch == 1 ? v.g : v.b);
}

// HDR finalize scalar: tanh shoulder above HDR_KNEE, identity below. Port of
// finish.rs::hdr_finalize (the scalar building block).
float hdr_finalize_scalar(float v) {
    if (v <= HDR_KNEE) return v;
    float span = HDR_HEADROOM - HDR_KNEE;
    return HDR_KNEE + span * tanh((v - HDR_KNEE) / span);
}

// Clipping overlay (detail-loss warning), on the finished display color.
int clipCode(float3 src, constant HdrUniforms& u) {
    float hiT = u.clip_strict > 0.5 ? CLIP_HI_STRICT : CLIP_HI;
    float loT = u.clip_strict > 0.5 ? CLIP_LO_STRICT : CLIP_LO;
    int code = 0;
    if (src.r >= hiT || src.g >= hiT || src.b >= hiT) code += 2;
    if (src.r <= loT || src.g <= loT || src.b <= loT) code += 1;
    return code;
}
float3 clipOverlay(float3 disp, int code, constant HdrUniforms& u) {
    if (u.clip_high_on > 0.5 && (code & 2) != 0) return float3(1.0, 0.15, 0.15);
    if (u.clip_low_on > 0.5 && (code & 1) != 0) return float3(0.2, 0.45, 1.0);
    return disp;
}

// sRGB EOTF with a LINEAR continuation above 1.0 — MUST equal hdr.rs
// srgb_to_linear_ext EXACTLY. The finished color is display-referred sRGB, but
// the CAMetalLayer is tagged extended-LINEAR sRGB, so the output must be
// linearized (super-white > 1.0 linearizes via the C1 linear extension, NOT a
// clamp) or midtones read as too bright (flat/low-contrast). Distinct from the
// [0,1]-only `srgbToLinear` used inside OKLab saturation.
float srgbToLinearExt(float v) {
    if (v <= 0.0) return 0.0;
    if (v <= 0.04045) return v / 12.92;
    if (v <= 1.0) return pow((v + 0.055) / 1.055, 2.4);
    return 1.0 + (v - 1.0) * (2.4 / 1.055);   // slope of the EOTF at v=1, extended linearly
}
float3 srgbToLinearExt3(float3 c) {
    return float3(srgbToLinearExt(c.r), srgbToLinearExt(c.g), srgbToLinearExt(c.b));
}

fragment float4 finish_frag(VOut in [[stage_in]],
                            texture2d<float> src [[texture(0)]],
                            texture2d<float> lut [[texture(1)]],
                            constant HdrUniforms& u [[buffer(0)]]) {
    constexpr sampler smp(filter::linear, address::clamp_to_edge);
    float3 raw = src.sample(smp, in.uv).rgb;
    // Per-zone WB (first op) + brightness/density gain (super-white preserved).
    float3 c = applyPerZoneWb(raw, u) * pow(10.0, u.brightness * FIN_BRIGHTNESS_RANGE);
    // Tone body (unclamped): carries super-white for the highlight reattach below.
    float3 bodyU = float3(toneBody(c.r, u), toneBody(c.g, u), toneBody(c.b, u));
    // Creative color ops run on the DISPLAY-FINALIZED body (Faithful shoulder +
    // lookS contrast) — EXACTLY as finish_pixel / GLSL finalize_body=true do — so
    // the HDR surface has the same contrast/look as SDR below white. displayFinalize
    // maps [0,∞) -> [0,1] (its shoulder handles the super-white body), matching SDR.
    float3 bodyFin = float3(displayFinalize(bodyU.r), displayFinalize(bodyU.g), displayFinalize(bodyU.b));
    float3 s = oklabSaturate(bodyFin, u);
    float3 cu = float3(lutSample(lut, s.r, 0), lutSample(lut, s.g, 1), lutSample(lut, s.b, 2));
    float3 disp = pointColor(colorMixer(colorGrade(cu, u), u), u);   // == the SDR finished color, in [0,1]
    // Chroma-preserving HDR finalize — mirror of film-core::hdr_finish. Below the
    // knee the output is EXACTLY the SDR color (parity). Above, the highlight is
    // the pre-shoulder BODY's chromaticity (`bodyU/mU`) scaled to the HDR shoulder
    // luminance `hdr_finalize_scalar(mU)` — so a blown highlight keeps its hue into
    // headroom instead of graying — blended from SDR (`disp`) at the knee to the
    // reconstructed highlight by HDR_W_HI.
    float mU = max(bodyU.r, max(bodyU.g, bodyU.b));
    float3 outc;
    if (mU <= HDR_KNEE) {
        outc = disp;                              // below knee: exact SDR parity
    } else {
        float lHdr = hdr_finalize_scalar(mU);     // tanh shoulder → [KNEE, HEADROOM)
        float3 highlight = bodyU * (lHdr / mU);   // body chromaticity at HDR luminance
        float w = smoothstep(HDR_KNEE, HDR_W_HI, mU);
        outc = mix(disp, highlight, w);
    }
    // Clipping overlay tests the finished display color (disp), matching the GLSL.
    int code = clipCode(disp, u);
    // The drawable/CAMetalLayer is extended-LINEAR sRGB, but `outc` is
    // display-referred sRGB — linearize (with the >1.0 linear continuation so
    // super-white survives) before writing, mirroring Sub-project A's
    // `hdr.rs::srgb_to_linear_ext` before its upload to the same linear layer.
    // The clip overlay's red/blue markers linearize too (they stay red/blue).
    return float4(srgbToLinearExt3(clipOverlay(outc, code, u)), 1.0);
}
"#;
