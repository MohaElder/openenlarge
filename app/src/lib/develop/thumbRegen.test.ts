import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("../api", () => ({
  api: {
    ensureDeveloped: vi.fn().mockResolvedValue(undefined),
    thumbnail: vi.fn().mockResolvedValue("data:new"),
    saveThumbnail: vi.fn().mockResolvedValue(undefined),
    invalidateThumbnails: vi.fn().mockResolvedValue(undefined),
    asShotWb: vi.fn().mockResolvedValue({}),
    perZoneWb: vi.fn().mockResolvedValue({ sh: 0, mid: 0, hi: 0 }),
  },
  defaultParams: () => ({}),
}));
vi.mock("./base", () => ({ withEffectiveBase: (p: any) => p }));
vi.mock("./wb", () => ({ applyAsShotWb: (p: any) => p }));
vi.mock("../library/folderScope", () => ({ imageDir: () => "/dir" }));
vi.mock("../library/gridHiRes", () => ({ gridThumbView: () => ({}), GRID_STATIC_EDGE: 320 }));

import { images, editsById, cropById, dustById } from "../store";
import { api } from "../api";
import { regenOne, pump, markThumbsStale } from "./thumbRegen";

const frame = (id: string, extra: Record<string, unknown> = {}) => ({
  id, path: `/p/${id}`, file_name: id, thumbnail: "old", metadata: {},
  offline: false, developed: true, has_ir: false, positive: false, thumb_stale: true, ...extra,
});

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  images.set([]); editsById.set({}); cropById.set({}); dustById.set({});
  vi.clearAllMocks();
});

describe("regenOne", () => {
  it("rebakes, persists, and clears thumb_stale", async () => {
    images.set([frame("a") as any]);
    editsById.set({ a: { foo: 1 } as any });
    await regenOne(get(images)[0] as any);
    expect(api.thumbnail).toHaveBeenCalledWith("a", { foo: 1 }, {});
    expect(api.saveThumbnail).toHaveBeenCalledWith("a", "data:new");
    const img = get(images)[0];
    expect(img.thumbnail).toBe("data:new");
    expect(img.thumb_stale).toBe(false);
  });

  it("leaves thumb_stale set when the render fails", async () => {
    images.set([frame("a") as any]);
    editsById.set({ a: { foo: 1 } as any });
    (api.thumbnail as any).mockRejectedValueOnce(new Error("boom"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    await regenOne(get(images)[0] as any);
    expect(get(images)[0].thumb_stale).toBe(true);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it("skips undeveloped frames", async () => {
    images.set([frame("a", { developed: false }) as any]);
    await regenOne(get(images)[0] as any);
    expect(api.thumbnail).not.toHaveBeenCalled();
  });

  it("edit-less fallback meters auto-WB against the frame's stored crop", async () => {
    images.set([frame("a") as any]);
    cropById.set({
      a: {
        rect: { x: 0.1, y: 0.2, w: 0.5, h: 0.6 },
        aspect: "original", orientation: "landscape",
        rot90: 1, flipH: true, flipV: false, angle: 2,
      } as any,
    });
    await regenOne(get(images)[0] as any);
    expect(api.asShotWb).toHaveBeenCalledWith(
      "a", expect.anything(), [0.1, 0.2, 0.5, 0.6],
      { rot90: 1, flip_h: true, flip_v: false, angle: 2 },
    );
    expect(api.perZoneWb).toHaveBeenCalledWith(
      "a", expect.anything(), [0.1, 0.2, 0.5, 0.6],
      { rot90: 1, flip_h: true, flip_v: false, angle: 2 },
    );
  });

  it("edit-less fallback without a stored crop meters the full frame", async () => {
    images.set([frame("a") as any]);
    await regenOne(get(images)[0] as any);
    expect(api.asShotWb).toHaveBeenCalledWith(
      "a", expect.anything(), null,
      { rot90: 0, flip_h: false, flip_v: false, angle: 0 },
    );
  });
});

describe("markThumbsStale + pump", () => {
  it("marks frames stale, persists invalidation, and drains them", async () => {
    images.set([frame("a", { thumb_stale: false }) as any, frame("b", { thumb_stale: false }) as any]);
    editsById.set({ a: {} as any, b: {} as any });
    markThumbsStale(["a", "b"], { persist: true });
    expect(api.invalidateThumbnails).toHaveBeenCalledWith(["a", "b"]);
    await flush(); await flush();
    expect(api.thumbnail).toHaveBeenCalledTimes(2);
    expect(get(images).every((i) => i.thumb_stale === false)).toBe(true);
  });

  it("does not persist when opts.persist is false", () => {
    images.set([frame("a", { thumb_stale: false }) as any]);
    markThumbsStale(["a"]);
    expect(api.invalidateThumbnails).not.toHaveBeenCalled();
  });

  it("pump renders every developed+stale frame once", async () => {
    images.set([frame("a") as any, frame("b") as any, frame("c") as any]);
    editsById.set({ a: {} as any, b: {} as any, c: {} as any });
    pump();
    await flush(); await flush();
    expect(api.thumbnail).toHaveBeenCalledTimes(3);
  });

  it("pump() terminates when every frame persistently fails — no hot-loop", async () => {
    images.set([frame("a") as any, frame("b") as any]);
    editsById.set({ a: {} as any, b: {} as any });
    (api.thumbnail as any).mockRejectedValue(new Error("always fail"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    pump();
    await flush(); await flush(); await flush();
    // Each frame attempted exactly once per drain — not looping
    expect(api.thumbnail).toHaveBeenCalledTimes(2);
    // Failed frames stay stale so they retry on the next pump
    expect(get(images).every((i) => i.thumb_stale === true)).toBe(true);
    warn.mockRestore();
    (api.thumbnail as any).mockResolvedValue("data:new"); // restore default impl
  });
});
