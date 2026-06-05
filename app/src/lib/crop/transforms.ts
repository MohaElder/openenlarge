import type { Rect } from "./types";

/** Transform a normalized rect when the IMAGE is rotated 90° clockwise. */
export function rotateRectCW(r: Rect): Rect {
  return { x: 1 - r.y - r.h, y: r.x, w: r.h, h: r.w };
}
export function rotateRectCCW(r: Rect): Rect {
  return { x: r.y, y: 1 - r.x - r.w, w: r.h, h: r.w };
}
export function flipRectH(r: Rect): Rect { return { ...r, x: 1 - r.x - r.w }; }
export function flipRectV(r: Rect): Rect { return { ...r, y: 1 - r.y - r.h }; }

/** Oriented pixel dims after `rot90` clockwise quarter-turns. */
export function orientDims(w: number, h: number, rot90: number): [number, number] {
  return rot90 % 2 === 1 ? [h, w] : [w, h];
}
