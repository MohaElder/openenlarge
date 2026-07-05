// "Temporary contact sheet" proof layer for the Roll view (upstream #18): a
// non-destructive preview that shows every frame at its OWN solved auto exposure
// and/or auto color, with the roll-wide sliders applying RELATIVE to those auto
// baselines. Nothing here ever writes editsById — the overlay exists only in the
// contact-sheet preview render, and Roll's persist pass skips thumbnail reuse
// while it is active (the regen worker rebakes clean thumbnails from the stored
// edits instead).
import { writable, get } from "svelte/store";
import { api, type ImageEntry, type InvertParams } from "../api";
import { cropById } from "../store";
import { withEffectiveBase } from "../develop/base";
import { applyAsShotWb } from "../develop/wb";
import { imageDir } from "../library/folderScope";

export interface ProofMode {
  on: boolean;
  autoExposure: boolean; // per-frame solved auto exposure replaces the stored one
  autoColor: boolean; // per-frame as-shot WB replaces the stored temp/tint
}

/** The proof toggle + sub-toggles. Module-level (not component state) so the
 * choice survives leaving and re-entering the Roll tab, per the request. */
export const proofMode = writable<ProofMode>({ on: false, autoExposure: true, autoColor: true });

/** True while per-frame solves are running (drives the "solving…" hint). */
export const proofSolving = writable(false);

/** Bumped whenever new solves land, so the Roll preview re-renders. */
export const proofRev = writable(0);

/** One frame's solved auto baseline. Solved lazily, cached for the session so
 * re-opening the proof toggle is instant (the issue's caching requirement). */
interface ProofSolve {
  exposure: number | null;
  temp: number | null;
  tint: number | null;
}

let cacheFolder: string | null = null;
const cache = new Map<string, ProofSolve>();

/** Start a fresh cache when the roll (folder) changes. */
export function proofEnterFolder(folder: string | null): void {
  if (folder === cacheFolder) return;
  cacheFolder = folder;
  cache.clear();
}

/** Drop all solves — call when the metering inputs change under the cache
 * (roll base recalibration, roll crop), since auto exposure/WB are measured
 * against base + crop. */
export function proofInvalidate(): void {
  cache.clear();
  proofRev.update((n) => n + 1);
}

/** Apply the cached proof solves onto a frame's own params (pure; no store
 * writes). Fields without a cached solve pass through unchanged, so frames
 * still solving keep their stored look until their result lands. */
export function proofOverlay(id: string, own: InvertParams, mode: ProofMode): InvertParams {
  if (!mode.on) return own;
  const s = cache.get(id);
  if (!s) return own;
  let p = own;
  if (mode.autoExposure && s.exposure != null) {
    p = { ...p, exposure: s.exposure };
  }
  if (mode.autoColor && s.temp != null && s.tint != null) {
    p = applyAsShotWb(p, { temp: s.temp, tint: s.tint, gains: [1, 1, 1] });
  }
  return p;
}

/** Solve the auto baselines for every frame that has no cache entry yet, then
 * bump proofRev once so the sheet re-renders. Crop/orientation-aware like
 * seedFrame — auto exposure and WB are metered inside each frame's crop. WB is
 * measured first and the exposure solve runs on the balanced params (WB is
 * exposure-dependent; mirrors seedFrame's ordering). Failures leave null fields
 * (frame keeps its stored look for that axis). */
export async function ensureProofSolves(
  frames: ImageEntry[],
  paramsOf: (id: string) => InvertParams,
): Promise<void> {
  const todo = frames.filter((f) => f.developed && !cache.has(f.id));
  if (todo.length === 0) return;
  proofSolving.set(true);
  try {
    for (const img of todo) {
      const dir = imageDir(img);
      const c = get(cropById)[img.id] ?? null;
      const crop = c
        ? ([c.rect.x, c.rect.y, c.rect.w, c.rect.h] as [number, number, number, number])
        : null;
      const geom = c ? { rot90: c.rot90, flip_h: c.flipH, flip_v: c.flipV, angle: c.angle } : {};
      const solve: ProofSolve = { exposure: null, temp: null, tint: null };
      let seed = paramsOf(img.id);
      try {
        const wb = await api.asShotWb(img.id, withEffectiveBase(seed, dir), crop, geom);
        solve.temp = wb.temp;
        solve.tint = wb.tint;
        seed = applyAsShotWb(seed, wb);
      } catch { /* not resident — keep stored WB for this frame */ }
      try {
        const { exposure } = await api.autoBrightness(img.id, withEffectiveBase(seed, dir), crop, geom);
        solve.exposure = exposure;
      } catch { /* not resident — keep stored exposure for this frame */ }
      cache.set(img.id, solve);
    }
  } finally {
    proofSolving.set(false);
    proofRev.update((n) => n + 1);
  }
}
