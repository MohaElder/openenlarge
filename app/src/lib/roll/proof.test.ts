import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("../api", () => ({
  api: {
    asShotWb: vi.fn().mockResolvedValue({ temp: 6200, tint: 12, gains: [1, 1, 1] }),
    autoBrightness: vi.fn().mockResolvedValue({ exposure: -1.25 }),
  },
  defaultParams: () => ({}),
}));
vi.mock("../develop/base", () => ({ withEffectiveBase: (p: unknown) => p }));
vi.mock("../library/folderScope", () => ({ imageDir: () => "/roll" }));

import { cropById } from "../store";
import { api, type InvertParams } from "../api";
import {
  proofMode, proofRev, proofOverlay, ensureProofSolves, proofEnterFolder, proofInvalidate,
} from "./proof";

const frame = (id: string) =>
  ({ id, path: `/roll/${id}.dng`, file_name: `${id}.dng`, thumbnail: "", metadata: {},
     offline: false, developed: true, has_ir: false, positive: false }) as never;

const own = { exposure: 0.5, temp: 5500, tint: 0, wb_baseline: [2, 1, 1] } as unknown as InvertParams;
const paramsOf = () => own;

beforeEach(() => {
  vi.clearAllMocks();
  cropById.set({});
  proofMode.set({ on: true, autoExposure: true, autoColor: true });
  // A folder change clears the module-level solve cache between tests.
  proofEnterFolder(`/roll-${Math.random()}`);
});

describe("proofOverlay", () => {
  it("passes params through when the mode is off or nothing is solved", () => {
    expect(proofOverlay("a", own, { on: false, autoExposure: true, autoColor: true })).toBe(own);
    expect(proofOverlay("a", own, { on: true, autoExposure: true, autoColor: true })).toBe(own);
  });

  it("applies solved exposure and WB per sub-toggle after solving", async () => {
    await ensureProofSolves([frame("a")], paramsOf);

    const both = proofOverlay("a", own, { on: true, autoExposure: true, autoColor: true });
    expect(both.exposure).toBe(-1.25);
    expect(both.temp).toBe(6200);
    expect(both.tint).toBe(12);
    expect(both.wb_baseline).toEqual([1, 1, 1]); // as-shot model resets the hidden baseline

    const expOnly = proofOverlay("a", own, { on: true, autoExposure: true, autoColor: false });
    expect(expOnly.exposure).toBe(-1.25);
    expect(expOnly.temp).toBe(5500); // stored WB kept

    const colorOnly = proofOverlay("a", own, { on: true, autoExposure: false, autoColor: true });
    expect(colorOnly.exposure).toBe(0.5); // stored exposure kept
    expect(colorOnly.temp).toBe(6200);
  });
});

describe("ensureProofSolves", () => {
  it("meters against the frame's stored crop + orientation", async () => {
    cropById.set({
      a: { rect: { x: 0.1, y: 0.2, w: 0.5, h: 0.6 }, aspect: "original", orientation: "landscape",
           rot90: 1, flipH: true, flipV: false, angle: 2 } as never,
    });
    await ensureProofSolves([frame("a")], paramsOf);
    expect(api.asShotWb).toHaveBeenCalledWith(
      "a", expect.anything(), [0.1, 0.2, 0.5, 0.6],
      { rot90: 1, flip_h: true, flip_v: false, angle: 2 },
    );
    expect(api.autoBrightness).toHaveBeenCalledWith(
      "a", expect.anything(), [0.1, 0.2, 0.5, 0.6],
      { rot90: 1, flip_h: true, flip_v: false, angle: 2 },
    );
  });

  it("caches solves (no re-solve on second call) and bumps proofRev only when it solved", async () => {
    const rev0 = get(proofRev);
    await ensureProofSolves([frame("a")], paramsOf);
    expect(get(proofRev)).toBe(rev0 + 1);
    await ensureProofSolves([frame("a")], paramsOf);
    expect(api.asShotWb).toHaveBeenCalledTimes(1); // cached — no second solve
    expect(get(proofRev)).toBe(rev0 + 1); // no spurious re-render trigger
  });

  it("a failed solve leaves that axis on the stored value", async () => {
    (api.autoBrightness as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("not resident"));
    await ensureProofSolves([frame("a")], paramsOf);
    const p = proofOverlay("a", own, { on: true, autoExposure: true, autoColor: true });
    expect(p.exposure).toBe(0.5); // stored exposure kept
    expect(p.temp).toBe(6200); // WB still applied
  });

  it("proofInvalidate drops solves so the next ensure re-meters", async () => {
    await ensureProofSolves([frame("a")], paramsOf);
    proofInvalidate();
    await ensureProofSolves([frame("a")], paramsOf);
    expect(api.asShotWb).toHaveBeenCalledTimes(2);
  });
});
