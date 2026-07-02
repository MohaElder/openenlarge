import { describe, it, expect } from "vitest";
import {
  packHdrUniforms, HDR_UNIFORMS_BYTES,
  OFF_CONTRAST, OFF_BRIGHTNESS, OFF_TEXEL, OFF_CG_SH_OFF, OFF_CM_HUE, OFF_PC_COUNT,
  OFF_PZ_ENABLED, OFF_PZ_SH, OFF_FINISH_MODE, OFF_BASE, OFF_M_PRE, OFF_M_POST,
  OFF_D_MAX, OFF_MODE, OFF_CAM_BALANCE, OFF_CROP_OFF, OFF_ORIENT, OFF_VIEW_SCALE,
  type HdrUniformsInput,
} from "./uniforms";

const IDENT3 = [1, 0, 0, 0, 1, 0, 0, 0, 1];
const ZERO8 = new Float32Array(8);

// Minimal-but-complete fixture: every field of HdrUniformsInput is set.
const SAMPLE: HdrUniformsInput = {
  contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
  texture: 0, vibrance: 0, saturation: 0, brightness: 0,
  texel: [1 / 1024, 1 / 768],

  cg_sh_off: [0, 0, 0], cg_mid_off: [0, 0, 0], cg_hi_off: [0, 0, 0], cg_glob_off: [0, 0, 0],
  cg_sh_lum: 0, cg_mid_lum: 0, cg_hi_lum: 0, cg_glob_lum: 0,
  cg_sh_edge: 0.33, cg_hi_edge: 0.66, cg_soft: 0.1,

  cm_hue: ZERO8, cm_sat: ZERO8, cm_lum: ZERO8,

  pc_count: 0,
  pc_hue: ZERO8, pc_sat: ZERO8, pc_lum: ZERO8,
  pc_hue_shift: ZERO8, pc_sat_shift: ZERO8, pc_lum_shift: ZERO8,
  pc_variance: ZERO8, pc_range: ZERO8,

  pz_enabled: 0, pz_sh: [1, 1, 1], pz_mid: [1, 1, 1], pz_hi: [1, 1, 1],

  clip_high_on: 0, clip_low_on: 0, clip_strict: 0, soft_clip: 0.9,

  finish_mode: 0, finalize_body: 1,

  base: [1, 1, 1], wb: [1, 1, 1],
  m_pre: IDENT3, m_post: IDENT3,
  exposure: 1, black: 0, gamma: 1, d_max: 1.5,
  print_exposure: 1, paper_black: 0, paper_grade: 0,
  mode: 3, raw: 0, positive: 0, wb_mode: 0, tone_mode: 1,
  hi_recovery: 0, lo_recovery: 0,
  cam_balance: [1, 1, 1],

  crop_off: [0, 0], crop_scale: [1, 1],
  angle: 0, aspect: 1,
  orient: [1, 0, 0, 1],
  view_off: [0, 0], view_scale: [1, 1],
};

describe("packHdrUniforms", () => {
  it("produces a 16-byte-aligned buffer of the fixed struct size", () => {
    const buf = packHdrUniforms(SAMPLE);
    expect(buf.byteLength).toBe(HDR_UNIFORMS_BYTES);
    expect(buf.byteLength % 16).toBe(0);
  });

  it("writes scalar fields at their declared offsets", () => {
    const buf = packHdrUniforms({ ...SAMPLE, contrast: 0.5, d_max: 1.5 });
    const f = new Float32Array(buf);
    expect(f[OFF_CONTRAST / 4]).toBeCloseTo(0.5);
    expect(f[OFF_D_MAX / 4]).toBeCloseTo(1.5);
  });

  it("writes vec2/vec3 fields (texel, cg_sh_off) at their aligned offsets", () => {
    const buf = packHdrUniforms({ ...SAMPLE, texel: [0.25, 0.5], cg_sh_off: [0.1, 0.2, 0.3] });
    const f = new Float32Array(buf);
    expect(f[OFF_TEXEL / 4]).toBeCloseTo(0.25);
    expect(f[OFF_TEXEL / 4 + 1]).toBeCloseTo(0.5);
    expect(f[OFF_CG_SH_OFF / 4]).toBeCloseTo(0.1);
    expect(f[OFF_CG_SH_OFF / 4 + 1]).toBeCloseTo(0.2);
    expect(f[OFF_CG_SH_OFF / 4 + 2]).toBeCloseTo(0.3);
  });

  it("writes the cm_hue f32x8 array tightly packed (4-byte stride)", () => {
    const cm_hue = new Float32Array([1, 2, 3, 4, 5, 6, 7, 8]);
    const buf = packHdrUniforms({ ...SAMPLE, cm_hue });
    const f = new Float32Array(buf);
    for (let i = 0; i < 8; i++) expect(f[OFF_CM_HUE / 4 + i]).toBeCloseTo(i + 1);
  });

  it("writes i32 fields (pc_count, pz_enabled, finish_mode, mode) as integers", () => {
    const buf = packHdrUniforms({ ...SAMPLE, pc_count: 3, pz_enabled: 1, finish_mode: 1, mode: 2 });
    const i = new Int32Array(buf);
    expect(i[OFF_PC_COUNT / 4]).toBe(3);
    expect(i[OFF_PZ_ENABLED / 4]).toBe(1);
    expect(i[OFF_FINISH_MODE / 4]).toBe(1);
    expect(i[OFF_MODE / 4]).toBe(2);
  });

  it("writes pz_sh vec3 at its 16-byte-aligned offset", () => {
    const buf = packHdrUniforms({ ...SAMPLE, pz_sh: [0.4, 0.5, 0.6] });
    const f = new Float32Array(buf);
    expect(f[OFF_PZ_SH / 4]).toBeCloseTo(0.4);
    expect(f[OFF_PZ_SH / 4 + 1]).toBeCloseTo(0.5);
    expect(f[OFF_PZ_SH / 4 + 2]).toBeCloseTo(0.6);
  });

  it("writes base vec3 and m_pre/m_post mat3x3 (3x 16-byte columns) correctly", () => {
    const m_pre = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    const m_post = [9, 8, 7, 6, 5, 4, 3, 2, 1];
    const buf = packHdrUniforms({ ...SAMPLE, base: [0.7, 0.8, 0.9], m_pre, m_post });
    const f = new Float32Array(buf);
    expect(f[OFF_BASE / 4]).toBeCloseTo(0.7);
    expect(f[OFF_BASE / 4 + 1]).toBeCloseTo(0.8);
    expect(f[OFF_BASE / 4 + 2]).toBeCloseTo(0.9);
    // Column 0 at OFF_M_PRE, column 1 at +16 bytes, column 2 at +32 bytes.
    expect(f[OFF_M_PRE / 4 + 0]).toBeCloseTo(1);
    expect(f[OFF_M_PRE / 4 + 1]).toBeCloseTo(2);
    expect(f[OFF_M_PRE / 4 + 2]).toBeCloseTo(3);
    expect(f[OFF_M_PRE / 4 + 4]).toBeCloseTo(4);
    expect(f[OFF_M_PRE / 4 + 5]).toBeCloseTo(5);
    expect(f[OFF_M_PRE / 4 + 6]).toBeCloseTo(6);
    expect(f[OFF_M_PRE / 4 + 8]).toBeCloseTo(7);
    expect(f[OFF_M_PRE / 4 + 9]).toBeCloseTo(8);
    expect(f[OFF_M_PRE / 4 + 10]).toBeCloseTo(9);
    expect(f[OFF_M_POST / 4 + 0]).toBeCloseTo(9);
    expect(f[OFF_M_POST / 4 + 4]).toBeCloseTo(6);
    expect(f[OFF_M_POST / 4 + 8]).toBeCloseTo(3);
  });

  it("writes cam_balance, crop_off, orient (mat2x2) and view_scale at their offsets", () => {
    const buf = packHdrUniforms({
      ...SAMPLE,
      cam_balance: [1.1, 1.2, 1.3],
      crop_off: [0.05, 0.06],
      orient: [0, -1, 1, 0],
      view_scale: [2, 3],
    });
    const f = new Float32Array(buf);
    expect(f[OFF_CAM_BALANCE / 4]).toBeCloseTo(1.1);
    expect(f[OFF_CAM_BALANCE / 4 + 1]).toBeCloseTo(1.2);
    expect(f[OFF_CAM_BALANCE / 4 + 2]).toBeCloseTo(1.3);
    expect(f[OFF_CROP_OFF / 4]).toBeCloseTo(0.05);
    expect(f[OFF_CROP_OFF / 4 + 1]).toBeCloseTo(0.06);
    expect(f[OFF_ORIENT / 4]).toBeCloseTo(0);
    expect(f[OFF_ORIENT / 4 + 1]).toBeCloseTo(-1);
    expect(f[OFF_ORIENT / 4 + 2]).toBeCloseTo(1);
    expect(f[OFF_ORIENT / 4 + 3]).toBeCloseTo(0);
    expect(f[OFF_VIEW_SCALE / 4]).toBeCloseTo(2);
    expect(f[OFF_VIEW_SCALE / 4 + 1]).toBeCloseTo(3);
  });
});
