import { describe, it, expect } from "vitest";
import { compareVersions } from "./version";

describe("compareVersions", () => {
  it("orders newer above older", () => {
    expect(compareVersions("0.1.2", "0.1.1")).toBe(1);
    expect(compareVersions("0.1.1", "0.1.2")).toBe(-1);
  });
  it("treats equal versions as 0", () => {
    expect(compareVersions("0.1.0", "0.1.0")).toBe(0);
  });
  it("strips a leading v", () => {
    expect(compareVersions("v0.1.2", "0.1.2")).toBe(0);
    expect(compareVersions("v0.2.0", "v0.1.9")).toBe(1);
  });
  it("compares numerically, not lexically", () => {
    expect(compareVersions("0.1.10", "0.1.9")).toBe(1);
  });
  it("pads missing trailing segments with 0", () => {
    expect(compareVersions("0.1", "0.1.0")).toBe(0);
    expect(compareVersions("1", "0.9.9")).toBe(1);
  });
});
