// Fullscreen-triangle vertex shader (no buffers; uses gl_VertexID).
export const VERT = `#version 300 es
out vec2 v_uv;
void main() {
  vec2 uv = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  v_uv = uv;
  gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}`;

// Fragment shader: ports finish.rs. tone_curve + saturation per pixel; texture
// (unsharp) computed by re-evaluating finish() on a 3x3 (outer-product 0.25/0.5/
// 0.25) neighbourhood — numerically equal to blur(finish_pixel) then unsharp.
export const FRAG = `#version 300 es
precision highp float;
in vec2 v_uv;
out vec4 o;
uniform sampler2D u_src;
uniform sampler2D u_lut;         // 256x1 composed tone LUT (R/G/B per channel)
uniform vec2 u_texel;            // 1/width, 1/height
uniform float u_contrast, u_highlights, u_shadows, u_whites, u_blacks;
uniform float u_vibrance, u_saturation, u_texture;
// Color grading (precomputed offsets + masks; mirror finish.rs::ColorGrade).
uniform vec3 u_cg_sh_off, u_cg_mid_off, u_cg_hi_off, u_cg_glob_off;
uniform float u_cg_sh_lum, u_cg_mid_lum, u_cg_hi_lum, u_cg_glob_lum;
uniform float u_cg_sh_edge, u_cg_hi_edge, u_cg_soft;
// Color Mixer (HSL): per-band sliders pre-divided to unit. Band centers are const.
uniform float u_cm_hue[8];
uniform float u_cm_sat[8];
uniform float u_cm_lum[8];
// Point Color: up to 8 samples.
uniform int u_pc_count;
uniform float u_pc_hue[8];
uniform float u_pc_sat[8];
uniform float u_pc_lum[8];
uniform float u_pc_hue_shift[8];
uniform float u_pc_sat_shift[8];
uniform float u_pc_lum_shift[8];
uniform float u_pc_variance[8];
uniform float u_pc_range[8];

float tone(float v) {
  v = clamp(v, 0.0, 1.0);
  v += u_whites * 0.20 * v * v * v;
  v += u_blacks * 0.20 * pow(1.0 - v, 3.0);
  v += u_shadows * 0.30 * (1.0 - v) * (1.0 - v) * v;
  v += u_highlights * 0.30 * v * v * (1.0 - v);
  v = 0.5 + (v - 0.5) * (1.0 + u_contrast);
  return clamp(v, 0.0, 1.0);
}

vec3 colorGrade(vec3 rgb) {
  float L = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
  float wsh = 1.0 - smoothstep(u_cg_sh_edge - u_cg_soft, u_cg_sh_edge + u_cg_soft, L);
  float whi = smoothstep(u_cg_hi_edge - u_cg_soft, u_cg_hi_edge + u_cg_soft, L);
  float wmid = clamp(1.0 - wsh - whi, 0.0, 1.0);
  vec3 outc = rgb
    + wsh * (u_cg_sh_off + vec3(u_cg_sh_lum))
    + wmid * (u_cg_mid_off + vec3(u_cg_mid_lum))
    + whi * (u_cg_hi_off + vec3(u_cg_hi_lum))
    + (u_cg_glob_off + vec3(u_cg_glob_lum));
  return clamp(outc, 0.0, 1.0);
}

const float PI_F = 3.14159265358979;
const float BAND_CENTERS[8] = float[8](0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0);
const float CM_FALLOFF_DEG = 50.0;
const float CM_HUE_SHIFT_MAX = 30.0;
const float CM_LUM_GAIN = 0.25;
const float CM_SAT_GATE_LO = 0.05;
const float CM_SAT_GATE_HI = 0.20;
const float PC_RANGE_MIN_DEG = 5.0;
const float PC_RANGE_MAX_DEG = 60.0;
const float PC_SAT_TOL = 0.25;
const float PC_LUM_TOL = 0.25;
const float PC_VAR_SPAN = 2.0;

vec3 rgb2hsl(vec3 c) {
  float mx = max(max(c.r, c.g), c.b);
  float mn = min(min(c.r, c.g), c.b);
  float l = (mx + mn) * 0.5;
  if (mx - mn < 1e-7) return vec3(0.0, 0.0, l);
  float d = mx - mn;
  float s = l > 0.5 ? d / (2.0 - mx - mn) : d / (mx + mn);
  float h;
  if (mx == c.r) h = (c.g - c.b) / d + (c.g < c.b ? 6.0 : 0.0);
  else if (mx == c.g) h = (c.b - c.r) / d + 2.0;
  else h = (c.r - c.g) / d + 4.0;
  return vec3(h * 60.0, s, l);
}
float hue2rgb(float p, float q, float t) {
  t = fract(t);
  if (t < 1.0/6.0) return p + (q - p) * 6.0 * t;
  if (t < 0.5) return q;
  if (t < 2.0/3.0) return p + (q - p) * (2.0/3.0 - t) * 6.0;
  return p;
}
vec3 hsl2rgb(float h, float s, float l) {
  if (s <= 0.0) return vec3(l);
  float q = l < 0.5 ? l * (1.0 + s) : l + s - l * s;
  float p = 2.0 * l - q;
  float hk = h / 360.0;
  return vec3(hue2rgb(p, q, hk + 1.0/3.0), hue2rgb(p, q, hk), hue2rgb(p, q, hk - 1.0/3.0));
}
float wrap180(float d) {
  float x = mod(d + 180.0, 360.0) - 180.0;
  return x <= -180.0 ? x + 360.0 : x;
}
float bandWeight(float h, float center) {
  float d = abs(wrap180(h - center));
  return d >= CM_FALLOFF_DEG ? 0.0 : 0.5 * (1.0 + cos(PI_F * d / CM_FALLOFF_DEG));
}
vec3 colorMixer(vec3 rgb) {
  vec3 hsl = rgb2hsl(rgb);
  float h = hsl.x, s = hsl.y, l = hsl.z;
  float gate = smoothstep(CM_SAT_GATE_LO, CM_SAT_GATE_HI, s);
  float hueDelta = 0.0, satFactor = 1.0, lumDelta = 0.0;
  for (int i = 0; i < 8; i++) {
    float w = bandWeight(h, BAND_CENTERS[i]);
    hueDelta += w * gate * u_cm_hue[i] * CM_HUE_SHIFT_MAX;
    satFactor += w * gate * u_cm_sat[i];
    lumDelta += w * u_cm_lum[i] * CM_LUM_GAIN;
  }
  return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}
float pcTol(float base, float variance) {
  return max(0.02, base * (1.0 + (variance / 100.0) * PC_VAR_SPAN));
}
float pcHueWeight(float h, float target, float range) {
  float hw = PC_RANGE_MIN_DEG + (range / 100.0) * (PC_RANGE_MAX_DEG - PC_RANGE_MIN_DEG);
  float d = abs(wrap180(h - target));
  return d >= hw ? 0.0 : 0.5 * (1.0 + cos(PI_F * d / hw));
}
vec3 pointColor(vec3 rgb) {
  if (u_pc_count <= 0) return rgb;
  vec3 hsl = rgb2hsl(rgb);
  float h = hsl.x, s = hsl.y, l = hsl.z;
  float hueDelta = 0.0, satFactor = 1.0, lumDelta = 0.0;
  for (int k = 0; k < 8; k++) {
    if (k >= u_pc_count) break;
    float wh = pcHueWeight(h, u_pc_hue[k], u_pc_range[k]);
    if (wh <= 0.0) continue;
    float ws = clamp(1.0 - abs(s - u_pc_sat[k]) / pcTol(PC_SAT_TOL, u_pc_variance[k]), 0.0, 1.0);
    float wl = clamp(1.0 - abs(l - u_pc_lum[k]) / pcTol(PC_LUM_TOL, u_pc_variance[k]), 0.0, 1.0);
    float w = wh * ws * wl;
    hueDelta += w * u_pc_hue_shift[k] * CM_HUE_SHIFT_MAX;
    satFactor += w * u_pc_sat_shift[k];
    lumDelta += w * u_pc_lum_shift[k] * CM_LUM_GAIN;
  }
  return hsl2rgb(h + hueDelta, clamp(s * satFactor, 0.0, 1.0), clamp(l + lumDelta, 0.0, 1.0));
}
vec3 finishAt(vec2 uv) {
  vec3 c = texture(u_src, uv).rgb;
  vec3 t = vec3(tone(c.r), tone(c.g), tone(c.b));
  float y = 0.2126 * t.r + 0.7152 * t.g + 0.0722 * t.b;
  float mx = max(max(t.r, t.g), t.b);
  float mn = min(min(t.r, t.g), t.b);
  float cur = mx > 1e-5 ? (mx - mn) / mx : 0.0;
  float f = 1.0 + u_saturation + u_vibrance * (1.0 - cur);
  vec3 s = clamp(vec3(y) + (t - vec3(y)) * f, 0.0, 1.0);
  // Tone curve LUT (per channel: sample at the channel's own value).
  vec3 cu = vec3(
    texture(u_lut, vec2(s.r, 0.5)).r,
    texture(u_lut, vec2(s.g, 0.5)).g,
    texture(u_lut, vec2(s.b, 0.5)).b);
  return pointColor(colorMixer(colorGrade(cu)));
}

void main() {
  vec3 c = finishAt(v_uv);
  if (abs(u_texture) < 1e-5) { o = vec4(c, 1.0); return; }
  vec2 d = u_texel;
  vec3 b =
    finishAt(v_uv + vec2(-d.x, -d.y)) * 0.0625 +
    finishAt(v_uv + vec2( 0.0, -d.y)) * 0.125  +
    finishAt(v_uv + vec2( d.x, -d.y)) * 0.0625 +
    finishAt(v_uv + vec2(-d.x,  0.0)) * 0.125  +
    c * 0.25 +
    finishAt(v_uv + vec2( d.x,  0.0)) * 0.125  +
    finishAt(v_uv + vec2(-d.x,  d.y)) * 0.0625 +
    finishAt(v_uv + vec2( 0.0,  d.y)) * 0.125  +
    finishAt(v_uv + vec2( d.x,  d.y)) * 0.0625;
  float k = 1.5 * u_texture;
  o = vec4(clamp(c + k * (c - b), 0.0, 1.0), 1.0);
}`;

// INVERT pass: samples the raw linear negative (RGBA16F), applies geometry
// (orient/flip/straighten/crop) as a UV transform, then ports engine.rs
// invert_b/c/naive + tone. Writes the inverted positive to an RGBA16F FBO that
// the existing FRAG (finishing) pass then reads. Geometry uniforms map the
// output [0,1] UV into source [0,1] UV; out-of-source samples render black.
export const INVERT_FRAG = `#version 300 es
precision highp float;
in vec2 v_uv;                 // output UV in [0,1]
out vec4 o;
uniform sampler2D u_src;      // raw negative, RGBA16F
uniform vec3 u_base;
uniform vec3 u_wb;
uniform mat3 u_m_pre;
uniform mat3 u_m_post;
uniform float u_exposure, u_black, u_gamma;
uniform int u_mode;           // 0=B 1=C 2=Naive
uniform bool u_raw;           // true → output the scan (display gamma), no inversion
// Geometry: output→source UV mapping. The output is the crop sub-rect of the
// (straightened) oriented image, so we invert the backend's source→output order
// (orient → straighten → crop) by going crop → un-straighten → un-orient.
uniform vec2 u_crop_off;      // crop origin in oriented-image UV
uniform vec2 u_crop_scale;    // crop size in oriented-image UV
uniform float u_angle;        // straighten radians (clockwise)
uniform float u_aspect;       // oriented-image height/width (for pixel-space straighten)
uniform mat2 u_orient;        // oriented-UV → source-UV (undoes rot90/flip)

const float EPS = 1e-5;
const float LOG10 = 0.30102999566; // 1/log2(10): log10(x) = log2(x)*LOG10

float tone(float v, float gain) {
  v = max(v * u_exposure * gain - u_black, 0.0);
  return pow(v, u_gamma);
}

vec3 invert(vec3 rgbIn) {
  // normalise against base, clamp like engine.rs
  vec3 r = clamp(vec3(
    rgbIn.r / max(u_base.r, EPS),
    rgbIn.g / max(u_base.g, EPS),
    rgbIn.b / max(u_base.b, EPS)), EPS, 1.0);
  if (u_mode == 2) {           // Naive: 1 - clamp(I/base,0,1). Intentionally uses
    // its own [0,1] clamp (engine.rs invert_naive), not the [EPS,1] r above.
    vec3 n = clamp(vec3(rgbIn.r/max(u_base.r,EPS), rgbIn.g/max(u_base.g,EPS), rgbIn.b/max(u_base.b,EPS)), 0.0, 1.0);
    return 1.0 - n;
  }
  if (u_mode == 1) {           // Mode C: per-channel log density
    vec3 dens = -vec3(log2(r.r), log2(r.g), log2(r.b)) * LOG10;
    return vec3(tone(dens.r, u_wb.r), tone(dens.g, u_wb.g), tone(dens.b, u_wb.b));
  }
  // Mode B: M_post * (-log10(M_pre * r)) then tone
  vec3 mixed = u_m_pre * r;
  vec3 dens = -vec3(
    log2(max(mixed.r, EPS)), log2(max(mixed.g, EPS)), log2(max(mixed.b, EPS))) * LOG10;
  vec3 unmixed = u_m_post * dens;
  return vec3(tone(unmixed.r, u_wb.r), tone(unmixed.g, u_wb.g), tone(unmixed.b, u_wb.b));
}

// Map output UV → source UV through crop + straighten + orient.
vec2 sourceUV(vec2 uv) {
  // Clip-space v_uv is y-up (v=1 at canvas top); image/texture space is y-down
  // (row 0 = top). Convert before geometry so crop/orient/straighten operate in
  // the image-space convention the JS-side matrices (mirroring convert.rs) assume.
  uv.y = 1.0 - uv.y;
  // 1. map the output UV into the (straightened) oriented-image frame, centred.
  vec2 c = u_crop_off + uv * u_crop_scale - 0.5;
  // 2. un-straighten: the backend rotates in oriented PIXEL space, so scale by the
  //    oriented aspect before/after the rotation (no-op when u_angle == 0).
  float s = sin(u_angle), co = cos(u_angle);
  c = mat2(co, -s / u_aspect, s * u_aspect, co) * c;
  // 3. un-orient (rot90/flip) into source UV, then back to [0,1].
  c = u_orient * c;
  return c + 0.5;
}

void main() {
  vec2 suv = sourceUV(v_uv);
  if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
    o = vec4(0.0, 0.0, 0.0, 1.0); return;     // outside source (straighten corners) = black
  }
  vec3 rgb = texture(u_src, suv).rgb;
  if (u_raw) { o = vec4(pow(clamp(rgb, 0.0, 1.0), vec3(1.0/2.2)), 1.0); return; }
  o = vec4(invert(rgb), 1.0);
}`;
