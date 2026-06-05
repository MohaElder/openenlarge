import { describe, it, expect } from "vitest";
import { sampleCurve, curveLut, sampleLut, LUT_SIZE } from "./curve";
import type { CurvePoint } from "../api";

const IDENTITY: CurvePoint[] = [[0, 0], [1, 1]];

describe("sampleCurve", () => {
  it("identity curve returns the input", () => {
    for (const x of [0, 0.1, 0.25, 0.5, 0.75, 0.9, 1]) {
      expect(sampleCurve(IDENTITY, x)).toBeCloseTo(x, 5);
    }
  });

  it("clamps output to [0,1]", () => {
    const steep: CurvePoint[] = [[0, 0], [0.5, 1], [1, 1]];
    for (const x of [0, 0.5, 1]) {
      const y = sampleCurve(steep, x);
      expect(y).toBeGreaterThanOrEqual(0);
      expect(y).toBeLessThanOrEqual(1);
    }
  });

  it("honors endpoint values (flat extrapolation)", () => {
    const lifted: CurvePoint[] = [[0, 0.1], [1, 0.9]];
    expect(sampleCurve(lifted, 0)).toBeCloseTo(0.1, 5);
    expect(sampleCurve(lifted, 1)).toBeCloseTo(0.9, 5);
    expect(sampleCurve(lifted, -1)).toBeCloseTo(0.1, 5);
    expect(sampleCurve(lifted, 2)).toBeCloseTo(0.9, 5);
  });
});

describe("curveLut", () => {
  it("identity LUT is the ramp", () => {
    const lut = curveLut(IDENTITY);
    expect(lut.length).toBe(LUT_SIZE);
    expect(lut[0]).toBeCloseTo(0, 5);
    expect(lut[LUT_SIZE - 1]).toBeCloseTo(1, 5);
    expect(lut[128]).toBeCloseTo(128 / 255, 4);
  });

  it("stays monotone with a midtone lift", () => {
    const lut = curveLut([[0, 0], [0.25, 0.45], [0.75, 0.6], [1, 1]]);
    for (let i = 1; i < lut.length; i++) {
      expect(lut[i]).toBeGreaterThanOrEqual(lut[i - 1] - 1e-6);
    }
  });
});

describe("sampleLut", () => {
  it("linearly interpolates a ramp", () => {
    const lut = curveLut(IDENTITY);
    expect(sampleLut(lut, 0.5)).toBeCloseTo(0.5, 3);
    expect(sampleLut(lut, 0)).toBeCloseTo(0, 5);
    expect(sampleLut(lut, 1)).toBeCloseTo(1, 5);
  });
});
