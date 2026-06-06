/** Compare dot-separated numeric versions. Returns -1, 0, or 1.
 * Strips a leading "v"; missing trailing segments count as 0 (0.1 === 0.1.0);
 * segments compare numerically (0.1.10 > 0.1.9). */
export function compareVersions(a: string, b: string): -1 | 0 | 1 {
  const pa = parse(a);
  const pb = parse(b);
  const n = Math.max(pa.length, pb.length);
  for (let i = 0; i < n; i++) {
    const da = pa[i] ?? 0;
    const db = pb[i] ?? 0;
    if (da < db) return -1;
    if (da > db) return 1;
  }
  return 0;
}

function parse(v: string): number[] {
  return v.replace(/^v/i, "").split(".").map((s) => parseInt(s, 10) || 0);
}
