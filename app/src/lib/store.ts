import { writable, derived, get } from "svelte/store";
import type { ImageEntry, Quality, MetaOverride } from "./api";
import { defaultParams } from "./api";
import type { CropRect } from "./crop/types";
import { createPerImageParams } from "./perImage";
import { emptyDust, type DustEdits } from "./develop/dust";
import { scopeToFolder } from "./library/folderScope";

export const images = writable<ImageEntry[]>([]);
export const activeId = writable<string | null>(null);
export const module = writable<"library" | "develop">("library");

// Per-image edits: $params is the ACTIVE image's params; writes go to the active
// image only. Every image uses the density inversion (Mode B); the per-channel
// mode was removed as a confusing, lower-quality alternative.
const _perImage = createPerImageParams(activeId, () => defaultParams());
export const params = _perImage.params;
export const editsById = _perImage.editsById;

export const quality = writable<Quality>("performance");

/** Per-image committed crop (null = full image). */
export const cropById = writable<Record<string, CropRect | null>>({});
/** The active image's committed crop. */
export const activeCrop = derived([cropById, activeId], ([m, id]) => (id ? m[id] ?? null : null));

/** Per-image dust edits (eraser strokes). */
export const dustById = writable<Record<string, DustEdits>>({});
/** The active image's dust edits. */
export const activeDust = derived([dustById, activeId], ([m, id]) =>
  id ? m[id] ?? emptyDust() : emptyDust());

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

/** Folder/roll-default film base, keyed by image directory path. Persisted via
 * app_state as `folder_base:{dir}`. A per-image base_override wins over this. */
export const folderBaseByPath = writable<Record<string, [number, number, number]>>({});

/** The imported images that live in the selected folder (recursive on parents).
 * The grid, filmstrip, and Develop navigation/range all scope to this. */
export const folderImages = derived(
  [images, selectedFolder],
  ([$i, $sel]) => scopeToFolder($i, $sel),
);
export const undevelopedCount = derived(folderImages, ($i) => $i.filter((x) => !x.developed).length);

/** Select a folder. Per design, if the active image is not in the new folder,
 * make the folder's first image active so the grid/filmstrip/metadata stay in sync. */
export function selectFolder(path: string | null): void {
  selectedFolder.set(path);
  const scoped = scopeToFolder(get(images), path);
  const cur = get(activeId);
  if (!scoped.some((i) => i.id === cur)) activeId.set(scoped[0]?.id ?? null);
}

/** Data-URL of the latest rendered develop preview; drives the histogram. */
export const previewSrc = writable<string>("");

export type Tool = "edit" | "crop" | "eraser" | "base_picker";
export const tool = writable<Tool>("edit");

/** Id of the image awaiting a delete confirmation (null = no dialog). */
export const deleteTarget = writable<string | null>(null);

/** Bumped on any dust change and on undo/redo so the Viewport re-renders. */
export const dustRev = writable<number>(0);
