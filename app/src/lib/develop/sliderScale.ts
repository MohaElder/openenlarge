/**
 * Reciprocal ("mired") slider scale for colour temperature.
 *
 * Colour-temperature perception is roughly linear in reciprocal kelvin
 * (mired = 1e6 / K), not in kelvin. A slider linear in kelvin over e.g.
 * 2000–50000 K therefore crams the whole warm (low-K) half into a sliver near
 * the minimum and stretches the cool (high-K) half across the rest — blue races,
 * yellow crawls. Mapping the <input> domain to `1e6/min - 1e6/value` makes equal
 * thumb travel produce an equal perceptual white-balance shift, with `min`
 * pinned to the left edge of the track (so warm→cool still reads left→right).
 */

/** Slider position (input value) for a natural value on the reciprocal scale. */
export function reciprocalPos(value: number, min: number): number {
  return 1e6 / min - 1e6 / value;
}

/** Inverse of {@link reciprocalPos}: natural value for a slider position. */
export function reciprocalValue(pos: number, min: number): number {
  return 1e6 / (1e6 / min - pos);
}

/** Input-domain span `[0, span]` covering the natural range `[min, max]`. */
export function reciprocalSpan(min: number, max: number): number {
  return 1e6 / min - 1e6 / max;
}

/**
 * Centered variant: the neutral `center` is pinned to the geometric middle of
 * the track, and EACH HALF is linear in mired. The plain reciprocal scale put
 * neutral 5500 K at ~73% (cool travel 318 mired vs warm 142) — the "zero isn't
 * centered" half of issue #17. Piecewise-mired keeps equal thumb travel ≈ equal
 * perceptual shift within a side while the resting state reads centered.
 * The input domain is a fixed [0, CENTERED_SPAN].
 */
export const CENTERED_SPAN = 1000;

/** Slider position (input value, 0..CENTERED_SPAN) for a natural value. */
export function centeredPos(value: number, min: number, max: number, center: number): number {
  const m = 1e6 / value;
  const mCool = 1e6 / min;   // largest mired (cool/left edge)
  const mWarm = 1e6 / max;   // smallest mired (warm/right edge)
  const mC = 1e6 / center;
  const half = CENTERED_SPAN / 2;
  if (m >= mC) {
    return half * (mCool - m) / (mCool - mC); // cool side: [min..center] → [0..half]
  }
  return half + half * (mC - m) / (mC - mWarm); // warm side: [center..max] → [half..span]
}

/** Inverse of {@link centeredPos}: natural value for a slider position. */
export function centeredValue(pos: number, min: number, max: number, center: number): number {
  const mCool = 1e6 / min;
  const mWarm = 1e6 / max;
  const mC = 1e6 / center;
  const half = CENTERED_SPAN / 2;
  const m = pos <= half
    ? mCool - (pos / half) * (mCool - mC)
    : mC - ((pos - half) / half) * (mC - mWarm);
  return 1e6 / m;
}
