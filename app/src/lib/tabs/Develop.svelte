<script lang="ts">
  import { api } from "../api";
  import { activeId, params } from "../store";
  import Adjustments from "../panels/Adjustments.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";

  let preview = "";
  let inverted = false;
  let busy = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function render() {
    if (!$activeId) return;
    busy = true;
    try { preview = await api.invertedPreview($activeId, $params); }
    catch (e) { preview = ""; }
    busy = false;
  }

  function scheduleRender() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(render, 120);
  }

  $: if (inverted && ($params, $activeId)) scheduleRender();

  function invert() { inverted = true; render(); }
</script>

<div class="layout">
  <aside class="left">
    <button class="invert" class:done={inverted} on:click={invert} disabled={!$activeId}>
      {inverted ? "Re-invert" : "Invert"}
    </button>
  </aside>
  <section class="center">
    {#if preview}<img src={preview} alt="inverted" class:busy />
    {:else}<div class="hint">{$activeId ? "Press Invert" : "Select an image in Library"}</div>{/if}
  </section>
  <aside class="right"><Adjustments /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 220px 1fr 260px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .center { grid-area: center; display: grid; place-items: center; min-height: 0; }
  .center img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 10px;
    transition: opacity 0.1s; }
  .center img.busy { opacity: 0.75; }
  .invert { width: 100%; padding: 11px; border: 0; border-radius: 10px;
    background: var(--accent); color: white; font-weight: 700; letter-spacing: 0.3px; }
  .invert.done { background: rgba(224,52,52,0.18); box-shadow: inset 0 0 0 1px rgba(224,52,52,0.5); }
  .invert:disabled { opacity: 0.5; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
