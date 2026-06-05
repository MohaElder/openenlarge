import type { InvertParams, MetaOverride } from "../api";
import type { CropRect } from "../crop/types";
import type { DustEdits } from "./dust";

/** A point-in-time snapshot of one image's complete editable state. */
export interface EditSnapshot {
  params: InvertParams;
  crop: CropRect | null;
  dust: DustEdits;
  meta: MetaOverride;
}

/** Undo/redo stacks for a single image. `present` is the live committed state. */
export interface ImageHistory {
  past: EditSnapshot[];     // states before present (undo targets, oldest first)
  future: EditSnapshot[];   // states undone (redo targets, next-redo first)
  present: EditSnapshot;
}

/** Max undo steps kept per image (in-memory only). */
export const HISTORY_CAP = 50;

/** Value equality for snapshots. Snapshots are plain JSON-safe data. */
export function snapEqual(a: EditSnapshot, b: EditSnapshot): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function seeded(present: EditSnapshot): ImageHistory {
  return { past: [], future: [], present };
}

/** Record `snap` as the new present. No-op (returns same ref) if unchanged. */
export function pushed(h: ImageHistory, snap: EditSnapshot, cap = HISTORY_CAP): ImageHistory {
  if (snapEqual(snap, h.present)) return h;
  let past = [...h.past, h.present];
  if (past.length > cap) past = past.slice(past.length - cap);
  return { past, future: [], present: snap };
}

/** Step back. `snapshot` is the state to apply, or null if nothing to undo. */
export function undone(h: ImageHistory): { history: ImageHistory; snapshot: EditSnapshot | null } {
  if (h.past.length === 0) return { history: h, snapshot: null };
  const present = h.past[h.past.length - 1];
  return {
    history: { past: h.past.slice(0, -1), future: [h.present, ...h.future], present },
    snapshot: present,
  };
}

/** Step forward. `snapshot` is the state to apply, or null if nothing to redo. */
export function redone(h: ImageHistory): { history: ImageHistory; snapshot: EditSnapshot | null } {
  if (h.future.length === 0) return { history: h, snapshot: null };
  const present = h.future[0];
  return {
    history: { past: [...h.past, h.present], future: h.future.slice(1), present },
    snapshot: present,
  };
}

export const canUndo = (h: ImageHistory): boolean => h.past.length > 0;
export const canRedo = (h: ImageHistory): boolean => h.future.length > 0;

export type UndoRedo = "undo" | "redo" | null;

/** Classify a keyboard event: ⌘Z/Ctrl+Z = undo, ⌘⇧Z/Ctrl+⇧Z/Ctrl+Y = redo. */
export function matchUndoRedo(
  e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean },
): UndoRedo {
  if (!e.metaKey && !e.ctrlKey) return null;
  const k = e.key.toLowerCase();
  if (k === "z") return e.shiftKey ? "redo" : "undo";
  if (k === "y") return "redo";
  return null;
}
