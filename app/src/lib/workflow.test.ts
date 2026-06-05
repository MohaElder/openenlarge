import { describe, it, expect, vi } from "vitest";
import { get } from "svelte/store";
import { undevelopedIds, applyStockToIds } from "./workflow";
import type { ImageEntry } from "./api";
import { defaultParams } from "./api";

vi.mock("./api", async (orig) => {
  const actual = await orig<typeof import("./api")>();
  return { ...actual, api: { ...actual.api, developImage: vi.fn(async (id: string) => ({
    id, path: `/x/${id}.dng`, file_name: `${id}.dng`, thumbnail: "t",
    metadata: { width: 10, height: 10, file_size: 0 }, developed: true, has_ir: false, offline: false,
  })) } };
});

const mk = (id: string, developed: boolean): ImageEntry => ({
  id, path: "", file_name: id, thumbnail: "", developed, has_ir: false, offline: false,
  metadata: { width: 0, height: 0, file_size: 0 },
});

describe("undevelopedIds", () => {
  it("returns only not-developed ids in order", () => {
    const list = [mk("a", true), mk("b", false), mk("c", false)];
    expect(undevelopedIds(list)).toEqual(["b", "c"]);
  });
  it("returns empty when all developed", () => {
    expect(undevelopedIds([mk("a", true)])).toEqual([]);
  });
});

describe("applyStockToIds", () => {
  it("sets stock on the listed ids, seeding from defaults when absent", () => {
    const map = { a: { ...defaultParams(), exposure: 1.2 } };
    const out = applyStockToIds(map, ["a", "b"], "portra400", defaultParams);
    expect(out.a.stock).toBe("portra400");
    expect(out.a.exposure).toBe(1.2); // existing fields preserved
    expect(out.b.stock).toBe("portra400"); // absent id seeded from defaults
    expect(out.b.exposure).toBe(0);
  });

  it("leaves out-of-scope ids untouched and does not mutate the input", () => {
    const map = { a: { ...defaultParams(), stock: "none" as const }, z: { ...defaultParams(), stock: "fujic200" as const } };
    const out = applyStockToIds(map, ["a"], "portra400", defaultParams);
    expect(out.z.stock).toBe("fujic200"); // untouched
    expect(map.a.stock).toBe("none"); // input not mutated
    expect(out).not.toBe(map);
  });

  it("returns the map unchanged-shape for an empty id list", () => {
    const map = { a: defaultParams() };
    const out = applyStockToIds(map, [], "portra400", defaultParams);
    expect(out.a.stock).toBe("none");
  });
});

describe("developAll stock application", () => {
  it("sets the chosen stock on the undeveloped folder images", async () => {
    const { images, selectedFolder, editsById } = await import("./store");
    const { developAll } = await import("./workflow");
    selectedFolder.set(null); // null = whole library in scope
    editsById.set({});
    images.set([
      { id: "a", path: "/x/a.dng", file_name: "a.dng", thumbnail: "t", metadata: { width: 10, height: 10, file_size: 0 }, developed: false, has_ir: false, offline: false },
      { id: "b", path: "/x/b.dng", file_name: "b.dng", thumbnail: "t", metadata: { width: 10, height: 10, file_size: 0 }, developed: true, has_ir: false, offline: false },
    ]);
    await developAll("portra400");
    expect(get(editsById).a?.stock).toBe("portra400"); // undeveloped → set
    expect(get(editsById).b).toBeUndefined();           // already developed → untouched
  });

  it("does not touch editsById when stock is none/omitted", async () => {
    const { images, selectedFolder, editsById } = await import("./store");
    const { developAll } = await import("./workflow");
    selectedFolder.set(null);
    editsById.set({});
    images.set([
      { id: "a", path: "/x/a.dng", file_name: "a.dng", thumbnail: "t", metadata: { width: 10, height: 10, file_size: 0 }, developed: false, has_ir: false, offline: false },
    ]);
    await developAll("none");
    expect(get(editsById).a).toBeUndefined();
    await developAll();
    expect(get(editsById).a).toBeUndefined();
  });
});
