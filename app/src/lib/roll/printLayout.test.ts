import { describe, it, expect } from "vitest";
import { PAPERS, paperPx, stripsPerPage, paginateStrips, pagePath } from "./printLayout";

describe("paperPx", () => {
  it("A4 at scale 1 is the 150dpi pixel size", () => {
    const p = paperPx("a4", 1);
    expect(p.w).toBe(Math.round(210 / 25.4 * 150)); // 1240
    expect(p.h).toBe(Math.round(297 / 25.4 * 150)); // 1754
    expect(p.pxPerMm).toBeCloseTo(150 / 25.4, 9);
  });

  it("US Letter at scale 2 is exactly 8.5x11in at 300dpi", () => {
    const p = paperPx("letter", 2);
    expect(p.w).toBe(2550);
    expect(p.h).toBe(3300);
  });

  it("keeps the paper aspect ratio across scales", () => {
    const a1 = paperPx("a4", 1);
    const a4x = paperPx("a4", 4);
    expect(a4x.w / a4x.h).toBeCloseTo(PAPERS.a4.wMm / PAPERS.a4.hMm, 3);
    expect(a4x.w / a1.w).toBeCloseTo(4, 2);
  });
});

describe("stripsPerPage", () => {
  it("counts blocks including the trailing gap correctly", () => {
    // 3 blocks of 100 + 2 gaps of 10 = 320 exactly
    expect(stripsPerPage(320, 100, 10)).toBe(3);
    expect(stripsPerPage(319, 100, 10)).toBe(2);
  });

  it("never returns less than 1 (an oversized strip still gets a page)", () => {
    expect(stripsPerPage(50, 100, 10)).toBe(1);
    expect(stripsPerPage(0, 100, 10)).toBe(1);
  });
});

describe("paginateStrips", () => {
  it("gives page 1 its own (header-reduced) capacity", () => {
    // page 1 fits 2, later pages fit 3
    expect(paginateStrips(7, 100, 10, 250, 320)).toEqual([2, 3, 2]);
  });

  it("sums to the strip count and is a single page when everything fits", () => {
    expect(paginateStrips(3, 100, 10, 320, 320)).toEqual([3]);
    const pages = paginateStrips(11, 100, 10, 210, 320);
    expect(pages.reduce((a, b) => a + b, 0)).toBe(11);
  });

  it("is empty for an empty roll", () => {
    expect(paginateStrips(0, 100, 10, 320, 320)).toEqual([]);
  });
});

describe("pagePath", () => {
  it("leaves a single-page export at the chosen path", () => {
    expect(pagePath("C:\\out\\contact-sheet.png", 1, 1)).toBe("C:\\out\\contact-sheet.png");
  });

  it("suffixes -pN before the extension for multi-page runs", () => {
    expect(pagePath("/out/contact-sheet.jpg", 2, 3)).toBe("/out/contact-sheet-p2.jpg");
    expect(pagePath("C:\\out\\sheet.png", 1, 2)).toBe("C:\\out\\sheet-p1.png");
  });

  it("does not mistake a dot in a folder name for an extension", () => {
    expect(pagePath("/out.dir/sheet", 2, 2)).toBe("/out.dir/sheet-p2");
  });
});
