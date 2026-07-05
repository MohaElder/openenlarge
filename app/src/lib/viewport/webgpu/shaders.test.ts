import { describe, it, expect } from "vitest";
import { WGSL_UNIFORMS, INVERT_WGSL, FINISH_WGSL } from "./shaders";

describe("WGSL shaders — constants parity with the canonical pipeline", () => {
  const all = INVERT_WGSL + "\n" + FINISH_WGSL;
  // Values copied from crates/film-core/src/{engine,finish}.rs — MUST match.
  const consts: Record<string, string> = {
    HDR_KNEE: "0.8", HDR_HEADROOM: "2.5", HDR_W_HI: "1.2",
    FAITHFUL_GAMMA: "1.590", FAITHFUL_KNEE: "0.892",
    LOOK_K: "2.0", EXPO_K: "0.14", CMY_STRENGTH: "1.6",
    FAITHFUL_BASELINE_EV: "-2.25",
    FILMIC_K: "5.0", FILMIC_PIVOT: "0.44", FILMIC_WHITE_T: "1.05",
  };
  for (const [name, val] of Object.entries(consts)) {
    it(`declares const ${name} = ${val}`, () => {
      // matches e.g.  const HDR_KNEE: f32 = 0.8;
      const re = new RegExp(`const\\s+${name}\\s*:\\s*f32\\s*=\\s*${val.replace(".", "\\.")}\\b`);
      expect(all).toMatch(re);
    });
  }
  it("finish output linearizes via srgbToLinearExt3", () => {
    expect(FINISH_WGSL).toMatch(/srgbToLinearExt3\s*\(/);
  });
  it("finish contains the chroma-preserving hdr_finish", () => {
    expect(FINISH_WGSL).toMatch(/fn\s+hdr_finish\s*\(/);
    expect(FINISH_WGSL).toMatch(/fn\s+hdr_finalize_scalar\s*\(/);
  });
  it("uniforms struct declares the invert + finish fields", () => {
    for (const f of ["base", "wb", "d_max", "mode", "tone_mode", "contrast", "saturation", "crop_off", "orient"]) {
      expect(WGSL_UNIFORMS).toMatch(new RegExp(`\\b${f}\\b`));
    }
  });
});
