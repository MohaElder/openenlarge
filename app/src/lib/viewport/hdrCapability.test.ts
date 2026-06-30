import { describe, it, expect } from "vitest";
import { detectHdrMode } from "./hdrCapability";

describe("detectHdrMode", () => {
  it("hides on linux regardless of display", () => {
    expect(detectHdrMode({ os: "linux", displayHdr: true, surfaceSupported: true })).toBe("hidden");
    expect(detectHdrMode({ os: "linux", displayHdr: false, surfaceSupported: false })).toBe("hidden");
  });
  it("live-edr on macos/windows with hdr display + surface", () => {
    expect(detectHdrMode({ os: "macos", displayHdr: true, surfaceSupported: true })).toBe("live-edr");
    expect(detectHdrMode({ os: "windows", displayHdr: true, surfaceSupported: true })).toBe("live-edr");
  });
  it("gainmap fallback on macos/windows without hdr display", () => {
    expect(detectHdrMode({ os: "macos", displayHdr: false, surfaceSupported: true })).toBe("gainmap-fallback");
    expect(detectHdrMode({ os: "windows", displayHdr: true, surfaceSupported: false })).toBe("gainmap-fallback");
  });
});
