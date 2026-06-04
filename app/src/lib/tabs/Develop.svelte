<script lang="ts">
  import { activeId, params, images } from "../store";
  import { api } from "../api";
  import Adjustments from "../panels/Adjustments.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";
  import QualityMenu from "../viewport/QualityMenu.svelte";

  $: active = $images.find((i) => i.id === $activeId);

  let thumbTimer: ReturnType<typeof setTimeout> | null = null;
  function refreshThumb() {
    if (thumbTimer) clearTimeout(thumbTimer);
    const id = $activeId;
    if (!id) return;
    thumbTimer = setTimeout(async () => {
      try {
        const t = await api.thumbnail(id, $params);
        images.update((xs) => xs.map((i) => (i.id === id ? { ...i, thumbnail: t } : i)));
      } catch { /* ignore (e.g. not developed) */ }
    }, 400);
  }
  $: $params, $activeId, refreshThumb();
  let menu: { x: number; y: number } | null = null;
  function onContext(e: MouseEvent) { e.preventDefault(); menu = { x: e.clientX, y: e.clientY }; }
</script>

<div class="layout" on:contextmenu={onContext}>
  <aside class="left"></aside>
  <section class="center">
    {#if active?.developed}
      <Viewport id={$activeId} params={$params}
                imgW={active.metadata.width} imgH={active.metadata.height} />
    {:else}<div class="hint">Not developed yet</div>{/if}
  </section>
  <aside class="right"><Adjustments /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>
{#if menu}<QualityMenu x={menu.x} y={menu.y} on:close={() => (menu = null)} />{/if}

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; min-height: 0; display: grid; place-items: center; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
