<script lang="ts">
  import { activeId, params, images } from "../store";
  import Adjustments from "../panels/Adjustments.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";

  $: active = $images.find((i) => i.id === $activeId);
</script>

<div class="layout">
  <aside class="left"></aside>
  <section class="center">
    <Viewport id={$activeId} params={$params}
              imgW={active?.metadata.width ?? 0} imgH={active?.metadata.height ?? 0} />
  </section>
  <aside class="right"><Adjustments /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; min-height: 0; }
  .bottom { grid-area: bottom; }
</style>
