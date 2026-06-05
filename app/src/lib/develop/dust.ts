/** A point normalized to the displayed image ([0,1] in both axes). */
export interface DustPoint { x: number; y: number }
/** A brush stroke: a polyline + radius normalized to the displayed image WIDTH. */
export interface DustStroke { points: DustPoint[]; r: number }
/** Per-image dust edit state. */
export interface DustEdits { strokes: DustStroke[] }

export const emptyDust = (): DustEdits => ({ strokes: [] });

export function addStroke(d: DustEdits, s: DustStroke): DustEdits {
  return { strokes: [...d.strokes, s] };
}
export function undoStroke(d: DustEdits): DustEdits {
  return { strokes: d.strokes.slice(0, -1) };
}
export function resetDust(): DustEdits {
  return { strokes: [] };
}

/** Normalized-to-width radius → on-screen pixels at the current zoom `eff`. */
export function screenRadius(normR: number, imgW: number, eff: number): number {
  return normR * imgW * eff;
}
/** On-screen pixel radius → normalized-to-width radius. */
export function normRadius(screenR: number, imgW: number, eff: number): number {
  return imgW > 0 && eff > 0 ? screenR / (imgW * eff) : 0;
}
