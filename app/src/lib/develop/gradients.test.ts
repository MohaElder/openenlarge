import { describe, it, expect } from "vitest";
import { signed, ev, kelvin } from "./gradients";

describe("formatters", () => {
  it("signed adds + for positives, − stays, 0 is 0", () => {
    expect(signed(24)).toBe("+24");
    expect(signed(-13)).toBe("-13");
    expect(signed(0)).toBe("0");
  });
  it("ev shows two decimals with sign", () => {
    expect(ev(1.3)).toBe("+1.30");
    expect(ev(0)).toBe("0.00");
  });
  it("kelvin rounds to nearest 10", () => {
    expect(kelvin(8437)).toBe("8440");
  });
});
