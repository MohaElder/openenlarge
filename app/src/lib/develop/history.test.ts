import { describe, it, expect } from "vitest";
import {
  seeded, pushed, undone, redone, canUndo, canRedo, snapEqual, matchUndoRedo,
  HISTORY_CAP, type EditSnapshot, type ImageHistory,
} from "./history";

// Minimal snapshot factory: only `params.exposure` varies in these tests.
const snap = (exposure: number): EditSnapshot => ({
  params: { exposure } as unknown as EditSnapshot["params"],
  crop: null,
  dust: { strokes: [], irRemoval: { enabled: false, sensitivity: 50 } },
  meta: {},
});

describe("history engine — pure ops", () => {
  it("seeds with empty stacks and the given present", () => {
    const h = seeded(snap(0));
    expect(h.past).toEqual([]);
    expect(h.future).toEqual([]);
    expect(h.present.params.exposure).toBe(0);
    expect(canUndo(h)).toBe(false);
    expect(canRedo(h)).toBe(false);
  });

  it("push moves present→past, sets new present, clears future", () => {
    let h: ImageHistory = seeded(snap(0));
    h = pushed(h, snap(1));
    expect(h.past.length).toBe(1);
    expect(h.past[0].params.exposure).toBe(0);
    expect(h.present.params.exposure).toBe(1);
    expect(canUndo(h)).toBe(true);
  });

  it("push is a no-op when the snapshot equals present", () => {
    const h0 = seeded(snap(0));
    const h1 = pushed(h0, snap(0));
    expect(h1).toBe(h0); // same reference, nothing recorded
  });

  it("undo returns the prior snapshot and shifts present into future", () => {
    let h: ImageHistory = pushed(seeded(snap(0)), snap(1));
    const r = undone(h);
    expect(r.snapshot?.params.exposure).toBe(0);
    expect(r.history.present.params.exposure).toBe(0);
    expect(r.history.future.length).toBe(1);
    expect(r.history.future[0].params.exposure).toBe(1);
  });

  it("undo on a fresh history returns null and leaves history untouched", () => {
    const h = seeded(snap(0));
    const r = undone(h);
    expect(r.snapshot).toBeNull();
    expect(r.history).toBe(h);
  });

  it("redo replays the undone snapshot", () => {
    let h: ImageHistory = pushed(seeded(snap(0)), snap(1));
    h = undone(h).history;
    const r = redone(h);
    expect(r.snapshot?.params.exposure).toBe(1);
    expect(r.history.present.params.exposure).toBe(1);
    expect(r.history.past.length).toBe(1);
    expect(r.history.future.length).toBe(0);
  });

  it("a new push after undo clears the redo future", () => {
    let h: ImageHistory = pushed(seeded(snap(0)), snap(1));
    h = undone(h).history;        // present=0, future=[1]
    h = pushed(h, snap(2));        // present=2, future cleared
    expect(h.future).toEqual([]);
    expect(canRedo(h)).toBe(false);
  });

  it("cap trims the oldest past entry", () => {
    let h: ImageHistory = seeded(snap(0));
    for (let i = 1; i <= HISTORY_CAP + 5; i++) h = pushed(h, snap(i));
    expect(h.past.length).toBe(HISTORY_CAP);
    // oldest survivor is exposure (HISTORY_CAP+5) - HISTORY_CAP in past[0]
    expect(h.past[0].params.exposure).toBe(5);
    expect(h.present.params.exposure).toBe(HISTORY_CAP + 5);
  });

  it("snapEqual compares by value", () => {
    expect(snapEqual(snap(1), snap(1))).toBe(true);
    expect(snapEqual(snap(1), snap(2))).toBe(false);
  });

  it("redo on a fresh history returns null and leaves history untouched", () => {
    const h = seeded(snap(0));
    const r = redone(h);
    expect(r.snapshot).toBeNull();
    expect(r.history).toBe(h);
  });

  it("traverses a multi-step chain undo→undo→redo→redo", () => {
    let h: ImageHistory = seeded(snap(0));
    h = pushed(h, snap(1));
    h = pushed(h, snap(2));
    h = undone(h).history; // present 1
    expect(h.present.params.exposure).toBe(1);
    h = undone(h).history; // present 0
    expect(h.present.params.exposure).toBe(0);
    h = redone(h).history; // present 1
    expect(h.present.params.exposure).toBe(1);
    h = redone(h).history; // present 2
    expect(h.present.params.exposure).toBe(2);
    expect(canRedo(h)).toBe(false);
  });
});

describe("matchUndoRedo", () => {
  const ev = (over: Partial<{ key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }>) =>
    ({ key: "z", metaKey: false, ctrlKey: false, shiftKey: false, ...over });

  it("⌘Z → undo, ⌘⇧Z → redo", () => {
    expect(matchUndoRedo(ev({ metaKey: true }))).toBe("undo");
    expect(matchUndoRedo(ev({ metaKey: true, shiftKey: true }))).toBe("redo");
  });
  it("Ctrl+Z / Ctrl+Shift+Z mirror on Windows", () => {
    expect(matchUndoRedo(ev({ ctrlKey: true }))).toBe("undo");
    expect(matchUndoRedo(ev({ ctrlKey: true, shiftKey: true }))).toBe("redo");
  });
  it("Ctrl+Y → redo", () => {
    expect(matchUndoRedo(ev({ key: "y", ctrlKey: true }))).toBe("redo");
  });
  it("⌘Y (metaKey) does NOT map to redo — Ctrl+Y only", () => {
    expect(matchUndoRedo(ev({ key: "y", metaKey: true }))).toBeNull();
  });
  it("is case-insensitive on the key", () => {
    expect(matchUndoRedo(ev({ key: "Z", metaKey: true }))).toBe("undo");
  });
  it("returns null without a modifier or for other keys", () => {
    expect(matchUndoRedo(ev({}))).toBeNull();
    expect(matchUndoRedo(ev({ key: "a", metaKey: true }))).toBeNull();
  });
});
