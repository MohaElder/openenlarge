import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { activeId, editsById, cropById, dustById, metaById, dustRev } from "../store";
import { defaultParams } from "../api";
import {
  historyById, seedActive, reseedActive, commitActive, undoActive, redoActive, dropHistory,
} from "./historyStore";

const DEF_EXP = defaultParams().exposure;

beforeEach(() => {
  activeId.set("img1");
  editsById.set({});
  cropById.set({});
  dustById.set({});
  metaById.set({});
  historyById.set({});
  dustRev.set(0);
});

const setExposure = (v: number) =>
  editsById.update((m) => ({ ...m, img1: { ...defaultParams(), exposure: v } }));

describe("historyStore", () => {
  it("seedActive creates a pristine entry from the current state", () => {
    seedActive();
    const h = get(historyById)["img1"];
    expect(h.past).toEqual([]);
    expect(h.present.params.exposure).toBe(DEF_EXP);
  });

  it("seedActive is idempotent (won't clobber an existing entry)", () => {
    seedActive();
    setExposure(2);
    commitActive();           // past now has 1 entry
    seedActive();             // must NOT reset
    expect(get(historyById)["img1"].past.length).toBe(1);
  });

  it("commit after an edit records one undo step", () => {
    seedActive();
    setExposure(2);
    commitActive();
    const h = get(historyById)["img1"];
    expect(h.past.length).toBe(1);
    expect(h.present.params.exposure).toBe(2);
  });

  it("commit with no change is a no-op", () => {
    seedActive();
    commitActive();
    expect(get(historyById)["img1"].past.length).toBe(0);
  });

  it("undo writes the previous params back into editsById", () => {
    seedActive();
    setExposure(2);
    commitActive();
    undoActive();
    expect(get(editsById)["img1"].exposure).toBe(DEF_EXP);
  });

  it("redo re-applies the undone params", () => {
    seedActive();
    setExposure(2);
    commitActive();
    undoActive();
    redoActive();
    expect(get(editsById)["img1"].exposure).toBe(2);
  });

  it("undo bumps dustRev so the Viewport re-renders", () => {
    seedActive();
    setExposure(2);
    commitActive();
    const before = get(dustRev);
    undoActive();
    expect(get(dustRev)).toBe(before + 1);
  });

  it("reseedActive re-baselines a pristine image but not a touched one", () => {
    seedActive();
    setExposure(1);
    reseedActive();           // still pristine → present tracks exposure 1
    expect(get(historyById)["img1"].present.params.exposure).toBe(1);
    setExposure(2);
    commitActive();           // now touched (past length 1)
    setExposure(3);
    reseedActive();           // touched → no-op
    expect(get(historyById)["img1"].present.params.exposure).toBe(2);
  });

  it("dropHistory removes an image's stack", () => {
    seedActive();
    dropHistory("img1");
    expect(get(historyById)["img1"]).toBeUndefined();
  });
});
