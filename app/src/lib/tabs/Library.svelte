<script lang="ts">
  import FolderNav from "../library/FolderNav.svelte";
  import Grid from "../library/Grid.svelte";
  import Metadata from "../panels/Metadata.svelte";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import { images, activeId, selectedFolder } from "../store";

  // Skip nav while a form control (e.g. the thumb-size slider) is focused, so its
  // own arrow-key behaviour wins.
  function formFocused(): boolean {
    const tag = document.activeElement?.tagName;
    return tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA";
  }

  // Arrow keys navigate images from anywhere in Library (grid or filmstrip), no
  // focus required: ←/→ step, ↑ first, ↓ last. Also switch the grid to the active
  // image's folder so grid, filmstrip, and metadata stay in sync.
  function onKey(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const arrows = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"];
    if (!arrows.includes(e.key) || formFocused()) return;
    const list = $images;
    if (list.length === 0) return;
    let idx = list.findIndex((i) => i.id === $activeId);
    if (idx < 0) idx = 0;
    if (e.key === "ArrowLeft") idx = Math.max(0, idx - 1);
    else if (e.key === "ArrowRight") idx = Math.min(list.length - 1, idx + 1);
    else if (e.key === "ArrowUp") idx = 0;
    else idx = list.length - 1;
    e.preventDefault();
    const img = list[idx];
    activeId.set(img.id);
    selectedFolder.set(img.path.replace(/\\/g, "/").split("/").slice(0, -1).join("/"));
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="layout">
  <aside class="left"><FolderNav /></aside>
  <section class="center"><div class="pad"><Grid /></div></section>
  <aside class="right"><Metadata /></aside>
  <footer class="bottom"><Filmstrip /></footer>
</div>

<style>
  .layout { display: grid; height: 100%; gap: 14px;
    grid-template-columns: 232px 1fr 268px; grid-template-rows: 1fr 88px;
    grid-template-areas: "left center right" "bottom bottom bottom"; }
  .left { grid-area: left; } .right { grid-area: right; }
  .left, .right { min-height: 0; }
  .center { grid-area: center; min-height: 0; background: var(--glass-bg); border: 1px solid var(--glass-brd);
    border-radius: 14px; box-shadow: inset 0 1px 0 var(--glass-hi), 0 10px 30px rgba(0,0,0,0.32);
    backdrop-filter: blur(22px); }
  .pad { padding: 14px; height: 100%; }
  .bottom { grid-area: bottom; }
</style>
