// WGSL port of the canonical Metal HDR shaders
// (`app/src-tauri/src/hdr_surface/msl.rs`: `HDR_UNIFORMS_STRUCT_MSL`,
// `INVERT_FRAG_MSL`, `FINISH_FRAG_MSL`), which themselves mirror
// `crates/film-core/src/{engine,finish}.rs` and the live WebGL2 twin
// (`app/src/lib/viewport/gl/shaders.ts`). This is a DERIVE-FROM-SOURCE port —
// every function/constant here has a 1:1 counterpart in `msl.rs`, same name,
// same math, no simplification. See the module-level "PORT NOTES" below for
// the mechanical MSL -> WGSL transformations applied uniformly throughout.
//
// PORT NOTES (apply everywhere, not re-noted at each call site):
//  - float/float2/float3/float3x3/float2x2/int -> f32/vec2f/vec3f/
//    mat3x3<f32>/mat2x2<f32>/i32. `constant float K = ...;` -> `const K: f32
//    = ...;`. `.r/.g/.b`, `.x/.y/.z` swizzles are valid in WGSL unchanged.
//  - Arithmetic operators (+ - * /) broadcast vector<->scalar automatically
//    in WGSL, same as MSL, so e.g. `c - 0.5` (c: vec2f) needs no rewrite.
//  - Builtin FUNCTIONS (clamp/min/max) do NOT broadcast in WGSL (unlike
//    Metal's convenience overloads) — every `clamp(vec3, f32, f32)` /
//    `max(vec3, f32)` in the MSL source is rewritten to
//    `clamp(vec3, vec3f(f32), vec3f(f32))` / `max(vec3, vec3f(f32))` here.
//    `mix(vecN, vecN, f32)` and `smoothstep` ARE defined with scalar/vector
//    mixes in the WGSL spec matching every use in this file, so those are
//    unchanged.
//  - WGSL has no `?:` ternary operator — every MSL ternary is rewritten as
//    an equivalent `if`/`else` (rgb2hsl, hsl2rgb, wrap180, bandWeight,
//    pcHueWeight, fin_hueDist, srgbToLinear, linearToSrgb, lutSample,
//    clipCode's `>0.5?...`) — same branches, same result, not a math change.
//  - MSL passes `constant HdrUniforms& u` as a function parameter; WGSL
//    functions take `u: HdrUniforms` BY VALUE (WGSL permits struct-by-value
//    params, including ones with array members) — same call sites, same
//    parameter name, so `u.field` reads are textually identical to the MSL.
//  - MSL's per-call `constexpr sampler smp(filter::linear,
//    address::clamp_to_edge)` (used identically by invert_frag, finish_frag,
//    and lutSample) becomes ONE shared `@group(0) @binding(1) var smp:
//    sampler;` resource, since WGSL samplers must be pipeline-bound
//    resources, not inline constants. Texture/LUT params are likewise
//    dropped from function signatures in favor of referencing the global
//    `src`/`lut` bindings directly (WGSL allows any module-scope fn to read
//    module-scope resource vars) — this is a structural necessity, not a
//    math change.
//  - `hdr_finish` is pulled out of `finish_frag`'s inline mU/highlight/blend
//    tail into its own named function, matching `finish.rs::hdr_finish`
//    (`body: [f32;3], sdr: [f32;3] -> [f32;3]`) exactly — the MSL fragment
//    keeps this logic inline, but the brief calls out `hdr_finish` as one of
//    the required named sub-functions, and factoring it makes the parity
//    with the Rust source (the documented "single source of truth") explicit.
//
// KNOWN ON-DEVICE RISK (flagged, not fixed here — see task report): the
// `cm_*`/`pc_*` fields are `array<f32, 8>` inside a `var<uniform>`-address-
// space struct. WGSL requires array element stride to be a multiple of 16
// bytes for the uniform address space, so a literal `array<f32, 8>` field
// here is expected to fail real WebGPU shader-module validation (unlike
// MSL's `constant` buffers, which pack scalar arrays tightly at 4 bytes).
// This file cannot be compiled in this environment to confirm; the fix
// belongs to whichever task owns the buffer/bind-group wiring (either widen
// these arrays to `array<vec4f, 2>` groups with index-math, or bind
// `HdrUniforms` as a `storage` buffer instead of `uniform`, which has no such
// restriction and would in fact pack identically to the tight MSL layout).

/** WGSL mirror of `HDR_UNIFORMS_STRUCT_MSL` — same field names, same order,
 * so a later JS packer can walk the two side by side. See the KNOWN ON-DEVICE
 * RISK note above re: the `cm_*`/`pc_*` scalar-array fields. */
export const WGSL_UNIFORMS = `
struct HdrUniforms {
  // --- finish (finish.rs::FinishParams / shaders.ts FRAG) ---
  contrast: f32,
  highlights: f32,
  shadows: f32,
  whites: f32,
  blacks: f32,
  texture: f32,
  vibrance: f32,
  saturation: f32,
  brightness: f32,

  texel: vec2f,                // 1/out_w, 1/out_h

  // Color grading (finish.rs::ColorGrade / colorGrade()).
  cg_sh_off: vec3f,
  cg_mid_off: vec3f,
  cg_hi_off: vec3f,
  cg_glob_off: vec3f,
  cg_sh_lum: f32,
  cg_mid_lum: f32,
  cg_hi_lum: f32,
  cg_glob_lum: f32,
  cg_sh_edge: f32,
  cg_hi_edge: f32,
  cg_soft: f32,

  // Color Mixer: 8-band HSL (finish.rs::ColorMix / colorMix()).
  cm_hue: array<f32, 8>,
  cm_sat: array<f32, 8>,
  cm_lum: array<f32, 8>,

  // Point Color: up to 8 samples.
  pc_count: i32,
  pc_hue: array<f32, 8>,
  pc_sat: array<f32, 8>,
  pc_lum: array<f32, 8>,
  pc_hue_shift: array<f32, 8>,
  pc_sat_shift: array<f32, 8>,
  pc_lum_shift: array<f32, 8>,
  pc_variance: array<f32, 8>,
  pc_range: array<f32, 8>,

  // Per-zone WB neutralizer (finish.rs::PerZoneWb / perZoneWb()).
  pz_enabled: i32,
  pz_sh: vec3f,
  pz_mid: vec3f,
  pz_hi: vec3f,

  // Clip-warning overlay (B1) — enables + the shared soft-clip knee.
  clip_high_on: f32,
  clip_low_on: f32,
  clip_strict: f32,
  soft_clip: f32,             // shared with invert (InversionParams::soft_clip)

  finish_mode: i32,           // 0 = present w/ clip overlay, 1 = plain FBO write
  finalize_body: f32,         // 1.0 = Faithful body finalize, 0.0 = already display-referred

  // --- invert (engine.rs::InversionParams / shaders.ts INVERT_FRAG) ---
  base: vec3f,
  wb: vec3f,
  m_pre: mat3x3<f32>,
  m_post: mat3x3<f32>,
  exposure: f32,
  black: f32,
  gamma: f32,
  d_max: f32,
  print_exposure: f32,
  paper_black: f32,
  paper_grade: f32,
  mode: i32,                   // 0=B 1=C 2=Naive 3=D (always 3 today)
  raw: i32,                    // GLSL \`uniform bool\`, wired via uniform1i — kept int
  positive: i32,                // GLSL \`uniform bool\`, wired via uniform1i — kept int
  wb_mode: i32,                // 0 = gain (post-curve), 1 = subtractive (pre-curve)
  tone_mode: i32,               // 0 = filmic, 1 = faithful (always 1 today)
  hi_recovery: f32,
  lo_recovery: f32,
  cam_balance: vec3f,

  // --- geometry: output UV -> source UV mapping ---
  crop_off: vec2f,
  crop_scale: vec2f,
  angle: f32,
  aspect: f32,
  orient: mat2x2<f32>,
  view_off: vec2f,
  view_scale: vec2f,
};
`;

/** Shared full-screen-triangle vertex stage, concatenated into both
 * INVERT_WGSL and FINISH_WGSL so each compiles standalone. Emits `uv` in
 * [0,1] over the covered screen area. */
const VERTEX_WGSL = `
struct VOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VOut {
  // Overscanned 3-vertex full-screen triangle, no vertex buffer needed.
  // WebGPU's NDC is Y-up (like OpenGL) but texture row 0 is the image TOP
  // (like D3D/Vulkan) — unlike Metal, whose NDC is already Y-down so
  // INVERT_FRAG_MSL/FINISH_FRAG_MSL need no extra flip (see their doc
  // comments), the WebGPU vertex stage bakes uv.y = (1 - ndc.y) / 2 in here
  // so screen-top samples texture-row-0. Mirrors WebGL's explicit
  // \`v_uv.y = 1.0 - v_uv.y\` (gl/shaders.ts), just applied once here instead
  // of in \`hdr_source_uv\`, which — like the MSL twin — applies NO further
  // flip.
  var pos = array<vec2f, 3>(
    vec2f(-1.0, -1.0),
    vec2f(3.0, -1.0),
    vec2f(-1.0, 3.0),
  );
  let p = pos[vertex_index];
  var out: VOut;
  out.pos = vec4f(p, 0.0, 1.0);
  out.uv = vec2f((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
  return out;
}
`;

/** WGSL invert stage — port of `INVERT_FRAG_MSL` / `engine.rs` invert.
 * Concatenates WGSL_UNIFORMS + VERTEX_WGSL so it compiles standalone.
 * Binding layout (this file's own design choice — MSL binds via per-entry
 * `[[buffer]]`/`[[texture]]` args, WGSL requires module-scope resource
 * vars): binding 0 = uniforms, binding 1 = shared linear/clamp sampler,
 * binding 2 = the raw linear-negative source texture. */
export const INVERT_WGSL =
  WGSL_UNIFORMS +
  VERTEX_WGSL +
  `
@group(0) @binding(0) var<uniform> u: HdrUniforms;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

// ---- invert stage: port of INVERT_FRAG (shaders.ts) / engine.rs invert ----
const INV_EPS: f32 = 1e-5;
const LOG10: f32 = 0.30102999566;          // log10(x) = log2(x) * LOG10
const EXPO_K: f32 = 0.14;                  // MUST equal engine.rs EXPO_K
const FAITHFUL_EXPO_K: f32 = 1.0;          // MUST equal engine.rs FAITHFUL_EXPO_K
const CMY_STRENGTH: f32 = 1.6;             // MUST equal engine.rs CMY_STRENGTH
const FAITHFUL_GAMMA: f32 = 1.590;         // MUST equal engine.rs
const FAITHFUL_SCALE: f32 = 1.0 / 0.700;   // MUST equal engine.rs (1/recommended_d_max)
const FILMIC_K: f32 = 5.0;                 // MUST equal engine.rs FILMIC_K
const FILMIC_PIVOT: f32 = 0.44;            // MUST equal engine.rs FILMIC_PIVOT
const FILMIC_WHITE_T: f32 = 1.05;          // MUST equal engine.rs FILMIC_WHITE_T

// Faithful gamma BODY only (no shoulder/look/clamp) — super-white preserved.
fn gammaBody(x: f32) -> f32 { return pow(max(x, 0.0), 1.0 / FAITHFUL_GAMMA); }
fn filmicL(x: f32) -> f32 { return 1.0 / (1.0 + exp(-FILMIC_K * (x - FILMIC_PIVOT))); }
fn filmicSraw(t: f32) -> f32 {
  let l0 = filmicL(0.0);
  let lw = filmicL(FILMIC_WHITE_T);
  return (filmicL(t) - l0) / (lw - l0);
}
fn filmicS(t: f32) -> f32 { return clamp(filmicSraw(t), 0.0, 1.0); }
fn filmicInv(y: f32) -> f32 {
  let l0 = filmicL(0.0);
  let lw = filmicL(FILMIC_WHITE_T);
  let big = clamp(y * (lw - l0) + l0, 1e-6, 1.0 - 1e-6);
  return FILMIC_PIVOT + log(big / (1.0 - big)) / FILMIC_K;
}
fn tone(v_in: f32, gain: f32, exposure: f32, black: f32, gamma: f32) -> f32 {
  let v = max(v_in * exposure * gain - black, 0.0);
  return pow(v, gamma);
}

fn hdr_invert(rgbIn: vec3f, u: HdrUniforms) -> vec3f {
  let r = clamp(vec3f(
      rgbIn.r / max(u.base.r, INV_EPS),
      rgbIn.g / max(u.base.g, INV_EPS),
      rgbIn.b / max(u.base.b, INV_EPS)), vec3f(INV_EPS), vec3f(1.0));
  if (u.mode == 3) {                 // Mode D: Cineon negadoctor. Mirrors engine.rs invert_d.
    let THRESH: f32 = 2.3283064e-10;
    let clamped = max(rgbIn, vec3f(THRESH));
    let dmin = max(u.base, vec3f(INV_EPS));
    let d = max(log2(dmin / clamped) * LOG10, vec3f(0.0));   // log10(dmin/clamped)
    let ev = log2(max(u.print_exposure, INV_EPS));
    let expo_gain = exp2(EXPO_K * ev);
    let t = d / max(u.d_max, INV_EPS);                        // d == d_max -> t == 1 (white)
    var v: vec3f;
    if (u.tone_mode == 1) {
      // Faithful: gamma body (unclamped); finish stage applies shoulder + look.
      let lScene = max(pow(vec3f(10.0), d * u.cam_balance) - vec3f(1.0), vec3f(0.0));
      let lit = lScene * exp2(FAITHFUL_EXPO_K * ev);
      let te = log2(lit + vec3f(1.0)) * LOG10 * FAITHFUL_SCALE;
      if (u.wb_mode == 1) {
        let s = pow(max(u.wb, vec3f(INV_EPS)), vec3f(CMY_STRENGTH));
        v = vec3f(gammaBody(te.r * s.r), gammaBody(te.g * s.g), gammaBody(te.b * s.b));
      } else {
        v = vec3f(gammaBody(te.r) * u.wb.r, gammaBody(te.g) * u.wb.g, gammaBody(te.b) * u.wb.b);
      }
      return v;   // super-white body, UNCLAMPED — finish stage finalizes.
    } else {
      // Filmic: logistic S-curve on WB-neutralised log-density.
      if (u.wb_mode == 1) {
        let s = pow(max(u.wb, vec3f(INV_EPS)), vec3f(CMY_STRENGTH));
        v = vec3f(
          filmicS(t.r * s.r * expo_gain),
          filmicS(t.g * s.g * expo_gain),
          filmicS(t.b * s.b * expo_gain));
      } else {
        let y = vec3f(filmicSraw(t.r), filmicSraw(t.g), filmicSraw(t.b)) * u.wb;
        v = vec3f(
          filmicS(filmicInv(y.r) * expo_gain),
          filmicS(filmicInv(y.g) * expo_gain),
          filmicS(filmicInv(y.b) * expo_gain));
      }
    }
    return clamp(v, vec3f(0.0), vec3f(1.0));
  }
  if (u.mode == 2) {                 // Naive: 1 - clamp(I/base, 0, 1). engine.rs invert_naive.
    let n = clamp(vec3f(
        rgbIn.r / max(u.base.r, INV_EPS),
        rgbIn.g / max(u.base.g, INV_EPS),
        rgbIn.b / max(u.base.b, INV_EPS)), vec3f(0.0), vec3f(1.0));
    return vec3f(1.0) - n;
  }
  if (u.mode == 1) {                 // Mode C: per-channel log density.
    let dens = -vec3f(log2(r.r), log2(r.g), log2(r.b)) * LOG10;
    return vec3f(
      tone(dens.r, u.wb.r, u.exposure, u.black, u.gamma),
      tone(dens.g, u.wb.g, u.exposure, u.black, u.gamma),
      tone(dens.b, u.wb.b, u.exposure, u.black, u.gamma));
  }
  // Mode B: M_post * (-log10(M_pre * r)) then tone.
  let mixed = u.m_pre * r;
  let dens2 = -vec3f(
      log2(max(mixed.r, INV_EPS)), log2(max(mixed.g, INV_EPS)), log2(max(mixed.b, INV_EPS))) * LOG10;
  let unmixed = u.m_post * dens2;
  return vec3f(
    tone(unmixed.r, u.wb.r, u.exposure, u.black, u.gamma),
    tone(unmixed.g, u.wb.g, u.exposure, u.black, u.gamma),
    tone(unmixed.b, u.wb.b, u.exposure, u.black, u.gamma));
}

// Map output UV -> source UV through crop + un-straighten + un-orient. The
// output is the crop sub-rect of the (straightened) oriented image, so this
// inverts the backend's source->output order (orient -> straighten -> crop).
// No y-flip (already applied once in vs_main above; see its doc comment).
fn hdr_source_uv(uv_in: vec2f, u: HdrUniforms) -> vec2f {
  var uv = u.view_off + uv_in * u.view_scale;          // deep-zoom window (identity = off 0, scale 1)
  var c = u.crop_off + uv * u.crop_scale - 0.5;         // into the (straightened) oriented frame, centred
  let s = sin(u.angle);
  let co = cos(u.angle);                                // un-straighten in oriented PIXEL space
  c = mat2x2<f32>(vec2f(co, -s / u.aspect), vec2f(s * u.aspect, co)) * c;
  c = u.orient * c;                                     // un-orient (rot90/flip) into source UV
  return c + 0.5;
}

@fragment
fn fs_invert(in: VOut) -> @location(0) vec4f {
  let suv = hdr_source_uv(in.uv, u);
  if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
    return vec4f(0.0, 0.0, 0.0, 1.0);               // outside source (straighten corners) = black
  }
  let rgb = textureSample(src, smp, suv).rgb;
  if (u.raw != 0) {                                    // output the scan (display gamma), no inversion
    return vec4f(pow(clamp(rgb, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / 2.2)), 1.0);
  }
  if (u.positive != 0) {                               // positive passthrough (scan + exposure/WB)
    return vec4f(pow(max(rgb * u.print_exposure * u.wb, vec3f(0.0)), vec3f(1.0 / 2.2)), 1.0);
  }
  return vec4f(hdr_invert(rgb, u), 1.0);
}
`;

/** WGSL finish stage — port of `FINISH_FRAG_MSL` / `finish.rs::finish_pixel`.
 * Concatenates WGSL_UNIFORMS + VERTEX_WGSL so it compiles standalone.
 * Binding layout: binding 0 = uniforms, binding 1 = shared linear/clamp
 * sampler, binding 2 = the inverted intermediate texture, binding 3 = the
 * 256x1 composed tone LUT. */
export const FINISH_WGSL =
  WGSL_UNIFORMS +
  VERTEX_WGSL +
  `
@group(0) @binding(0) var<uniform> u: HdrUniforms;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var lut: texture_2d<f32>;

// ---- finish stage: port of FRAG (shaders.ts) / finish.rs::finish_pixel ----
const FIN_PI: f32 = 3.14159265358979;
const FIN_BRIGHTNESS_RANGE: f32 = 0.5;      // MUST equal finish.rs BRIGHTNESS_DENSITY_RANGE
const HDR_KNEE: f32 = 0.8;                  // MUST equal engine.rs HDR_KNEE
const HDR_HEADROOM: f32 = 2.5;              // MUST equal engine.rs HDR_HEADROOM
const HDR_W_HI: f32 = 1.2;                  // MUST equal finish.rs HDR_W_HI (blend top)
// Faithful display-finalize (shoulder + lookS contrast) — MUST equal engine.rs
// display_finalize / shaders.ts. FAITHFUL_GAMMA is already declared by the invert stage.
const FAITHFUL_KNEE: f32 = 0.892;
const LOOK_K: f32 = 2.0;
// OKLab saturation constants (MUST equal finish.rs).
const SAT_C_REF: f32 = 0.20;
const SAT_C_NEUTRAL: f32 = 0.025;
const SKIN_HUE: f32 = 0.70;
const SKIN_WIDTH: f32 = 0.55;
const SKIN_DAMP: f32 = 0.5;
// Color-mixer / point-color constants (MUST equal shaders.ts).
const BAND_CENTERS: array<f32, 8> = array<f32, 8>(0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0);
const CM_FALLOFF_DEG: f32 = 50.0;
const CM_HUE_SHIFT_MAX: f32 = 30.0;
const CM_LUM_GAIN: f32 = 0.25;
const CM_SAT_GATE_LO: f32 = 0.05;
const CM_SAT_GATE_HI: f32 = 0.20;
const PC_RANGE_MIN_DEG: f32 = 5.0;
const PC_RANGE_MAX_DEG: f32 = 60.0;
const PC_SAT_TOL: f32 = 0.25;
const PC_LUM_TOL: f32 = 0.25;
const PC_VAR_SPAN: f32 = 2.0;
// Clipping-overlay thresholds (MUST equal shaders.ts).
const CLIP_LO: f32 = 2.0 / 255.0;
const CLIP_LO_STRICT: f32 = 8.0 / 255.0;
const CLIP_HI: f32 = 0.992;
const CLIP_HI_STRICT: f32 = 0.96;

// GLSL \`mod\` (result takes sign of the divisor), not C fmod — needed for the hue wraps.
fn fin_mod(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }

fn applyPerZoneWb(rgb: vec3f, u: HdrUniforms) -> vec3f {
  if (u.pz_enabled == 0) { return rgb; }
  let L = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
  let wsh = 1.0 - smoothstep(0.08, 0.58, L);
  let whi = smoothstep(0.41, 0.91, L);
  let wmid = clamp(1.0 - wsh - whi, 0.0, 1.0);
  let gain = wsh * u.pz_sh + wmid * u.pz_mid + whi * u.pz_hi;
  return max(rgb * gain, vec3f(0.0));
}

// Tone body: tone sliders on the (possibly super-white) body. NO leading clamp,
// NO SDR display-finalize — the HDR finalize handles the shoulder. Blacks cube via
// multiply (matches finish.rs \`.powi(3)\`; \`pow(neg,3.0)\` is NaN for v>1).
fn toneBody(v_in: f32, u: HdrUniforms) -> f32 {
  var v = v_in;
  v += u.whites * 0.20 * v * v * v;
  let omv = 1.0 - v;
  v += u.blacks * 0.20 * (omv * omv * omv);
  v += u.highlights * 0.18 * smoothstep(0.5, 1.0, v);
  v += u.shadows * 0.18 * (1.0 - smoothstep(0.0, 0.5, v));
  v = 0.5 + (v - 0.5) * (1.0 + u.contrast);
  return v;
}

// Faithful SDR display-finalize (shoulder roll-off to 1.0 + lookS contrast), the
// tail of the Faithful body. MUST equal engine.rs display_finalize /
// shaders.ts displayFinalize (recovery retired -> hi=lo=0). The creative color ops
// run on THIS (so the HDR surface matches the SDR contrast below white).
fn shoulderOnly(raw: f32, ceil_val: f32) -> f32 {
  if (raw <= FAITHFUL_KNEE) { return min(raw, ceil_val); }
  let k = FAITHFUL_KNEE;
  let scale = 1.0 - k;                 // hi_recovery retired -> 0
  return k + (ceil_val - k) * (1.0 - exp(-(raw - k) / scale));
}
fn lookS(v: f32) -> f32 {                       // lo_recovery retired -> 0
  return clamp(0.5 + 0.5 * tanh(LOOK_K * (v - 0.5)) / tanh(LOOK_K * 0.5), 0.0, 1.0);
}
fn displayFinalize(v: f32) -> f32 { return lookS(shoulderOnly(v, 1.0)); }

fn colorGrade(rgb: vec3f, u: HdrUniforms) -> vec3f {
  let L = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
  let wsh = 1.0 - smoothstep(u.cg_sh_edge - u.cg_soft, u.cg_sh_edge + u.cg_soft, L);
  let whi = smoothstep(u.cg_hi_edge - u.cg_soft, u.cg_hi_edge + u.cg_soft, L);
  let wmid = clamp(1.0 - wsh - whi, 0.0, 1.0);
  let outc = rgb
    + wsh * (u.cg_sh_off + vec3f(u.cg_sh_lum))
    + wmid * (u.cg_mid_off + vec3f(u.cg_mid_lum))
    + whi * (u.cg_hi_off + vec3f(u.cg_hi_lum))
    + (u.cg_glob_off + vec3f(u.cg_glob_lum));
  return clamp(outc, vec3f(0.0), vec3f(1.0));
}

fn rgb2hsl(c: vec3f) -> vec3f {
  let mx = max(max(c.r, c.g), c.b);
  let mn = min(min(c.r, c.g), c.b);
  let l = (mx + mn) * 0.5;
  if (mx - mn < 1e-7) { return vec3f(0.0, 0.0, l); }
  let d = mx - mn;
  var s: f32;
  if (l > 0.5) { s = d / (2.0 - mx - mn); } else { s = d / (mx + mn); }
  var h: f32;
  if (mx == c.r) {
    var base = (c.g - c.b) / d;
    if (c.g < c.b) { base = base + 6.0; }
    h = base;
  } else if (mx == c.g) {
    h = (c.b - c.r) / d + 2.0;
  } else {
    h = (c.r - c.g) / d + 4.0;
  }
  return vec3f(h * 60.0, s, l);
}
fn hue2rgb(p: f32, q: f32, t_in: f32) -> f32 {
  let t = fract(t_in);
  if (t < 1.0 / 6.0) { return p + (q - p) * 6.0 * t; }
  if (t < 0.5) { return q; }
  if (t < 2.0 / 3.0) { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
  return p;
}
fn hsl2rgb(h: f32, s: f32, l: f32) -> vec3f {
  if (s <= 0.0) { return vec3f(l); }
  var q: f32;
  if (l < 0.5) { q = l * (1.0 + s); } else { q = l + s - l * s; }
  let p = 2.0 * l - q;
  let hk = h / 360.0;
  return vec3f(hue2rgb(p, q, hk + 1.0 / 3.0), hue2rgb(p, q, hk), hue2rgb(p, q, hk - 1.0 / 3.0));
}
fn wrap180(d: f32) -> f32 {
  let x = fin_mod(d + 180.0, 360.0) - 180.0;
  if (x <= -180.0) { return x + 360.0; }
  return x;
}
fn bandWeight(h: f32, center: f32) -> f32 {
  let d = abs(wrap180(h - center));
  if (d >= CM_FALLOFF_DEG) { return 0.0; }
  return 0.5 * (1.0 + cos(FIN_PI * d / CM_FALLOFF_DEG));
}
fn colorMixer(rgb: vec3f, u: HdrUniforms) -> vec3f {
  let hsl = rgb2hsl(rgb);
  let h = hsl.x; let s = hsl.y; let l = hsl.z;
  let gate = smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s);
  var hueDelta: f32 = 0.0;
  var satFactor: f32 = 1.0;
  var lumDelta: f32 = 0.0;
  for (var i: i32 = 0; i < 8; i++) {
    let w = bandWeight(h, BAND_CENTERS[i]);
    hueDelta += w * gate * u.cm_hue[i] * CM_HUE_SHIFT_MAX;
    satFactor += w * gate * u.cm_sat[i];
    lumDelta += w * u.cm_lum[i] * CM_LUM_GAIN;
  }
  return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}
fn pcTol(base: f32, variance: f32) -> f32 {
  return max(0.02, base * (1.0 + (variance / 100.0) * PC_VAR_SPAN));
}
fn pcHueWeight(h: f32, target: f32, range: f32) -> f32 {
  let hw = PC_RANGE_MIN_DEG + (range / 100.0) * (PC_RANGE_MAX_DEG - PC_RANGE_MIN_DEG);
  let d = abs(wrap180(h - target));
  if (d >= hw) { return 0.0; }
  return 0.5 * (1.0 + cos(FIN_PI * d / hw));
}
fn pointColor(rgb: vec3f, u: HdrUniforms) -> vec3f {
  if (u.pc_count <= 0) { return rgb; }
  let hsl = rgb2hsl(rgb);
  let h = hsl.x; let s = hsl.y; let l = hsl.z;
  var hueDelta: f32 = 0.0;
  var satFactor: f32 = 1.0;
  var lumDelta: f32 = 0.0;
  for (var k: i32 = 0; k < 8; k++) {
    if (k >= u.pc_count) { break; }
    let wh = pcHueWeight(h, u.pc_hue[k], u.pc_range[k]);
    if (wh <= 0.0) { continue; }
    let ws = clamp(1.0 - abs(s - u.pc_sat[k]) / pcTol(PC_SAT_TOL, u.pc_variance[k]), 0.0, 1.0);
    let wl = clamp(1.0 - abs(l - u.pc_lum[k]) / pcTol(PC_LUM_TOL, u.pc_variance[k]), 0.0, 1.0);
    let w = wh * ws * wl;
    hueDelta += w * u.pc_hue_shift[k] * CM_HUE_SHIFT_MAX;
    satFactor += w * u.pc_sat_shift[k];
    lumDelta += w * u.pc_lum_shift[k] * CM_LUM_GAIN;
  }
  return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}

// OKLab perceptual saturation (MUST equal finish.rs apply_saturation).
fn srgbToLinear(c: f32) -> f32 {
  if (c <= 0.04045) { return c / 12.92; }
  return pow((c + 0.055) / 1.055, 2.4);
}
fn linearToSrgb(c: f32) -> f32 {
  if (c <= 0.0031308) { return 12.92 * c; }
  return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}
fn srgbToLinear3(c: vec3f) -> vec3f { return vec3f(srgbToLinear(c.r), srgbToLinear(c.g), srgbToLinear(c.b)); }
fn linearToSrgb3(c: vec3f) -> vec3f { return vec3f(linearToSrgb(c.r), linearToSrgb(c.g), linearToSrgb(c.b)); }
fn linearToOklab(rgb: vec3f) -> vec3f {
  let l = 0.4122214708 * rgb.r + 0.5363325363 * rgb.g + 0.0514459929 * rgb.b;
  let m = 0.2119034982 * rgb.r + 0.6806995451 * rgb.g + 0.1073969566 * rgb.b;
  let s = 0.0883024619 * rgb.r + 0.2817188376 * rgb.g + 0.6299787005 * rgb.b;
  let lms_ = pow(max(vec3f(l, m, s), vec3f(0.0)), vec3f(1.0 / 3.0));
  return vec3f(
    0.2104542553 * lms_.x + 0.7936177850 * lms_.y - 0.0040720468 * lms_.z,
    1.9779984951 * lms_.x - 2.4285922050 * lms_.y + 0.4505937099 * lms_.z,
    0.0259040371 * lms_.x + 0.7827717662 * lms_.y - 0.8086757660 * lms_.z);
}
fn oklabToLinear(lab: vec3f) -> vec3f {
  let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
  let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
  let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
  let lms = vec3f(l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
  return vec3f(
     4.0767416621 * lms.x - 3.3077115913 * lms.y + 0.2309699292 * lms.z,
    -1.2684380046 * lms.x + 2.6097574011 * lms.y - 0.3413193965 * lms.z,
    -0.0041960863 * lms.x - 0.7034186147 * lms.y + 1.7076147010 * lms.z);
}
fn fin_hueDist(a: f32, b: f32) -> f32 {
  let d = fin_mod(abs(a - b), 2.0 * FIN_PI);
  if (d > FIN_PI) { return 2.0 * FIN_PI - d; }
  return d;
}
fn oklabSaturate(rgb: vec3f, u: HdrUniforms) -> vec3f {
  if (abs(u.saturation) < 1e-5 && abs(u.vibrance) < 1e-5) { return rgb; }
  let lab = linearToOklab(srgbToLinear3(rgb));
  let c = length(lab.yz);
  if (c < 1e-5) { return rgb; }
  let hh = atan2(lab.z, lab.y);
  let vibW = 1.0 - clamp(c / SAT_C_REF, 0.0, 1.0);
  var gain = u.saturation + u.vibrance * vibW;
  let neutral = smoothstep(0.0, SAT_C_NEUTRAL, c);
  let skin = 1.0 - SKIN_DAMP * smoothstep(SKIN_WIDTH, 0.0, fin_hueDist(hh, SKIN_HUE));
  gain *= neutral * skin;
  let scale = max(1.0 + gain, 0.0);
  let lab2 = vec3f(lab.x, lab.y * scale, lab.z * scale);
  let gray = oklabToLinear(vec3f(lab.x, 0.0, 0.0));
  let col = oklabToLinear(lab2);
  var tg: f32 = 1.0;
  for (var ch: i32 = 0; ch < 3; ch++) {
    let g0 = gray[ch];
    let c0 = col[ch];
    if (c0 > 1.0) { tg = min(tg, (1.0 - g0) / (c0 - g0)); }
    else if (c0 < 0.0) { tg = min(tg, g0 / (g0 - c0)); }
  }
  tg = clamp(tg, 0.0, 1.0);
  let outLin = clamp(mix(gray, col, tg), vec3f(0.0), vec3f(1.0));
  return linearToSrgb3(outLin);
}

// Per-channel tone LUT sample (256x1 RGBA; R=ch0 G=ch1 B=ch2). clamp_to_edge +
// linear filter reproduce sample_lut's clamp + interpolation. References the
// shared \`lut\`/\`smp\` module-scope bindings instead of taking them as params
// (WGSL texture/sampler bindings live at module scope, unlike MSL's per-call
// constexpr sampler + texture argument).
fn lutSample(x: f32, ch: i32) -> f32 {
  let v = textureSample(lut, smp, vec2f(x, 0.5));
  if (ch == 0) { return v.r; }
  if (ch == 1) { return v.g; }
  return v.b;
}

// HDR finalize scalar: tanh shoulder above HDR_KNEE, identity below. Port of
// finish.rs::hdr_finalize (the scalar building block).
fn hdr_finalize_scalar(v: f32) -> f32 {
  if (v <= HDR_KNEE) { return v; }
  let span = HDR_HEADROOM - HDR_KNEE;
  return HDR_KNEE + span * tanh((v - HDR_KNEE) / span);
}

// Chroma-preserving HDR finalize — direct port of finish.rs::hdr_finish
// (the documented single source of truth for this blend). Below the knee,
// \`body\`'s max channel, the output is exactly \`sdr\` (SDR parity). Above,
// the highlight reconstructs \`body\`'s chromaticity at the tanh-shouldered
// luminance, blended in from \`sdr\` across [HDR_KNEE, HDR_W_HI].
fn hdr_finish(body: vec3f, sdr: vec3f) -> vec3f {
  let mU = max(body.r, max(body.g, body.b));
  if (mU <= HDR_KNEE) { return sdr; }
  let lHdr = hdr_finalize_scalar(mU);       // tanh shoulder -> [KNEE, HEADROOM)
  let highlight = body * (lHdr / mU);       // body chromaticity at HDR luminance
  let w = smoothstep(HDR_KNEE, HDR_W_HI, mU);
  return mix(sdr, highlight, w);
}

// Clipping overlay (detail-loss warning), on the finished display color.
fn clipCode(src_c: vec3f, u: HdrUniforms) -> i32 {
  var hiT = CLIP_HI;
  if (u.clip_strict > 0.5) { hiT = CLIP_HI_STRICT; }
  var loT = CLIP_LO;
  if (u.clip_strict > 0.5) { loT = CLIP_LO_STRICT; }
  var code: i32 = 0;
  if (src_c.r >= hiT || src_c.g >= hiT || src_c.b >= hiT) { code += 2; }
  if (src_c.r <= loT || src_c.g <= loT || src_c.b <= loT) { code += 1; }
  return code;
}
fn clipOverlay(disp: vec3f, code: i32, u: HdrUniforms) -> vec3f {
  if (u.clip_high_on > 0.5 && (code & 2) != 0) { return vec3f(1.0, 0.15, 0.15); }
  if (u.clip_low_on > 0.5 && (code & 1) != 0) { return vec3f(0.2, 0.45, 1.0); }
  return disp;
}

// sRGB EOTF with a LINEAR continuation above 1.0 — MUST equal hdr.rs
// srgb_to_linear_ext EXACTLY. The finished color is display-referred sRGB, but
// an EDR-tagged surface is extended-LINEAR sRGB, so the output must be
// linearized (super-white > 1.0 linearizes via the C1 linear extension, NOT a
// clamp) or midtones read as too bright (flat/low-contrast). Distinct from the
// [0,1]-only \`srgbToLinear\` used inside OKLab saturation.
fn srgbToLinearExt(v: f32) -> f32 {
  if (v <= 0.0) { return 0.0; }
  if (v <= 0.04045) { return v / 12.92; }
  if (v <= 1.0) { return pow((v + 0.055) / 1.055, 2.4); }
  return 1.0 + (v - 1.0) * (2.4 / 1.055);   // slope of the EOTF at v=1, extended linearly
}
fn srgbToLinearExt3(c: vec3f) -> vec3f {
  return vec3f(srgbToLinearExt(c.r), srgbToLinearExt(c.g), srgbToLinearExt(c.b));
}

@fragment
fn fs_finish(in: VOut) -> @location(0) vec4f {
  let raw = textureSample(src, smp, in.uv).rgb;
  // Per-zone WB (first op) + brightness/density gain (super-white preserved).
  let c = applyPerZoneWb(raw, u) * pow(10.0, u.brightness * FIN_BRIGHTNESS_RANGE);
  // Tone body (unclamped): carries super-white for the highlight reattach below.
  let bodyU = vec3f(toneBody(c.r, u), toneBody(c.g, u), toneBody(c.b, u));
  // Creative color ops run on the DISPLAY-FINALIZED body (Faithful shoulder +
  // lookS contrast) — EXACTLY as finish_pixel / GLSL finalize_body=true do — so
  // the HDR surface has the same contrast/look as SDR below white.
  let bodyFin = vec3f(displayFinalize(bodyU.r), displayFinalize(bodyU.g), displayFinalize(bodyU.b));
  let s = oklabSaturate(bodyFin, u);
  let cu = vec3f(lutSample(s.r, 0), lutSample(s.g, 1), lutSample(s.b, 2));
  let disp = pointColor(colorMixer(colorGrade(cu, u), u), u);   // == the SDR finished color, in [0,1]
  // Chroma-preserving HDR finalize — see hdr_finish's doc comment above.
  let outc = hdr_finish(bodyU, disp);
  // Clipping overlay tests the finished display color (disp), matching the GLSL.
  let code = clipCode(disp, u);
  // Linearize (with the >1.0 linear continuation so super-white survives)
  // before writing, mirroring Sub-project A's \`hdr.rs::srgb_to_linear_ext\`
  // before its upload to a linear-tagged surface. The clip overlay's
  // red/blue markers linearize too (they stay red/blue).
  return vec4f(srgbToLinearExt3(clipOverlay(outc, code, u)), 1.0);
}
`;
