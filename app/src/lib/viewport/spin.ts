export interface SpinRect { left: number; top: number; width: number; height: number }
export interface Spin { dir: number; k: number; rect: SpinRect }

/** Geometry for animating a single 90° turn. `imgW/imgH` are the NEW oriented
 *  dims (the props after the turn). Returns null unless it's a single quarter-turn. */
export function spinGeometry(
  prevRot90: number, rot90: number, imgW: number, imgH: number,
  vpW: number, vpH: number, pad: number,
): Spin | null {
  const d = (((rot90 - prevRot90) % 4) + 4) % 4;
  if (d !== 1 && d !== 3) return null;
  const dir = d === 1 ? 1 : -1;
  const oldImgW = imgH, oldImgH = imgW; // 90° swap
  const avW = Math.max(1, vpW - 2 * pad), avH = Math.max(1, vpH - 2 * pad);
  const oldFit = Math.min(avW / oldImgW, avH / oldImgH);
  const newFit = Math.min(avW / imgW, avH / imgH);
  if (!(oldFit > 0) || !(newFit > 0)) return null;
  const w = oldImgW * oldFit, h = oldImgH * oldFit;
  return { dir, k: newFit / oldFit, rect: { left: (vpW - w) / 2, top: (vpH - h) / 2, width: w, height: h } };
}
