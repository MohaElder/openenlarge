<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { images, activeId } from "../store";
  import GlassPanel from "../glass/GlassPanel.svelte";

  let importing = false;
  let error = "";

  async function pickAndImport() {
    const sel = await open({ multiple: true, filters: [{ name: "Film scans", extensions: ["dng", "tif", "tiff", "raf"] }] });
    if (!sel) return;
    const paths = Array.isArray(sel) ? sel : [sel];
    importing = true; error = "";
    for (const path of paths) {
      try {
        const entry = await api.importImage(path as string);
        images.update((xs) => [...xs, entry]);
        activeId.update((id) => id ?? entry.id);
      } catch (e) { error = String(e); }
    }
    importing = false;
  }
</script>

<GlassPanel>
  <button class="import" on:click={pickAndImport} disabled={importing}>
    {importing ? "Importing…" : "Import"}
  </button>
  {#if error}<div class="err">{error}</div>{/if}
  <ul>
    {#each $images as img}
      <li class:active={$activeId === img.id} on:click={() => activeId.set(img.id)}>
        {img.file_name}
      </li>
    {/each}
  </ul>
</GlassPanel>

<style>
  .import { width: 100%; padding: 9px; border-radius: 10px; border: 0;
    background: var(--accent); color: white; font-weight: 600; }
  .import:disabled { opacity: 0.6; }
  .err { color: var(--accent); margin-top: 8px; font-size: 12px; }
  ul { list-style: none; padding: 0; margin: 12px 0 0; }
  li { padding: 7px 9px; border-radius: 8px; color: var(--text-dim); cursor: pointer;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  li.active { background: rgba(255,255,255,0.06); color: var(--text); }
</style>
