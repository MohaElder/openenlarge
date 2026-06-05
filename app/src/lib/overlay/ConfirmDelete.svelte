<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade, scale } from "svelte/transition";
  export let name = "";
  const dispatch = createEventDispatcher<{ remove: void; trash: void; cancel: void }>();
</script>

<div class="scrim" on:click|self={() => dispatch("cancel")} transition:fade={{ duration: 150 }}>
  <div class="card" transition:scale={{ duration: 160, start: 0.96, opacity: 0 }}>
    <div class="title">Delete {name || "this image"}?</div>
    <div class="sub">Remove it from OpenEnlarge, or also move the original file to the Trash.</div>
    <div class="warn">This can't be undone.</div>
    <div class="row">
      <button class="ghost" on:click={() => dispatch("cancel")}>Cancel</button>
      <button class="ghost" on:click={() => dispatch("remove")}>Remove from OpenEnlarge</button>
      <button class="go" on:click={() => dispatch("trash")}>Move file to Trash</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.5); backdrop-filter: blur(6px);
    display: grid; place-items: center; z-index: 80; }
  .card { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    padding: 22px; min-width: 360px; max-width: 460px; box-shadow: 0 20px 60px rgba(0,0,0,0.5); }
  .title { font-weight: 600; margin-bottom: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { color: var(--text-dim); font-size: 12px; }
  .warn { color: var(--accent); font-size: 12px; font-weight: 600; margin-bottom: 18px; }
  .row { display: flex; gap: 10px; justify-content: flex-end; flex-wrap: wrap; }
  button { padding: 8px 14px; border-radius: 9px; border: 1px solid var(--glass-brd); background: transparent; }
  .go { background: var(--accent-grad); color: white; border: 0; font-weight: 600; }
</style>
