<script lang="ts">
  import { images, activeId } from "../store";
  import GlassPanel from "../glass/GlassPanel.svelte";
  $: active = $images.find((i) => i.id === $activeId);
  const fmtSize = (b: number) => (b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : (b / 1e3).toFixed(0) + " KB");
</script>

<GlassPanel>
  {#if active}
    {@const m = active.metadata}
    <h3>{active.file_name}</h3>
    <dl>
      <dt>Camera</dt><dd>{m.camera ?? "—"}</dd>
      <dt>Lens</dt><dd>{m.lens ?? "—"}</dd>
      <dt>ISO</dt><dd>{m.iso ?? "—"}</dd>
      <dt>Shutter</dt><dd>{m.shutter ?? "—"}</dd>
      <dt>Aperture</dt><dd>{m.aperture ?? "—"}</dd>
      <dt>Dimensions</dt><dd>{m.width} × {m.height}</dd>
      <dt>Size</dt><dd>{fmtSize(m.file_size)}</dd>
      <dt>Date</dt><dd>{m.date ?? "—"}</dd>
    </dl>
  {:else}
    <div class="empty">No image selected</div>
  {/if}
</GlassPanel>

<style>
  h3 { margin: 0 0 12px; font-size: 13px; word-break: break-all; }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 6px 12px; margin: 0; }
  dt { color: var(--text-dim); } dd { margin: 0; text-align: right; }
  .empty { color: var(--text-dim); }
</style>
