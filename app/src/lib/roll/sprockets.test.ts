import { describe, it, expect } from "vitest";
import {
  perfLayout, perfTileDataUri,
  KS_PITCH_MM, KS_PERF_W_MM, KS_PERF_H_MM, KS_RADIUS_MM,
  PERFS_PER_FRAME_135, SHORT_EDGE_120_MM,
} from "./sprockets";

// Sheet design constants (mirror exportSheet.ts)
const FRAME_W = 260;
const FRAME_GAP = 7;
const FRAME_PAD = 6;
const FRAME_H = 173; // 260 / (3:2)

describe("perfLayout — 135 (KS-1870)", () => {
  const p = perfLayout("135", FRAME_W, FRAME_H, FRAME_GAP, FRAME_PAD);

  it("pins the pitch to one eighth of the frame period (8 perfs per frame)", () => {
    expect(p.pitch).toBeCloseTo((FRAME_W + FRAME_GAP) / 8, 10);
    expect((FRAME_W + FRAME_GAP) / p.pitch).toBeCloseTo(PERFS_PER_FRAME_135, 10);
  });

  it("preserves the KS-1870 millimetre ratios", () => {
    expect(p.perfW / p.perfH).toBeCloseTo(KS_PERF_W_MM / KS_PERF_H_MM, 6);
    expect(p.perfW / p.pitch).toBeCloseTo(KS_PERF_W_MM / KS_PITCH_MM, 6);
    expect(p.radius / p.perfH).toBeCloseTo(KS_RADIUS_MM / KS_PERF_H_MM, 6);
  });

  it("phases the first hole so 8 whole perfs sit under each frame", () => {
    // Tile starts at framePad, hole centred in the tile ⇒ first centre at
    // frameLeft + pitch/2; the 8th hole's right edge stays inside the period.
    expect(p.offset).toBe(FRAME_PAD);
    const lastCenter = p.pitch / 2 + 7 * p.pitch;
    expect(lastCenter + p.perfW / 2).toBeLessThanOrEqual(FRAME_W + FRAME_GAP);
  });

  it("band grows to fit the perf plus breathing room (~18px at 260px frames)", () => {
    expect(p.bandH).toBeGreaterThanOrEqual(Math.ceil(p.perfH) + 2);
    expect(p.bandH).toBe(18);
  });
});

describe("perfLayout — 120 (short-edge scaled)", () => {
  const p = perfLayout("120", FRAME_W, FRAME_H, FRAME_GAP, FRAME_PAD);

  it("derives px-per-mm from the frame short edge / 56mm", () => {
    const scale = FRAME_H / SHORT_EDGE_120_MM;
    expect(p.pitch).toBeCloseTo(KS_PITCH_MM * scale, 6);
    expect(p.perfW).toBeCloseTo(KS_PERF_W_MM * scale, 6);
    expect(p.perfH).toBeCloseTo(KS_PERF_H_MM * scale, 6);
    expect(p.radius).toBeCloseTo(KS_RADIUS_MM * scale, 6);
  });

  it("is not frame-aligned: pitch does not divide the frame period", () => {
    expect(p.offset).toBe(0);
    const perfsPerPeriod = (FRAME_W + FRAME_GAP) / p.pitch;
    expect(Math.abs(perfsPerPeriod - Math.round(perfsPerPeriod))).toBeGreaterThan(1e-3);
  });

  it("holes come out visibly smaller than 135 at the same frame size", () => {
    const p135 = perfLayout("135", FRAME_W, FRAME_H, FRAME_GAP, FRAME_PAD);
    expect(p.perfH).toBeLessThan(p135.perfH);
    expect(p.bandH).toBeLessThan(p135.bandH);
  });
});

describe("perfTileDataUri", () => {
  const p = perfLayout("135", FRAME_W, FRAME_H, FRAME_GAP, FRAME_PAD);
  const uri = perfTileDataUri(p);

  it("is a percent-encoded SVG data URI safe for unquoted CSS url()", () => {
    expect(uri.startsWith("data:image/svg+xml,")).toBe(true);
    const payload = uri.slice("data:image/svg+xml,".length);
    // No raw characters that would terminate/derail an unquoted CSS url(...)
    expect(payload).not.toMatch(/["'()\s]/);
  });

  it("encodes one rounded-rect hole centred in a pitch-wide tile", () => {
    const svg = decodeURIComponent(uri.slice("data:image/svg+xml,".length));
    expect(svg).toContain(`height="${p.bandH}"`);
    const rx = svg.match(/rx="([\d.]+)"/);
    expect(rx).not.toBeNull();
    expect(Number(rx![1])).toBeCloseTo(p.radius, 2);
    const x = Number(svg.match(/<rect x="([\d.]+)"/)![1]);
    const w = Number(svg.match(/width="([\d.]+)" height="[\d.]+" rx/)![1]);
    expect(x + w / 2).toBeCloseTo(p.pitch / 2, 2); // hole centred in the tile
  });
});
