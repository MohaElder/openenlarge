import { describe, it, expect } from "vitest";
import {
  reciprocalPos, reciprocalValue, reciprocalSpan,
  centeredPos, centeredValue, CENTERED_SPAN,
} from "./sliderScale";

const MIN = 2000;
const MAX = 50000;

describe("reciprocal slider scale", () => {
  it("pins min to the left (pos 0) and max to the right (pos = span)", () => {
    expect(reciprocalPos(MIN, MIN)).toBeCloseTo(0, 6);
    expect(reciprocalPos(MAX, MIN)).toBeCloseTo(reciprocalSpan(MIN, MAX), 6);
  });

  it("round-trips position <-> value", () => {
    for (const v of [2000, 3200, 5500, 6500, 12000, 50000]) {
      expect(reciprocalValue(reciprocalPos(v, MIN), MIN)).toBeCloseTo(v, 3);
    }
  });

  it("gives equal perceptual (mired) shift for equal thumb travel", () => {
    // Equal position deltas anywhere on the track must map to equal mired deltas.
    const span = reciprocalSpan(MIN, MAX);
    const at = (frac: number) => reciprocalValue(frac * span, MIN);
    const miredOf = (v: number) => 1e6 / v;
    const lowEnd = Math.abs(miredOf(at(0.1)) - miredOf(at(0.2)));
    const midPoint = Math.abs(miredOf(at(0.45)) - miredOf(at(0.55)));
    const highEnd = Math.abs(miredOf(at(0.8)) - miredOf(at(0.9)));
    expect(midPoint).toBeCloseTo(lowEnd, 3);
    expect(highEnd).toBeCloseTo(lowEnd, 3);
  });

  it("moves neutral (5500K) off the 7% linear-kelvin position toward mid-track", () => {
    const frac = reciprocalPos(5500, MIN) / reciprocalSpan(MIN, MAX);
    // Linear kelvin would put 5500K at ~7%; reciprocal lifts it to ~66%.
    expect(frac).toBeGreaterThan(0.6);
    expect(frac).toBeLessThan(0.7);
  });
});

// Temp slider — issue #17: the plain reciprocal scale over 2000–15000 K put
// neutral 5500 K at ~73% of the track (cool travel 318 mired vs warm 115) and the
// warm end couldn't rescue blue-hour frames. The centered piecewise-mired scale
// pins the neutral `def` to the geometric middle, and the range is widened to
// 25000 K (the Kim-locus limit). The gradient grey stop sits at 50% to match.
describe("Temp slider centered scale 2000–25000 K", () => {
  const TEMP_MIN = 2000;
  const TEMP_MAX = 25000;
  const NEUTRAL = 5500;

  it("pins the neutral to the exact middle of the track", () => {
    expect(centeredPos(NEUTRAL, TEMP_MIN, TEMP_MAX, NEUTRAL)).toBeCloseTo(CENTERED_SPAN / 2, 6);
  });

  it("pins min to the left edge and max to the right edge", () => {
    expect(centeredPos(TEMP_MIN, TEMP_MIN, TEMP_MAX, NEUTRAL)).toBeCloseTo(0, 6);
    expect(centeredPos(TEMP_MAX, TEMP_MIN, TEMP_MAX, NEUTRAL)).toBeCloseTo(CENTERED_SPAN, 6);
  });

  it("round-trips position <-> value across both halves", () => {
    for (const v of [2000, 3200, 5500, 6500, 12000, 20000, 25000]) {
      expect(centeredValue(centeredPos(v, TEMP_MIN, TEMP_MAX, NEUTRAL), TEMP_MIN, TEMP_MAX, NEUTRAL))
        .toBeCloseTo(v, 3);
    }
  });

  it("keeps equal thumb travel ≈ equal mired shift within each half", () => {
    const miredOf = (v: number) => 1e6 / v;
    const at = (pos: number) => centeredValue(pos, TEMP_MIN, TEMP_MAX, NEUTRAL);
    // Cool half
    const c1 = Math.abs(miredOf(at(100)) - miredOf(at(200)));
    const c2 = Math.abs(miredOf(at(300)) - miredOf(at(400)));
    expect(c2).toBeCloseTo(c1, 3);
    // Warm half
    const w1 = Math.abs(miredOf(at(600)) - miredOf(at(700)));
    const w2 = Math.abs(miredOf(at(800)) - miredOf(at(900)));
    expect(w2).toBeCloseTo(w1, 3);
  });

  it("is monotone across the center junction", () => {
    let prev = centeredValue(0, TEMP_MIN, TEMP_MAX, NEUTRAL);
    for (let pos = 10; pos <= CENTERED_SPAN; pos += 10) {
      const v = centeredValue(pos, TEMP_MIN, TEMP_MAX, NEUTRAL);
      expect(v).toBeGreaterThan(prev);
      prev = v;
    }
  });
});
