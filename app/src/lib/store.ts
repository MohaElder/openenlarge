import { writable, derived } from "svelte/store";
import type { ImageEntry, InvertParams, Quality } from "./api";
import { defaultParams } from "./api";

export const images = writable<ImageEntry[]>([]);
export const activeId = writable<string | null>(null);
export const module = writable<"library" | "develop">("library");
export const params = writable<InvertParams>(defaultParams());
export const quality = writable<Quality>("performance");

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
