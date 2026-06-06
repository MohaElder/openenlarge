import { writable, derived, type Readable, type Writable } from "svelte/store";
import type { InvertParams } from "./api";

/** The active image's params, or a fresh default if it has no edits yet. */
export function entryFor(
  map: Record<string, InvertParams>,
  id: string | null,
  makeDefault: () => InvertParams,
): InvertParams {
  return (id !== null && map[id]) || makeDefault();
}

/** Store interface matching a writable<InvertParams> (subscribe/set/update). */
export interface ParamsStore {
  subscribe: Readable<InvertParams>["subscribe"];
  set: (p: InvertParams) => void;
  update: (fn: (p: InvertParams) => InvertParams) => void;
}

/**
 * Per-image params: `$params` reads the active image's entry; set/update write
 * only to the active image. New images lazily resolve to makeDefault().
 */
export function createPerImageParams(
  activeId: Readable<string | null>,
  makeDefault: () => InvertParams,
): { params: ParamsStore; editsById: Writable<Record<string, InvertParams>> } {
  const editsById = writable<Record<string, InvertParams>>({});
  let activeIdVal: string | null = null;
  activeId.subscribe((v) => (activeIdVal = v));

  const view = derived([editsById, activeId], ([m, id]) => entryFor(m, id, makeDefault));

  const params: ParamsStore = {
    subscribe: view.subscribe,
    // Always store a fresh object. Svelte's `bind:value={$params.field}` mutates
    // the live entry in place and calls set() with the same reference; the
    // catalog write-through (wireRecord) persists only entries whose reference
    // changed, so reusing the reference would silently drop those edits (tint,
    // tone sliders) on reload. Shallow copy makes every edit a new reference.
    set: (p) => {
      if (activeIdVal !== null) editsById.update((m) => ({ ...m, [activeIdVal as string]: { ...p } }));
    },
    update: (fn) => {
      if (activeIdVal !== null)
        editsById.update((m) => ({ ...m, [activeIdVal as string]: { ...fn(entryFor(m, activeIdVal, makeDefault)) } }));
    },
  };

  return { params, editsById };
}
