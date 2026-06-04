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
