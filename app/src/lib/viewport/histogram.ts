export interface Bins { r: number[]; g: number[]; b: number[] }

/** Bin RGBA bytes (from canvas getImageData) into 256 buckets per channel. */
export function binPixels(data: Uint8ClampedArray): Bins {
  const r = new Array(256).fill(0);
  const g = new Array(256).fill(0);
  const b = new Array(256).fill(0);
  for (let i = 0; i < data.length; i += 4) {
    r[data[i]]++; g[data[i + 1]]++; b[data[i + 2]]++;
  }
  return { r, g, b };
}

/** Build an SVG polyline points string for one channel, normalized to height h. */
export function channelPath(bins: number[], w: number, h: number): string {
  const max = Math.max(1, ...bins);
  return bins.map((v, i) => {
    const x = (i / 255) * w;
    const y = h - (v / max) * h;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(" ");
}
