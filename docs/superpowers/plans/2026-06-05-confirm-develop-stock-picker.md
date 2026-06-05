# Film-Stock Picker on Confirm Develop — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional film-stock picker to the Confirm Develop dialog that applies the chosen stock to every image being developed in that run.

**Architecture:** A pure `applyStockToIds` helper batch-sets `params.stock` in the `editsById` map; `developAll(stock?)` calls it for the undeveloped folder images before developing; `ConfirmDevelop.svelte` gains an optional `<select>` and emits the choice; `+page.svelte` passes it through. Persistence reuses the existing debounced `editsById` write-through.

**Tech Stack:** Svelte 5 + TypeScript, Vitest, i18n via `i18n-strings.csv` + `scripts/gen-i18n.py`.

**Spec:** `docs/superpowers/specs/2026-06-05-confirm-develop-stock-picker-design.md`

---

## File Structure

- `app/src/lib/workflow.ts` — add pure `applyStockToIds`; `developAll` gains an optional `stock` arg.
- `app/src/lib/workflow.test.ts` — tests for `applyStockToIds` and the `developAll` stock path.
- `app/src/lib/overlay/ConfirmDevelop.svelte` — optional stock `<select>`; emit `{stock}` on confirm.
- `app/src/routes/+page.svelte` — pass emitted stock into `developAll`.
- `i18n-strings.csv` + regenerated `app/src/lib/i18n/dict.ts` — two new `confirmDevelop.*` strings.

---

## Task 1: Pure `applyStockToIds` helper

**Files:**
- Modify: `app/src/lib/workflow.ts`
- Test: `app/src/lib/workflow.test.ts`

- [ ] **Step 1: Write the failing test.** Append to `app/src/lib/workflow.test.ts`. First add imports at the top of the file (the file currently imports `undevelopedIds` and `ImageEntry`; add `applyStockToIds` and `defaultParams`):

```typescript
import { undevelopedIds, applyStockToIds } from "./workflow";
import { defaultParams } from "./api";
```

Then add this describe block at the end of the file:

```typescript
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
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `cd app && npm run test:unit -- workflow`
Expected: FAIL — `applyStockToIds` is not exported.

- [ ] **Step 3: Implement `applyStockToIds`** in `app/src/lib/workflow.ts`. Add the `InvertParams` type to the existing `./api` import (currently `import { api, type ImageEntry } from "./api";` → add `type InvertParams`), then add the function after `undevelopedIds`:

```typescript
/** Return a new edits map with `stock` set on each id in `ids` (seeding absent
 * ids from `makeDefault()`). Pure — does not mutate the input map. */
export function applyStockToIds(
  editsMap: Record<string, InvertParams>,
  ids: string[],
  stock: string,
  makeDefault: () => InvertParams,
): Record<string, InvertParams> {
  if (ids.length === 0) return editsMap;
  const out = { ...editsMap };
  for (const id of ids) {
    out[id] = { ...(out[id] ?? makeDefault()), stock: stock as InvertParams["stock"] };
  }
  return out;
}
```

- [ ] **Step 4: Run the test to verify it passes.**

Run: `cd app && npm run test:unit -- workflow`
Expected: PASS (the `undevelopedIds` tests + 3 new `applyStockToIds` tests).

- [ ] **Step 5: Commit.**

```bash
git add app/src/lib/workflow.ts app/src/lib/workflow.test.ts
git commit -m "feat(develop): applyStockToIds pure helper"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Task 2: `developAll(stock?)` applies the stock

**Files:**
- Modify: `app/src/lib/workflow.ts`
- Test: `app/src/lib/workflow.test.ts`

- [ ] **Step 1: Write the failing test.** Add to `app/src/lib/workflow.test.ts`. At the VERY TOP of the file (before other imports, because `vi.mock` is hoisted) add the api mock and the extra imports:

```typescript
import { describe, it, expect, vi } from "vitest";
import { get } from "svelte/store";

vi.mock("./api", async (orig) => {
  const actual = await orig<typeof import("./api")>();
  return { ...actual, api: { ...actual.api, developImage: vi.fn(async (id: string) => ({
    id, path: `/x/${id}.dng`, file_name: `${id}.dng`, thumbnail: "t",
    metadata: { width: 10, height: 10, file_size: 0 }, developed: true, has_ir: false, offline: false,
  })) } };
});
```

Then add this describe block at the end of the file:

```typescript
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
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `cd app && npm run test:unit -- workflow`
Expected: FAIL — `developAll` takes no args / does not set stock.

- [ ] **Step 3: Implement.** In `app/src/lib/workflow.ts`, add `editsById` and `defaultParams` to the imports if not already present:
  - The store import line `import { images, activeId, module, developProgress, editsById, cropById, dustById, folderImages } from "./store";` already includes `editsById`.
  - Add `defaultParams` to the api import: `import { api, defaultParams, type ImageEntry, type InvertParams } from "./api";`.

Then change the `developAll` signature + add the stock step. Replace the start of `developAll`:

```typescript
export async function developAll(stock?: string): Promise<void> {
  const ids = undevelopedIds(get(folderImages));
  if (ids.length === 0) { module.set("develop"); return; }
  if (stock && stock !== "none") {
    editsById.update((m) => applyStockToIds(m, ids, stock, defaultParams));
  }
  developProgress.set({ active: true, done: 0, total: ids.length });
```

(The rest of `developAll` is unchanged.)

- [ ] **Step 4: Run the test to verify it passes.**

Run: `cd app && npm run test:unit -- workflow`
Expected: PASS (all workflow tests).

- [ ] **Step 5: Run full unit suite + typecheck.**

Run: `cd app && npm run test:unit && npm run check`
Expected: all pass; 0 type errors.

- [ ] **Step 6: Commit.**

```bash
git add app/src/lib/workflow.ts app/src/lib/workflow.test.ts
git commit -m "feat(develop): developAll applies chosen stock to undeveloped frames"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Task 3: i18n strings

**Files:**
- Modify: `i18n-strings.csv`, regenerated `app/src/lib/i18n/dict.ts`

- [ ] **Step 1: Add CSV rows.** In `/Users/mohaelder/Repos/filmrev/i18n-strings.csv`, after the `confirmDevelop.confirm` row (the `confirmDevelop.*` group), add two rows. The columns are `key,en,zh,file,note`:

```csv
confirmDevelop.filmStock,"Film stock","胶片型号","src/lib/overlay/ConfirmDevelop.svelte","label"
confirmDevelop.filmStockOptional,"optional","可选","src/lib/overlay/ConfirmDevelop.svelte","hint"
```

- [ ] **Step 2: Regenerate dict.ts.**

Run: `cd /Users/mohaelder/Repos/filmrev && python3 scripts/gen-i18n.py`
Expected: regenerates `app/src/lib/i18n/dict.ts`. Verify the new keys are present in both locales: `grep -c "confirmDevelop.filmStock\b" app/src/lib/i18n/dict.ts` should print `2` (one per locale), and `grep -c "confirmDevelop.filmStockOptional" app/src/lib/i18n/dict.ts` should print `2`.

- [ ] **Step 3: Typecheck.**

Run: `cd app && npm run check`
Expected: 0 errors.

- [ ] **Step 4: Commit.**

```bash
git add i18n-strings.csv app/src/lib/i18n/dict.ts
git commit -m "feat(develop): i18n strings for the confirm-develop stock picker"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Task 4: Dialog picker + wire-through

**Files:**
- Modify: `app/src/lib/overlay/ConfirmDevelop.svelte`, `app/src/routes/+page.svelte`

- [ ] **Step 1: Add the stock select to `ConfirmDevelop.svelte`.** Add a local `stock` var and emit it on confirm. Replace the full `<script>` and the dialog markup body:

In `<script>` (after `const dispatch = createEventDispatcher();`):

```typescript
  let stock = "none";
```

Change the confirm button line so it emits the choice:

```svelte
      <button class="go" on:click={() => dispatch("confirm", { stock })}>{$t('confirmDevelop.confirm')}</button>
```

Insert the stock picker between the `.sub` div and the `.row` div (after the `<div class="sub">…</div>` line):

```svelte
    <div class="stock">
      <label for="cd-stock">{$t('confirmDevelop.filmStock')} <span class="opt">({$t('confirmDevelop.filmStockOptional')})</span></label>
      <select id="cd-stock" bind:value={stock}>
        <option value="none">{$t('basic.noFilmProfile')}</option>
        <option value="portra400">{$t('basic.stock.portra400')}</option>
        <option value="fujic200">{$t('basic.stock.fujic200')}</option>
      </select>
    </div>
```

Add styles to the `<style>` block (before the closing `</style>`):

```css
  .stock { display: flex; flex-direction: column; gap: 6px; margin-bottom: 18px; }
  .stock label { font-size: 12px; color: var(--text-dim); }
  .stock .opt { color: var(--text-faint); }
  .stock select { width: 100%; padding: 8px 10px; border-radius: 9px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); font-size: 13px; }
```

- [ ] **Step 2: Wire the emitted stock in `+page.svelte`.** Change the `ConfirmDevelop` `on:confirm` handler (currently `on:confirm={() => { confirming = false; developAll(); }}`) to:

```svelte
  <ConfirmDevelop count={confirmCount}
    on:confirm={(e) => { confirming = false; developAll(e.detail?.stock); }}
    on:cancel={() => (confirming = false)} />
```

- [ ] **Step 3: Typecheck + unit tests.**

Run: `cd app && npm run check && npm run test:unit`
Expected: 0 errors; all tests pass. (Svelte may emit a pre-existing a11y warning style; only ERRORS matter.)

- [ ] **Step 4: Manual verification** (restart `cd app && npm run tauri dev`):
  - Import a fresh roll → "Develop all" → the dialog shows a "Film stock (optional)" select defaulting to "No film profile".
  - Pick **Kodak Portra 400** → Develop all → open several frames: each inverts with Portra (Film Profile = Portra 400 in the Basic panel) and persists across restart.
  - Repeat with **No film profile** selected → frames develop with no stock (unchanged behavior).
  - Already-developed frames in the folder (if any) keep their previous stock.

- [ ] **Step 5: Commit.**

```bash
git add app/src/lib/overlay/ConfirmDevelop.svelte app/src/routes/+page.svelte
git commit -m "feat(develop): optional film-stock picker in Confirm Develop dialog"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Self-Review Notes (verified during planning)

- **Spec coverage:** optional picker in dialog (Task 4); apply to undeveloped-only scope (Task 2 uses `undevelopedIds(folderImages)`); per-image `params.stock` batch-set via pure helper (Task 1); persistence via existing write-through (no new code — `editsById.update` triggers it); default `none` = no-op (Task 2 guard); strings (Task 3); option labels reuse `basic.*` keys (Task 4).
- **Type consistency:** `applyStockToIds(editsMap, ids, stock, makeDefault)` signature identical in Task 1 (impl) and Task 2 (caller); `developAll(stock?: string)`; stock cast to `InvertParams["stock"]`; dialog emits `{ stock }` and `+page.svelte` reads `e.detail?.stock`.
- **Edge cases:** empty ids (helper returns input map, develop early-returns); none/omitted stock (guard skips the update); absent editsById entry (seeded from `defaultParams()`).
- **Known limitation (documented, out of scope):** the develop-time grid thumbnail uses default params, so it reflects the stock only after the frame is opened in Develop.
