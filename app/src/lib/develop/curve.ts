// Monotone cubic (Fritsch–Carlson) interpolation for tone curves. Mirrors
// crates/film-core/src/curve.rs — keep the two numerically identical so the GPU
// LUT and the CPU finish produce matching results.

import type { CurvePoint } from "../api";

export const LUT_SIZE = 256;

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

interface Prepared {
  xs: number[];
  ys: number[];
  m: number[]; // tangents
}

/** Sort, dedupe by x, and compute monotone Hermite tangents. */
function prepare(points: CurvePoint[]): Prepared {
  const sorted = [...points].sort((a, b) => a[0] - b[0]);
  const xs: number[] = [];
  const ys: number[] = [];
  for (const [px, py] of sorted) {
    if (xs.length && Math.abs(px - xs[xs.length - 1]) < 1e-6) {
      ys[ys.length - 1] = py; // duplicate x: last point wins
    } else {
      xs.push(px);
      ys.push(py);
    }
  }
  const n = xs.length;
  if (n === 1) return { xs, ys, m: [0] };

  const d: number[] = []; // secant slopes
  for (let k = 0; k < n - 1; k++) d[k] = (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]);

  const m = new Array<number>(n);
  m[0] = d[0];
  m[n - 1] = d[n - 2];
  for (let k = 1; k < n - 1; k++) m[k] = (d[k - 1] + d[k]) / 2;

  // Fritsch–Carlson monotonicity filter.
  for (let k = 0; k < n - 1; k++) {
    if (d[k] === 0) {
      m[k] = 0;
      m[k + 1] = 0;
    } else {
      const a = m[k] / d[k];
      const b = m[k + 1] / d[k];
      const s = a * a + b * b;
      if (s > 9) {
        const t = 3 / Math.sqrt(s);
        m[k] = t * a * d[k];
        m[k + 1] = t * b * d[k];
      }
    }
  }
  return { xs, ys, m };
}

function evalPrepared(p: Prepared, x: number): number {
  const { xs, ys, m } = p;
  const n = xs.length;
  if (x <= xs[0]) return clamp01(ys[0]);
  if (x >= xs[n - 1]) return clamp01(ys[n - 1]);
  let k = 0;
  while (k < n - 1 && x > xs[k + 1]) k++;
  const h = xs[k + 1] - xs[k];
  const t = (x - xs[k]) / h;
  const t2 = t * t;
  const t3 = t2 * t;
  const h00 = 2 * t3 - 3 * t2 + 1;
  const h10 = t3 - 2 * t2 + t;
  const h01 = -2 * t3 + 3 * t2;
  const h11 = t3 - t2;
  const y = h00 * ys[k] + h10 * h * m[k] + h01 * ys[k + 1] + h11 * h * m[k + 1];
  return clamp01(y);
}

/** Sample the monotone-cubic curve through `points` at x ∈ [0,1] → [0,1]. */
export function sampleCurve(points: CurvePoint[], x: number): number {
  return evalPrepared(prepare(points), x);
}

/** Build a LUT_SIZE-entry table (output 0..1) sampling the curve at i/(N−1). */
export function curveLut(points: CurvePoint[]): Float32Array {
  const p = prepare(points);
  const out = new Float32Array(LUT_SIZE);
  for (let i = 0; i < LUT_SIZE; i++) out[i] = evalPrepared(p, i / (LUT_SIZE - 1));
  return out;
}

/** Linear lookup into a 0..1 LUT at x ∈ [0,1]. */
export function sampleLut(lut: ArrayLike<number>, x: number): number {
  const n = lut.length;
  const f = clamp01(x) * (n - 1);
  const i = Math.floor(f);
  if (i >= n - 1) return lut[n - 1];
  const t = f - i;
  return lut[i] * (1 - t) + lut[i + 1] * t;
}
