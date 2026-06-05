import type { PointColorSample } from "../api";

/** Convert an sRGB byte pixel to a fresh Point Color sample (zeroed shifts). */
export function rgbToHslSample(r8: number, g8: number, b8: number): PointColorSample {
  const r = r8 / 255, g = g8 / 255, b = b8 / 255;
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b);
  const l = (mx + mn) / 2;
  let h = 0, s = 0;
  if (mx - mn > 1e-7) {
    const d = mx - mn;
    s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn);
    if (mx === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (mx === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
  }
  return { hue: h, sat: s, lum: l,
    hue_shift: 0, sat_shift: 0, lum_shift: 0, variance: 0, range: 50 };
}

/** Read one pixel from a WebGL2 canvas (created with preserveDrawingBuffer:true).
 *  `cssX`/`cssY` are coordinates relative to the CANVAS element's top-left.
 *  Returns [r,g,b] bytes, or null if out of bounds / no GL context. */
export function readCanvasPixel(canvas: HTMLCanvasElement, cssX: number, cssY: number): [number, number, number] | null {
  if (cssX < 0 || cssY < 0 || cssX > canvas.clientWidth || cssY > canvas.clientHeight) return null;
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true });
  if (!gl) return null;
  const sx = Math.min(canvas.width - 1, Math.max(0, Math.round(cssX * (canvas.width / canvas.clientWidth))));
  const syTop = Math.round(cssY * (canvas.height / canvas.clientHeight));
  const sy = Math.min(canvas.height - 1, Math.max(0, canvas.height - 1 - syTop)); // GL origin is bottom-left
  const px = new Uint8Array(4);
  gl.readPixels(sx, sy, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
  return [px[0], px[1], px[2]];
}
