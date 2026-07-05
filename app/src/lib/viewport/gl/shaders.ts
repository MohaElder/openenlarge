// Fullscreen-triangle vertex shader (no buffers; uses gl_VertexID).
export const VERT = `#version 300 es
out vec2 v_uv;
void main() {
  vec2 uv = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  v_uv = uv;
  gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}`;

// Fragment shader: ports finish.rs. tone_curve + saturation + curve + grade per
// pixel. The texture (unsharp/clarity) effect is NOT done here — it needs a wide
// separable Gaussian, which would be far too costly to re-evaluate finish() for
// per tap. Instead this pass writes the finished color to an FBO (u_finish_mode
// == 1) and the separate USM_FRAG program blurs + unsharps it. When presenting
// directly (u_finish_mode == 0) it applies the clip-warning overlay.
export const FRAG = `#version 300 es
precision highp float;
in vec2 v_uv;
out vec4 o;
uniform sampler2D u_src;
uniform sampler2D u_lut;         // 256x1 composed tone LUT (R/G/B per channel)
uniform vec2 u_texel;            // 1/width, 1/height
uniform float u_contrast, u_highlights, u_shadows, u_whites, u_blacks;
uniform float u_vibrance, u_saturation, u_texture;
uniform float u_brightness;      // brightness/density (−1..1); log-curve gain, pre-tone
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

// Per-zone white-balance neutralizer (mirror finish.rs::apply_per_zone_wb).
// Applied FIRST in finishAt(), before brightness multiply + tone curve.
uniform int u_pz_enabled;
uniform vec3 u_pz_sh, u_pz_mid, u_pz_hi;

// Clipping-warning overlay (B1: OUTPUT detail-loss semantics). Enables only;
// the thresholds are derived from the engine soft-clip knee, not hard-coded
// display values — see clipCode().
uniform float u_clip_high_on;  // > 0.5 → paint blown highlights red
uniform float u_clip_low_on;   // > 0.5 → paint crushed shadows blue
uniform float u_clip_strict;   // > 0.5 → flag the ONSET of loss, not just true loss
uniform float u_soft_clip;     // engine highlight soft-clip knee (InversionParams.soft_clip)

// 0 = present to the bound framebuffer (apply clip overlay); 1 = write the plain
// finished color to an FBO for the texture (USM) pass to consume.
uniform int u_finish_mode;
// 1.0 = Faithful-SDR-negative body (finalize at end of tone()); 0.0 = already
// display-referred (positive passthrough or HDR rendition — use legacy clamp path).
// Mirrors FinishParams::finalize_body in finish.rs.
uniform float u_finalize_body;

// Faithful display finalizer — MUST equal engine.rs display_finalize (shoulderOnly + look_s).
// Recovery retired in finish stage (hi=lo=0), so scale is fixed at (1−k).
const float FAITHFUL_GAMMA = 1.590;
const float FAITHFUL_KNEE = 0.892;
const float LOOK_K = 2.0;
float shoulderOnly(float raw, float ceil_val) {
  if (raw <= FAITHFUL_KNEE) return min(raw, ceil_val);
  float k = FAITHFUL_KNEE;
  float scale = (1.0 - k);                  // hi_recovery retired → 0
  return k + (ceil_val - k) * (1.0 - exp(-(raw - k) / scale));
}
float lookS(float v) {                       // lo_recovery retired → 0
  return clamp(0.5 + 0.5 * tanh(LOOK_K * (v - 0.5)) / tanh(LOOK_K * 0.5), 0.0, 1.0);
}
float displayFinalize(float v) { return lookS(shoulderOnly(v, 1.0)); }

// Mirrors finish.rs::tone_curve. u_finalize_body controls the path:
//   1.0 → Faithful-SDR-negative body (may exceed 1.0): NO leading clamp; tone tools
//         see headroom; displayFinalize() applied at the end.
//   0.0 → already display-referred (positive passthrough / HDR rendition): leading
//         clamp(0,1) + trailing clamp(0,1), NO displayFinalize. Byte-identical to the
//         pre-headroom-recovery behavior for those paths.
float tone(float v) {
  if (u_finalize_body < 0.5) v = clamp(v, 0.0, 1.0);   // legacy: display value in
  v += u_whites * 0.20 * v * v * v;
  v += u_blacks * 0.20 * pow(1.0 - v, 3.0);
  v += u_highlights * 0.18 * smoothstep(0.5, 1.0, v);
  v += u_shadows * 0.18 * (1.0 - smoothstep(0.0, 0.5, v));
  v = 0.5 + (v - 0.5) * (1.0 + u_contrast);
  return u_finalize_body > 0.5 ? displayFinalize(v) : clamp(v, 0.0, 1.0);
}

// Per-zone WB neutralizer — mirrors finish.rs::apply_per_zone_wb EXACTLY.
// Luma weights use BT.709 coefficients; smoothstep edges are fixed:
//   w_sh = 1 - smoothstep(0.08, 0.58, L)   (shadow zone, L ≈ 0.33 centre)
//   w_hi =     smoothstep(0.41, 0.91, L)   (highlight zone, L ≈ 0.66 centre)
//   w_mid = clamp(1 - w_sh - w_hi, 0, 1)
// Example (sh=[1,1,1] mid=[1,1,1] hi=[1,1,1]) → gain=1 everywhere → identity.
// Example (L=0.0) → w_sh=1, w_mid=0, w_hi=0 → gain=u_pz_sh.
// Example (L=1.0) → w_sh=0, w_mid=0, w_hi=1 → gain=u_pz_hi.
vec3 applyPerZoneWb(vec3 rgb) {
  if (u_pz_enabled == 0) return rgb;
  float L = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
  float wsh = 1.0 - smoothstep(0.08, 0.58, L);
  float whi = smoothstep(0.41, 0.91, L);
  float wmid = clamp(1.0 - wsh - whi, 0.0, 1.0);
  vec3 gain = wsh * u_pz_sh + wmid * u_pz_mid + whi * u_pz_hi;
  return max(rgb * gain, vec3(0.0));
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
// Brightness/density slider span — MUST equal finish.rs BRIGHTNESS_DENSITY_RANGE
// so the GPU proxy preview matches the CPU full-res export.
const float BRIGHTNESS_DENSITY_RANGE = 0.5;

// OKLab perceptual saturation — MUST equal finish.rs apply_saturation + consts.
const float SAT_C_REF = 0.20;
const float SAT_C_NEUTRAL = 0.025;
const float SKIN_HUE = 0.70;
const float SKIN_WIDTH = 0.55;
const float SKIN_DAMP = 0.5;
const float PI = 3.14159265358979;

float srgbToLinear(float c) { return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4); }
float linearToSrgb(float c) { return c <= 0.0031308 ? 12.92 * c : 1.055 * pow(c, 1.0 / 2.4) - 0.055; }
vec3 srgbToLinear3(vec3 c) { return vec3(srgbToLinear(c.r), srgbToLinear(c.g), srgbToLinear(c.b)); }
vec3 linearToSrgb3(vec3 c) { return vec3(linearToSrgb(c.r), linearToSrgb(c.g), linearToSrgb(c.b)); }
vec3 linearToOklab(vec3 rgb) {
  float l = 0.4122214708*rgb.r + 0.5363325363*rgb.g + 0.0514459929*rgb.b;
  float m = 0.2119034982*rgb.r + 0.6806995451*rgb.g + 0.1073969566*rgb.b;
  float s = 0.0883024619*rgb.r + 0.2817188376*rgb.g + 0.6299787005*rgb.b;
  vec3 lms_ = pow(max(vec3(l, m, s), vec3(0.0)), vec3(1.0/3.0));
  return vec3(
    0.2104542553*lms_.x + 0.7936177850*lms_.y - 0.0040720468*lms_.z,
    1.9779984951*lms_.x - 2.4285922050*lms_.y + 0.4505937099*lms_.z,
    0.0259040371*lms_.x + 0.7827717662*lms_.y - 0.8086757660*lms_.z);
}
vec3 oklabToLinear(vec3 lab) {
  float l_ = lab.x + 0.3963377774*lab.y + 0.2158037573*lab.z;
  float m_ = lab.x - 0.1055613458*lab.y - 0.0638541728*lab.z;
  float s_ = lab.x - 0.0894841775*lab.y - 1.2914855480*lab.z;
  vec3 lms = vec3(l_*l_*l_, m_*m_*m_, s_*s_*s_);
  return vec3(
     4.0767416621*lms.x - 3.3077115913*lms.y + 0.2309699292*lms.z,
    -1.2684380046*lms.x + 2.6097574011*lms.y - 0.3413193965*lms.z,
    -0.0041960863*lms.x - 0.7034186147*lms.y + 1.7076147010*lms.z);
}
float hueDist(float a, float b) {
  float d = mod(abs(a - b), 2.0 * PI);
  return d > PI ? 2.0 * PI - d : d;
}
vec3 oklabSaturate(vec3 rgb) {
  if (abs(u_saturation) < 1e-5 && abs(u_vibrance) < 1e-5) return rgb;
  vec3 lab = linearToOklab(srgbToLinear3(rgb));
  float c = length(lab.yz);
  if (c < 1e-5) return rgb;
  float hh = atan(lab.z, lab.y);
  float vibW = 1.0 - clamp(c / SAT_C_REF, 0.0, 1.0);
  float gain = u_saturation + u_vibrance * vibW;
  float neutral = smoothstep(0.0, SAT_C_NEUTRAL, c);
  float skin = 1.0 - SKIN_DAMP * smoothstep(SKIN_WIDTH, 0.0, hueDist(hh, SKIN_HUE));
  gain *= neutral * skin;
  float scale = max(1.0 + gain, 0.0);
  vec3 lab2 = vec3(lab.x, lab.y * scale, lab.z * scale);
  vec3 gray = oklabToLinear(vec3(lab.x, 0.0, 0.0));
  vec3 col = oklabToLinear(lab2);
  float tg = 1.0;
  for (int ch = 0; ch < 3; ch++) {
    float g0 = gray[ch]; float c0 = col[ch];
    if (c0 > 1.0) tg = min(tg, (1.0 - g0) / (c0 - g0));
    else if (c0 < 0.0) tg = min(tg, g0 / (g0 - c0));
  }
  tg = clamp(tg, 0.0, 1.0);
  vec3 outLin = clamp(mix(gray, col, tg), 0.0, 1.0);
  return linearToSrgb3(outLin);
}

vec3 finishAt(vec2 uv) {
  // Per-zone WB: FIRST op in finish_pixel (mirror finish.rs::apply_per_zone_wb order).
  // Applied to the raw inverted positive BEFORE brightness gain + tone curve.
  vec3 src = texture(u_src, uv).rgb;
  // Brightness/density: log-curve gain (10^(b·RANGE)) before the tone curve, so
  // equal slider steps = equal density steps (mirror finish.rs::finish_pixel).
  vec3 c = applyPerZoneWb(src) * pow(10.0, u_brightness * BRIGHTNESS_DENSITY_RANGE);
  vec3 t = vec3(tone(c.r), tone(c.g), tone(c.b));
  vec3 s = oklabSaturate(t);
  // Tone curve LUT (per channel: sample at the channel's own value).
  vec3 cu = vec3(
    texture(u_lut, vec2(s.r, 0.5)).r,
    texture(u_lut, vec2(s.g, 0.5)).g,
    texture(u_lut, vec2(s.b, 0.5)).b);
  return pointColor(colorMixer(colorGrade(cu)));
}

// B1 — output detail-loss detection. With the Faithful display curve the engine
// reaches true white at 1.0 and rolls off gently through a shoulder, so highlight
// detail is effectively gone once a channel sits deep in that shoulder (near 1.0);
// shadows are lost near black. Input is the finished DISPLAY color (post-finalize),
// so CLIP_HI/CLIP_LO correctly reflect post-finish detail loss (lowering Whites/
// Highlights clears the warning). Strict mode flags the ONSET (entering the shoulder /
// near-black). Returns a 2-bit code: bit1 (=2) highlight loss, bit0 (=1) shadow loss.
const float CLIP_LO = 2.0 / 255.0;
const float CLIP_LO_STRICT = 8.0 / 255.0;
const float CLIP_HI = 0.992;       // deep in the shoulder → highlight detail gone
const float CLIP_HI_STRICT = 0.96; // onset: entering the highlight shoulder
int clipCode(vec3 src) {
  float hiT = u_clip_strict > 0.5 ? CLIP_HI_STRICT : CLIP_HI;
  float loT = u_clip_strict > 0.5 ? CLIP_LO_STRICT : CLIP_LO;
  int code = 0;
  if (src.r >= hiT || src.g >= hiT || src.b >= hiT) code += 2;
  if (src.r <= loT || src.g <= loT || src.b <= loT) code += 1;
  return code;
}

vec3 clipOverlay(vec3 disp, int code) {
  if (u_clip_high_on > 0.5 && (code & 2) != 0) return vec3(1.0, 0.15, 0.15); // highlight → red
  if (u_clip_low_on  > 0.5 && (code & 1) != 0) return vec3(0.2, 0.45, 1.0);  // shadow → blue
  return disp;
}

void main() {
  vec3 c = finishAt(v_uv);
  int code = clipCode(c);                    // test the finished DISPLAY color (was texture(u_src))
  // mode 1: hand the plain finished color to the USM pass, carrying the detail-loss
  // code in alpha (the USM pass can't see the inverted positive). No clip overlay.
  if (u_finish_mode == 1) { o = vec4(c, float(code)); return; }
  o = vec4(clipOverlay(c, code), 1.0);
}`;

// Texture (unsharp/clarity) blur sigma as a fraction of the smaller viewport
// dimension. MUST equal TEXTURE_SIGMA_FRAC in finish.rs so a CPU full-res export
// and a GPU (proxy) preview blur the same fraction of the frame. The renderer
// passes sigma = TEXTURE_SIGMA_FRAC * min(vw, vh) as u_sigma.
export const TEXTURE_SIGMA_FRAC = 0.0025;

// USM pass: separable Gaussian blur + unsharp composite of the finished color
// produced by FRAG (u_finish_mode == 1). Runs twice: u_mode 0 = horizontal blur
// (finishTex → scratch), u_mode 1 = vertical blur (→ full 2-D Gaussian) then
// out = clamp(center + k·(center − blur)) with the clip overlay. Mirrors
// finish.rs::apply_texture (same sigma, gain, clamp, edge-clamp). MAXR / POS_GAIN
// / NEG_GAIN MUST match TEXTURE_MAX_RADIUS / USM_POS_GAIN / USM_NEG_GAIN there.
export const USM_FRAG = `#version 300 es
precision highp float;
in vec2 v_uv;
out vec4 o;
uniform sampler2D u_blur;     // unit 0: texture to blur (finishTex, or h-blurred)
uniform sampler2D u_center;   // unit 2: finished color (center), used in mode 1
uniform vec2 u_texel;         // 1/vw, 1/vh of the finishing viewport
uniform int u_mode;           // 0 = horizontal blur, 1 = vertical blur + composite
uniform float u_sigma;        // gaussian sigma in pixels
uniform float u_texture;      // slider amount, -1..1
// B1 clip overlay: enables only. The detail-loss decision was computed by FRAG
// (which can see the inverted positive + soft-clip knee) and handed to us in the
// alpha of the finished-color texture (u_center) as a 2-bit code.
uniform float u_clip_high_on, u_clip_low_on;

const int MAXR = 64;          // == TEXTURE_MAX_RADIUS
const float POS_GAIN = 2.5;   // == USM_POS_GAIN
const float NEG_GAIN = 1.0;   // == USM_NEG_GAIN

vec3 clipOverlay(vec3 c, int code) {
  if (u_clip_high_on > 0.5 && (code & 2) != 0) return vec3(1.0, 0.15, 0.15);
  if (u_clip_low_on  > 0.5 && (code & 1) != 0) return vec3(0.2, 0.45, 1.0);
  return c;
}

// 1-D normalised Gaussian along 'step' (one texel in x or y). Radius = ceil(3σ),
// capped at MAXR; the loop runs to the constant MAXR (GLSL needs a constant
// bound) and skips taps beyond the radius. CLAMP_TO_EDGE on the sampler matches
// finish.rs's clamped indices.
vec3 gauss(vec2 step) {
  float sigma = max(u_sigma, 1e-3);
  int R = int(min(ceil(3.0 * sigma), float(MAXR)));
  float inv = 1.0 / (2.0 * sigma * sigma);
  vec3 acc = vec3(0.0);
  float wsum = 0.0;
  for (int i = -MAXR; i <= MAXR; i++) {
    if (i < -R || i > R) continue;
    float w = exp(-float(i * i) * inv);
    acc += w * texture(u_blur, v_uv + step * float(i)).rgb;
    wsum += w;
  }
  return acc / wsum;
}

void main() {
  if (u_mode == 0) {                              // horizontal blur only
    o = vec4(gauss(vec2(u_texel.x, 0.0)), 1.0);
    return;
  }
  vec3 blur = gauss(vec2(0.0, u_texel.y));        // → full 2-D gaussian
  vec4 ctr = texture(u_center, v_uv);             // .rgb = finished color, .a = clip code
  vec3 center = ctr.rgb;
  int code = int(ctr.a + 0.5);
  float k = u_texture >= 0.0 ? u_texture * POS_GAIN : u_texture * NEG_GAIN;
  vec3 outc = clamp(center + k * (center - blur), 0.0, 1.0);
  o = vec4(clipOverlay(outc, code), 1.0);
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
uniform float u_d_max, u_print_exposure, u_paper_black, u_paper_grade, u_soft_clip;
uniform int u_mode;           // 0=B 1=C 2=Naive 3=D
uniform bool u_raw;           // true → output the scan (display gamma), no inversion
uniform bool u_positive;      // true → positive passthrough (no inversion), WB+exposure only
uniform int u_wb_mode;        // 0 = gain (post-curve), 1 = subtractive (pre-curve)
uniform int u_tone_mode;      // 0 = filmic (default), 1 = faithful (gamma+shoulder)
uniform float u_hi_recovery, u_lo_recovery; // [0,1] SDR Faithful highlight/shadow recovery
uniform vec3 u_cam_balance;   // per-channel density neutralise (camera-matrix); [1,1,1]=identity
// Geometry: output→source UV mapping. The output is the crop sub-rect of the
// (straightened) oriented image, so we invert the backend's source→output order
// (orient → straighten → crop) by going crop → un-straighten → un-orient.
uniform vec2 u_crop_off;      // crop origin in oriented-image UV
uniform vec2 u_crop_scale;    // crop size in oriented-image UV
uniform float u_angle;        // straighten radians (clockwise)
uniform float u_aspect;       // oriented-image height/width (for pixel-space straighten)
uniform mat2 u_orient;        // oriented-UV → source-UV (undoes rot90/flip)
uniform vec2 u_view_off;      // deep-zoom visible-window origin (displayed-image UV, y-down)
uniform vec2 u_view_scale;    // deep-zoom visible-window size (displayed-image UV)

const float EPS = 1e-5;
const float LOG10 = 0.30102999566; // 1/log2(10): log10(x) = log2(x)*LOG10
// Exposure → t-multiply (pivot at black) — MUST equal engine.rs EXPO_K.
const float EXPO_K = 0.14;
// Faithful-path exposure sensitivity (photographic ~1 stop/EV) — MUST equal engine.rs
// FAITHFUL_EXPO_K. Replaces the weak shared EXPO_K for the faithful branch so auto-exposure
// + the slider actually move brightness.
const float FAITHFUL_EXPO_K = 1.0;
// Faithful 0 EV baseline anchor: base+0.6D renders at 8-bit 118 mid-gray (issue #41).
// MUST equal engine.rs FAITHFUL_BASELINE_EV.
const float FAITHFUL_BASELINE_EV = -2.25;
// Subtractive WB strength — MUST equal engine.rs CMY_STRENGTH.
const float CMY_STRENGTH = 1.6;

// Faithful reconstruction curve + FIXED density scale — MUST equal engine.rs.
// FAITHFUL_SCALE = 1/recommended_d_max (0.700) from the C400 fit. The faithful core
// scales the RAW density d by this CONSTANT (NOT the per-frame u_d_max): a frozen,
// faithful transfer identical on every frame. (The old (d/d_max)*ANCHOR form coupled the
// scale to per-frame d_max and blew highlights on any frame whose d_max != 2.896.)
const float FAITHFUL_GAMMA  = 1.590;
const float FAITHFUL_KNEE   = 0.892;
const float FAITHFUL_SCALE  = 1.0 / 0.700;
// Look-layer strength -- MUST equal engine.rs LOOK_K (the clean-punchy MEDIUM).
const float LOOK_K = 2.0;
const float REC_H_GAIN = 3.0; // MUST equal engine.rs REC_H_GAIN
const float REC_S_GAIN = 0.6; // MUST equal engine.rs REC_S_GAIN
// Faithful gamma BODY only — no shoulder, no look, no clamp. Super-white body
// (> 1) is preserved for the finish FRAG to finalize. MUST equal engine.rs gamma_body.
float gammaBody(float x) { return pow(max(x, 0.0), 1.0 / FAITHFUL_GAMMA); }

// Filmic display S-curve — MUST equal engine.rs FILMIC_K/FILMIC_PIVOT/FILMIC_WHITE_T
// and filmic_s(). Logistic on normalised log-density, rescaled so filmicS(0)==0
// (neutral black) and filmicS(FILMIC_WHITE_T)==1.0 (true white). Replaces the old
// paper-grade/soft-clip encode that capped white at ~0.90.
const float FILMIC_K = 5.0;
const float FILMIC_PIVOT = 0.44; // < 0.5: brighter mids (calibration lift); see engine.rs
const float FILMIC_WHITE_T = 1.05;
float filmicL(float x) { return 1.0 / (1.0 + exp(-FILMIC_K * (x - FILMIC_PIVOT))); }
// Unclamped filmic forward — mirrors engine.rs filmic_s_raw (super-white density
// stays > 1 for the WB round-trip; do NOT clamp here).
float filmicSraw(float t) {
  float l0 = filmicL(0.0);
  float lw = filmicL(FILMIC_WHITE_T);
  return (filmicL(t) - l0) / (lw - l0);
}
float filmicS(float t) { return clamp(filmicSraw(t), 0.0, 1.0); }
// Exact inverse of filmicSraw (a logit) — mirrors engine.rs filmic_inv. Maps a
// display density y back to normalised log-density; filmicInv(0)==0. big is
// clamped just inside (0,1) so the logit stays finite when WB pushes y past the
// white asymptote (y ≳ 1.053) — that channel is a blown highlight → white.
float filmicInv(float y) {
  float l0 = filmicL(0.0);
  float lw = filmicL(FILMIC_WHITE_T);
  float big = clamp(y * (lw - l0) + l0, 1e-6, 1.0 - 1e-6); // = filmicL(t)
  return FILMIC_PIVOT + log(big / (1.0 - big)) / FILMIC_K;
}

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
  if (u_mode == 3) {           // Mode D: negadoctor (Cineon). Mirrors engine.rs invert_d.
    // NOTE: Mode D does not use tone()/u_exposure/u_black/u_gamma. u_paper_black/
    // u_paper_grade/u_soft_clip are DEPRECATED by the filmic curve and inert here.
    // Like Naive, this re-derives from rgbIn and ignores the shared 'r' above
    // (it needs its own THRESH clamp, not r's [EPS,1] clamp).
    const float THRESH = 2.3283064e-10;
    vec3 clamped = max(rgbIn, vec3(THRESH));
    vec3 dmin = max(u_base, vec3(EPS));
    // Negative density d = log10(base/scan) >= 0 — linear in scene stops.
    vec3 d = max(log2(dmin / clamped) * LOG10, vec3(0.0));  // log10(dmin/clamped)
    // Exposure scales the WB-NEUTRALISED log-density (not raw t) so brightness
    // changes hue-free: EV stops scale by 2^(EXPO_K·EV). EV=0 → expo_gain==1 →
    // look unchanged. d_max sets the white anchor. (Mirrors engine.rs invert_d.)
    float ev = log2(max(u_print_exposure, EPS));
    float expo_gain = exp2(EXPO_K * ev);
    // Normalised log-density; d == d_max -> t == 1 (white point).
    vec3 t = d / max(u_d_max, EPS);
    // WB is a linear gain on the positive OUTPUT (filmic value), NOT a t-scale:
    // keeps black neutral (filmicS(0)*wb = 0) and stays consistent with the
    // gray-world auto-WB + gray-point picker. y is the WB-neutralised EV-0
    // display density (UNCLAMPED forward, so super-white highlights stay distinct);
    // exposure then scales its log-density filmicInv(y) and re-applies the curve —
    // so a neutral patch (equal y across channels) stays neutral at every exposure
    // (fixes the ±5-EV temperature shift).
    // WB mode (mirror engine.rs invert_d):
    //  0 gain: WB as post-curve display gain, exposure via the filmicInv round-trip.
    //  1 subtractive (color head): per-channel density multiply BEFORE the curve,
    //    folding exposure into the same t-multiply; anchored at black (t=0 → 0).
    vec3 v;
    if (u_tone_mode == 1) {
      // Faithful: gamma body + soft shoulder. The Faithful branch returns the unclamped gamma body;
      // display-finalize (shoulder + look) happens later in the FRAG finish pass.
      // Exposure is a LINEAR-LIGHT gain on the reconstructed scene, BEFORE the contrast curve —
      // treat the log-inverted negative as a positive and expose it like a TIFF. Black-anchored
      // linear L = 10^d − 1 (d = 0 → 0, black pivots); gain ×2^EV; back to density. EV 0 is the
      // identity. gammaBody supplies the gamma; shoulder rolloff is applied later in FRAG displayFinalize.
      // Mirror: engine.rs invert_d Faithful.
      // Per-channel density neutralise (camera-matrix mode); [1,1,1] → d·1 == d (identity).
      vec3 lScene = max(pow(vec3(10.0), d * u_cam_balance) - 1.0, 0.0);
      vec3 lit = lScene * exp2(FAITHFUL_EXPO_K * ev + FAITHFUL_BASELINE_EV);
      vec3 te = log2(lit + 1.0) * LOG10 * FAITHFUL_SCALE;
      if (u_wb_mode == 1) {
        vec3 s = pow(max(u_wb, vec3(EPS)), vec3(CMY_STRENGTH));
        v = vec3(gammaBody(te.r * s.r), gammaBody(te.g * s.g), gammaBody(te.b * s.b));
      } else {
        v = vec3(gammaBody(te.r) * u_wb.r, gammaBody(te.g) * u_wb.g, gammaBody(te.b) * u_wb.b);
      }
      return v;   // super-white body, UNCLAMPED — finish FRAG finalizes to display.
    } else {
      // Filmic: logistic S-curve on WB-neutralised log-density.
      if (u_wb_mode == 1) {
        vec3 s = pow(max(u_wb, vec3(EPS)), vec3(CMY_STRENGTH));
        v = vec3(
          filmicS(t.r * s.r * expo_gain),
          filmicS(t.g * s.g * expo_gain),
          filmicS(t.b * s.b * expo_gain));
      } else {
        vec3 y = vec3(filmicSraw(t.r), filmicSraw(t.g), filmicSraw(t.b)) * u_wb;
        v = vec3(
          filmicS(filmicInv(y.r) * expo_gain),
          filmicS(filmicInv(y.g) * expo_gain),
          filmicS(filmicInv(y.b) * expo_gain));
      }
    }
    return clamp(v, 0.0, 1.0);
  }
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
  uv = u_view_off + uv * u_view_scale;   // deep-zoom window (identity = off 0, scale 1)
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
  if (u_positive) {
    // Positive passthrough: display-encode the linear scan with WB + exposure
    // gain. Mirrors engine.rs develop_positive_px (pow(rgb*pe*wb, 1/2.2)).
    o = vec4(pow(max(rgb * u_print_exposure * u_wb, 0.0), vec3(1.0/2.2)), 1.0); return;
  }
  o = vec4(invert(rgb), 1.0);
}`;
