import { get } from "svelte/store";
import { images, activeId, module, developProgress } from "./store";
import { api, type ImageEntry } from "./api";

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

/** Develop every not-yet-developed image sequentially, updating progress, then
 * switch to the Develop module. Resolves when done. */
export async function developAll(): Promise<void> {
  const ids = undevelopedIds(get(images));
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
    const first = get(images)[0];
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
