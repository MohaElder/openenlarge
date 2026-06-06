import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("../api", async (orig) => {
  const actual = await orig<typeof import("../api")>();
  return {
    ...actual,
    api: {
      ...actual.api,
      importImage: vi.fn(async (path: string) => ({
        id: path, path, file_name: path.split("/").pop()!, thumbnail: "t",
        metadata: { width: 0, height: 0, file_size: 0 }, developed: false, has_ir: false, offline: false,
      })),
      developImage: vi.fn(async (id: string) => ({
        id, path: id, file_name: id.split("/").pop()!, thumbnail: "dev",
        metadata: { width: 10, height: 10, file_size: 0 }, developed: true, has_ir: false, offline: false,
      })),
    },
  };
});

describe("processNewFile", () => {
  beforeEach(async () => {
    const { images, activeId, module } = await import("../store");
    const { tetherAutoAdvance, tetherLast } = await import("./store");
    images.set([]); activeId.set(null); module.set("library");
    tetherAutoAdvance.set(true); tetherLast.set(null);
  });

  it("imports, develops, adds the developed entry, and auto-advances", async () => {
    const { processNewFile } = await import("./controller");
    const { images, activeId, module } = await import("../store");
    await processNewFile("/roll/DSCF1.dng");
    const list = get(images);
    expect(list).toHaveLength(1);
    expect(list[0].developed).toBe(true);
    expect(get(activeId)).toBe("/roll/DSCF1.dng");
    expect(get(module)).toBe("develop");
    expect(get((await import("./store")).tetherLast)).toEqual({ name: "DSCF1.dng", ok: true });
  });

  it("does not change active/module when auto-advance is off", async () => {
    const { processNewFile } = await import("./controller");
    const { activeId, module } = await import("../store");
    const { tetherAutoAdvance } = await import("./store");
    tetherAutoAdvance.set(false);
    await processNewFile("/roll/DSCF2.dng");
    expect(get(activeId)).toBeNull();
    expect(get(module)).toBe("library");
  });

  it("records an error and does not throw when develop fails", async () => {
    const { api } = await import("../api");
    (api.developImage as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("decode boom"));
    const { processNewFile } = await import("./controller");
    const { tetherLast } = await import("./store");
    await expect(processNewFile("/roll/BAD.dng")).resolves.toBeUndefined();
    expect(get(tetherLast)).toEqual({ name: "BAD.dng", ok: false, error: "Error: decode boom" });
    expect(get((await import("../store")).images)).toHaveLength(0);
  });
});
