import { describe, it, expect } from "vitest";
import { rgbToHslSample } from "./colorPick";

describe("rgbToHslSample", () => {
  it("converts a mid red byte pixel to HSL fields", () => {
    const s = rgbToHslSample(204, 51, 51); // ~ [0.8,0.2,0.2]
    expect(s.hue).toBeCloseTo(0, 0);
    expect(s.sat).toBeGreaterThan(0.5);
    expect(s.lum).toBeCloseTo(0.5, 1);
    expect(s.hue_shift).toBe(0);
    expect(s.range).toBe(50);
  });
  it("gray maps to zero saturation", () => {
    const s = rgbToHslSample(128, 128, 128);
    expect(s.sat).toBeCloseTo(0, 2);
  });
});
