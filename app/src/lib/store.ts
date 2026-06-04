import { writable } from "svelte/store";
import type { ImageEntry, InvertParams } from "./api";
import { defaultParams } from "./api";

export const images = writable<ImageEntry[]>([]);
export const activeId = writable<string | null>(null);
export const module = writable<"library" | "develop">("library");
export const params = writable<InvertParams>(defaultParams());
