<script lang="ts">
  import FolderNav from "../library/FolderNav.svelte";
  import Grid from "../library/Grid.svelte";
  import Metadata from "../panels/Metadata.svelte";
</script>

<div class="layout">
  <aside class="left"><FolderNav /></aside>
  <section class="center"><div class="pad"><Grid /></div></section>
  <aside class="right"><Metadata /></aside>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 14px; grid-template-columns: 232px 1fr 268px; }
  .left, .right { min-height: 0; }
  .center { min-height: 0; background: var(--glass-bg); border: 1px solid var(--glass-brd);
    border-radius: 14px; box-shadow: inset 0 1px 0 var(--glass-hi), 0 10px 30px rgba(0,0,0,0.32);
    backdrop-filter: blur(22px); }
  .pad { padding: 14px; height: 100%; }
</style>
