import { defaultParams, type InvertParams } from "../api";
import type { CropRect } from "../crop/types";

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));

/** The selectable setting groups, mirroring the per-group apply helpers below.
 *  Shared by the "apply settings" picker dialog and both of its entry points
 *  (clipboard paste + Develop's "apply to whole roll"). */
export type SettingGroup = "toneColor" | "crop" | "base" | "exposure";
export type GroupSelection = Record<SettingGroup, boolean>;

/** Render/iteration order for the picker. */
export const ALL_GROUPS: readonly SettingGroup[] = [
  "toneColor", "crop", "base", "exposure",
];

/** A self-contained source of settings to apply (the active frame, or a clipboard
 *  snapshot). `params` carries every group's fields; `crop` lives in its own store. */
export interface SettingsSnapshot {
  params: InvertParams;
  crop: CropRect | null;
  /** The source's Temp as an offset from its own as-shot neutral (Kelvin). Temp is
   *  shown relative to each image's as-shot baseline, so to make a copied "+1000"
   *  land "+1000" on a target (not shift by the baseline difference) we re-base it:
   *  target.temp = target_as_shot + tempOffset. Undefined → apply temp absolutely. */
  tempOffset?: number;
  /** Which setting groups the user chose to carry, recorded at COPY time so paste
   *  applies them directly (no second dialog). Unset → fall back to the default. */
  groups?: GroupSelection;
}

/** Per-image film/calibration fields that are NOT part of the shared "look".
 *  `exposure` is here because Auto-Brightness solves a DISTINCT value per frame;
 *  a deliberate roll-wide exposure is re-applied separately (null-guarded) so it
 *  can't silently flatten those per-image values.
 *  `wb_baseline`/`pz_sh`/`pz_mid`/`pz_hi` are per-image measured gains (seeded
 *  from each frame's as-shot WB analysis) and must not leak to other frames.
 *  (These fields land in InvertParams with the per-zone WB feature; the cast
 *  below keeps this file forward-compatible before that merge.) */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const EXCLUDE = new Set<keyof InvertParams>([
  "mode", "stock", "base_override", "d_max_override", "hdr", "positive", "exposure",
  ...["wb_baseline", "pz_sh", "pz_mid", "pz_hi"] as any[],
]);

/** The tone/color subset of a params object (everything except EXCLUDE). */
export function toneColorOf(p: InvertParams): Partial<InvertParams> {
  const out: Record<string, unknown> = {};
  for (const k of Object.keys(p) as (keyof InvertParams)[]) {
    if (!EXCLUDE.has(k)) out[k] = p[k];
  }
  return out as Partial<InvertParams>;
}

/** True when the tone/color subset differs from a fresh default. */
export function hasToneColorEdits(p: InvertParams): boolean {
  return JSON.stringify(toneColorOf(p)) !== JSON.stringify(toneColorOf(defaultParams()));
}

const entry = (edits: Record<string, InvertParams>, id: string): InvertParams =>
  edits[id] ?? defaultParams();

export function framesWithToneColor(edits: Record<string, InvertParams>, ids: string[]): string[] {
  return ids.filter((id) => edits[id] && hasToneColorEdits(edits[id]));
}

export function applyToneColorToAll(
  edits: Record<string, InvertParams>, ids: string[], src: InvertParams,
): Record<string, InvertParams> {
  const next = { ...edits };
  const tc = clone(toneColorOf(src));
  for (const id of ids) next[id] = { ...entry(edits, id), ...clone(tc) };
  return next;
}

export function framesWithCrop(crops: Record<string, CropRect | null>, ids: string[]): string[] {
  return ids.filter((id) => crops[id] != null);
}

export function applyCropToAll(
  crops: Record<string, CropRect | null>, ids: string[], crop: CropRect | null,
): Record<string, CropRect | null> {
  const next = { ...crops };
  for (const id of ids) next[id] = crop ? clone(crop) : null;
  return next;
}

export function framesWithBase(edits: Record<string, InvertParams>, ids: string[]): string[] {
  return ids.filter((id) => edits[id]?.base_override != null);
}

export function applyBaseToAll(
  edits: Record<string, InvertParams>, ids: string[], base: [number, number, number] | null,
): Record<string, InvertParams> {
  const next = { ...edits };
  for (const id of ids) next[id] = { ...entry(edits, id), base_override: base ? [...base] : null };
  return next;
}

/** Apply ONE shared exposure to every frame — used only when the user deliberately
 *  sets a roll-wide exposure (else each frame keeps its own, e.g. Auto-Brightness). */
export function applyExposureToAll(
  edits: Record<string, InvertParams>, ids: string[], exposure: number,
): Record<string, InvertParams> {
  const next = { ...edits };
  for (const id of ids) next[id] = { ...entry(edits, id), exposure };
  return next;
}

export function framesWithWhitePoint(edits: Record<string, InvertParams>, ids: string[]): string[] {
  return ids.filter((id) => edits[id]?.d_max_override != null);
}

export function applyWhitePointToAll(
  edits: Record<string, InvertParams>, ids: string[], dmax: number | null,
): Record<string, InvertParams> {
  const next = { ...edits };
  for (const id of ids) next[id] = { ...entry(edits, id), d_max_override: dmax };
  return next;
}

// --- Relative roll adjustment (scalar sliders apply as an offset, preserving per-frame diffs) ---

/** Roll-panel scalar sliders applied RELATIVELY: the roll value is an offset added to each
 *  frame's own value (so per-frame differences survive), clamped to the slider range. `neutral`
 *  is the no-op default; ranges mirror the Frame sliders. The structured look (tone curve, color
 *  grade, …) and base/d_max/crop stay broadcast-absolute. */
export const ROLL_RELATIVE: { key: keyof InvertParams; neutral: number; min: number; max: number }[] = [
  { key: "temp", neutral: 5500, min: 2000, max: 25000 },
  { key: "tint", neutral: 0, min: -100, max: 100 },
  { key: "exposure", neutral: 0, min: -5, max: 5 },
  { key: "contrast", neutral: 0, min: -100, max: 100 },
  { key: "highlights", neutral: 0, min: -100, max: 100 },
  { key: "shadows", neutral: 0, min: -100, max: 100 },
  { key: "whites", neutral: 0, min: -100, max: 100 },
  { key: "blacks", neutral: 0, min: -100, max: 100 },
  { key: "vibrance", neutral: 0, min: -100, max: 100 },
  { key: "saturation", neutral: 0, min: -100, max: 100 },
];
const RELATIVE_KEYS = new Set<string>(ROLL_RELATIVE.map((r) => r.key as string));
const clampN = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

/** Apply one relative field's roll offset to a frame's own value. Temp offsets in MIRED
 *  (1e6/K) — linear in colour shift, so the same roll nudge looks the same on a 5000 K and an
 *  8000 K frame; a raw Kelvin offset would not (that was the old asShotTemp bug). Other fields
 *  add linearly. `applied` is the offset already folded in (incremental delta). */
function relField(key: string, cur: number, draft: number, applied: number, min: number, max: number): number {
  if (key === "temp") {
    const dM = 1e6 / draft - 1e6 / applied;        // roll offset, in mired
    return clampN(1e6 / (1e6 / cur + dM), min, max);
  }
  return clampN(cur + (draft - applied), min, max);
}

/** The structured/absolute look broadcast to every frame (tone curve, color grade, mixer, …):
 *  the tone/color subset MINUS the relative scalars. */
export function rollAbsoluteLook(p: InvertParams): Partial<InvertParams> {
  const tc = toneColorOf(p) as Record<string, unknown>;
  for (const k of RELATIVE_KEYS) delete tc[k];
  return tc as Partial<InvertParams>;
}

/** Persist pass: fold the roll draft into every frame. Each relative scalar applies the DELTA
 *  since `applied` (the offset already baked in) on top of each frame's current value; the
 *  structured look is broadcast absolutely. The caller advances `applied` to `draft` after. */
export function applyRollDelta(
  edits: Record<string, InvertParams>, ids: string[], draft: InvertParams, applied: InvertParams,
): Record<string, InvertParams> {
  const next = { ...edits };
  const abs = clone(rollAbsoluteLook(draft));
  for (const id of ids) {
    const cur = entry(edits, id);
    const merged = { ...cur, ...clone(abs) } as Record<string, unknown>;
    for (const r of ROLL_RELATIVE) {
      merged[r.key as string] = relField(r.key as string, cur[r.key] as number, draft[r.key] as number, applied[r.key] as number, r.min, r.max);
    }
    next[id] = merged as unknown as InvertParams;
  }
  return next;
}

/** Live-preview for ONE frame: same math as applyRollDelta, against the frame's own params. */
export function rollPreviewFrame(frame: InvertParams, draft: InvertParams, applied: InvertParams): InvertParams {
  const merged = { ...frame, ...rollAbsoluteLook(draft) } as Record<string, unknown>;
  for (const r of ROLL_RELATIVE) {
    merged[r.key as string] = relField(r.key as string, frame[r.key] as number, draft[r.key] as number, applied[r.key] as number, r.min, r.max);
  }
  return merged as unknown as InvertParams;
}
