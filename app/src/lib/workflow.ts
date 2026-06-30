import { get } from "svelte/store";
import { images, activeId, module, developProgress, editsById, cropById, dustById, folderImages, invalidatePreview, undevelopableIds, selectFolder } from "./store";
import { api, defaultParams, type ImageEntry, type InvertParams } from "./api";
import type { CropRect } from "./crop/types";
import { dropHistory, reseedActive } from "./develop/historyStore";
import { track } from "./telemetry";
import { showToast } from "./toast";
import { translate } from "./i18n";
import { developedFolderImages } from "./export/eligible";
import { withEffectiveBase, setFolderBase } from "./develop/base";
import { applyAsShotWb } from "./develop/wb";
import { imageDir, pickImportFolder } from "./library/folderScope";
import { gridThumbView, GRID_STATIC_EDGE } from "./library/gridHiRes";

/** Ids of images not yet developed, in order. Pure helper (testable). */
export function undevelopedIds(list: ImageEntry[]): string[] {
  return list.filter((i) => !i.developed).map((i) => i.id);
}

/**
 * Fold a freshly `ensure_developed`'d entry back into the existing one. Takes the
 * refreshed status/metadata from `updated`, but KEEPS the frontend's live thumbnail.
 *
 * `ensure_developed` always returns the develop-time, DEFAULT-params thumbnail (the
 * backend never sees the user's edits — `refreshThumb` re-renders the edited look on
 * the frontend only). Letting it overwrite the live thumbnail made the filmstrip flash
 * the un-adjusted "base" look for ~400ms on every navigation before refreshThumb caught
 * up. The backend thumbnail is only a fallback for when the frontend has none yet.
 */
export function mergeEnsured(existing: ImageEntry, updated: ImageEntry): ImageEntry {
  return { ...updated, thumbnail: existing.thumbnail || updated.thumbnail };
}

/** Extensions we accept on import (file dialog filter + drag-drop). */
export const IMPORT_EXTENSIONS = [
  "jpg", "jpeg", "png", "dng", "tif", "tiff", "raf", "rw2", "nef", "arw", "cr2", "cr3", "3fr", "orf", "raw",
];

/** Final path segment, handling both Windows `\` and POSIX `/` separators. */
function basename(path: string): string {
  const norm = path.replace(/\\/g, "/");
  const slash = norm.lastIndexOf("/");
  return slash >= 0 ? norm.slice(slash + 1) : norm;
}

/**
 * Keep only paths whose extension we can import (case-insensitive), and drop
 * dot-prefixed files. The latter rejects macOS AppleDouble sidecars (`._photo.CR3`)
 * and other hidden/system files: they carry a real extension so they pass the
 * extension test, but aren't decodable — each one otherwise costs a wasted full-file
 * read and a failed-decode retry, which is painfully slow on a mechanical disk.
 */
export function filterImportable(paths: string[]): string[] {
  return paths.filter((p) => {
    const name = basename(p);
    if (name.startsWith(".")) return false;
    const ext = name.split(".").pop()?.toLowerCase();
    return !!ext && IMPORT_EXTENSIONS.includes(ext);
  });
}

/** Extensions cameras emit as embedded previews of a raw/master file. */
const PREVIEW_EXTENSIONS = new Set(["jpg", "jpeg", "png"]);

/** Lowercased "directory/stem" key, so a preview only pairs with a master in the
 *  same folder. Handles Windows separators and multi-dot names. */
function dirStemKey(path: string): string {
  const norm = path.replace(/\\/g, "/");
  const slash = norm.lastIndexOf("/");
  const dir = slash >= 0 ? norm.slice(0, slash) : "";
  const file = slash >= 0 ? norm.slice(slash + 1) : norm;
  const dot = file.lastIndexOf(".");
  const stem = dot > 0 ? file.slice(0, dot) : file;
  return (dir + "/" + stem).toLowerCase();
}

const extOf = (path: string): string => path.split(".").pop()?.toLowerCase() ?? "";

/**
 * Drop jpg/jpeg/png paths that are just camera previews of a raw/master file —
 * i.e. a same-folder, same-base-name non-preview sibling exists in the batch.
 * Standalone jpgs (no raw twin) are kept. Pure + order-preserving.
 */
export function omitPreviewSidecars(paths: string[]): string[] {
  const masters = new Set<string>();
  for (const p of paths) {
    if (!PREVIEW_EXTENSIONS.has(extOf(p))) masters.add(dirStemKey(p));
  }
  return paths.filter((p) => !(PREVIEW_EXTENSIONS.has(extOf(p)) && masters.has(dirStemKey(p))));
}

/** Reduce a raw list of folder file paths to the ones we'll actually import:
 * keep importable extensions, then optionally drop camera preview sidecars.
 * Pure + order-preserving — shared by the folder picker (and testable). */
export function selectImportPaths(paths: string[], omitPreviews: boolean): string[] {
  const importable = filterImportable(paths);
  return omitPreviews ? omitPreviewSidecars(importable) : importable;
}

/** Flatten per-folder file listings into one import list, deduping paths.
 * `list_dir_files` recurses, so selecting both a parent and one of its children
 * surfaces the child's files twice — keep the first occurrence, drop later dupes.
 * Order-preserving + pure (testable without the dialog). */
export function mergeFolderFiles(lists: string[][]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const list of lists) {
    for (const p of list) {
      if (seen.has(p)) continue;
      seen.add(p);
      out.push(p);
    }
  }
  return out;
}

/** How many imports run at once. Each `import_image` does a full-file disk read +
 * decode on a blocking thread, so a serial loop leaves the disk idle between files.
 * A small pool overlaps reads — crucial on a mechanical HDD where per-file seek
 * latency dominates — without thrashing the seek head the way unbounded parallelism
 * would. Kept modest so it stays a net win on spinning disks. */
const IMPORT_CONCURRENCY = 4;

/** Import one path into the catalog, upserting into `images` and making it active
 * if nothing is. Returns the imported entry, or `null` on failure (logged, not
 * thrown, so one bad file doesn't abort the batch). */
async function importOne(path: string): Promise<ImageEntry | null> {
  try {
    const entry = await api.importImage(path);
    images.update((xs) =>
      xs.some((i) => i.id === entry.id)
        ? xs.map((i) => (i.id === entry.id ? entry : i))
        : [...xs, entry]);
    activeId.update((id) => id ?? entry.id);
    if (entry.auto_crop) {
      const a = entry.auto_crop;
      const crop: CropRect = {
        rect: { x: a.x, y: a.y, w: a.w, h: a.h },
        aspect: "original",
        orientation: "landscape",
        rot90: 0,
        flipH: false,
        flipV: false,
        angle: 0,
      };
      cropById.update((m) => ({ ...m, [entry.id]: crop }));
      showToast(translate("toast.frameTrimmed"));
    }
    return entry;
  } catch (e) { console.error("import failed", path, e); return null; }
}

/** Import each path into the catalog with bounded concurrency, upserting into
 * `images` and making the first import active if nothing is. Shared by the file
 * dialog and drag-drop. Store updates are race-free: JS runs them between awaits.
 * `onProgress` (optional) fires after each import completes with the running
 * done/total count — used by the folder picker's live counter.
 *
 * When the batch finishes, jumps the library to the just-imported folder
 * (Lightroom "Last Import"): the user lands on their new frames instead of
 * hunting the tree. See `pickImportFolder` for the multi-folder tie-break. */
export async function importPaths(
  paths: string[],
  onProgress?: (done: number, total: number) => void,
): Promise<void> {
  let next = 0;
  let done = 0;
  const results: (ImageEntry | null)[] = new Array(paths.length).fill(null);
  const worker = async (): Promise<void> => {
    for (let i = next++; i < paths.length; i = next++) {
      results[i] = await importOne(paths[i]);
      onProgress?.(++done, paths.length);
    }
  };
  const lanes = Math.min(IMPORT_CONCURRENCY, paths.length);
  await Promise.all(Array.from({ length: lanes }, worker));

  // Land on the just-imported batch's folder (in input order, so the tie-break
  // is deterministic regardless of which lane finished first).
  const target = pickImportFolder(
    results.flatMap((e) => (e ? [imageDir(e)] : [])),
  );
  if (target !== null) selectFolder(target);
}

/** Resolve after the browser has had a chance to paint (two rAFs). Falls back to a
 * macrotask in non-DOM contexts (tests). */
function nextPaint(): Promise<void> {
  if (typeof requestAnimationFrame === "undefined") return new Promise((r) => setTimeout(r, 0));
  return new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
}

/** Auto-brightness the WHOLE ROLL: every DEVELOPED image in the folder gets its OWN
 * solved exposure (highlight-preserving filmic lift), applied in one atomic store
 * write with the progress overlay. Per-image values — never a single shared
 * brightness — so it must write editsById directly (the Roll look-mirror would
 * otherwise flatten them, since exposure isn't an excluded field). */
export async function autoBrightnessRoll(): Promise<void> {
  const frames = get(developedFolderImages);
  if (frames.length === 0) return;
  developProgress.set({ active: true, done: 0, total: frames.length });
  await nextPaint();
  const solved: Record<string, number> = {};
  for (const img of frames) {
    try {
      const p = withEffectiveBase(get(editsById)[img.id] ?? defaultParams(), imageDir(img));
      const c = get(cropById)[img.id] ?? null;
      const crop = c
        ? ([c.rect.x, c.rect.y, c.rect.w, c.rect.h] as [number, number, number, number])
        : null;
      const geom = c ? { rot90: c.rot90, flip_h: c.flipH, flip_v: c.flipV, angle: c.angle } : {};
      const { exposure } = await api.autoBrightness(img.id, p, crop, geom);
      solved[img.id] = exposure;
    } catch (e) {
      console.error("auto-brightness failed", img.id, e);
    }
    developProgress.update((s) => ({ ...s, done: s.done + 1 }));
  }
  // One atomic write; each frame keeps its OWN exposure.
  editsById.update((m) => {
    const next = { ...m };
    for (const id of Object.keys(solved)) {
      next[id] = { ...(next[id] ?? defaultParams()), exposure: solved[id] };
    }
    return next;
  });
  for (const img of frames) invalidatePreview(img.id);
  reseedActive(); // fold the active image's new exposure into its per-image undo baseline
  await nextPaint();
  developProgress.set({ active: false, done: frames.length, total: frames.length });
  showToast(translate("toast.autoBrightnessDone", { count: Object.keys(solved).length }));
}

/** Seed one developed frame's look against its EFFECTIVE base (per-image override →
 * roll/folder base → backend auto): auto-WB, optional auto-exposure, then re-WB at the
 * final exposure (WB is exposure-dependent — see the 2026-06-21 freeze fix), and bake the
 * catalog thumbnail. `solveExposure=false` keeps the frame's existing exposure (used by the
 * migration / roll recalibrate, where only the base changed). Writes editsById + saves the
 * thumbnail. Overwrites temp/tint, so callers must skip protected frames themselves. */
export async function seedFrame(id: string, img: ImageEntry, solveExposure: boolean): Promise<void> {
  const dir = imageDir(img);
  const prior = get(editsById)[id];
  const seed: InvertParams = prior ? { ...prior } : { ...defaultParams(), positive: img.positive };
  // Seed-time analysis must meter the cropped frame, not the full scan — otherwise
  // the auto-crop's excluded region (sprocket rebate / dark margins) skews exposure
  // and WB, and the frame only looks right after a manual auto-exp tap in Frame
  // (which is crop-aware). Mirror Basic.svelte's manual seed: pass crop + geom.
  const c = get(cropById)[id] ?? null;
  const crop = c ? ([c.rect.x, c.rect.y, c.rect.w, c.rect.h] as [number, number, number, number]) : null;
  const geom = c ? { rot90: c.rot90, flip_h: c.flipH, flip_v: c.flipV, angle: c.angle } : {};
  try {
    const wb = await api.asShotWb(id, withEffectiveBase(seed, dir), crop, geom);
    Object.assign(seed, applyAsShotWb(seed, wb), { wb_manual: false });
  } catch { return; /* not resident — per-image seed retries on activation */ }
  if (solveExposure) {
    try {
      const { exposure } = await api.autoBrightness(id, withEffectiveBase(seed, dir), crop, geom);
      seed.exposure = exposure;
    } catch { /* not resident */ }
    try {
      const wb2 = await api.asShotWb(id, withEffectiveBase(seed, dir), crop, geom);
      Object.assign(seed, applyAsShotWb(seed, wb2), { wb_manual: false });
    } catch { /* not resident */ }
  }
  try {
    const pz = await api.perZoneWb(id, withEffectiveBase(seed, dir), crop, geom);
    seed.pz_sh = pz.sh; seed.pz_mid = pz.mid; seed.pz_hi = pz.hi;
  } catch { /* keep identity pz on failure */ }
  editsById.update((m) => ({ ...m, [id]: seed }));
  try {
    const params = withEffectiveBase(get(editsById)[id] ?? seed, dir);
    const view = gridThumbView(get(cropById)[id], get(dustById)[id], GRID_STATIC_EDGE);
    const url = await api.thumbnail(id, params, view);
    await api.saveThumbnail(id, url);
    images.update((list) => list.map((i) => (i.id === id ? { ...i, thumbnail: url, thumb_stale: false } : i)));
  } catch { /* not resident — re-bakes on first view */ }
}

/** Develop every not-yet-developed image IN THE SELECTED FOLDER sequentially,
 * updating progress, then switch to the target module. Resolves when done. */
export async function developAll(target: "develop" | "roll" = "develop"): Promise<void> {
  const ids = undevelopedIds(get(folderImages));
  if (ids.length === 0) { module.set(target); return; }
  developProgress.set({ active: true, done: 0, total: ids.length });
  // Let the overlay paint (and fade in) before kicking off the first develop call.
  await nextPaint();
  // Collect per-image failures so a stuck "undeveloped" frame (which otherwise just
  // leaves a permanent badge with no feedback) surfaces to the user.
  const failures: string[] = [];
  const nameOf = (id: string) => get(images).find((i) => i.id === id)?.file_name ?? id;

  // Phase 1: develop every frame (decode + sample its base). No seeding yet — the
  // roll base needs every frame's sample first.
  const developed: ImageEntry[] = [];
  for (const id of ids) {
    let ok = false;
    try {
      const updated = await api.developImage(id);
      images.update((list) => list.map((i) => (i.id === id ? updated : i)));
      if (updated.developed) { developed.push(updated); ok = true; }
      else { failures.push(`${nameOf(id)}: develop returned but not marked developed`); console.error("develop returned developed=false", id, nameOf(id)); }
    } catch (e) {
      failures.push(`${nameOf(id)}: ${e}`); console.error("develop failed", id, nameOf(id), e);
    }
    undevelopableIds.update((s) => {
      const has = s.has(id);
      if (ok && has) { const n = new Set(s); n.delete(id); return n; }
      if (!ok && !has) { const n = new Set(s); n.add(id); return n; }
      return s;
    });
    developProgress.update((p) => ({ ...p, done: p.done + 1 }));
  }

  // Phase 1.5: compute + set the roll base BEFORE any WB seed, so WB seeds against it.
  // `withEffectiveBase` (used inside seedFrame) then prefers this folder base automatically.
  if (developed.length > 0) {
    const dir = imageDir(developed[0]);
    try {
      const rb = await api.rollBase(developed.map((i) => i.id));
      if (rb) setFolderBase(dir, rb.base);
    } catch (e) { console.error("rollBase failed", dir, e); }
  }

  // Phase 2: seed each freshly-developed frame against the roll base (only if it has no
  // stored edits yet — re-develop / manual overrides are untouched).
  for (const updated of developed) {
    if (!get(editsById)[updated.id]) await seedFrame(updated.id, updated, true);
  }

  if (failures.length > 0) {
    showToast(translate("toast.developFailed", { count: failures.length, detail: failures[0] }), 8000);
  }
  if (!get(activeId)) {
    const first = get(folderImages)[0];
    if (first) activeId.set(first.id);
  }
  track("images_developed", { count: ids.length });
  module.set(target);
  // Keep the overlay up while the (heavy) Develop view mounts, then fade it out on
  // a free main thread so the dismiss animates instead of snapping.
  await nextPaint();
  developProgress.set({ active: false, done: ids.length, total: ids.length });
}


/**
 * Delete an image: forget it in the backend (optionally trashing the file), then
 * drop it from every per-image store. If it was the active image, select the next
 * neighbour (or the previous one if it was last). Falls back to Library when empty.
 */
export async function deleteImage(id: string, deleteFile: boolean): Promise<void> {
  const list = get(images);
  const idx = list.findIndex((i) => i.id === id);
  if (idx < 0) return;
  try {
    await api.deleteImage(id, deleteFile);
  } catch (e) {
    console.error("delete failed", id, e);
    return; // leave app state untouched if the backend/trash step failed
  }
  const wasActive = get(activeId) === id;
  const neighbour = list[idx + 1] ?? list[idx - 1] ?? null;

  images.update((xs) => xs.filter((i) => i.id !== id));
  const drop = <T,>(m: Record<string, T>): Record<string, T> => {
    const n = { ...m }; delete n[id]; return n;
  };
  editsById.update(drop);
  cropById.update(drop);
  dustById.update(drop);
  dropHistory(id);
  invalidatePreview(id);

  if (wasActive) activeId.set(neighbour ? neighbour.id : null);
  if (get(images).length === 0) module.set("library");
}
