export interface AspectPreset { id: string; label: string; ratio: number | null } // w/h; null = original

export const PRESETS: AspectPreset[] = [
  { id: "original", label: "crop.aspect.original", ratio: null },
  { id: "1:1", label: "crop.aspect.1x1", ratio: 1 },
  { id: "4:5", label: "crop.aspect.4x5", ratio: 4 / 5 },
  { id: "8.5:11", label: "crop.aspect.8_5x11", ratio: 8.5 / 11 },
  { id: "5:7", label: "crop.aspect.5x7", ratio: 5 / 7 },
  { id: "2:3", label: "crop.aspect.2x3", ratio: 2 / 3 },
  { id: "4:4", label: "crop.aspect.4x4", ratio: 1 },
  { id: "16:9", label: "crop.aspect.16x9", ratio: 16 / 9 },
  { id: "16:10", label: "crop.aspect.16x10", ratio: 16 / 10 },
];

/** Effective target ratio (w/h) for a preset under an orientation.
 *  landscape → ≥1, portrait → ≤1. "original"/"custom" use the native ratio. */
export function effectiveRatio(
  id: string, nativeRatio: number, orientation: "landscape" | "portrait",
): number {
  const p = PRESETS.find((x) => x.id === id);
  const base = p && p.ratio != null ? p.ratio : nativeRatio;
  return orientation === "landscape" ? Math.max(base, 1 / base) : Math.min(base, 1 / base);
}

/** Normalized aspect (w_norm/h_norm) for a preset, so the ON-SCREEN box has the
 *  intended pixel ratio. screenRatio = (w_norm/h_norm) × nativeRatio, hence we
 *  divide the pixel ratio by nativeRatio. */
export function presetNormAspect(
  id: string, nativeRatio: number, orientation: "landscape" | "portrait",
): number {
  return effectiveRatio(id, nativeRatio, orientation) / nativeRatio;
}

export function labelFor(id: string): string {
  if (id === "custom") return "Custom";
  return PRESETS.find((p) => p.id === id)?.label ?? "Custom";
}
