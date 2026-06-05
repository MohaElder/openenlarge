import { describe, it, expect, vi } from "vitest";
import { debounce, applySnapshot } from "./catalog";
import { get } from "svelte/store";
import {
  images, editsById, cropById, dustById, metaById, quality,
  selectedFolder, gridZoom, module as moduleStore, activeId,
} from "./store";
import type { CatalogSnapshot } from "./api";
import { defaultParams } from "./api";

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

describe("applySnapshot", () => {
  it("populates every store from a catalog snapshot", () => {
    const snap: CatalogSnapshot = {
      images: [{
        id: "a", path: "/x/a.dng", file_name: "a.dng", thumbnail: "t",
        metadata: { width: 100, height: 100, file_size: 0 }, offline: false,
        developed: true, has_ir: false,
      }],
      edits: [{
        image_id: "a",
        params: { ...defaultParams(), exposure: 1.5 },
        crop: null,
        dust: { strokes: [], irRemoval: { enabled: false, sensitivity: 50 } },
        meta: { camera: "Leica M6", note: "roll 12" },
      }],
      prefs: { quality: "quality" },
      app_state: { selected_folder: "/x", grid_zoom: "70", module: "develop", active_id: "a" },
    };
    applySnapshot(snap);
    expect(get(images).length).toBe(1);
    expect(get(images)[0].developed).toBe(true);
    expect(get(editsById)["a"].exposure).toBe(1.5);
    expect(get(dustById)["a"].irRemoval.sensitivity).toBe(50);
    expect(get(metaById)["a"].camera).toBe("Leica M6");
    expect(get(quality)).toBe("quality");
    expect(get(selectedFolder)).toBe("/x");
    expect(get(gridZoom)).toBe(70);
    expect(get(moduleStore)).toBe("develop");
    expect(get(activeId)).toBe("a");
  });
});
