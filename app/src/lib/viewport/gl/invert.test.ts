import { describe, it, expect } from "vitest";
import { toInversionUniforms, type ResolvedInversion } from "./invert";

const RES: ResolvedInversion = {
  base: [0.8, 0.6, 0.4],
  wb: [1.1, 1.0, 0.9],
  m_pre: [1, 0, 0, 0, 1, 0, 0, 0, 1],
  m_post: [2, 0, 0, 0, 1, 0, 0, 0, 1],
  exposure: 2.0,
  black: 0.05,
  gamma: 0.4545,
  mode: 0,
};

describe("toInversionUniforms", () => {
  it("passes scalars through and builds Float32Array mat3s", () => {
    const u = toInversionUniforms(RES);
    expect(u.exposure).toBe(2.0);
    expect(u.black).toBeCloseTo(0.05);
    expect(u.gamma).toBeCloseTo(0.4545);
    expect(u.mode).toBe(0);
    expect(Array.from(u.base)).toEqual(Array.from(new Float32Array([0.8, 0.6, 0.4])));
    expect(Array.from(u.wb)).toEqual(Array.from(new Float32Array([1.1, 1.0, 0.9])));
    expect(u.m_post).toBeInstanceOf(Float32Array);
    expect(u.m_post.length).toBe(9);
    expect(Array.from(u.m_post)).toEqual([2, 0, 0, 0, 1, 0, 0, 0, 1]);
  });
});
