import { describe, it, expect } from "vitest";
import { shouldAutoCheck, shouldPrompt, DAY_MS } from "./policy";

describe("shouldAutoCheck", () => {
  it("allows the first check (lastCheck 0)", () => {
    expect(shouldAutoCheck(DAY_MS, 0)).toBe(true);
  });
  it("blocks within the interval", () => {
    expect(shouldAutoCheck(DAY_MS - 1, 0)).toBe(false);
    expect(shouldAutoCheck(1000, 1000)).toBe(false);
  });
  it("allows exactly at the interval boundary", () => {
    expect(shouldAutoCheck(2 * DAY_MS, DAY_MS)).toBe(true);
  });
});

describe("shouldPrompt", () => {
  it("prompts for a newer, non-skipped version", () => {
    expect(shouldPrompt("0.1.2", "0.1.1", "")).toBe(true);
  });
  it("suppresses the exact skipped version", () => {
    expect(shouldPrompt("0.1.2", "0.1.1", "0.1.2")).toBe(false);
  });
  it("still prompts for a version newer than the skipped one", () => {
    expect(shouldPrompt("0.1.3", "0.1.1", "0.1.2")).toBe(true);
  });
  it("does not prompt when not newer than current", () => {
    expect(shouldPrompt("0.1.1", "0.1.1", "")).toBe(false);
  });
});
