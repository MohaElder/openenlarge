import { get } from "svelte/store";
import { images, activeId, module, developProgress, editsById, cropById, dustById, folderImages } from "./store";
import { api, type ImageEntry } from "./api";
import { dropHistory } from "./develop/historyStore";

/** Ids of images not yet developed, in order. Pure helper (testable). */
export function undevelopedIds(list: ImageEntry[]): string[] {
  return list.filter((i) => !i.developed).map((i) => i.id);
}

/** Resolve after the browser has had a chance to paint (two rAFs). Falls back to a
 * macrotask in non-DOM contexts (tests). */
function nextPaint(): Promise<void> {
  if (typeof requestAnimationFrame === "undefined") return new Promise((r) => setTimeout(r, 0));
  return new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
}

/** Develop every not-yet-developed image IN THE SELECTED FOLDER sequentially,
 * updating progress, then switch to the Develop module. Resolves when done. */
export async function developAll(): Promise<void> {
  const ids = undevelopedIds(get(folderImages));
  if (ids.length === 0) { module.set("develop"); return; }
  developProgress.set({ active: true, done: 0, total: ids.length });
  // Let the overlay paint (and fade in) before kicking off the first develop call.
  await nextPaint();
  for (const id of ids) {
    try {
      const updated = await api.developImage(id);
      images.update((list) => list.map((i) => (i.id === id ? updated : i)));
    } catch (e) {
      console.error("develop failed", id, e);
    }
    developProgress.update((p) => ({ ...p, done: p.done + 1 }));
  }
  if (!get(activeId)) {
    const first = get(folderImages)[0];
    if (first) activeId.set(first.id);
  }
  module.set("develop");
  // Keep the overlay up while the (heavy) Develop view mounts, then fade it out on
  // a free main thread so the dismiss animates instead of snapping.
  await nextPaint();
  developProgress.set({ active: false, done: ids.length, total: ids.length });
}

/** Mark all images undeveloped (used when the quality setting changes). */
export function markAllUndeveloped(): void {
  images.update((list) => list.map((i) => ({ ...i, developed: false })));
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

  if (wasActive) activeId.set(neighbour ? neighbour.id : null);
  if (get(images).length === 0) module.set("library");
}
