<script lang="ts">
  import { api } from "../api";
  import { activeId } from "../store";
  import Source from "../panels/Source.svelte";
  import Metadata from "../panels/Metadata.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";

  let preview = "";
  $: if ($activeId) { api.rawPreview($activeId).then((d) => (preview = d)).catch(() => (preview = "")); }
</script>

<div class="layout">
  <aside class="left"><Source /></aside>
  <section class="center">
    {#if preview}<img src={preview} alt="raw scan" />{:else}<div class="hint">Import a film scan to begin</div>{/if}
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
  .center img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 10px; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
</style>
