// Display source-of-truth for the keyboard-shortcuts popup (KeymapModal).
// This registry is for SHOWING shortcuts only — the actual key handling lives in
// each component's on:keydown (Develop, Library, crop tools). Keep the two in sync
// by hand when shortcuts change.

/** A set of tokens pressed together, e.g. ["Mod", "⌫"]. The "Mod" token is
 *  resolved to ⌘ on macOS and Ctrl elsewhere at render time. */
export type Combo = string[];

/** One shortcut row: a label plus one-or-more alternative key combos. */
export type Hotkey = { keys: Combo[]; label: string };

/** A titled group of shortcut rows. `heading`/`label` are i18n keys. */
export type HotkeyGroup = { heading: string; items: Hotkey[] };

export const hotkeyGroups: HotkeyGroup[] = [
  {
    heading: "keymap.group.navigation",
    items: [
      { keys: [["←"], ["→"]], label: "keymap.nav.prevNext" },
      { keys: [["↑"], ["↓"]], label: "keymap.nav.firstLast" },
      { keys: [["Mod", "⌫"]], label: "keymap.nav.delete" },
    ],
  },
  {
    heading: "keymap.group.editing",
    items: [
      { keys: [["Mod", "Z"]], label: "keymap.edit.undo" },
      { keys: [["Mod", "["], ["Mod", "]"]], label: "keymap.edit.rotate" },
    ],
  },
  {
    heading: "keymap.group.crop",
    items: [
      { keys: [["Enter"]], label: "keymap.crop.commit" },
      { keys: [["Esc"]], label: "keymap.crop.discard" },
      { keys: [["X"]], label: "keymap.crop.swap" },
      { keys: [["Shift"]], label: "keymap.crop.lockAspect" },
    ],
  },
];

/** True when running on macOS, where the platform modifier is ⌘ rather than Ctrl. */
function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const p = (navigator.platform || navigator.userAgent || "").toLowerCase();
  return p.includes("mac");
}

/** Resolve a display token to its on-screen glyph (currently just the Mod key). */
export function keyLabel(token: string): string {
  if (token === "Mod") return isMac() ? "⌘" : "Ctrl";
  return token;
}
