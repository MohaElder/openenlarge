# RedRoom Library Redesign + App Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign Library as a macOS-style folder navigator + zoomable thumbnail grid (replacing the filmstrip), with live thumbnails that flip to the inverted/edited result, a Develop-tab undeveloped badge, the window opening at 90% titled "RedRoom", and a polish pass.

**Architecture:** Backend exposes each image's `path` and renders an inverted thumbnail at develop time plus an on-demand `thumbnail(id,params)` for live grid refresh. The frontend builds a folder tree from session paths (pure, tested), renders it with inline Lucide SVG icons, and shows the selected folder's images in a zoomable grid. Styling is ported from the validated mockup.

**Tech Stack:** Rust (Tauri 2 commands + window setup), Svelte 5 (SvelteKit) + TS, vitest, inline Lucide SVGs.

**Reference spec:** `docs/superpowers/specs/2026-06-04-redroom-library-redesign-design.md`
**Reference mockup:** `.superpowers/brainstorm/18092-1780603254/content/library-layout-v2.html`

**Environment:** `/Users/mohaelder/Repos/filmrev`, branch `feat/inversion-poc`. `cargo` not on PATH → prefix `source "$HOME/.cargo/env" && `. Backend test `(cd app/src-tauri && cargo test)`; frontend build `(cd app && npm run build)`; unit `(cd app && npx vitest run)`.

---

## File Structure

```
app/src-tauri/
├── tauri.conf.json   productName/title "RedRoom"; main window visible:false
└── src/
    ├── commands.rs   ImageEntry.path; develop_image renders inverted thumbnail;
    │                 new thumbnail(id,params); default_invert_params()
    ├── session.rs    ImageEntry += path; insert() passes path
    └── lib.rs        setup(): size window to 90% + center + show; register thumbnail
app/src/lib/
├── api.ts            ImageEntry += path; api.thumbnail(id,params)
├── store.ts          selectedFolder, gridZoom, undevelopedCount
├── library/
│   ├── folderTree.ts        buildTree(), countImages()
│   ├── folderTree.test.ts   vitest
│   ├── FolderNav.svelte     tree + Import button
│   ├── TreeNode.svelte      recursive row (svelte:self)
│   └── Grid.svelte          zoomable thumbnail grid
├── icons/Icon.svelte        inline Lucide SVGs
├── tabs/Library.svelte      FolderNav + Grid + Metadata
└── routes/+page.svelte      Develop tab badge = undevelopedCount
app/src/lib/viewport or tabs/Develop.svelte  live thumbnail refresh of active image on edit
```

---

## Task 1: Backend — ImageEntry.path, inverted develop thumbnail, thumbnail command

**Files:** Modify `app/src-tauri/src/session.rs`, `app/src-tauri/src/commands.rs`, `app/src-tauri/src/lib.rs`.

- [ ] **Step 1: Add `path` to ImageEntry (session.rs)**

In `app/src-tauri/src/session.rs`, add `pub path: String,` to `ImageEntry` and set it in
`insert()`:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ImageEntry {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub thumbnail: String,
    pub metadata: Metadata,
    pub developed: bool,
}
```
In `Session::insert`, build the entry with `path: img.path.clone(),` (add the field).

- [ ] **Step 2: commands.rs — default params helper + inverted develop thumbnail + thumbnail command + return path**

In `app/src-tauri/src/commands.rs`:

(a) Add a default-params helper near the top (after the consts):
```rust
fn default_invert_params() -> InvertParams {
    InvertParams {
        mode: "b".into(), stock: "none".into(), base_rect: None,
        exposure: 1.0, black: 0.0, gamma: 0.4545, auto_wb: true, temp: 0.0, tint: 0.0,
    }
}
```

(b) `import_image` already builds `ImageEntry` via `session.insert` — `insert` now includes
`path`, so no change there except ensure `CachedImage.path` is set (it is).

(c) Rewrite `develop_image` so it (1) builds working/thumb/base, (2) renders a ~320px **inverted**
thumbnail with default params, (3) stores that as the image's `thumbnail`, (4) returns the entry
with `path` + new thumbnail:

```rust
#[tauri::command]
pub fn develop_image(id: String, session: State<Session>) -> Result<ImageEntry, String> {
    let cap = session.quality.lock().unwrap().cap();
    let path = {
        let images = session.images.lock().unwrap();
        images.get(&id).ok_or("unknown image id")?.path.clone()
    };
    let full = decode_any(Path::new(&path))?;
    let working = proxy(&full, cap);
    let thumb = proxy(&full, AUTOWB_EDGE);
    let base = sample_base(&working, None);
    let (w, h) = (full.width as u32, full.height as u32);
    drop(full);

    // inverted thumbnail (default look) for the Library grid
    let small = proxy(&working, THUMB_EDGE);
    let ip = resolve_params(&default_invert_params(), &thumb, base);
    let inv_thumb = invert_image(&small, &ip, Mode::B);
    let thumbnail = to_jpeg_b64(&inv_thumb, false, 82)?;

    let mut images = session.images.lock().unwrap();
    let img = images.get_mut(&id).ok_or("unknown image id")?;
    img.metadata.width = w;
    img.metadata.height = h;
    img.thumbnail = thumbnail.clone();
    img.developed = Some(Developed { working, thumb, base });
    Ok(ImageEntry {
        id: id.clone(),
        path: img.path.clone(),
        file_name: img.file_name.clone(),
        thumbnail,
        metadata: img.metadata.clone(),
        developed: true,
    })
}
```

(d) Add the `thumbnail` command (after `render_view`):
```rust
/// Render a small (~320px) inverted JPEG of the developed image at the given
/// params — used to live-refresh the Library grid cell while editing.
#[tauri::command]
pub fn thumbnail(id: String, params: InvertParams, session: State<Session>) -> Result<String, String> {
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let small = proxy(&dev.working, THUMB_EDGE);
    let ip = resolve_params(&params, &dev.thumb, dev.base);
    let inv = invert_image(&small, &ip, mode_from(&params.mode));
    to_jpeg_b64(&inv, false, 82)
}
```
(`THUMB_EDGE` is already 320 in commands.rs.)

- [ ] **Step 3: Register `thumbnail` in lib.rs**

In `app/src-tauri/src/lib.rs` handler list, add `commands::thumbnail,`:
```rust
        .invoke_handler(tauri::generate_handler![
            commands::import_image,
            commands::develop_image,
            commands::set_quality,
            commands::render_view,
            commands::thumbnail,
            commands::export_image,
        ])
```

- [ ] **Step 4: Build + test + clippy**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo build 2>&1 | tail -8 && cargo test 2>&1 | grep 'test result' && cargo clippy 2>&1 | grep -cE 'warning|error' | xargs echo issues:)`
Expected: compiles; tests pass (session insert test still passes — it builds CachedImage with the
existing fields; note the test's CachedImage uses `path` already); clippy clean.

NOTE: the existing `session::tests` `insert_reports_undeveloped_then_assigns_ids` builds a
`CachedImage { path: ..., ... }` — confirm it still has `path`; ImageEntry now also has `path`
but the test only checks `id`/`developed`, so it stays valid.

- [ ] **Step 5: Commit**

```bash
git add app/src-tauri/src/session.rs app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(redroom): expose path, inverted develop thumbnail, thumbnail(id,params) command"
```

---

## Task 2: Window 90% + "RedRoom" name

**Files:** Modify `app/src-tauri/tauri.conf.json`, `app/src-tauri/src/lib.rs`.

- [ ] **Step 1: tauri.conf.json — name + hidden-until-sized window**

In `app/src-tauri/tauri.conf.json`: set top-level `"productName": "RedRoom"`. In the
`app.windows` array's first window object, set `"title": "RedRoom"` and `"visible": false`
(keep other fields). If a `"label"` exists keep it (default "main").

- [ ] **Step 2: lib.rs — size to 90% in setup, then show**

In `app/src-tauri/src/lib.rs`, add a `.setup(...)` to the builder (before `.invoke_handler`):

```rust
        .setup(|app| {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = win.primary_monitor() {
                    let size = monitor.size();
                    let scale = monitor.scale_factor();
                    let w = (size.width as f64 * 0.9) / scale;
                    let h = (size.height as f64 * 0.9) / scale;
                    let _ = win.set_size(tauri::LogicalSize::new(w, h));
                    let _ = win.center();
                }
                let _ = win.show();
            }
            Ok(())
        })
```
(Requires `tauri::Manager` in scope — the `use` inside the closure handles it. `LogicalSize` is
`tauri::LogicalSize`.)

- [ ] **Step 3: Build + run check**

Run: `source "$HOME/.cargo/env" && (cd app/src-tauri && cargo build 2>&1 | tail -6)`
Expected: compiles. (Window-size behavior is verified manually in Task 6.)

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/tauri.conf.json app/src-tauri/src/lib.rs
git commit -m "feat(redroom): open window at 90% of screen, named RedRoom"
```

---

## Task 3: Frontend api/store + folder tree (vitest)

**Files:** Modify `app/src/lib/api.ts`, `app/src/lib/store.ts`; create `app/src/lib/library/folderTree.ts`, `app/src/lib/library/folderTree.test.ts`.

- [ ] **Step 1: api.ts — path + thumbnail**

In `app/src/lib/api.ts`: add `path: string` to `ImageEntry`:
```ts
export interface ImageEntry {
  id: string; path: string; file_name: string; thumbnail: string; metadata: Metadata; developed: boolean;
}
```
Add to the `api` object:
```ts
  thumbnail: (id: string, params: InvertParams) => invoke<string>("thumbnail", { id, params }),
```

- [ ] **Step 2: store.ts — selectedFolder, gridZoom, undevelopedCount**

Append to `app/src/lib/store.ts`:
```ts
export const selectedFolder = writable<string | null>(null);
export const gridZoom = writable<number>(55);
export const undevelopedCount = derived(images, ($i) => $i.filter((x) => !x.developed).length);
```
(`derived` is already imported in store.ts.)

- [ ] **Step 3: folderTree.ts**

Create `app/src/lib/library/folderTree.ts`:
```ts
export interface FolderNode {
  name: string;
  fullPath: string;
  children: FolderNode[];
  imageIds: string[];
}

/** Build a macOS-style folder tree from imported image paths. Roots are volumes
 * (/Volumes/X) or "Macintosh HD" for everything else. Each folder lists the ids
 * of images directly inside it. */
export function buildTree(entries: { id: string; path: string }[]): FolderNode[] {
  const roots: FolderNode[] = [];
  const byPath = new Map<string, FolderNode>();
  const ensure = (fullPath: string, name: string, parent: FolderNode[]): FolderNode => {
    let n = byPath.get(fullPath);
    if (!n) { n = { name, fullPath, children: [], imageIds: [] }; byPath.set(fullPath, n); parent.push(n); }
    return n;
  };
  for (const e of entries) {
    const parts = e.path.replace(/\\/g, "/").split("/").filter(Boolean);
    parts.pop(); // drop filename
    let rootName: string, rootPath: string, dirParts: string[];
    if (parts[0] === "Volumes" && parts.length >= 2) {
      rootName = parts[1]; rootPath = "/Volumes/" + parts[1]; dirParts = parts.slice(2);
    } else {
      rootName = "Macintosh HD"; rootPath = "/MacintoshHD"; dirParts = parts;
    }
    let node = ensure(rootPath, rootName, roots);
    let acc = rootPath;
    for (const d of dirParts) {
      acc = acc + "/" + d;
      node = ensure(acc, d, node.children);
    }
    node.imageIds.push(e.id);
  }
  return roots;
}

/** Total images in a folder subtree (recursive). */
export function countImages(node: FolderNode): number {
  return node.imageIds.length + node.children.reduce((s, c) => s + countImages(c), 0);
}
```

- [ ] **Step 4: folderTree.test.ts**

```ts
import { describe, it, expect } from "vitest";
import { buildTree, countImages, type FolderNode } from "./folderTree";

const find = (nodes: FolderNode[], name: string): FolderNode | undefined => {
  for (const n of nodes) {
    if (n.name === name) return n;
    const f = find(n.children, name);
    if (f) return f;
  }
};

describe("buildTree", () => {
  const tree = buildTree([
    { id: "a", path: "/Volumes/Disk2/Film Scans/ny2026/1.dng" },
    { id: "b", path: "/Volumes/Disk2/Film Scans/ny2026/2.dng" },
    { id: "c", path: "/Volumes/Disk2/Film Scans/ny2027/3.dng" },
    { id: "d", path: "/Users/me/scans/4.dng" },
  ]);

  it("creates a volume root and a Macintosh HD root", () => {
    expect(tree.map((n) => n.name).sort()).toEqual(["Disk2", "Macintosh HD"]);
  });
  it("groups images by their folder", () => {
    expect(find(tree, "ny2026")!.imageIds.sort()).toEqual(["a", "b"]);
    expect(find(tree, "ny2027")!.imageIds).toEqual(["c"]);
    expect(find(tree, "scans")!.imageIds).toEqual(["d"]);
  });
  it("countImages sums the subtree", () => {
    expect(countImages(find(tree, "Film Scans")!)).toBe(3);
    expect(countImages(find(tree, "Disk2")!)).toBe(3);
  });
});
```

- [ ] **Step 5: Run vitest**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npx vitest run src/lib/library/folderTree.test.ts`
Expected: 3 PASS.

- [ ] **Step 6: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/api.ts app/src/lib/store.ts app/src/lib/library/folderTree.ts app/src/lib/library/folderTree.test.ts
git commit -m "feat(redroom): api path/thumbnail, library stores, tested buildTree"
```

---

## Task 4: Icon + FolderNav + Grid components

**Files:** Create `app/src/lib/icons/Icon.svelte`, `app/src/lib/library/TreeNode.svelte`, `app/src/lib/library/FolderNav.svelte`, `app/src/lib/library/Grid.svelte`.

- [ ] **Step 1: Icon.svelte (inline Lucide)**

```svelte
<script lang="ts">
  export let name: string;
  export let size = 15;
  const paths: Record<string, string> = {
    "chevron-down": '<polyline points="6 9 12 15 18 9"/>',
    "chevron-right": '<polyline points="9 18 15 12 9 6"/>',
    folder: '<path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"/>',
    "hard-drive": '<line x1="22" x2="2" y1="12" y2="12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><line x1="6" x2="6.01" y1="16" y2="16"/><line x1="10" x2="10.01" y1="16" y2="16"/>',
    plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
  };
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
  stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex:0 0 auto">
  {@html paths[name] ?? ""}
</svg>
```

- [ ] **Step 2: TreeNode.svelte (recursive)**

```svelte
<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { selectedFolder } from "../store";
  import { countImages, type FolderNode } from "./folderTree";
  export let node: FolderNode;
  export let depth = 0;
  export let isRoot = false;
  let open = true;
  $: hasChildren = node.children.length > 0;
  $: count = countImages(node);
</script>

<div class="row" class:sel={$selectedFolder === node.fullPath}
  style="padding-left:{8 + depth * 16}px"
  on:click={() => { selectedFolder.set(node.fullPath); if (hasChildren) open = !open; }}>
  <span class="chev">
    {#if hasChildren}<Icon name={open ? "chevron-down" : "chevron-right"} size={12} />{/if}
  </span>
  <Icon name={isRoot ? "hard-drive" : "folder"} />
  <span class="lbl">{node.name}</span>
  {#if count > 0}<span class="ct">{count}</span>{/if}
</div>
{#if open}
  {#each node.children as child}
    <svelte:self node={child} depth={depth + 1} />
  {/each}
{/if}

<style>
  .row { display: flex; align-items: center; gap: 7px; padding: 6px 8px; border-radius: 8px;
    color: var(--text-dim); cursor: pointer; white-space: nowrap; }
  .row:hover { background: rgba(255,255,255,0.04); }
  .row.sel { background: rgba(255,255,255,0.07); color: var(--text); }
  .chev { color: var(--text-faint); display: inline-flex; width: 12px; }
  .lbl { overflow: hidden; text-overflow: ellipsis; }
  .ct { margin-left: auto; font-size: 11px; color: var(--text-faint); padding-left: 8px; }
</style>
```

- [ ] **Step 3: FolderNav.svelte**

```svelte
<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { images, activeId, selectedFolder } from "../store";
  import { buildTree } from "./folderTree";
  import TreeNode from "./TreeNode.svelte";
  import Icon from "../icons/Icon.svelte";
  import GlassPanel from "../glass/GlassPanel.svelte";

  let importing = false;
  $: tree = buildTree($images);
  // default-select the folder of the most recent import if nothing selected
  $: if (!$selectedFolder && $images.length) {
    const last = $images[$images.length - 1];
    const dir = last.path.replace(/\\/g, "/").split("/").slice(0, -1).join("/");
    selectedFolder.set(dir);
  }

  async function pickAndImport() {
    const sel = await openDialog({ multiple: true, filters: [{ name: "Film scans", extensions: ["dng", "tif", "tiff", "raf"] }] });
    if (!sel) return;
    const paths = Array.isArray(sel) ? sel : [sel];
    importing = true;
    for (const path of paths) {
      try {
        const entry = await api.importImage(path as string);
        images.update((xs) => [...xs, entry]);
        activeId.update((id) => id ?? entry.id);
      } catch (e) { console.error(e); }
    }
    importing = false;
  }
</script>

<GlassPanel>
  <div class="wrap">
    <div class="ttl">Imported</div>
    <div class="tree">
      {#each tree as root}<TreeNode node={root} isRoot={true} />{/each}
      {#if $images.length === 0}<div class="empty">No images yet</div>{/if}
    </div>
    <button class="import" on:click={pickAndImport} disabled={importing}>
      <Icon name="plus" /> {importing ? "Importing…" : "Import"}
    </button>
  </div>
</GlassPanel>

<style>
  .wrap { display: flex; flex-direction: column; height: 100%; }
  .ttl { font-size: 11px; text-transform: uppercase; letter-spacing: 0.7px; color: var(--text-faint); padding: 2px 6px 10px; }
  .tree { flex: 1; overflow: auto; }
  .empty { color: var(--text-faint); padding: 8px; }
  .import { margin-top: 10px; width: 100%; padding: 11px; border: 0; border-radius: 11px;
    background: var(--accent); color: #fff; font: inherit; font-weight: 700; cursor: pointer;
    display: flex; align-items: center; justify-content: center; gap: 7px;
    box-shadow: 0 6px 18px rgba(224,52,52,0.35); }
  .import:disabled { opacity: 0.6; }
</style>
```

- [ ] **Step 4: Grid.svelte**

```svelte
<script lang="ts">
  import { images, activeId, selectedFolder, gridZoom } from "../store";
  // images whose containing folder == selectedFolder
  $: shown = $images.filter((i) => {
    const dir = i.path.replace(/\\/g, "/").split("/").slice(0, -1).join("/");
    return dir === $selectedFolder;
  });
  $: minCol = 120 + ($gridZoom / 100) * 200; // 120–320px
</script>

<div class="center">
  <div class="head">
    <div class="where"><b>{$selectedFolder?.split("/").pop() ?? "—"}</b> · {shown.length} image{shown.length === 1 ? "" : "s"}</div>
    <div class="right">Thumb size <input class="zoom" type="range" min="0" max="100" bind:value={$gridZoom} /></div>
  </div>
  <div class="grid" style="grid-template-columns:repeat(auto-fill,minmax({minCol}px,1fr))">
    {#each shown as img (img.id)}
      <button class="cell" class:sel={$activeId === img.id} on:click={() => activeId.set(img.id)}>
        <img src={img.thumbnail} alt={img.file_name} />
      </button>
    {/each}
    {#if shown.length === 0}<div class="empty">Select a folder with images</div>{/if}
  </div>
</div>

<style>
  .center { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .head { display: flex; align-items: center; gap: 12px; padding: 2px 4px 12px; }
  .where { color: var(--text-dim); } .where b { color: var(--text); }
  .right { margin-left: auto; display: flex; align-items: center; gap: 9px; color: var(--text-faint); font-size: 12px; }
  .zoom { appearance: none; width: 120px; height: 4px; border-radius: 2px; background: rgba(255,255,255,0.14); outline: 0; }
  .zoom::-webkit-slider-thumb { appearance: none; width: 13px; height: 13px; border-radius: 50%; background: var(--accent); }
  .grid { flex: 1; overflow: auto; display: grid; gap: 12px; align-content: start; padding-right: 4px; }
  .cell { padding: 0; border: 1px solid var(--glass-brd); border-radius: 11px; overflow: hidden;
    aspect-ratio: 3/2; background: #111; cursor: pointer; transition: transform 0.12s, box-shadow 0.12s; }
  .cell:hover { transform: translateY(-2px); box-shadow: 0 12px 26px rgba(0,0,0,0.5); }
  .cell.sel { box-shadow: 0 0 0 2px var(--accent), 0 12px 26px rgba(0,0,0,0.5); }
  .cell img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .empty { color: var(--text-faint); padding: 16px; }
</style>
```

- [ ] **Step 5: Build**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6`
Expected: builds (components not yet wired into Library — that's Task 5; they must compile).

- [ ] **Step 6: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src/lib/icons app/src/lib/library/Icon.svelte app/src/lib/library/TreeNode.svelte app/src/lib/library/FolderNav.svelte app/src/lib/library/Grid.svelte 2>/dev/null; git add app/src/lib
git commit -m "feat(redroom): Icon, recursive TreeNode, FolderNav, zoomable Grid"
```

---

## Task 5: Wire Library + Develop badge + add the theme token + live thumbnail refresh

**Files:** Modify `app/src/lib/tabs/Library.svelte`, `app/src/routes/+page.svelte`, `app/src/styles/theme.css`, `app/src/lib/tabs/Develop.svelte`.

- [ ] **Step 1: Add the `--text-faint` token (used by the new components)**

In `app/src/styles/theme.css`, in `:root`, add after `--text-dim`:
```css
  --text-faint: #5f5f68;
```

- [ ] **Step 2: Library.svelte — FolderNav + Grid + Metadata (no filmstrip)**

Replace `app/src/lib/tabs/Library.svelte` with:
```svelte
<script lang="ts">
  import FolderNav from "../library/FolderNav.svelte";
  import Grid from "../library/Grid.svelte";
  import Metadata from "../panels/Metadata.svelte";
</script>

<div class="layout">
  <aside class="left"><FolderNav /></aside>
  <section class="center"><Grid /></section>
  <aside class="right"><Metadata /></aside>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 14px;
    grid-template-columns: 232px 1fr 268px; }
  .left, .right, .center { min-height: 0; }
</style>
```
(The Grid panel needs glass styling — wrap the center in a GlassPanel for consistency: import
`GlassPanel` and wrap, OR add panel styling. Use a GlassPanel wrapper:)
Update the center line to:
```svelte
  <section class="center"><div class="pad"><Grid /></div></section>
```
and add to the layout's `<style>`:
```css
  .center { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    box-shadow: inset 0 1px 0 var(--glass-hi), 0 10px 30px rgba(0,0,0,0.32); backdrop-filter: blur(22px); }
  .pad { padding: 14px; height: 100%; }
```
(FolderNav and Metadata already use GlassPanel internally.)

- [ ] **Step 3: +page.svelte — Develop tab badge from undevelopedCount**

In `app/src/routes/+page.svelte`, import `undevelopedCount` from `$lib/store` and render a badge
on the Develop tab. Change the Develop `<button>` to:
```svelte
      <button class:active={$module === "develop"} disabled={!$hasImages} on:click={gotoDevelop}>
        Develop
        {#if $undevelopedCount > 0}<span class="badge">{$undevelopedCount}</span>{/if}
      </button>
```
Add to the script imports: `undevelopedCount` (from `$lib/store`). Make the `.tabs button` style
`position: relative;` and add:
```css
  .badge { position: absolute; top: -7px; right: -8px; min-width: 18px; height: 18px; padding: 0 5px;
    border-radius: 9px; background: var(--accent); color: #fff; font-size: 11px; font-weight: 700;
    display: grid; place-items: center; box-shadow: 0 2px 8px rgba(224,52,52,0.6); }
```

- [ ] **Step 4: Develop.svelte — live-refresh the active image's grid thumbnail on edit**

In `app/src/lib/tabs/Develop.svelte`, add a debounced effect that, when the active image's
`params` change, calls `api.thumbnail` and writes the result into the store entry's `thumbnail`.
Add to the `<script>`:
```ts
  import { api } from "../api";
  import { images } from "../store";
  let thumbTimer: ReturnType<typeof setTimeout> | null = null;
  function refreshThumb() {
    if (thumbTimer) clearTimeout(thumbTimer);
    const id = $activeId; if (!id) return;
    thumbTimer = setTimeout(async () => {
      try {
        const t = await api.thumbnail(id, $params);
        images.update((xs) => xs.map((i) => (i.id === id ? { ...i, thumbnail: t } : i)));
      } catch { /* ignore */ }
    }, 400);
  }
  $: $params, $activeId, refreshThumb();
```
(`activeId`, `params`, `images` imported from `../store`; `api` from `../api`.)

- [ ] **Step 5: Build + vitest**

Run: `cd /Users/mohaelder/Repos/filmrev/app && npm run build 2>&1 | tail -6 && npx vitest run 2>&1 | grep -E 'Test Files|Tests '`
Expected: build succeeds (a11y warnings OK); vitest all pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/mohaelder/Repos/filmrev
git add app/src
git commit -m "feat(redroom): wire Library (nav+grid+metadata), Develop badge, live thumbnail refresh, --text-faint"
```

---

## Task 6: Verify + findings

**Files:** `docs/superpowers/poc-findings.md`.

- [ ] **Step 1: Full verification**

```
source "$HOME/.cargo/env"
(cd app/src-tauri && cargo test 2>&1 | grep 'test result' && cargo build 2>&1 | tail -1)
(cd app && npx vitest run 2>&1 | grep -E 'Test Files|Tests ')
(cd app && npm run build 2>&1 | tail -3)
```
Expected: all green.

- [ ] **Step 2: Record results + manual checklist**

Add a "Library redesign — results" section to `docs/superpowers/poc-findings.md`: folder
navigator + zoomable grid implemented; thumbnails flip to inverted on develop + live-refresh on
edit; Develop badge; window 90% + RedRoom name. Manual E2E to run live: import from two folders →
tree shows both with counts → select each → grid filters; thumb-size slider; develop → thumbnails
become positives; edit black/white → active thumbnail updates; badge counts undeveloped; window
opens at 90% titled RedRoom.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/poc-findings.md
git commit -m "docs: Library redesign results + manual checklist"
```

---

## Definition of Done

- [ ] `cargo test` + `cargo build` green; `thumbnail` command registered; ImageEntry has `path`.
- [ ] `npx vitest run` green incl. `buildTree`/`countImages`.
- [ ] `npm run build` succeeds.
- [ ] Library: folder navigator (Lucide icons) + zoomable grid (no filmstrip); Import at bottom;
      thumbnails inverted after develop + live on edit; Develop tab badge; window 90% + RedRoom.
- [ ] Findings + manual checklist recorded.
```
