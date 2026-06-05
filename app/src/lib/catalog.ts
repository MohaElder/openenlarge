import { get } from "svelte/store";
import { api, defaultParams, type InvertParams, type CatalogSnapshot, type ImageEntry } from "./api";
import type { CropRect } from "./crop/types";
import type { DustEdits } from "./develop/dust";
import {
  images, editsById, cropById, dustById, developMode, quality,
  selectedFolder, gridZoom, module as moduleStore, activeId,
} from "./store";

/** A debounced function with a `flush()` that fires any pending call now. */
export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  flush(): void;
}

/** Trailing-edge debounce: coalesce rapid calls; last args win. `flush()` fires now. */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: A | null = null;
  const wrapped = ((...args: A) => {
    pending = args;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const p = pending; pending = null;
      if (p) fn(...p);
    }, ms);
  }) as Debounced<A>;
  wrapped.flush = () => {
    if (timer) { clearTimeout(timer); timer = null; }
    const p = pending; pending = null;
    if (p) fn(...p);
  };
  return wrapped;
}

/** Populate every store from a snapshot. Pure w.r.t. the stores (no IO). */
export function applySnapshot(snap: CatalogSnapshot): void {
  const editsMap: Record<string, InvertParams> = {};
  const cropMap: Record<string, CropRect | null> = {};
  const dustMap: Record<string, DustEdits> = {};
  for (const e of snap.edits) {
    // Backfill any fields absent from older stored blobs (e.g. tone curve /
    // color grading added later) so the frontend always has a complete schema.
    if (e.params) editsMap[e.image_id] = { ...defaultParams(), ...e.params };
    if (e.crop !== undefined) cropMap[e.image_id] = e.crop;
    if (e.dust) dustMap[e.image_id] = e.dust;
  }
  const entries: ImageEntry[] = snap.images.map((ci) => ({
    id: ci.id, path: ci.path, file_name: ci.file_name, thumbnail: ci.thumbnail,
    metadata: ci.metadata, developed: false, has_ir: false, offline: ci.offline,
  }));
  images.set(entries);
  editsById.set(editsMap);
  cropById.set(cropMap);
  dustById.set(dustMap);

  if (snap.prefs.develop_mode === "b" || snap.prefs.develop_mode === "c")
    developMode.set(snap.prefs.develop_mode);
  if (snap.prefs.quality === "performance" || snap.prefs.quality === "quality")
    quality.set(snap.prefs.quality);

  const st = snap.app_state;
  if (st.selected_folder !== undefined)
    selectedFolder.set(st.selected_folder === "" ? null : st.selected_folder);
  if (st.grid_zoom !== undefined) {
    const z = Number(st.grid_zoom);
    if (Number.isFinite(z)) gridZoom.set(z);
  }
  if (st.module === "library" || st.module === "develop") moduleStore.set(st.module);
  if (st.active_id && entries.some((e) => e.id === st.active_id)) activeId.set(st.active_id);
  else if (entries.length) activeId.set(entries[0].id);
}

/** Load the catalog from the backend and populate the stores. Call once on mount. */
export async function hydrate(): Promise<void> {
  try {
    const snap = await api.loadCatalog();
    applySnapshot(snap);
    await api.setQuality(get(quality)).catch(() => {});
  } catch (e) {
    console.error("catalog hydrate failed", e);
  }
}

// --- Write-through (debounced) ---------------------------------------------

// Each image id gets its own debounce instance per edit family, so a save for
// image A is never dropped when image B is edited within the debounce window.
function perIdSaver(
  apiSave: (id: string, json: string) => Promise<void>,
): { save: (id: string, json: string) => void; flushAll: () => void } {
  const timers = new Map<string, Debounced<[string, string]>>();
  const save = (id: string, json: string) => {
    let d = timers.get(id);
    if (!d) {
      d = debounce((i: string, j: string) => { void apiSave(i, j); }, 400);
      timers.set(id, d);
    }
    d(id, json);
  };
  const flushAll = () => timers.forEach((d) => d.flush());
  return { save, flushAll };
}

const edits = perIdSaver(api.saveEdits);
const crop = perIdSaver(api.saveCrop);
const dust = perIdSaver(api.saveDust);
const savePref = debounce((k: string, v: string) => { void api.savePref(k, v); }, 400);
const saveState = debounce((k: string, v: string) => { void api.saveAppState(k, v); }, 400);

/** Persist whichever entries changed (by reference) since the last snapshot. */
function wireRecord<T>(
  store: { subscribe: (cb: (v: Record<string, T>) => void) => () => void },
  save: (id: string, json: string) => void,
): () => void {
  let prev: Record<string, T> = {};
  let first = true;
  return store.subscribe((map) => {
    if (first) { prev = map; first = false; return; } // skip hydration's initial set
    // Deletions are not detected here; removing an image must go through
    // api.deleteImage (the backend command), which deletes its catalog rows.
    for (const id in map) {
      if (map[id] !== prev[id]) save(id, JSON.stringify(map[id]));
    }
    prev = map;
  });
}

let started = false;

/** Wire all stores to debounced write-through. Idempotent. Returns a flush fn. */
export function initPersistence(): () => void {
  if (started) return () => {};
  started = true;

  wireRecord(editsById, edits.save);
  wireRecord(cropById, crop.save);
  wireRecord(dustById, dust.save);

  let first = { dm: true, q: true, sf: true, gz: true, mod: true, aid: true };
  developMode.subscribe((m) => { if (first.dm) { first.dm = false; return; } savePref("develop_mode", m); });
  quality.subscribe((q) => { if (first.q) { first.q = false; return; } savePref("quality", q); });
  selectedFolder.subscribe((p) => { if (first.sf) { first.sf = false; return; } saveState("selected_folder", p ?? ""); });
  gridZoom.subscribe((z) => { if (first.gz) { first.gz = false; return; } saveState("grid_zoom", String(z)); });
  moduleStore.subscribe((m) => { if (first.mod) { first.mod = false; return; } saveState("module", m); });
  activeId.subscribe((a) => { if (first.aid) { first.aid = false; return; } saveState("active_id", a ?? ""); });

  const flush = () => {
    edits.flushAll(); crop.flushAll(); dust.flushAll();
    savePref.flush(); saveState.flush();
  };
  if (typeof window !== "undefined")
    window.addEventListener("beforeunload", flush);
  return flush;
}
