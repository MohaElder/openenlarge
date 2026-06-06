import { writable } from "svelte/store";

/** True while a watch-folder session is active. */
export const tetherWatching = writable<boolean>(false);

/** The folder currently being watched (also the active roll), or null. */
export const tetherDir = writable<string | null>(null);

/** When true, each new shot becomes active and switches to Develop. */
export const tetherAutoAdvance = writable<boolean>(true);

/** Status of the most recent capture, for the panel's status line. */
export const tetherLast = writable<{ name: string; ok: boolean; error?: string } | null>(null);
