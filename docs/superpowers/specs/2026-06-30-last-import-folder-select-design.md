# Last Import: auto-select the just-imported batch's folder

**Issue:** [#19](https://github.com/MohaElder/openenlarge/issues/19) — after each import the user must manually hunt the newly-imported folder in the tree, which is confusing. Mirror Lightroom's "Last Import" intent: land on the new frames automatically.

## Decision

Auto-**select the folder** of the just-imported batch (the folder-based model), not a virtual "Last Import" collection. The catalog is reference-based — importing a roll/folder lands all frames under one real parent folder — so selecting that folder *is* the imported batch in the common case.

## Where

One hook at the end of `importPaths()` in `app/src/lib/workflow.ts`. This is the shared chokepoint for all three import entry points (file picker, folder picker, drag-and-drop), so no per-caller changes are needed.

## Logic

1. `importOne()` returns the resulting `ImageEntry` on success, `null` on failure (today it returns `void`). `importPaths()` collects results positionally so order follows the input `paths`.
2. After all imports resolve, derive each succeeded entry's directory via the existing `imageDir()` helper.
3. Compute the **target folder** = the directory containing the most newly-imported frames (modal). Ties broken toward the **latest** frame (the folder whose last imported frame appears latest in input order). Single-folder batch → that folder.
4. If a target exists, call `selectFolder(target)` — which already cascades grid, filmstrip, and active image.
5. If zero files imported (all failed / empty input), leave the current selection untouched.

The folder-picking rule lives in a new pure helper `pickImportFolder(dirs: string[]): string | null` in `folderScope.ts`, unit-tested independently.

## Behavior notes

- **Unconditional** after a successful import: always jump to the batch, even if the user was mid-work in another folder. This is the Lightroom behavior and what the issue asks for.
- The cold-start reactive auto-select in `FolderNav.svelte` (fires only when `!$selectedFolder`) is unaffected — once we explicitly select, it won't re-fire.
- No backend/Rust changes. No new persistent state.

## Testing

- Unit (`folderScope.test.ts`): single-folder → that folder; multi-folder → modal; tie → latest frame's folder; empty → null.
- Manual GUI smoke: import a roll → grid jumps to it.
