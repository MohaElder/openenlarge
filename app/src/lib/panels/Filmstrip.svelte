<script lang="ts">
  import { tick } from "svelte";
  import { images, activeId } from "../store";
  let stripEl: HTMLDivElement;

  // Left/Right step through all images; Up jumps to first, Down to last.
  async function onKey(e: KeyboardEvent) {
    const list = $images;
    if (list.length === 0) return;
    let idx = list.findIndex((i) => i.id === $activeId);
    if (idx < 0) idx = 0;
    if (e.key === "ArrowLeft") idx = Math.max(0, idx - 1);
    else if (e.key === "ArrowRight") idx = Math.min(list.length - 1, idx + 1);
    else if (e.key === "ArrowUp") idx = 0;
    else if (e.key === "ArrowDown") idx = list.length - 1;
    else return;
    e.preventDefault();
    activeId.set(list[idx].id);
    await tick();
    stripEl?.querySelector(`[data-id="${list[idx].id}"]`)?.scrollIntoView({ inline: "nearest", block: "nearest" });
  }
</script>

<div class="strip" bind:this={stripEl} tabindex="0" role="listbox" aria-label="Imported images" on:keydown={onKey}>
  {#each $images as img}
    <button data-id={img.id} class:active={$activeId === img.id} on:click={() => activeId.set(img.id)}>
      <img src={img.thumbnail} alt={img.file_name} />
    </button>
  {/each}
</div>

<style>
  .strip { display: flex; gap: 8px; overflow-x: auto; padding: 6px; height: 100%; align-items: center; outline: none; }
  button { padding: 0; border: 1px solid var(--glass-brd); border-radius: 8px; background: none;
    flex: 0 0 auto; cursor: pointer; }
  button.active { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  img { height: 64px; display: block; border-radius: 7px; }
</style>
