import { describe, it, expect } from "vitest";
import { undevelopedIds } from "./workflow";
import type { ImageEntry } from "./api";

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
