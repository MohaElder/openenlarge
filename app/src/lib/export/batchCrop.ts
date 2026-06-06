import type { CropRect } from "../crop/types";
import { defaultFull } from "../crop/cropMath";

/** Mutable draft for the export batch-crop controls — same shape as Develop's
 *  crop draft, kept flat so it can bind directly to CropView/CropPanel. */
export interface CropDraft {
  rect: CropRect["rect"];
  aspect: string;
  orientation: "landscape" | "portrait";
  rot90: 0 | 1 | 2 | 3;
  flipH: boolean;
  flipV: boolean;
  angle: number;
}

/** First id in display order that is currently selected, or null. */
export function firstSelected(ids: string[], selected: Set<string>): string | null {
  for (const id of ids) if (selected.has(id)) return id;
  return null;
}

/** Seed a draft from an existing committed crop, or full-frame if none.
 *  `origW`/`origH` decide the default orientation when there's no prior crop. */
export function seedDraft(crop: CropRect | null, origW: number, origH: number): CropDraft {
  if (crop) {
    return {
      rect: { ...crop.rect }, aspect: crop.aspect, orientation: crop.orientation,
      rot90: crop.rot90, flipH: crop.flipH, flipV: crop.flipV, angle: crop.angle,
    };
  }
  return {
    rect: defaultFull(), aspect: "original",
    orientation: origW >= origH ? "landscape" : "portrait",
    rot90: 0, flipH: false, flipV: false, angle: 0,
  };
}

/** A draft → committed CropRect (what export feeds to the bake spec). */
export function draftToCrop(d: CropDraft): CropRect {
  return {
    rect: d.rect, aspect: d.aspect, orientation: d.orientation,
    rot90: d.rot90, flipH: d.flipH, flipV: d.flipV, angle: d.angle,
  };
}

/** Resolve the crop applied to one image during export: the shared batch crop
 *  when batch mode is on, otherwise the image's own committed crop (or none). */
export function resolveCrop(
  batchOn: boolean, draft: CropDraft, stored: CropRect | null,
): CropRect | null {
  return batchOn ? draftToCrop(draft) : stored;
}
