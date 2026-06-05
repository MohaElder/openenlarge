import { describe, it, expect } from "vitest";
import { spinGeometry } from "./spin";

describe("spinGeometry", () => {
  it("returns null for no change or a 180° jump", () => {
    expect(spinGeometry(0, 0, 200, 300, 1000, 800, 60)).toBeNull();
    expect(spinGeometry(0, 2, 200, 300, 1000, 800, 60)).toBeNull();
  });
  it("CW (+1) is dir 1, CCW (+3) is dir -1", () => {
    expect(spinGeometry(0, 1, 200, 300, 1000, 800, 60)!.dir).toBe(1);
    expect(spinGeometry(1, 0, 200, 300, 1000, 800, 60)!.dir).toBe(-1);
  });
  it("k = newFit/oldFit and rect is centered at the old fitted size", () => {
    const g = spinGeometry(0, 1, 200, 300, 1000, 800, 60)!;
    const avW = 1000 - 120, avH = 800 - 120;
    const oldFit = Math.min(avW / 300, avH / 200);
    const newFit = Math.min(avW / 200, avH / 300);
    expect(g.k).toBeCloseTo(newFit / oldFit, 5);
    expect(g.rect.width).toBeCloseTo(300 * oldFit, 3);
    expect(g.rect.left).toBeCloseTo((1000 - 300 * oldFit) / 2, 3);
    expect(g.rect.top).toBeCloseTo((800 - 200 * oldFit) / 2, 3);
  });
});
