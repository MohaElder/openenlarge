import { writable, derived, get } from "svelte/store";
import type { ImageEntry, Quality, InvertParams } from "./api";
import { defaultParams } from "./api";
import type { CropRect } from "./crop/types";
import { createPerImageParams } from "./perImage";
import { emptyDust, type DustEdits } from "./develop/dust";

export const images = writable<ImageEntry[]>([]);
export const activeId = writable<string | null>(null);
export const module = writable<"library" | "develop">("library");

/** Global develop mode (B·density / C·per-chan). Set once in Settings; applies to
 * every image, current and future. Persisted across sessions. */
const storedMode = typeof localStorage !== "undefined" ? localStorage.getItem("developMode") : null;
export const developMode = writable<"b" | "c">(storedMode === "c" ? "c" : "b");
if (typeof localStorage !== "undefined")
  developMode.subscribe((m) => { try { localStorage.setItem("developMode", m); } catch { /* ignore */ } });

// Per-image edits: $params is the ACTIVE image's params; writes go to the active
// image only. New images inherit the current global develop mode.
const _perImage = createPerImageParams(activeId, () => ({ ...defaultParams(), mode: get(developMode) }));
export const params = _perImage.params;
export const editsById = _perImage.editsById;

// Develop mode is global: when it changes, re-apply it to every image's params.
developMode.subscribe((m) =>
  editsById.update((map) => {
    let changed = false;
    const out: Record<string, InvertParams> = {};
    for (const k in map) {
      out[k] = map[k].mode === m ? map[k] : { ...map[k], mode: m };
      if (out[k] !== map[k]) changed = true;
    }
    return changed ? out : map;
  })
);
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

/** Develop-all progress. active=true shows the overlay. */
export const developProgress = writable<{ active: boolean; done: number; total: number }>({
  active: false, done: 0, total: 0,
});

export const hasImages = derived(images, ($i) => $i.length > 0);
export const allDeveloped = derived(images, ($i) => $i.length > 0 && $i.every((x) => x.developed));

export const selectedFolder = writable<string | null>(null);
export const gridZoom = writable<number>(55);
export const undevelopedCount = derived(images, ($i) => $i.filter((x) => !x.developed).length);

/** Data-URL of the latest rendered develop preview; drives the histogram. */
export const previewSrc = writable<string>("");

export type Tool = "edit" | "crop" | "eraser";
export const tool = writable<Tool>("edit");
