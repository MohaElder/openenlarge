# RedRoom — Library Redesign + App Polish Design

**Date:** 2026-06-04
**Status:** Approved (design phase; mockup validated via visual companion)
**Depends on:** RedRoom UI + develop-workflow specs

## Goals

Redesign the Library module and apply app-level polish, per validated mockup
(`.superpowers/brainstorm/.../library-layout-v2.html`):

1. App opens at **90% of the screen**; window/product name is **RedRoom** (not "app").
2. Library left panel = a **macOS-style folder navigator** (Lucide icons) of imported files'
   locations, replacing the flat file-name list. Selecting a folder filters the center grid.
   **Import** button (label just "Import") pinned at the bottom.
3. Library center = a **zoomable thumbnail grid** (replaces the bottom filmstrip in Library
   only; Develop keeps its filmstrip). A "Thumb size" slider scales cells.
4. Grid **just shows images** — no badges/pills/wash. Developed images show the inverted
   positive; undeveloped show the raw negative (as-is).
5. **Thumbnails update live** to reflect the developed/edited result — when the active image's
   adjustments change (white/black tone, etc.), its grid thumbnail re-renders.
6. **Develop tab** shows a **count badge** (top-right) of undeveloped images.
7. General **polish**: tighter spacing, Lucide iconography, refined glass/red, typography.

Persistence (remembering folders/images across launches) is **explicitly out of scope** here —
a separate task. The navigator is built from the **current session's** imported paths.

## App-level changes

- **Window 90% + name** — `app/src-tauri/tauri.conf.json`: set `productName` to `RedRoom` and
  the main window `title` to `RedRoom`. Open at 90%: since a static config can't read screen
  size, set the window `visible: false` initially and, in `lib.rs` `run()` setup, compute the
  primary monitor size and `set_size`/`center` to 90% before showing. (Tauri 2:
  `window.primary_monitor()`, `window.set_size(LogicalSize)`, `window.center()`, `window.show()`.)
- **Lucide icons** — add `lucide-svelte` (or inline a tiny set of SVG icon components). Use for
  the tree (hard-drive, folder, chevron), Import (plus), and elsewhere as polish.

## Frontend architecture

```
app/src/lib/
├── api.ts            ImageEntry gains `path: string` (backend already has it) so the
│                     navigator can group by folder.
├── library/
│   ├── folderTree.ts        pure: buildTree(entries) -> tree of {name, fullPath, children, imageIds}
│   ├── folderTree.test.ts   vitest for buildTree + grouping
│   ├── FolderNav.svelte     the macOS-style tree (Lucide icons); emits selected folder path
│   └── Grid.svelte          zoomable thumbnail grid for the selected folder
├── icons/  (Icon.svelte or lucide-svelte usage)
├── store.ts          add: selectedFolder writable (string|null); gridZoom writable (number);
│                     derived undevelopedCount.
├── tabs/Library.svelte   left=FolderNav, center=Grid, right=Metadata (no filmstrip)
└── tabs/Develop.svelte   unchanged (keeps Viewport + Filmstrip)
```

### Folder navigator
- `buildTree(entries)`: split each `entry.path` into components, build a nested tree; leaf
  folders carry the ids of images directly inside them. Volumes (`/Volumes/X`, `/`→"Macintosh
  HD") are roots. Collapse single-child chains is NOT done (show full macOS-like hierarchy).
- `FolderNav.svelte`: renders the tree with disclosure chevrons (expand/collapse local state),
  Lucide hard-drive/folder icons, per-folder image counts (recursive). Clicking a folder sets
  `selectedFolder`. Import button at the bottom (existing import logic from Source.svelte moves
  here; label "Import").
- Default selection: the folder of the most recently imported image (or first root with images).

### Grid
- `Grid.svelte`: shows thumbnails for images whose folder == `selectedFolder` (direct children).
- CSS grid; cell size driven by `gridZoom` (slider 0–100 → column min-width, e.g. 120–320px).
- Each cell = the image's **current thumbnail** (`entry.thumbnail`). Selecting a cell sets
  `activeId`. Double-click (or a button) → develop+open in Develop (optional; v1 single-click
  selects, Develop tab opens the flow). Hover lift; selected = red ring.
- No badges/pills/wash.

### Live thumbnail updates
- After `develop_image`, the returned entry's thumbnail should become the **inverted** preview
  (not the raw embedded preview). Backend change: `develop_image` also renders a small inverted
  thumbnail (using default params) and sets it as the entry's `thumbnail`. So once developed, the
  Library grid shows positives.
- While editing in Develop, when the **active image's** params change, re-render its grid
  thumbnail (debounced) and update the store entry's `thumbnail`, so the grid reflects tone
  edits. Implemented via a new lightweight `thumbnail(id, params)` command returning a small JPEG
  (e.g. 320px) from the developed working image — reuses `render_view` with a fit ViewSpec at
  thumbnail size; the frontend writes the result into `images[id].thumbnail`.

## Backend changes

- `ImageEntry` += `path: String` (expose the already-stored path).
- `develop_image`: after building `Developed`, render a ~320px **inverted** thumbnail with
  default params and store it on the entry/cached image as `thumbnail` (replacing the raw
  embedded preview). Return entry with the new thumbnail + `path`.
- New command `thumbnail(id, params) -> String`: render a ~320px inverted JPEG of the developed
  image at the given params (fit view). Used for live grid updates while editing. Returns
  `Err("not developed")` if not developed.
- `render_view`/`export_image` unchanged except they already operate on developed images.

## Store additions

```ts
export const selectedFolder = writable<string | null>(null);
export const gridZoom = writable<number>(55);
export const undevelopedCount = derived(images, ($i) => $i.filter((x) => !x.developed).length);
```

## Data flow

```
import_image -> entry{path, thumbnail=embedded} -> images store -> buildTree -> FolderNav
select folder -> selectedFolder -> Grid shows that folder's images
develop_image -> entry.thumbnail = inverted thumb -> grid cell flips to positive
edit active image (params change) -> thumbnail(id,params) (debounced) -> update images[id].thumbnail
Develop tab badge = undevelopedCount
```

## Error handling

- A path that doesn't fit the tree (no parent) → placed under a synthetic "Other" root.
- `thumbnail`/`develop` errors → keep the previous thumbnail; toast on develop failure (existing).
- Window-size setup failure (no monitor info) → fall back to the configured default size, still
  named RedRoom.

## Testing

- **vitest:** `buildTree` groups paths into the right hierarchy with correct per-folder image
  ids/counts; `undevelopedCount` derivation; gridZoom → column-size mapping (if extracted as a
  pure helper).
- **Backend (Rust unit):** none new strictly required (thumbnail/develop need real files);
  existing tests stay green.
- **Manual E2E:** import from two folders → tree shows both with counts → select each → grid
  filters; thumb-size slider scales; develop → thumbnails flip to positives; edit white/black →
  active thumbnail updates; Develop badge counts undeveloped; window opens at 90% titled RedRoom.

## Scope

**In:** window 90% + RedRoom name; folder navigator (session paths, Lucide icons); zoomable grid
replacing the Library filmstrip; grid shows raw/positive with no badges; live thumbnail updates
on develop + edit; Develop-tab undeveloped badge; polish pass (spacing/icons/typography).

**Out (later/separate):** **file persistence** (remembering imports across launches);
drag-and-drop import; multi-select in the grid; folder-level operations (develop-folder); grid
sorting/filtering; true filesystem browsing of non-imported folders.

## Assumptions

1. The navigator reflects only imported files this session (persistence is a separate task).
2. `lucide-svelte` is acceptable as a dependency (else inline a handful of SVG icons).
3. Live per-edit thumbnail refresh for the **active** image only is sufficient (not all images
   re-rendered on every edit).
