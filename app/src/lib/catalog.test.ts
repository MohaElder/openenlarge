import { describe, it, expect, vi } from "vitest";
import { debounce } from "./catalog";

describe("debounce", () => {
  it("coalesces rapid calls into one trailing invocation", async () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const d = debounce(fn, 400);
    d("a"); d("b"); d("c");
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(400);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith("c"); // last args win
    vi.useRealTimers();
  });

  it("flush() invokes the pending call immediately", () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const d = debounce(fn, 400);
    d("x");
    d.flush();
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith("x");
    vi.useRealTimers();
  });
});
