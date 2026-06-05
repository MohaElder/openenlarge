import { writable, get } from "svelte/store";
import {
  activeId, params, editsById, cropById, dustById, metaById,
  activeCrop, activeDust, activeMeta, dustRev,
} from "../store";
import {
  seeded, pushed, undone, redone, type EditSnapshot, type ImageHistory,
} from "./history";

/** Per-image undo/redo stacks. In-memory only — never persisted to the catalog. */
export const historyById = writable<Record<string, ImageHistory>>({});

/** Decouple a snapshot from live store objects (data is JSON-safe). */
const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));

/** Snapshot the active image's complete editable tuple. */
export function currentSnapshot(): EditSnapshot {
  return {
    params: clone(get(params)),
    crop: clone(get(activeCrop)),
    dust: clone(get(activeDust)),
    meta: clone(get(activeMeta)),
  };
}

/** Write a snapshot back into every per-image store for `id`. */
function applySnapshot(id: string, snap: EditSnapshot): void {
  editsById.update((m) => ({ ...m, [id]: clone(snap.params) }));
  cropById.update((m) => ({ ...m, [id]: clone(snap.crop) }));
  dustById.update((m) => ({ ...m, [id]: clone(snap.dust) }));
  metaById.update((m) => ({ ...m, [id]: clone(snap.meta) }));
  dustRev.update((n) => n + 1);
}

/** Start tracking the active image (no-op if already tracked). */
export function seedActive(): void {
  const id = get(activeId);
  if (!id || get(historyById)[id]) return;
  historyById.update((m) => ({ ...m, [id]: seeded(currentSnapshot()) }));
}

/**
 * Re-baseline a still-pristine image to the current state. Used after a
 * programmatic init (e.g. Basic.svelte's auto white-balance) so that init is
 * folded into the baseline instead of becoming the first undo step.
 */
export function reseedActive(): void {
  const id = get(activeId);
  if (!id) return;
  const h = get(historyById)[id];
  if (!h || h.past.length || h.future.length) return; // only while untouched
  historyById.update((m) => ({ ...m, [id]: seeded(currentSnapshot()) }));
}

/** Commit the active image's current state as a history step (deduped). */
export function commitActive(): void {
  const id = get(activeId);
  if (!id) return;
  const h = get(historyById)[id];
  if (!h) { seedActive(); return; } // defensive: not yet seeded
  const next = pushed(h, currentSnapshot());
  if (next === h) return; // unchanged → nothing recorded
  historyById.update((m) => ({ ...m, [id]: next }));
}

export function undoActive(): void {
  const id = get(activeId);
  if (!id) return;
  const h = get(historyById)[id];
  if (!h) return;
  const { history, snapshot } = undone(h);
  if (!snapshot) return;
  historyById.update((m) => ({ ...m, [id]: history }));
  applySnapshot(id, snapshot);
}

export function redoActive(): void {
  const id = get(activeId);
  if (!id) return;
  const h = get(historyById)[id];
  if (!h) return;
  const { history, snapshot } = redone(h);
  if (!snapshot) return;
  historyById.update((m) => ({ ...m, [id]: history }));
  applySnapshot(id, snapshot);
}

/** Free an image's history (called when the image is deleted). */
export function dropHistory(id: string): void {
  historyById.update((m) => {
    const n = { ...m };
    delete n[id];
    return n;
  });
}
