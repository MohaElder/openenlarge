import { describe, it, expect } from "vitest";
import { binPixels, channelPath } from "./histogram";

describe("binPixels", () => {
  it("counts each channel value into its bucket", () => {
    // two pixels: (255,0,0) and (255,128,0)
    const data = new Uint8ClampedArray([255, 0, 0, 255, 255, 128, 0, 255]);
    const bins = binPixels(data);
    expect(bins.r[255]).toBe(2);
    expect(bins.g[0]).toBe(1);
    expect(bins.g[128]).toBe(1);
    expect(bins.b[0]).toBe(2);
  });
});

describe("channelPath", () => {
  it("maps the peak bucket to y=0 (top)", () => {
    const bins = new Array(256).fill(0);
    bins[0] = 10;
    const pts = channelPath(bins, 256, 80);
    expect(pts.startsWith("0.0,0.0")).toBe(true);
  });
});
