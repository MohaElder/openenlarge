// View-independent thumbnail-regeneration worker. `thumb_stale` is the single
// "needs rebake" flag (engine-version bump OR look change); edits mark frames stale
// and this worker bakes them in the background, regardless of which tab is mounted.
// Replaces Grid.svelte's local regenStale/sweepStale and copySettings' regenAppliedThumbs.

import { get } from "svelte/store";
import { images, editsById, cropById, dustById } from "../store";
import { api, defaultParams, type ImageEntry } from "../api";
import { withEffectiveBase } from "./base";
import { applyAsShotWb } from "./wb";
import { imageDir } from "../library/folderScope";
import { gridThumbView, GRID_STATIC_EDGE } from "../library/gridHiRes";

const POOL = 3;
const inFlight = new Set<string>();
let pumping = false;

/** Rebake one developed frame's catalog thumbnail and clear its thumb_stale flag.
 *  Reuses saved edits if present, else the develop-time auto-WB seed (matching the
 *  look the frame opens with). saveThumbnail stamps the current engine version.
 *  Returns true on successful rebake, false on skip (undeveloped/offline/in-flight)
 *  or caught failure (so pump() can avoid re-queueing the same frame in one drain). */
export async function regenOne(img: ImageEntry): Promise<boolean> {
  if (!img.developed || img.offline || inFlight.has(img.id)) return false;
  inFlight.add(img.id);
  try {
    // Ensure the decoded working buffer is resident (rehydrates from the .oecache
    // sidecar if it was evicted). Cheap when already cached.
    await api.ensureDeveloped(img.id);
    const dir = imageDir(img);
    const saved = get(editsById)[img.id];
    let params;
    if (saved) {
      params = withEffectiveBase(saved, dir);
    } else {
      const seed = withEffectiveBase({ ...defaultParams(), positive: img.positive }, dir);
      // Auto-WB is crop-dependent: meter the frame's stored crop (like seedFrame /
      // Basic.svelte's seed) so the baked thumbnail matches what Develop will show.
      const c = get(cropById)[img.id] ?? null;
      const crop = c
        ? ([c.rect.x, c.rect.y, c.rect.w, c.rect.h] as [number, number, number, number])
        : null;
      const geom = c
        ? { rot90: c.rot90, flip_h: c.flipH, flip_v: c.flipV, angle: c.angle }
        : { rot90: 0, flip_h: false, flip_v: false, angle: 0 };
      const wb = await api.asShotWb(img.id, seed, crop, geom);
      params = applyAsShotWb(seed, wb);
      try {
        const pz = await api.perZoneWb(img.id, params, crop, geom);
        params = { ...params, pz_sh: pz.sh, pz_mid: pz.mid, pz_hi: pz.hi };
      } catch { /* keep identity pz on failure */ }
    }
    const view = gridThumbView(get(cropById)[img.id], get(dustById)[img.id], GRID_STATIC_EDGE);
    const url = await api.thumbnail(img.id, params, view);
    await api.saveThumbnail(img.id, url);
    images.update((list) =>
      list.map((i) => (i.id === img.id ? { ...i, thumbnail: url, thumb_stale: false } : i)));
    return true;
  } catch (e) {
    // Leave thumb_stale set so the frame is retried on the next pump / next session.
    console.warn(`thumbRegen: regenOne failed for ${img.id}`, e);
    return false;
  } finally {
    inFlight.delete(img.id);
  }
}

/** Drain every developed+stale frame with bounded concurrency. Singleton: a second
 *  call while running is a no-op; marks added mid-drain are picked up by the re-read.
 *  A per-drain `claimed` set (shared across all workers) ensures each frame is attempted
 *  at most once per pump() invocation, preventing hot-loops on a persistently-failing
 *  frame. Claimed synchronously before the first await so no two workers race for the
 *  same frame. A fresh pump() (from a later markThumbsStale/startThumbRegen) starts with
 *  an empty claimed, so frames that failed are retried on the next drain. */
export function pump(): void {
  if (pumping) return;
  pumping = true;
  const claimed = new Set<string>(); // per-drain: claim before any await to avoid inter-worker races
  const worker = async () => {
    for (;;) {
      const next = get(images).find((i) => i.developed && i.thumb_stale && !inFlight.has(i.id) && !claimed.has(i.id));
      if (!next) return;
      claimed.add(next.id); // synchronous: blocks other workers from picking the same frame
      await regenOne(next);
    }
  };
  void Promise.all(Array.from({ length: POOL }, worker)).finally(() => { pumping = false; });
}

/** Mark frames as needing a rebake (look change) and kick the worker. When persist,
 *  also invalidate the DB thumb_version so the mark survives a crash mid-regen.
 *  Only developed frames are marked and persisted; undeveloped ids are silently ignored. */
export function markThumbsStale(ids: string[], opts: { persist?: boolean } = {}): void {
  if (!ids.length) return;
  const developedSet = new Set(get(images).filter((i) => i.developed).map((i) => i.id));
  const filteredIds = ids.filter((id) => developedSet.has(id));
  if (filteredIds.length) {
    const toMark = new Set(filteredIds);
    images.update((list) =>
      list.map((i) => (toMark.has(i.id) ? { ...i, thumb_stale: true } : i)));
    if (opts.persist) api.invalidateThumbnails(filteredIds).catch(() => { /* best-effort durability */ });
  }
  pump();
}

let swept = false;
/** Once-per-session initial sweep, kicked after the catalog loads — rebakes every
 *  developed+stale thumbnail (e.g. after an ENGINE_VERSION bump). */
export function startThumbRegen(): void {
  if (swept) return;
  swept = true;
  pump();
}
