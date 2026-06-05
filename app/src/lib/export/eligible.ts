import { derived } from "svelte/store";
import { images } from "../store";

/** Images that have been developed — the only ones eligible for export. */
export const developedImages = derived(images, ($i) => $i.filter((x) => x.developed));

/** True when at least one image is developed. */
export const hasDeveloped = derived(images, ($i) => $i.some((x) => x.developed));
