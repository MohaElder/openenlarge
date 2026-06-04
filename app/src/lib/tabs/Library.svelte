<script lang="ts">
  import { activeId, params, images } from "../store";
  import Source from "../panels/Source.svelte";
  import Metadata from "../panels/Metadata.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";

  $: active = $images.find((i) => i.id === $activeId);
</script>

<div class="layout">
  <aside class="left"><Source /></aside>
  <section class="center">
    {#if active}
      <Viewport id={$activeId} params={$params} raw={true} interactive={false}
                imgW={active.metadata.width} imgH={active.metadata.height} />
    {:else}<div class="hint">Import a film scan to begin</div>{/if}
  </section>
  <aside class="right"><Metadata /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; display: grid; place-items: center; min-height: 0; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
