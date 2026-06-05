<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { images, activeId, allDeveloped } from "../store";
  import { developAll } from "../workflow";
  import GlassPanel from "../glass/GlassPanel.svelte";
  import { t } from "$lib/i18n";

  let importing = false;
  let error = "";
  $: filterFilmScans = $t("source.filterFilmScans");

  async function pickAndImport() {
    const sel = await open({ multiple: true, filters: [{ name: filterFilmScans, extensions: ["dng", "tif", "tiff", "raf"] }] });
    if (!sel) return;
    const paths = Array.isArray(sel) ? sel : [sel];
    importing = true; error = "";
    for (const path of paths) {
      try {
        const entry = await api.importImage(path as string);
        images.update((xs) =>
          xs.some((i) => i.id === entry.id)
            ? xs.map((i) => (i.id === entry.id ? entry : i))
            : [...xs, entry]);
        activeId.update((id) => id ?? entry.id);
      } catch (e) { error = String(e); }
    }
    importing = false;
  }
</script>

<GlassPanel>
  <div class="wrap">
    <button class="import" on:click={pickAndImport} disabled={importing}>
      {importing ? $t('source.importing') : $t('source.import')}
    </button>
    {#if error}<div class="err">{error}</div>{/if}
    <ul>
      {#each $images as img}
        <li class:active={$activeId === img.id} on:click={() => activeId.set(img.id)}>
          <span class="name">{img.file_name}</span>
          {#if img.developed}<span class="dot" title={$t('source.developed')}></span>{/if}
        </li>
      {/each}
    </ul>
    {#if $images.length > 0 && !$allDeveloped}
      <button class="develop" on:click={() => developAll()}>{$t('source.developAll')}</button>
    {/if}
  </div>
</GlassPanel>

<style>
  .wrap { display: flex; flex-direction: column; height: 100%; }
  .import { width: 100%; padding: 9px; border-radius: 10px; border: 0;
    background: rgba(255,255,255,0.08); color: var(--text); font-weight: 600; }
  .import:disabled { opacity: 0.6; }
  .err { color: var(--accent); margin-top: 8px; font-size: 12px; }
  ul { list-style: none; padding: 0; margin: 12px 0; flex: 1; overflow: auto; }
  li { padding: 7px 9px; border-radius: 8px; color: var(--text-dim); cursor: pointer;
    display: flex; align-items: center; gap: 8px; }
  li.active { background: rgba(255,255,255,0.06); color: var(--text); }
  .name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1; }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--accent); flex: 0 0 auto; }
  .develop { width: 100%; padding: 10px; border: 0; border-radius: 10px; margin-top: auto;
    background: var(--accent-grad); color: white; font-weight: 700; }
</style>
