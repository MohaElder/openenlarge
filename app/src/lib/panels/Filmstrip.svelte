<script lang="ts">
  import { images, activeId } from "../store";
</script>

<div class="strip">
  {#each $images as img}
    <button class:active={$activeId === img.id} on:click={() => activeId.set(img.id)}>
      <img src={img.thumbnail} alt={img.file_name} />
    </button>
  {/each}
</div>

<style>
  .strip { display: flex; gap: 8px; overflow-x: auto; padding: 6px; height: 100%; align-items: center; }
  button { padding: 0; border: 1px solid var(--glass-brd); border-radius: 8px; background: none;
    flex: 0 0 auto; cursor: pointer; }
  button.active { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  img { height: 64px; display: block; border-radius: 7px; }
</style>
