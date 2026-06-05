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

float tone(float v) {
  v = clamp(v, 0.0, 1.0);
  v += u_whites * 0.20 * v * v * v;
  v -= u_blacks * 0.20 * pow(1.0 - v, 3.0);
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
  return colorGrade(cu);
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
// Geometry: output→source UV mapping. crop sub-rect (in source UV) + straighten
// rotation about the crop centre; orient handled by remapping in JS-set u_uvA/u_uvB.
uniform vec2 u_crop_off;      // source-UV offset of the crop origin
uniform vec2 u_crop_scale;    // source-UV size of the crop
uniform float u_angle;        // straighten radians (clockwise)
uniform mat2 u_orient;        // rot90/flip as a 2x2 on centred UV

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
  // centre, apply orient (rot90/flip) and straighten rotation, then map into crop.
  vec2 c = uv - 0.5;
  c = u_orient * c;
  float s = sin(u_angle), co = cos(u_angle);
  c = mat2(co, -s, s, co) * c;
  vec2 cuv = c + 0.5;                         // back to [0,1] within the (oriented) crop
  return u_crop_off + cuv * u_crop_scale;     // into full source UV
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
