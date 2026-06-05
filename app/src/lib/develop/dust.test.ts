import { describe, it, expect } from "vitest";
import {
  emptyDust, addStroke, undoStroke, resetDust,
  screenRadius, normRadius, type DustEdits, type DustStroke,
} from "./dust";

const stroke = (r: number): DustStroke => ({ points: [{ x: 0.5, y: 0.5 }], r });

describe("dust edit-state", () => {
  it("adds, undoes, and resets strokes immutably", () => {
    const d0 = emptyDust();
    const d1 = addStroke(d0, stroke(0.02));
    const d2 = addStroke(d1, stroke(0.03));
    expect(d0.strokes.length).toBe(0); // original untouched
    expect(d2.strokes.length).toBe(2);
    expect(undoStroke(d2).strokes.length).toBe(1);
    expect(resetDust().strokes.length).toBe(0);
  });
  it("undo on empty is safe", () => {
    expect(undoStroke(emptyDust()).strokes.length).toBe(0);
  });
});

describe("brush radius mapping", () => {
  it("round-trips normalized ↔ screen radius", () => {
    const imgW = 4000, eff = 0.25; // 0.25 display px per image px
    const screen = screenRadius(0.02, imgW, eff); // 0.02*4000*0.25 = 20
    expect(screen).toBeCloseTo(20, 5);
    expect(normRadius(screen, imgW, eff)).toBeCloseTo(0.02, 5);
  });
  it("normRadius is safe at zero", () => {
    expect(normRadius(10, 0, 0)).toBe(0);
  });
});
