# Per-Image Edits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Develop slider edits per-image instead of global, so editing one image no longer changes every imported image.

**Architecture:** Replace the single global `params` writable with per-image storage (`editsById: Record<id, InvertParams>`) behind a custom store whose `$params` reads the *active* image's entry and whose `set`/`update` write to the active image. The store interface (`subscribe`/`set`/`update`) is unchanged, so every existing consumer keeps working. The pure logic lives in a testable, Tauri-free helper module.

**Tech Stack:** Svelte 5 stores, TypeScript, vitest. cwd for npm/npx: `/Users/mohaelder/Repos/filmrev/app`.

**Spec:** `docs/superpowers/specs/2026-06-04-develop-interactivity-design.md` (Plan 1 section).

**Branch:** `feat/develop-redesign`.

---

## File Structure

**Create:**
- `app/src/lib/perImage.ts` — `entryFor()` (resolve active image's params or defaults) and `createPerImageParams()` (the custom store factory). Pure: imports only `svelte/store` and a TYPE from `./api` (erased at runtime), so it is unit-testable without Tauri.
- `app/src/lib/perImage.test.ts` — vitest for `entryFor` + per-image isolation behavior.

**Modify:**
- `app/src/lib/store.ts` — replace the global `params` writable with `createPerImageParams(activeId, defaultParams)`.

No backend, no `.svelte`, no API changes. `Basic.svelte`, `Develop.svelte`, and the sliders are untouched (they use `$params` / `params.update` / `bind:value`, all still supported).

---

## Task 1: Per-image params helper + store factory (pure, tested)

**Files:**
- Create: `app/src/lib/perImage.ts`
- Create: `app/src/lib/perImage.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `app/src/lib/perImage.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { get, writable } from "svelte/store";
import { entryFor, createPerImageParams } from "./perImage";
import type { InvertParams } from "./api";

// Minimal stand-in default; the real defaultParams is injected in app code.
const mk = (): InvertParams => ({ exposure: 0 } as InvertParams);

describe("entryFor", () => {
  it("returns the stored entry for a known id", () => {
    const a = { exposure: 5 } as InvertParams;
    expect(entryFor({ A: a }, "A", mk)).toBe(a);
  });
  it("returns a fresh default for an unknown id or null", () => {
    expect(entryFor({}, "X", mk).exposure).toBe(0);
    expect(entryFor({}, null, mk).exposure).toBe(0);
  });
});

describe("createPerImageParams", () => {
  it("isolates edits per active image and restores them on switch", () => {
    const activeId = writable<string | null>(null);
    const { params } = createPerImageParams(activeId, mk);

    activeId.set("A");
    params.update((p) => ({ ...p, exposure: 5 }));
    expect(get(params as any).exposure).toBe(5);

    // Switching to a new image shows its defaults, NOT A's edits.
    activeId.set("B");
    expect(get(params as any).exposure).toBe(0);
    params.set({ exposure: -3 } as InvertParams);
    expect(get(params as any).exposure).toBe(-3);

    // Switching back restores each image's own edits.
    activeId.set("A");
    expect(get(params as any).exposure).toBe(5);
    activeId.set("B");
    expect(get(params as any).exposure).toBe(-3);
  });

  it("ignores writes when no image is active", () => {
    const activeId = writable<string | null>(null);
    const { params, editsById } = createPerImageParams(activeId, mk);
    params.update((p) => ({ ...p, exposure: 9 }));
    expect(get(editsById)).toEqual({});
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/perImage.test.ts`
Expected: FAIL — `./perImage` has no such exports yet (module not found / undefined).

- [ ] **Step 3: Implement `perImage.ts`**

Create `app/src/lib/perImage.ts`:

```ts
import { writable, derived, type Readable, type Writable } from "svelte/store";
import type { InvertParams } from "./api";

/** The active image's params, or a fresh default if it has no edits yet. */
export function entryFor(
  map: Record<string, InvertParams>,
  id: string | null,
  makeDefault: () => InvertParams,
): InvertParams {
  return (id !== null && map[id]) || makeDefault();
}

/** Store interface matching a writable<InvertParams> (subscribe/set/update). */
export interface ParamsStore {
  subscribe: Readable<InvertParams>["subscribe"];
  set: (p: InvertParams) => void;
  update: (fn: (p: InvertParams) => InvertParams) => void;
}

/**
 * Per-image params: `$params` reads the active image's entry; set/update write
 * only to the active image. New images lazily resolve to makeDefault().
 */
export function createPerImageParams(
  activeId: Readable<string | null>,
  makeDefault: () => InvertParams,
): { params: ParamsStore; editsById: Writable<Record<string, InvertParams>> } {
  const editsById = writable<Record<string, InvertParams>>({});
  let activeIdVal: string | null = null;
  activeId.subscribe((v) => (activeIdVal = v));

  const view = derived([editsById, activeId], ([m, id]) => entryFor(m, id, makeDefault));

  const params: ParamsStore = {
    subscribe: view.subscribe,
    set: (p) => {
      if (activeIdVal !== null) editsById.update((m) => ({ ...m, [activeIdVal as string]: p }));
    },
    update: (fn) => {
      if (activeIdVal !== null)
        editsById.update((m) => ({ ...m, [activeIdVal as string]: fn(entryFor(m, activeIdVal, makeDefault)) }));
    },
  };

  return { params, editsById };
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/perImage.test.ts`
Expected: PASS (4 assertions across 3 tests).

- [ ] **Step 5: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/perImage.ts app/src/lib/perImage.test.ts
git commit -m "feat(app): per-image params helper + store factory (tested)"
```

---

## Task 2: Wire `store.ts` to per-image params

**Files:**
- Modify: `app/src/lib/store.ts`

- [ ] **Step 1: Replace the global `params` writable**

In `app/src/lib/store.ts`:

Add to the imports at the top (after the existing `import { defaultParams } from "./api";`):
```ts
import { createPerImageParams } from "./perImage";
```

Replace this line (currently line 8):
```ts
export const params = writable<InvertParams>(defaultParams());
```
with:
```ts
// Per-image edits: $params is the ACTIVE image's params; writes go to the active
// image only. activeId is declared above, which createPerImageParams subscribes to.
const _perImage = createPerImageParams(activeId, defaultParams);
export const params = _perImage.params;
export const editsById = _perImage.editsById;
```

(`activeId` is already declared on line 6, before this line — required, since the
factory subscribes to it.)

- [ ] **Step 2: Typecheck**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run check 2>&1 | tail -20`
Expected: NO new errors. The `InvertParams` import in store.ts may now be unused
(only `defaultParams` is referenced) — if svelte-check/tsc flags
`'InvertParams' is declared but never used`, fix it by removing `InvertParams` from
the `import type { ImageEntry, InvertParams, Quality }` line (keep `ImageEntry` and
`Quality`). Re-run `npm run check` to confirm clean. Ignore the pre-existing
`workflow.test.ts` `path` ERROR and a11y WARNINGS.

- [ ] **Step 3: Run the full unit-test suite**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run`
Expected: all pass (existing tests + the new `perImage` tests).

- [ ] **Step 4: Manual smoke (user, in the running app)**

In Develop with ≥2 developed images:
- Edit Exposure (or any slider) on image A, switch to image B → B is unchanged
  (shows its own defaults / its own prior edits), NOT A's edits.
- Switch back to A → A's edit is still there.
- WB Temp/Tint still auto-seed per image on first activation.

- [ ] **Step 5: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/store.ts
git commit -m "feat(app): per-image edit params (fix global-params bleed across images)"
```

---

## Self-Review notes

- **Spec coverage (Plan 1 section):** `editsById` + custom store (Task 2);
  `entryFor` pure helper unit-tested + per-image isolation behavioral test
  (Task 1); new images lazily default, WB seed unchanged (relies on existing
  `Basic.svelte`, untouched). Edge case "writes with no active image are a no-op"
  is tested (Task 1, test 3).
- **Placeholder scan:** none — all steps contain full code/commands.
- **Type consistency:** `entryFor(map, id, makeDefault)` and
  `createPerImageParams(activeId, makeDefault) -> { params, editsById }` are used
  identically in the test (Task 1) and in `store.ts` (Task 2). `ParamsStore`
  exposes `subscribe`/`set`/`update`, matching every existing `params` consumer
  (`$params`, `params.update`, `bind:value={$params.X}` → `set`).
- **Known carry-over:** the pre-existing `app/src/lib/workflow.test.ts` `path`
  fixture error is unrelated and out of scope.
