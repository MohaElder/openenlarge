/** A point normalized to the displayed image ([0,1] in both axes). */
export interface DustPoint { x: number; y: number }
/** A brush stroke: a polyline + radius normalized to the displayed image WIDTH. */
export interface DustStroke { points: DustPoint[]; r: number }
/** IR-driven automatic dust removal settings. */
export interface IrRemoval { enabled: boolean; sensitivity: number }
/** Per-image dust edit state. */
export interface DustEdits { strokes: DustStroke[]; irRemoval: IrRemoval }

export const emptyDust = (): DustEdits => ({ strokes: [], irRemoval: { enabled: false, sensitivity: 50 } });

export function addStroke(d: DustEdits, s: DustStroke): DustEdits {
  return { ...d, strokes: [...d.strokes, s] };
}
export function undoStroke(d: DustEdits): DustEdits {
  return { ...d, strokes: d.strokes.slice(0, -1) };
}
export function resetDust(d: DustEdits): DustEdits {
  return { ...d, strokes: [] };
}
export function setIrEnabled(d: DustEdits, enabled: boolean): DustEdits {
  return { ...d, irRemoval: { ...d.irRemoval, enabled } };
}
export function setIrSensitivity(d: DustEdits, sensitivity: number): DustEdits {
  return { ...d, irRemoval: { ...d.irRemoval, sensitivity } };
}

/** Normalized-to-width radius → on-screen pixels at the current zoom `eff`. */
export function screenRadius(normR: number, imgW: number, eff: number): number {
  return normR * imgW * eff;
}
/** On-screen pixel radius → normalized-to-width radius. */
export function normRadius(screenR: number, imgW: number, eff: number): number {
  return imgW > 0 && eff > 0 ? screenR / (imgW * eff) : 0;
}
