// app/src/lib/roll/printLayout.ts
//
// Pure paper/pagination math for the contact-sheet print layouts — issue #24
// (upstream). Kept free of canvas/store imports so it is unit-testable: the
// exporter (exportSheet.ts) turns these numbers into pixels.

export type PaperSize = "strip" | "a4" | "letter";

/** Physical paper dimensions, portrait, millimetres. */
export const PAPERS: Record<Exclude<PaperSize, "strip">, { wMm: number; hMm: number }> = {
  a4: { wMm: 210, hMm: 297 },
  letter: { wMm: 215.9, hMm: 279.4 }, // 8.5 × 11 in
};

/** Printable-area margin on all four sides (mm equivalent). */
export const PAGE_MARGIN_MM = 12;

// The dialog's resolution scale doubles as print quality on paper sizes:
// scale 1 = 150dpi equivalent, 2 = 300dpi, 4 = 600dpi.
const BASE_DPI = 150;
const MM_PER_IN = 25.4;

/** Pixel dimensions (and px-per-mm) of one page at the given resolution scale. */
export function paperPx(
  paper: Exclude<PaperSize, "strip">,
  scale: number,
): { w: number; h: number; pxPerMm: number } {
  const dpi = BASE_DPI * Math.max(1, scale || 1);
  const pxPerMm = dpi / MM_PER_IN;
  const { wMm, hMm } = PAPERS[paper];
  return { w: Math.round(wMm * pxPerMm), h: Math.round(hMm * pxPerMm), pxPerMm };
}

/** How many strip blocks of height `stripH` fit into `availH` with `gap`
 *  between consecutive blocks. Never less than 1 — a strip taller than the
 *  page still gets its own page rather than paginating forever. */
export function stripsPerPage(availH: number, stripH: number, gap: number): number {
  if (stripH <= 0) return 1;
  return Math.max(1, Math.floor((availH + gap) / (stripH + gap)));
}

/** Split `count` strips into pages, page 1 having its own (usually smaller,
 *  header-bearing) available height. Returns the per-page strip counts;
 *  their sum is always `count`. */
export function paginateStrips(
  count: number,
  stripH: number,
  gap: number,
  page1AvailH: number,
  pageNAvailH: number,
): number[] {
  if (count <= 0) return [];
  const pages: number[] = [Math.min(count, stripsPerPage(page1AvailH, stripH, gap))];
  let remaining = count - pages[0];
  const perPage = stripsPerPage(pageNAvailH, stripH, gap);
  while (remaining > 0) {
    const n = Math.min(remaining, perPage);
    pages.push(n);
    remaining -= n;
  }
  return pages;
}

/** Path for page `page` of `total`: one page saves exactly where the user
 *  chose; multi-page runs suffix -p1, -p2, … before the extension so the
 *  siblings sort next to the base name. */
export function pagePath(basePath: string, page: number, total: number): string {
  if (total <= 1) return basePath;
  const dot = basePath.lastIndexOf(".");
  const sep = Math.max(basePath.lastIndexOf("/"), basePath.lastIndexOf("\\"));
  if (dot > sep) return `${basePath.slice(0, dot)}-p${page}${basePath.slice(dot)}`;
  return `${basePath}-p${page}`; // extension-less path: just append
}
