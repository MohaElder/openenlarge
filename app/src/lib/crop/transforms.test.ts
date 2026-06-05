import { describe, it, expect } from "vitest";
import { rotateRectCW, rotateRectCCW, flipRectH, flipRectV, orientDims } from "./transforms";
import type { Rect } from "./types";
const r = (x: number, y: number, w: number, h: number): Rect => ({ x, y, w, h });
const close = (a: Rect, b: Rect) => {
  for (const k of ["x", "y", "w", "h"] as const) expect(a[k]).toBeCloseTo(b[k], 6);
};

describe("rect transforms", () => {
  it("rotateRectCW four times is identity", () => {
    let c = r(0.1, 0.2, 0.3, 0.4); const start = { ...c };
    for (let i = 0; i < 4; i++) c = rotateRectCW(c);
    close(c, start);
  });
  it("CW then CCW is identity", () => {
    const c = r(0.1, 0.2, 0.3, 0.4);
    close(rotateRectCCW(rotateRectCW(c)), c);
  });
  it("flipRectH twice is identity; mirrors x once", () => {
    const c = r(0.1, 0.2, 0.3, 0.4);
    close(flipRectH(flipRectH(c)), c);
    expect(flipRectH(c).x).toBeCloseTo(1 - 0.1 - 0.3, 6);
  });
  it("flipRectV mirrors y", () => {
    expect(flipRectV(r(0.1, 0.2, 0.3, 0.4)).y).toBeCloseTo(1 - 0.2 - 0.4, 6);
  });
  it("rotateRectCW swaps w/h", () => {
    const c = rotateRectCW(r(0.1, 0.2, 0.3, 0.4));
    expect(c.w).toBeCloseTo(0.4, 6); expect(c.h).toBeCloseTo(0.3, 6);
  });
  it("orientDims swaps on quarter turns", () => {
    expect(orientDims(2, 3, 0)).toEqual([2, 3]);
    expect(orientDims(2, 3, 1)).toEqual([3, 2]);
    expect(orientDims(2, 3, 2)).toEqual([2, 3]);
    expect(orientDims(2, 3, 3)).toEqual([3, 2]);
  });
});
