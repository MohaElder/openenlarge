/** The normalized real directory an image lives in (its path minus the filename). */
export function imageDir(img: { path: string }): string {
  return img.path.replace(/\\/g, "/").split("/").slice(0, -1).join("/");
}

/** Is an image directory inside the selected folder? Recursive on parents:
 * selecting a parent captures every descendant. `null` selection = show all.
 * Uses a "/"-boundary so a name prefix (".../ny2026") never captures a longer
 * sibling (".../ny2026-2"). */
export function inFolder(dir: string, selected: string | null): boolean {
  if (selected == null) return true;
  return dir === selected || dir.startsWith(selected + "/");
}

/** The subset of images that live in the selected folder (recursive on parents). */
export function scopeToFolder<T extends { path: string }>(
  images: T[],
  selected: string | null,
): T[] {
  return images.filter((i) => inFolder(imageDir(i), selected));
}

/** Pick the folder to jump to after an import: the directory holding the most
 * just-imported frames (modal). Ties break toward the latest frame — the folder
 * whose last frame appears latest in `dirs` (input order). `null` for an empty
 * batch (nothing imported → leave the current selection alone). A single-folder
 * roll, the common case, simply returns that folder. */
export function pickImportFolder(dirs: string[]): string | null {
  const count = new Map<string, number>();
  const lastIdx = new Map<string, number>();
  dirs.forEach((d, i) => {
    count.set(d, (count.get(d) ?? 0) + 1);
    lastIdx.set(d, i);
  });
  let best: string | null = null;
  for (const d of count.keys()) {
    if (
      best === null ||
      count.get(d)! > count.get(best)! ||
      (count.get(d)! === count.get(best)! && lastIdx.get(d)! > lastIdx.get(best)!)
    ) {
      best = d;
    }
  }
  return best;
}
