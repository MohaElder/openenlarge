import type { Rect, Handle } from "./types";

export const MIN = 0.05; // minimum normalized box size

export interface ScreenRect { left: number; top: number; width: number; height: number }

/** Normalized rect → screen px rect within the image's screen rect. */
export function toScreen(r: Rect, img: ScreenRect): ScreenRect {
  return {
    left: img.left + r.x * img.width,
    top: img.top + r.y * img.height,
    width: r.w * img.width,
    height: r.h * img.height,
  };
}

/** Container px point → normalized image point (clamped 0..1). */
export function toNorm(px: number, py: number, img: ScreenRect): [number, number] {
  const nx = (px - img.left) / Math.max(1, img.width);
  const ny = (py - img.top) / Math.max(1, img.height);
  return [Math.max(0, Math.min(1, nx)), Math.max(0, Math.min(1, ny))];
}

/** Which handle (if any) is under a container-px point. `tol` in px. */
export function handleAt(px: number, py: number, box: ScreenRect, tol: number): Handle {
  const l = box.left, t = box.top, rgt = box.left + box.width, b = box.top + box.height;
  const cx = box.left + box.width / 2, cy = box.top + box.height / 2;
  const near = (a: number, bb: number) => Math.abs(a - bb) <= tol;
  const onL = near(px, l), onR = near(px, rgt), onT = near(py, t), onB = near(py, b);
  const midX = near(px, cx), midY = near(py, cy);
  const inX = px >= l - tol && px <= rgt + tol, inY = py >= t - tol && py <= b + tol;
  if (onL && onT) return "nw"; if (onR && onT) return "ne";
  if (onL && onB) return "sw"; if (onR && onB) return "se";
  if (onT && midX && inX) return "n"; if (onB && midX && inX) return "s";
  if (onL && midY && inY) return "w"; if (onR && midY && inY) return "e";
  if (px > l && px < rgt && py > t && py < b) return "move";
  return null;
}

/** Apply a normalized drag delta for a handle. aspect = normalized w/h locks the
 *  ratio; null = freeform. The aspect-locked path anchors the opposite edge/corner
 *  and limits growth by the first image boundary so the ratio is never distorted. */
export function applyDrag(h: Handle, r: Rect, dnx: number, dny: number, aspect: number | null): Rect {
  if (h === "move") return clampRect({ ...r, x: r.x + dnx, y: r.y + dny }, true);

  const east = h === "e" || h === "ne" || h === "se";
  const west = h === "w" || h === "nw" || h === "sw";
  const south = h === "s" || h === "se" || h === "sw";
  const north = h === "n" || h === "ne" || h === "nw";

  // Freeform: edges/corners move independently, then clamp per-axis.
  if (aspect == null) {
    let x = r.x, y = r.y, w = r.w, hh = r.h;
    if (east) w += dnx;
    if (west) { x += dnx; w -= dnx; }
    if (south) hh += dny;
    if (north) { y += dny; hh -= dny; }
    return clampRect({ x, y, w, h: hh });
  }

  // Aspect-locked: keep w/h = aspect.
  const right = r.x + r.w, bottom = r.y + r.h;
  const cx = r.x + r.w / 2, cy = r.y + r.h / 2;

  // Desired width (height = w/aspect). Corners follow the larger-magnitude axis.
  let wDes: number;
  if ((east || west) && (north || south)) {
    const wFromW = r.w + (east ? dnx : -dnx);
    const wFromH = (r.h + (south ? dny : -dny)) * aspect;
    wDes = Math.abs(wFromH - r.w) > Math.abs(wFromW - r.w) ? wFromH : wFromW;
  } else if (east || west) {
    wDes = r.w + (east ? dnx : -dnx);
  } else {
    wDes = (r.h + (south ? dny : -dny)) * aspect;
  }

  // Max width allowed by the anchored side(s), keeping the box within [0,1].
  const wMaxH = east ? 1 - r.x : west ? right : 2 * Math.min(cx, 1 - cx);
  const hMax = south ? 1 - r.y : north ? bottom : 2 * Math.min(cy, 1 - cy);
  const wMaxV = hMax * aspect;
  const wMin = Math.max(MIN, MIN * aspect);

  let w = Math.min(Math.max(wDes, wMin), Math.min(wMaxH, wMaxV));
  if (!(w >= wMin)) w = wMin;
  const hh = w / aspect;

  const x0 = east ? r.x : west ? right - w : cx - w / 2;
  const y0 = south ? r.y : north ? bottom - hh : cy - hh / 2;
  return {
    x: Math.max(0, Math.min(1 - w, x0)),
    y: Math.max(0, Math.min(1 - hh, y0)),
    w, h: hh,
  };
}

/** Clamp into [0,1] with a MIN size. When move=true, preserve size (just shift). */
export function clampRect(r: Rect, move = false): Rect {
  if (move) {
    const w = Math.min(r.w, 1), h = Math.min(r.h, 1);
    return { w, h, x: Math.max(0, Math.min(1 - w, r.x)), y: Math.max(0, Math.min(1 - h, r.y)) };
  }
  let { x, y, w, h } = r;
  if (w < 0) { x += w; w = -w; }
  if (h < 0) { y += h; h = -h; }
  w = Math.max(MIN, Math.min(1, w));
  h = Math.max(MIN, Math.min(1, h));
  x = Math.max(0, Math.min(1 - w, x));
  y = Math.max(0, Math.min(1 - h, y));
  return { x, y, w, h };
}

/** Centered rect of a target aspect (w/h) fitting within [0,1], ~80% of the frame. */
export function conform(r: Rect, aspect: number): Rect {
  const cx = r.x + r.w / 2, cy = r.y + r.h / 2;
  let w = Math.min(r.w, 1), h = w / aspect;
  if (h > 1) { h = 1; w = h * aspect; }
  if (w > 1) { w = 1; h = w / aspect; }
  return clampRect({ x: cx - w / 2, y: cy - h / 2, w, h });
}

/** Default centered 80% box. In normalized space an 80%×80% box already carries
 *  the image's native pixel ratio ("Original"). */
export function default80(): Rect {
  return { x: 0.1, y: 0.1, w: 0.8, h: 0.8 };
}

/** Shrink `rect` about its centre to the largest factor where all four corners,
 *  inverse-rotated by `deg` about the oriented-image centre, stay inside the
 *  image — so a straightened crop never includes the blank wedges. ow/oh are the
 *  oriented pixel dims (rotation must be computed in pixel space). */
export function constrainToRotated(rect: Rect, deg: number, ow: number, oh: number): Rect {
  if (Math.abs(deg) < 1e-4) return rect;
  const rad = (deg * Math.PI) / 180;
  const cos = Math.cos(rad), sin = Math.sin(rad);
  const cx = ow / 2, cy = oh / 2;
  const inside = (s: number): boolean => {
    const rw = rect.w * s, rh = rect.h * s;
    const rx = rect.x + (rect.w - rw) / 2, ry = rect.y + (rect.h - rh) / 2;
    const corners: Array<[number, number]> = [
      [rx, ry], [rx + rw, ry], [rx, ry + rh], [rx + rw, ry + rh],
    ];
    for (const [nx, ny] of corners) {
      const dx = nx * ow - cx, dy = ny * oh - cy;
      const sx = cos * dx + sin * dy + cx;
      const sy = -sin * dx + cos * dy + cy;
      if (sx < 0 || sx > ow || sy < 0 || sy > oh) return false;
    }
    return true;
  };
  if (inside(1)) return rect;
  let lo = 0, hi = 1;
  for (let i = 0; i < 24; i++) { const mid = (lo + hi) / 2; if (inside(mid)) lo = mid; else hi = mid; }
  const s = lo, rw = rect.w * s, rh = rect.h * s;
  return { x: rect.x + (rect.w - rw) / 2, y: rect.y + (rect.h - rh) / 2, w: rw, h: rh };
}
