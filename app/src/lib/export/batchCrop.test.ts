import { describe, it, expect } from "vitest";
import { firstSelected, seedDraft, draftToCrop, resolveCrop } from "./batchCrop";
import type { CropRect } from "../crop/types";

const ids = ["a", "b", "c"];

const sampleCrop: CropRect = {
  rect: { x: 0.1, y: 0.2, w: 0.5, h: 0.6 },
  aspect: "4:5", orientation: "portrait",
  rot90: 1, flipH: true, flipV: false, angle: 3,
};

describe("firstSelected", () => {
  it("returns the first selected id in display order", () => {
    expect(firstSelected(ids, new Set(["c", "b"]))).toBe("b");
  });
  it("returns null when nothing is selected", () => {
    expect(firstSelected(ids, new Set())).toBeNull();
  });
});

describe("seedDraft", () => {
  it("copies an existing crop", () => {
    const d = seedDraft(sampleCrop, 100, 50);
    expect(d).toEqual({
      rect: { x: 0.1, y: 0.2, w: 0.5, h: 0.6 },
      aspect: "4:5", orientation: "portrait",
      rot90: 1, flipH: true, flipV: false, angle: 3,
    });
    // rect is a copy, not the same reference
    expect(d.rect).not.toBe(sampleCrop.rect);
  });
  it("defaults to full-frame landscape for a wide image", () => {
    const d = seedDraft(null, 200, 100);
    expect(d.orientation).toBe("landscape");
    expect(d.aspect).toBe("original");
    expect(d.rot90).toBe(0);
    expect(d.rect).toEqual({ x: 0, y: 0, w: 1, h: 1 });
  });
  it("defaults to portrait for a tall image", () => {
    expect(seedDraft(null, 100, 200).orientation).toBe("portrait");
  });
});

describe("resolveCrop", () => {
  const draft = seedDraft(sampleCrop, 100, 50);
  it("uses the batch draft when batch mode is on", () => {
    expect(resolveCrop(true, draft, null)).toEqual(draftToCrop(draft));
  });
  it("uses the image's stored crop when batch mode is off", () => {
    const stored: CropRect = { ...sampleCrop, aspect: "1:1" };
    expect(resolveCrop(false, draft, stored)).toBe(stored);
  });
  it("returns null when off and the image has no crop", () => {
    expect(resolveCrop(false, draft, null)).toBeNull();
  });
});
