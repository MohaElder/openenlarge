import { describe, it, expect } from "vitest";
import { orientUVMatrix, orientDims } from "./transforms";
import type { Rect } from "./types";

// Backend ground truth (output UV → source UV).
//
// The backend renders source → output as: orient (flip_h → flip_v → rot90 CW,
// per convert.rs) → straighten (rotate the oriented image in PIXEL space) →
// crop. This file proves the GPU shader's inverse mapping (crop → un-straighten
// → un-orient, see shaders.ts `sourceUV` + Viewport `applyGeometryAndDraw`)
// reproduces it for every orientation, angle, aspect and off-centre crop — the
// class of bugs where a rotated/flipped crop rendered misaligned.

// Inverse of one rot90 CW turn, and the flips, in normalized space.
const invCW = ([x, y]: number[]) => [y, 1 - x];
const flipH = ([x, y]: number[]) => [1 - x, y];
const flipV = ([x, y]: number[]) => [x, 1 - y];
function orientedToSource(p: number[], rot90: number, fH: boolean, fV: boolean): number[] {
  for (let i = 0; i < rot90 % 4; i++) p = invCW(p);
  if (fV) p = flipV(p);
  if (fH) p = flipH(p);
  return p;
}

// Backend: output uv (y-down) → source uv, the WYSIWYG target.
function backend(out: number[], r: Rect, rot90: number, fH: boolean, fV: boolean, deg: number, texW: number, texH: number): number[] {
  const [oW, oH] = orientDims(texW, texH, rot90);
  const rad = (deg * Math.PI) / 180, co = Math.cos(rad), si = Math.sin(rad);
  const px = r.x + out[0] * r.w, py = r.y + out[1] * r.h; // into oriented frame
  const dx = (px - 0.5) * oW, dy = (py - 0.5) * oH;        // un-straighten in pixel space
  const dox = co * dx + si * dy, doy = -si * dx + co * dy; // backend rotate() output→source
  return orientedToSource([dox / oW + 0.5, doy / oH + 0.5], rot90, fH, fV);
}

// GPU shader + Viewport reproduction (must match `backend`).
function shader(out: number[], r: Rect, rot90: number, fH: boolean, fV: boolean, deg: number, texW: number, texH: number): number[] {
  const [oW, oH] = orientDims(texW, texH, rot90);
  const aspect = oH / oW; // Viewport: aspect = oH / oW
  const rad = (deg * Math.PI) / 180, co = Math.cos(rad), si = Math.sin(rad);
  // 1. crop into oriented frame, centred (shaders.ts step 1)
  let cx = r.x + out[0] * r.w - 0.5, cy = r.y + out[1] * r.h - 0.5;
  // 2. un-straighten: GLSL mat2(co, -si/aspect, si*aspect, co) = [[co, si*aspect],[-si/aspect, co]]
  [cx, cy] = [co * cx + si * aspect * cy, (-si / aspect) * cx + co * cy];
  // 3. un-orient: u_orient column-major [a,b,c,d] → matrix [[a,c],[b,d]]
  const [a, b, c, d] = orientUVMatrix(rot90, fH, fV);
  [cx, cy] = [a * cx + c * cy, b * cx + d * cy];
  return [cx + 0.5, cy + 0.5];
}

describe("GPU crop/orient/straighten geometry matches the backend", () => {
  const samples = [[0, 0], [1, 0], [0, 1], [1, 1], [0.3, 0.7], [0.5, 0.5]];
  const crop: Rect = { x: 0.2, y: 0.1, w: 0.6, h: 0.35 }; // off-centre, non-square

  it("reproduces the backend mapping for every orientation, angle and aspect", () => {
    for (const [texW, texH] of [[100, 100], [160, 90], [90, 160]])
      for (const deg of [0, 7, -12])
        for (let rot90 = 0; rot90 < 4; rot90++)
          for (const fH of [false, true])
            for (const fV of [false, true])
              for (const out of samples) {
                const want = backend(out, crop, rot90, fH, fV, deg, texW, texH);
                const got = shader(out, crop, rot90, fH, fV, deg, texW, texH);
                const ctx = `dims=${texW}x${texH} deg=${deg} rot90=${rot90} fH=${fH} fV=${fV} out=${out}`;
                expect(got[0], `x: ${ctx}`).toBeCloseTo(want[0], 6);
                expect(got[1], `y: ${ctx}`).toBeCloseTo(want[1], 6);
              }
  });
});
