<script lang="ts">
  import { tick } from "svelte";
  import { images, activeId } from "../store";
  let stripEl: HTMLDivElement;

  // Arrow-key navigation lives in Develop.svelte (window-level) so it works from
  // anywhere in Develop, not only when the strip is focused.

  // Keep the active image visible in the strip whenever selection changes
  // (from the strip, the grid, or arrow keys).
  async function revealActive() {
    await tick();
    if (stripEl && $activeId) {
      stripEl.querySelector(`[data-id="${$activeId}"]`)?.scrollIntoView({ inline: "nearest", block: "nearest" });
    }
  }
  $: $activeId, revealActive();
</script>

<div class="strip" bind:this={stripEl} role="listbox" aria-label="Imported images">
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
  button.active { border-color: #fff; box-shadow: 0 0 0 1px #fff; }
  img { height: 64px; display: block; border-radius: 7px; }
</style>
