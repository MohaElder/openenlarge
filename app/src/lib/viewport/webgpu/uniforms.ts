// Byte-layout packer for the WGSL `HdrUniforms` storage buffer declared in
// `./shaders` (`WGSL_UNIFORMS`) — the WebGPU twin of
// `app/src-tauri/src/hdr_surface/uniforms.rs`'s `HdrUniforms` (MSL `constant`
// struct). Field order, offsets, and sizes here MUST match that WGSL struct
// exactly, field-for-field, or the on-device render reads garbage.
//
// Because the WGSL struct is bound `var<storage, read>` (not `var<uniform>` —
// see shaders.ts's "UNIFORM ADDRESS-SPACE FIX" note), its layout follows
// WGSL's storage-address-space rules, which for THIS struct are byte-for-byte
// identical to the MSL `constant`-address-space layout already pinned by
// `uniforms.rs`'s `hdr_uniforms_layout_matches_msl` test:
//   - scalar (f32/i32): 4 bytes, align 4.
//   - vec2f: 8 bytes, align 8.
//   - vec3f: align 16, but per the WGSL spec its bare SIZE is only 12 bytes —
//     a scalar following a bare vec3f would get packed into that 4-byte gap
//     instead of landing at +16 like the MSL/Rust `float3` (which is
//     `#[repr(C, align(16))]` with an explicit `_pad: f32`). To make the two
//     match, every `vec3f` member in `WGSL_UNIFORMS` (shaders.ts) MUST carry
//     an explicit `@size(16)` attribute, forcing it to occupy the full 16
//     bytes (12 data + 4 pad) like vec4f. Don't add a bare `vec3f` field to
//     that struct without it, and don't drop `@size(16)` from an existing
//     one — either breaks this parity silently. See
//     `uniforms.test.ts`'s drift-guard test, which asserts every `vec3f` in
//     `WGSL_UNIFORMS` has a matching `@size(16)`.
//   - mat3x3<f32>: 3 columns of vec3f = 48 bytes, align 16.
//   - mat2x2<f32>: 2 columns of vec2f = 16 bytes, align 8.
//   - array<f32, 8>: storage-space arrays of a 4-byte-aligned scalar pack
//     tightly at 4-byte stride (no 16-byte minimum stride — that rule is
//     `uniform`-address-space-only, which is exactly why this buffer is
//     `storage` instead: a literal `array<f32,8>` is illegal in `uniform`).
// The whole struct is aligned to its largest member (16) and its size is
// padded up to a multiple of 16.
//
// Every offset below was hand-derived from `WGSL_UNIFORMS`'s field order
// (identical to `uniforms.rs`'s `HdrUniforms` field order) and cross-checked
// against the exact offsets `uniforms.rs`'s test pins for the MSL/Rust twin
// (contrast=0, brightness=32, texel=40, cg_sh_off=48, cm_hue=140,
// pc_count=236, pz_enabled=496, pz_sh=512, finish_mode=576, base=592,
// m_pre=624, m_post=672, mode=748, cam_balance=784, crop_off=800,
// orient=824, view_scale=848) — every one matches, confirming the two
// layouts are identical for this struct.

export interface HdrUniformsInput {
  // --- finish (finish.rs::FinishParams) ---
  contrast: number; highlights: number; shadows: number; whites: number; blacks: number;
  texture: number; vibrance: number; saturation: number; brightness: number;

  texel: readonly [number, number]; // 1/out_w, 1/out_h

  // Color grading (finish.rs::ColorGrade).
  cg_sh_off: readonly [number, number, number];
  cg_mid_off: readonly [number, number, number];
  cg_hi_off: readonly [number, number, number];
  cg_glob_off: readonly [number, number, number];
  cg_sh_lum: number; cg_mid_lum: number; cg_hi_lum: number; cg_glob_lum: number;
  cg_sh_edge: number; cg_hi_edge: number; cg_soft: number;

  // Color Mixer: 8-band HSL (finish.rs::ColorMix).
  cm_hue: ArrayLike<number>; cm_sat: ArrayLike<number>; cm_lum: ArrayLike<number>;

  // Point Color: up to 8 samples.
  pc_count: number;
  pc_hue: ArrayLike<number>; pc_sat: ArrayLike<number>; pc_lum: ArrayLike<number>;
  pc_hue_shift: ArrayLike<number>; pc_sat_shift: ArrayLike<number>; pc_lum_shift: ArrayLike<number>;
  pc_variance: ArrayLike<number>; pc_range: ArrayLike<number>;

  // Per-zone WB neutralizer (finish.rs::PerZoneWb).
  pz_enabled: number;
  pz_sh: readonly [number, number, number];
  pz_mid: readonly [number, number, number];
  pz_hi: readonly [number, number, number];

  // Clip-warning overlay + shared soft-clip knee.
  clip_high_on: number; clip_low_on: number; clip_strict: number; soft_clip: number;

  finish_mode: number;   // 0 = present w/ clip overlay, 1 = plain FBO write
  finalize_body: number; // 1.0 = Faithful body finalize, 0.0 = already display-referred

  // --- invert (engine.rs::InversionParams) ---
  base: readonly [number, number, number];
  wb: readonly [number, number, number];
  m_pre: ArrayLike<number>;  // 9, column-major (col0, col1, col2)
  m_post: ArrayLike<number>; // 9, column-major
  exposure: number; black: number; gamma: number; d_max: number;
  print_exposure: number; paper_black: number; paper_grade: number;
  mode: number;      // 0=B 1=C 2=Naive 3=D
  raw: number;       // bool as int
  positive: number;  // bool as int
  wb_mode: number;
  tone_mode: number;
  hi_recovery: number; lo_recovery: number;
  cam_balance: readonly [number, number, number];

  // --- geometry: output UV -> source UV mapping ---
  crop_off: readonly [number, number];
  crop_scale: readonly [number, number];
  angle: number; aspect: number;
  orient: ArrayLike<number>; // 4, column-major (col0, col1)
  view_off: readonly [number, number];
  view_scale: readonly [number, number];
}

// --- byte offsets (all multiples of 4; see module doc comment for derivation) ---
export const OFF_CONTRAST = 0;
export const OFF_HIGHLIGHTS = 4;
export const OFF_SHADOWS = 8;
export const OFF_WHITES = 12;
export const OFF_BLACKS = 16;
export const OFF_TEXTURE = 20;
export const OFF_VIBRANCE = 24;
export const OFF_SATURATION = 28;
export const OFF_BRIGHTNESS = 32;

export const OFF_TEXEL = 40;

export const OFF_CG_SH_OFF = 48;
export const OFF_CG_MID_OFF = 64;
export const OFF_CG_HI_OFF = 80;
export const OFF_CG_GLOB_OFF = 96;
export const OFF_CG_SH_LUM = 112;
export const OFF_CG_MID_LUM = 116;
export const OFF_CG_HI_LUM = 120;
export const OFF_CG_GLOB_LUM = 124;
export const OFF_CG_SH_EDGE = 128;
export const OFF_CG_HI_EDGE = 132;
export const OFF_CG_SOFT = 136;

export const OFF_CM_HUE = 140;
export const OFF_CM_SAT = 172;
export const OFF_CM_LUM = 204;

export const OFF_PC_COUNT = 236;
export const OFF_PC_HUE = 240;
export const OFF_PC_SAT = 272;
export const OFF_PC_LUM = 304;
export const OFF_PC_HUE_SHIFT = 336;
export const OFF_PC_SAT_SHIFT = 368;
export const OFF_PC_LUM_SHIFT = 400;
export const OFF_PC_VARIANCE = 432;
export const OFF_PC_RANGE = 464;

export const OFF_PZ_ENABLED = 496;
export const OFF_PZ_SH = 512;
export const OFF_PZ_MID = 528;
export const OFF_PZ_HI = 544;

export const OFF_CLIP_HIGH_ON = 560;
export const OFF_CLIP_LOW_ON = 564;
export const OFF_CLIP_STRICT = 568;
export const OFF_SOFT_CLIP = 572;

export const OFF_FINISH_MODE = 576;
export const OFF_FINALIZE_BODY = 580;

export const OFF_BASE = 592;
export const OFF_WB = 608;
export const OFF_M_PRE = 624;
export const OFF_M_POST = 672;

export const OFF_EXPOSURE = 720;
export const OFF_BLACK = 724;
export const OFF_GAMMA = 728;
export const OFF_D_MAX = 732;
export const OFF_PRINT_EXPOSURE = 736;
export const OFF_PAPER_BLACK = 740;
export const OFF_PAPER_GRADE = 744;

export const OFF_MODE = 748;
export const OFF_RAW = 752;
export const OFF_POSITIVE = 756;
export const OFF_WB_MODE = 760;
export const OFF_TONE_MODE = 764;

export const OFF_HI_RECOVERY = 768;
export const OFF_LO_RECOVERY = 772;

export const OFF_CAM_BALANCE = 784;

export const OFF_CROP_OFF = 800;
export const OFF_CROP_SCALE = 808;
export const OFF_ANGLE = 816;
export const OFF_ASPECT = 820;
export const OFF_ORIENT = 824;
export const OFF_VIEW_OFF = 840;
export const OFF_VIEW_SCALE = 848;

/** Total struct size, padded up to the struct's 16-byte alignment. */
export const HDR_UNIFORMS_BYTES = 864;

function putF32(f32: Float32Array, byteOffset: number, v: number): void {
  f32[byteOffset >> 2] = v;
}
function putI32(i32: Int32Array, byteOffset: number, v: number): void {
  i32[byteOffset >> 2] = v | 0;
}
function putVec2(f32: Float32Array, byteOffset: number, v: ArrayLike<number>): void {
  f32[(byteOffset >> 2) + 0] = v[0];
  f32[(byteOffset >> 2) + 1] = v[1];
}
function putVec3(f32: Float32Array, byteOffset: number, v: ArrayLike<number>): void {
  f32[(byteOffset >> 2) + 0] = v[0];
  f32[(byteOffset >> 2) + 1] = v[1];
  f32[(byteOffset >> 2) + 2] = v[2];
  // 4th lane is the vec3->vec4 pad; leave as zero-init.
}
function putArr8(f32: Float32Array, byteOffset: number, v: ArrayLike<number>): void {
  const base = byteOffset >> 2;
  for (let i = 0; i < 8; i++) f32[base + i] = v[i] ?? 0;
}
/** mat3x3<f32>: 3 columns, each a vec3f (16-byte slot, 12 bytes of data). */
function putMat3(f32: Float32Array, byteOffset: number, colMajor9: ArrayLike<number>): void {
  putVec3(f32, byteOffset, [colMajor9[0], colMajor9[1], colMajor9[2]]);
  putVec3(f32, byteOffset + 16, [colMajor9[3], colMajor9[4], colMajor9[5]]);
  putVec3(f32, byteOffset + 32, [colMajor9[6], colMajor9[7], colMajor9[8]]);
}
/** mat2x2<f32>: 2 columns, each a vec2f (8-byte slot). */
function putMat2(f32: Float32Array, byteOffset: number, colMajor4: ArrayLike<number>): void {
  putVec2(f32, byteOffset, [colMajor4[0], colMajor4[1]]);
  putVec2(f32, byteOffset + 8, [colMajor4[2], colMajor4[3]]);
}

/**
 * Lays already-resolved finish+invert+geometry values (the same values
 * `Viewport` hands the WebGL renderer — sliders already ÷100, etc.) into the
 * WGSL storage-buffer byte layout. Does NOT re-resolve/re-scale anything.
 */
export function packHdrUniforms(u: HdrUniformsInput): ArrayBuffer {
  const buf = new ArrayBuffer(HDR_UNIFORMS_BYTES);
  const f32 = new Float32Array(buf);
  const i32 = new Int32Array(buf);

  putF32(f32, OFF_CONTRAST, u.contrast);
  putF32(f32, OFF_HIGHLIGHTS, u.highlights);
  putF32(f32, OFF_SHADOWS, u.shadows);
  putF32(f32, OFF_WHITES, u.whites);
  putF32(f32, OFF_BLACKS, u.blacks);
  putF32(f32, OFF_TEXTURE, u.texture);
  putF32(f32, OFF_VIBRANCE, u.vibrance);
  putF32(f32, OFF_SATURATION, u.saturation);
  putF32(f32, OFF_BRIGHTNESS, u.brightness);

  putVec2(f32, OFF_TEXEL, u.texel);

  putVec3(f32, OFF_CG_SH_OFF, u.cg_sh_off);
  putVec3(f32, OFF_CG_MID_OFF, u.cg_mid_off);
  putVec3(f32, OFF_CG_HI_OFF, u.cg_hi_off);
  putVec3(f32, OFF_CG_GLOB_OFF, u.cg_glob_off);
  putF32(f32, OFF_CG_SH_LUM, u.cg_sh_lum);
  putF32(f32, OFF_CG_MID_LUM, u.cg_mid_lum);
  putF32(f32, OFF_CG_HI_LUM, u.cg_hi_lum);
  putF32(f32, OFF_CG_GLOB_LUM, u.cg_glob_lum);
  putF32(f32, OFF_CG_SH_EDGE, u.cg_sh_edge);
  putF32(f32, OFF_CG_HI_EDGE, u.cg_hi_edge);
  putF32(f32, OFF_CG_SOFT, u.cg_soft);

  putArr8(f32, OFF_CM_HUE, u.cm_hue);
  putArr8(f32, OFF_CM_SAT, u.cm_sat);
  putArr8(f32, OFF_CM_LUM, u.cm_lum);

  putI32(i32, OFF_PC_COUNT, u.pc_count);
  putArr8(f32, OFF_PC_HUE, u.pc_hue);
  putArr8(f32, OFF_PC_SAT, u.pc_sat);
  putArr8(f32, OFF_PC_LUM, u.pc_lum);
  putArr8(f32, OFF_PC_HUE_SHIFT, u.pc_hue_shift);
  putArr8(f32, OFF_PC_SAT_SHIFT, u.pc_sat_shift);
  putArr8(f32, OFF_PC_LUM_SHIFT, u.pc_lum_shift);
  putArr8(f32, OFF_PC_VARIANCE, u.pc_variance);
  putArr8(f32, OFF_PC_RANGE, u.pc_range);

  putI32(i32, OFF_PZ_ENABLED, u.pz_enabled);
  putVec3(f32, OFF_PZ_SH, u.pz_sh);
  putVec3(f32, OFF_PZ_MID, u.pz_mid);
  putVec3(f32, OFF_PZ_HI, u.pz_hi);

  putF32(f32, OFF_CLIP_HIGH_ON, u.clip_high_on);
  putF32(f32, OFF_CLIP_LOW_ON, u.clip_low_on);
  putF32(f32, OFF_CLIP_STRICT, u.clip_strict);
  putF32(f32, OFF_SOFT_CLIP, u.soft_clip);

  putI32(i32, OFF_FINISH_MODE, u.finish_mode);
  putF32(f32, OFF_FINALIZE_BODY, u.finalize_body);

  putVec3(f32, OFF_BASE, u.base);
  putVec3(f32, OFF_WB, u.wb);
  putMat3(f32, OFF_M_PRE, u.m_pre);
  putMat3(f32, OFF_M_POST, u.m_post);

  putF32(f32, OFF_EXPOSURE, u.exposure);
  putF32(f32, OFF_BLACK, u.black);
  putF32(f32, OFF_GAMMA, u.gamma);
  putF32(f32, OFF_D_MAX, u.d_max);
  putF32(f32, OFF_PRINT_EXPOSURE, u.print_exposure);
  putF32(f32, OFF_PAPER_BLACK, u.paper_black);
  putF32(f32, OFF_PAPER_GRADE, u.paper_grade);

  putI32(i32, OFF_MODE, u.mode);
  putI32(i32, OFF_RAW, u.raw);
  putI32(i32, OFF_POSITIVE, u.positive);
  putI32(i32, OFF_WB_MODE, u.wb_mode);
  putI32(i32, OFF_TONE_MODE, u.tone_mode);

  putF32(f32, OFF_HI_RECOVERY, u.hi_recovery);
  putF32(f32, OFF_LO_RECOVERY, u.lo_recovery);

  putVec3(f32, OFF_CAM_BALANCE, u.cam_balance);

  putVec2(f32, OFF_CROP_OFF, u.crop_off);
  putVec2(f32, OFF_CROP_SCALE, u.crop_scale);
  putF32(f32, OFF_ANGLE, u.angle);
  putF32(f32, OFF_ASPECT, u.aspect);
  putMat2(f32, OFF_ORIENT, u.orient);
  putVec2(f32, OFF_VIEW_OFF, u.view_off);
  putVec2(f32, OFF_VIEW_SCALE, u.view_scale);

  return buf;
}
