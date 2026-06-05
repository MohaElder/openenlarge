import { describe, it, expect } from "vitest";
import {
  emptyDust, addStroke, undoStroke, resetDust, setIrEnabled, setIrSensitivity,
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
    expect(resetDust(emptyDust()).strokes.length).toBe(0);
  });
  it("undo on empty is safe", () => {
    expect(undoStroke(emptyDust()).strokes.length).toBe(0);
  });
});

describe("ir removal state", () => {
  it("defaults disabled at sensitivity 50", () => {
    const d = emptyDust();
    expect(d.irRemoval.enabled).toBe(false);
    expect(d.irRemoval.sensitivity).toBe(50);
  });
  it("toggles enabled and sets sensitivity immutably, preserving strokes", () => {
    const d0 = addStroke(emptyDust(), { points: [{ x: 0.5, y: 0.5 }], r: 0.02 });
    const d1 = setIrEnabled(d0, true);
    const d2 = setIrSensitivity(d1, 70);
    expect(d0.irRemoval.enabled).toBe(false);
    expect(d2.irRemoval).toEqual({ enabled: true, sensitivity: 70 });
    expect(d2.strokes.length).toBe(1);
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
