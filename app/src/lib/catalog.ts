import { get } from "svelte/store";
import { api, defaultParams, type InvertParams, type CatalogSnapshot, type ImageEntry, type MetaOverride } from "./api";
import type { CropRect } from "./crop/types";
import { emptyDust, type DustEdits } from "./develop/dust";
import {
  images, editsById, cropById, dustById, metaById,
  selectedFolder, gridZoom, module as moduleStore, activeId, folderBaseByPath,
  updateLastCheck, updateSkipVersion, openaiApiKey, omitPreviewJpgs,
  telemetryEnabled, telemetryDecided, debugMode,
  rollFilmEdge, rollEdgeText, rollFilmFormat, undevelopableIds, hotkeyBindings,
  sheetHeaderPhotographer, sheetHeaderCamera, sheetHeaderFilm, sheetHeaderDate,
} from "./store";
import { locale, LOCALES, type Locale } from "./i18n";
import { installDebugHooks } from "./debug";
import { startThumbRegen } from "./develop/thumbRegen";

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
  const metaMap: Record<string, MetaOverride> = {};
  for (const e of snap.edits) {
    // Backfill any fields absent from older stored blobs (e.g. tone curve /
    // color grading added later) so the frontend always has a complete schema.
    if (e.params) editsMap[e.image_id] = { ...defaultParams(), ...e.params };
    if (e.crop !== undefined) cropMap[e.image_id] = e.crop;
    if (e.dust) dustMap[e.image_id] = { ...emptyDust(), ...e.dust };
    if (e.meta) metaMap[e.image_id] = e.meta;
  }
  const entries: ImageEntry[] = snap.images.map((ci) => ({
    id: ci.id, path: ci.path, file_name: ci.file_name, thumbnail: ci.thumbnail,
    metadata: ci.metadata, developed: ci.developed, has_ir: ci.has_ir, offline: ci.offline,
    positive: ci.positive ?? false, thumb_stale: ci.thumb_stale ?? false,
  }));
  images.set(entries);
  editsById.set(editsMap);
  cropById.set(cropMap);
  dustById.set(dustMap);
  metaById.set(metaMap);
  startThumbRegen(); // rebake any engine-version-stale thumbnails in the background

  if (LOCALES.some((l) => l.id === snap.prefs.locale))
    locale.set(snap.prefs.locale as Locale);
  if (typeof snap.prefs.openai_api_key === "string")
    openaiApiKey.set(snap.prefs.openai_api_key);
  // Absent → keep the default (on); only an explicit "false" turns it off.
  if (snap.prefs.omit_preview_jpgs !== undefined)
    omitPreviewJpgs.set(snap.prefs.omit_preview_jpgs !== "false");
  if (snap.prefs.roll_film_edge !== undefined)
    rollFilmEdge.set(snap.prefs.roll_film_edge !== "false");
  if (typeof snap.prefs.roll_edge_text === "string" && snap.prefs.roll_edge_text)
    rollEdgeText.set(snap.prefs.roll_edge_text);
  if (snap.prefs.roll_film_format === "135" || snap.prefs.roll_film_format === "120")
    rollFilmFormat.set(snap.prefs.roll_film_format);
  if (typeof snap.prefs.sheet_header_photographer === "string")
    sheetHeaderPhotographer.set(snap.prefs.sheet_header_photographer);
  if (typeof snap.prefs.sheet_header_camera === "string")
    sheetHeaderCamera.set(snap.prefs.sheet_header_camera);
  if (typeof snap.prefs.sheet_header_film === "string")
    sheetHeaderFilm.set(snap.prefs.sheet_header_film);
  if (typeof snap.prefs.sheet_header_date === "string")
    sheetHeaderDate.set(snap.prefs.sheet_header_date);
  if (typeof snap.prefs.hotkey_bindings === "string" && snap.prefs.hotkey_bindings) {
    try {
      const b = JSON.parse(snap.prefs.hotkey_bindings);
      if (b && typeof b === "object") hotkeyBindings.set(b);
    } catch { /* skip malformed */ }
  }
  // Analytics: "on"/"off" = a recorded choice; absent = undecided (the first-run
  // prompt shows and telemetryEnabled stays false until they answer).
  if (snap.prefs.telemetry === "on") { telemetryEnabled.set(true); telemetryDecided.set(true); }
  else if (snap.prefs.telemetry === "off") telemetryDecided.set(true);

  // Debug logging: install FE hooks immediately when the pref is on so this
  // session is captured from hydrate onward. Backend was already enabled at
  // startup via the same pref.
  if (snap.prefs.debug_mode === "on") {
    debugMode.set(true);
    installDebugHooks();
  }

  const st = snap.app_state;
  if (st.selected_folder !== undefined)
    selectedFolder.set(st.selected_folder === "" ? null : st.selected_folder);
  if (st.grid_zoom !== undefined) {
    const z = Number(st.grid_zoom);
    if (Number.isFinite(z)) gridZoom.set(z);
  }
  if (st.module === "library" || st.module === "roll" || st.module === "develop") moduleStore.set(st.module);
  if (st.active_id && entries.some((e) => e.id === st.active_id)) activeId.set(st.active_id);
  else if (entries.length) activeId.set(entries[0].id);
  if (st.update_skip_version !== undefined) updateSkipVersion.set(st.update_skip_version);
  if (st.update_last_check !== undefined) {
    const ms = Number(st.update_last_check);
    if (Number.isFinite(ms)) updateLastCheck.set(ms);
  }

  const fb: Record<string, [number, number, number]> = {};
  for (const [k, v] of Object.entries(st)) {
    if (k.startsWith("folder_base:")) {
      try {
        const arr = JSON.parse(v);
        if (Array.isArray(arr) && arr.length === 3) fb[k.slice("folder_base:".length)] = arr as [number, number, number];
      } catch { /* skip malformed */ }
    }
  }
  folderBaseByPath.set(fb);

  if (typeof st.undevelopable_ids === "string" && st.undevelopable_ids) {
    try {
      const arr = JSON.parse(st.undevelopable_ids);
      if (Array.isArray(arr)) undevelopableIds.set(new Set(arr.filter((x) => typeof x === "string")));
    } catch { /* skip malformed */ }
  }
}

/** Load the catalog from the backend and populate the stores. Call once on mount. */
export async function hydrate(): Promise<void> {
  try {
    const snap = await api.loadCatalog();
    applySnapshot(snap);
  } catch (e) {
    console.error("catalog hydrate failed", e);
  }
}

// --- Write-through (debounced) ---------------------------------------------

// Each key gets its OWN debounce instance, so a save for one key is never dropped
// when a different key is written within the debounce window. A single shared
// debounce keeps only the last call's args, so two keys changing together (e.g.
// `module` + `active_id` when entering Develop) would clobber each other — only one
// would persist. Used per image-id for edit families and per app_state/pref key.
export function perKeySaver(
  apiSave: (key: string, value: string) => Promise<void>,
): { save: (key: string, value: string) => void; flushAll: () => void } {
  const timers = new Map<string, Debounced<[string, string]>>();
  const save = (key: string, value: string) => {
    let d = timers.get(key);
    if (!d) {
      d = debounce((k: string, v: string) => { void apiSave(k, v); }, 400);
      timers.set(key, d);
    }
    d(key, value);
  };
  const flushAll = () => timers.forEach((d) => d.flush());
  return { save, flushAll };
}

const edits = perKeySaver(api.saveEdits);
const crop = perKeySaver(api.saveCrop);
const dust = perKeySaver(api.saveDust);
const meta = perKeySaver(api.saveMeta);
// Per-key (not a single shared debounce): app_state/pref keys often change in the
// same tick (module + active_id on entering Develop), and a shared debounce would
// keep only the last, silently dropping the others.
const prefs = perKeySaver(api.savePref);
const state = perKeySaver(api.saveAppState);

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
  wireRecord(metaById, meta.save);

  let first = { loc: true, sf: true, gz: true, mod: true, aid: true, usv: true, ulc: true, oak: true, opj: true, rfe: true, ret: true, rff: true, uid: true, hkb: true, shp: true, shc: true, shf: true, shd: true };
  locale.subscribe((l) => { if (first.loc) { first.loc = false; return; } prefs.save("locale", l); });
  openaiApiKey.subscribe((k) => { if (first.oak) { first.oak = false; return; } prefs.save("openai_api_key", k); });
  hotkeyBindings.subscribe((b) => { if (first.hkb) { first.hkb = false; return; } prefs.save("hotkey_bindings", JSON.stringify(b)); });
  omitPreviewJpgs.subscribe((b) => { if (first.opj) { first.opj = false; return; } prefs.save("omit_preview_jpgs", String(b)); });
  rollFilmEdge.subscribe((b) => { if (first.rfe) { first.rfe = false; return; } prefs.save("roll_film_edge", String(b)); });
  rollEdgeText.subscribe((v) => { if (first.ret) { first.ret = false; return; } prefs.save("roll_edge_text", v); });
  rollFilmFormat.subscribe((v) => { if (first.rff) { first.rff = false; return; } prefs.save("roll_film_format", v); });
  sheetHeaderPhotographer.subscribe((v) => { if (first.shp) { first.shp = false; return; } prefs.save("sheet_header_photographer", v); });
  sheetHeaderCamera.subscribe((v) => { if (first.shc) { first.shc = false; return; } prefs.save("sheet_header_camera", v); });
  sheetHeaderFilm.subscribe((v) => { if (first.shf) { first.shf = false; return; } prefs.save("sheet_header_film", v); });
  sheetHeaderDate.subscribe((v) => { if (first.shd) { first.shd = false; return; } prefs.save("sheet_header_date", v); });
  selectedFolder.subscribe((p) => { if (first.sf) { first.sf = false; return; } state.save("selected_folder", p ?? ""); });
  gridZoom.subscribe((z) => { if (first.gz) { first.gz = false; return; } state.save("grid_zoom", String(z)); });
  updateSkipVersion.subscribe((v) => { if (first.usv) { first.usv = false; return; } state.save("update_skip_version", v); });
  updateLastCheck.subscribe((v) => { if (first.ulc) { first.ulc = false; return; } state.save("update_last_check", String(v)); });
  moduleStore.subscribe((m) => { if (first.mod) { first.mod = false; return; } state.save("module", m); });
  activeId.subscribe((a) => { if (first.aid) { first.aid = false; return; } state.save("active_id", a ?? ""); });
  undevelopableIds.subscribe((s) => { if (first.uid) { first.uid = false; return; } state.save("undevelopable_ids", JSON.stringify([...s])); });

  const flush = () => {
    edits.flushAll(); crop.flushAll(); dust.flushAll(); meta.flushAll();
    prefs.flushAll(); state.flushAll();
  };
  if (typeof window !== "undefined")
    window.addEventListener("beforeunload", flush);
  return flush;
}
