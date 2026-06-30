import { writable, derived, get } from "svelte/store";
import type { ImageEntry, MetaOverride, InvertParams } from "./api";
import { defaultParams } from "./api";
import type { CropRect } from "./crop/types";
import { createPerImageParams } from "./perImage";
import { emptyDust, type DustEdits } from "./develop/dust";
import { scopeToFolder } from "./library/folderScope";
import { type SelState, type Mods, noneSelected, allSelected, click } from "./selection";
import type { SettingsSnapshot } from "./roll/apply";
import { detectHdrMode, probeHdrEnv, type HdrMode } from "./viewport/hdrCapability";

export const images = writable<ImageEntry[]>([]);
export const activeId = writable<string | null>(null);
export const module = writable<"library" | "roll" | "develop">("library");

// Per-image edits: $params is the ACTIVE image's params; writes go to the active
// image only. Every image uses the density inversion (Mode B); the per-channel
// mode was removed as a confusing, lower-quality alternative.
const _perImage = createPerImageParams(activeId, () => defaultParams());
export const params = _perImage.params;
export const editsById = _perImage.editsById;

/** Bumped whenever an image's resident working buffer is re-developed/upgraded,
 *  so the Viewport busts its GPU/CPU render caches and re-fetches the buffer. */
export const developRev = writable(0);

/** Per-image committed crop (null = full image). */
export const cropById = writable<Record<string, CropRect | null>>({});
/** The active image's committed crop. */
export const activeCrop = derived([cropById, activeId], ([m, id]) => (id ? m[id] ?? null : null));

/** Per-image dust edits (eraser strokes). */
export const dustById = writable<Record<string, DustEdits>>({});
/** The active image's dust edits. */
export const activeDust = derived([dustById, activeId], ([m, id]) =>
  id ? m[id] ?? emptyDust() : emptyDust());

/** Active image's global auto-dust spot centroids (normalized), from the backend
 *  `autodust://result` event. Cleared/replaced per image; drives the marker overlay. */
export const autodustSpotsById = writable<Record<string, { x: number; y: number }[]>>({});
export const activeAutodustSpots = derived([autodustSpotsById, activeId], ([m, id]) =>
  id ? m[id] ?? [] : []);

/** A selected heal spot: a brush stroke (by index into dust.strokes) or a global
 *  auto-dust spot (by index into activeAutodustSpots). null = nothing selected. */
export type SpotSel = { kind: "stroke" | "auto"; index: number };
export const selectedSpot = writable<SpotSel | null>(null);

/** Per-image editable metadata overrides (camera/lens/iso/…/note). */
export const metaById = writable<Record<string, MetaOverride>>({});
/** The active image's metadata override (empty object when none). */
export const activeMeta = derived([metaById, activeId], ([m, id]) =>
  id ? m[id] ?? {} : {});

/** Develop-all progress. active=true shows the overlay. */
export const developProgress = writable<{ active: boolean; done: number; total: number }>({
  active: false, done: 0, total: 0,
});

export const hasImages = derived(images, ($i) => $i.length > 0);
export const allDeveloped = derived(images, ($i) => $i.length > 0 && $i.every((x) => x.developed));

export const selectedFolder = writable<string | null>(null);
export const gridZoom = writable<number>(55);

/** Last in-app update check (epoch ms) and the version the user chose to skip.
 * Both persist via app_state as `update_last_check` / `update_skip_version`. */
export const updateLastCheck = writable<number>(0);
export const updateSkipVersion = writable<string>("");

/** Folder/roll-default film base, keyed by image directory path. Persisted via
 * app_state as `folder_base:{dir}`. A per-image base_override wins over this. */
export const folderBaseByPath = writable<Record<string, [number, number, number]>>({});

/** Ids of images that FAILED to develop (e.g. an undecodable/corrupt raw the
 * decoder panics on). They can never become `developed`, so we exclude them from
 * the "undeveloped" badge count instead of nagging forever. Re-tried on every
 * develop-all (cleared on success). Persisted via app_state as `undevelopable_ids`. */
export const undevelopableIds = writable<Set<string>>(new Set());

/** The imported images that live in the selected folder (recursive on parents).
 * The grid, filmstrip, and Develop navigation/range all scope to this. */
export const folderImages = derived(
  [images, selectedFolder],
  ([$i, $sel]) => scopeToFolder($i, $sel),
);
/** Count of frames still needing development — EXCLUDING ones known to be
 * undevelopable (corrupt/undecodable), so one bad file can't pin the badge. */
export const undevelopedCount = derived(
  [folderImages, undevelopableIds],
  ([$i, $undev]) => $i.filter((x) => !x.developed && !$undev.has(x.id)).length,
);

/** Multi-selection across the grid and filmstrips. `activeId` (the single image
 * Develop/Metadata render) stays coupled: a plain click collapses the selection
 * to that one image and makes it active; modifier clicks build the selection
 * without changing which image is active. Cleared on folder change, after a
 * delete, and on a plain click. */
export const selection = writable<SelState>(noneSelected());

const folderImageIds = (): string[] => get(folderImages).map((i) => i.id);

/** Make `id` the single active + selected image (plain click, arrow-nav, folder). */
export function setActive(id: string | null): void {
  activeId.set(id);
  selection.set(id ? { selected: new Set([id]), anchor: id } : noneSelected());
}

/** Handle a thumbnail click: plain -> setActive; Ctrl/Cmd or Shift -> extend the
 * selection (leaving the active image untouched). */
export function selectClick(id: string, mods: Mods): void {
  if (mods.meta || mods.shift) {
    selection.update((s) => click(s, folderImageIds(), id, mods));
  } else {
    setActive(id);
  }
}

/** Ctrl/Cmd+A: select every image in the current folder. */
export function selectAll(): void {
  selection.set(allSelected(folderImageIds()));
}

/** Ids a delete should affect: the multi-selection if any, else the active image. */
export function deleteSelectionIds(): string[] {
  const sel = get(selection).selected;
  if (sel.size > 0) return [...sel];
  const a = get(activeId);
  return a ? [a] : [];
}

/** Select a folder. Per design, if the active image is not in the new folder,
 * make the folder's first image active so the grid/filmstrip/metadata stay in sync. */
export function selectFolder(path: string | null): void {
  selectedFolder.set(path);
  const scoped = scopeToFolder(get(images), path);
  const cur = get(activeId);
  // Reset to a single active+selected image so the grid/filmstrip/metadata stay
  // in sync and any multi-selection is cleared on folder change.
  setActive(scoped.some((i) => i.id === cur) ? cur : scoped[0]?.id ?? null);
}

/** Data-URL of the latest rendered develop preview; drives the histogram. */
export const previewSrc = writable<string>("");

/** Per-image data-URL of the last *fit-view* developed frame. Shown instantly as a
 *  display overlay when switching images, so the canvas doesn't flash through the
 *  old/raw/unadjusted states while the GPU re-develops the new image. Display-only —
 *  the export pipeline re-decodes full-res independently and never reads this.
 *  LRU-capped so a large roll can't grow it without bound. */
export const previewById = writable<Record<string, string>>({});
// Holds both viewed-image fit snapshots and idle-prefetched filmstrip previews, so the
// cap is generous enough to keep a navigation neighbourhood warm (small JPEGs).
const PREVIEW_CACHE_CAP = 96;
/** Drop `id`'s cached preview so a later view/prefetch regenerates it. Call when an
 *  image's develop inputs change while it is NOT the active image (the active image
 *  self-refreshes its cache as the canvas redraws), or when it's deleted. */
export function invalidatePreview(id: string): void {
  previewById.update((m) => {
    if (!(id in m)) return m;
    const next = { ...m };
    delete next[id];
    return next;
  });
}

/** Store `url` as `id`'s fit-view preview (most-recent), evicting the oldest beyond the cap. */
export function cachePreview(id: string, url: string): void {
  previewById.update((m) => {
    if (m[id] === url) return m;
    const next: Record<string, string> = {};
    for (const k of Object.keys(m)) if (k !== id) next[k] = m[k]; // drop old slot…
    next[id] = url; // …re-insert as most-recent
    const keys = Object.keys(next);
    while (keys.length > PREVIEW_CACHE_CAP) delete next[keys.shift()!];
    return next;
  });
}

// Clipping-warning overlay toggles (Develop viewport). `high`/`low` enable the
// highlight (red) / shadow (blue) overlays; `strict` tightens the threshold from
// pure clip (255/0) to near-clip (253/2). Shared by Histogram (corner triangles)
// and Develop → Viewport.
export const clipWarn = writable<{ high: boolean; low: boolean; strict: boolean }>(
  { high: false, low: false, strict: false }
);

export type Tool = "edit" | "crop" | "eraser" | "enhance";
export const tool = writable<Tool>("edit");

/** OpenAI API key for the AI Enhance tool. Persisted via prefs as `openai_api_key`. */
export const openaiApiKey = writable<string>("");

/** User overrides for keyboard shortcuts: action id → replacement binding list.
 *  Defaults live in lib/keymap/hotkeys.ts; this only holds rebindings the user
 *  made. Persisted via prefs as `hotkey_bindings` (JSON). */
export const hotkeyBindings = writable<import("./keymap/hotkeys").BindingOverrides>({});

/** Whether the local upscaler runtime+model are installed (re-checked on tool open). */
export const upscalerInstalled = writable<boolean>(false);

/** Whether the local AI dust/hair models (+ shared runtime) are installed. */
export const autodustInstalled = writable<boolean>(false);

/** Anonymous usage-analytics consent. `telemetryEnabled` gates every event;
 *  `telemetryDecided` is false until the user answers the first-run prompt (an
 *  undecided launch shows it). Persisted via prefs as `telemetry` ("on"/"off");
 *  absent = undecided. See lib/telemetry.ts. */
export const telemetryEnabled = writable<boolean>(false);
export const telemetryDecided = writable<boolean>(false);

/** Debug logging consent. When on, FE hooks forward logs/errors/perf to the
 *  backend log file. Persisted via prefs as `debug_mode` ("on"/"off"). */
export const debugMode = writable<boolean>(false);

/** When true, importing skips camera-preview jpg/png files that share a folder and
 * base name with a raw/master file. Persisted via prefs as `omit_preview_jpgs`;
 * defaults on. */
export const omitPreviewJpgs = writable<boolean>(true);

/** When true, the roll contact sheet renders as a filmstrip with rebates.
 * When false, renders as a plain proof grid. Persisted via prefs as `roll_film_edge`. */
export const rollFilmEdge = writable<boolean>(true);

/** Edge text shown on the filmstrip rebate (e.g. film stock name).
 * Persisted via prefs as `roll_edge_text`. */
export const rollEdgeText = writable<string>("KODAK 400TX · SAFETY FILM · 5063");

/** Film-base recalibration: armed from the Basic panel's Film Base section. While
 * true the viewport shows the drag-to-sample overlay (the sidebar stays in edit
 * mode). `sampledBase` holds the most recently sampled linear base, or null. */
export const baseSampling = writable<boolean>(false);
export const sampledBase = writable<[number, number, number] | null>(null);

// White-point (exposed-leader) D_max anchor tool. `sampledDmax` carries a freshly
// measured D_max from the Tone-section picker (parent viewport crosshair) to
// Basic.svelte. `whitePointPinned` marks images whose D_max is user-pinned so the
// crop-change auto-reanalyze won't clobber it (frontend-only, non-persistent).
export const sampledDmax = writable<number | null>(null);
export const whitePointPinned = writable<Set<string>>(new Set());

/** Ids awaiting a delete confirmation (empty = no dialog). One id deletes a
 * single image; many drive the "Delete N items" multi-delete. */
export const deleteTarget = writable<string[]>([]);

/** Copied develop settings (⌘/Ctrl+C), pasted onto other images with ⌘/Ctrl+V.
 * Holds a full snapshot of the source frame (all params + its crop) plus the
 * groups the user chose at copy time; paste applies those groups directly.
 * null = nothing copied yet. */
export const settingsClipboard = writable<SettingsSnapshot | null>(null);

/** True while the "copy settings" group picker is open. The picker now shows at
 * COPY time (not paste): the user chooses what to carry, then paste applies it. */
export const copySettingsOpen = writable<boolean>(false);

/** Bumped on any dust change and on undo/redo so the Viewport re-renders. */
export const dustRev = writable<number>(0);

/** Resolved HDR capability for the current OS/display/GPU surface — "hidden" on
 *  Linux (HDR is impossible there), "live-edr" when the display+surface support
 *  it, else "gainmap-fallback". Drives whether Basic.svelte shows the HDR toggle.
 *  Defaults to the safe gainmap-fallback until the async probe below resolves;
 *  probed once at module load (app start). See lib/viewport/hdrCapability.ts. */
export const hdrMode = writable<HdrMode>("gainmap-fallback");
probeHdrEnv().then((env) => hdrMode.set(detectHdrMode(env)));
