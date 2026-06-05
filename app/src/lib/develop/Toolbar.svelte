<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { tool, type Tool } from "../store";

  const tools: { id: Tool; icon: string; label: string; enabled: boolean }[] = [
    { id: "edit", icon: "sliders", label: "Edit", enabled: true },
    { id: "crop", icon: "crop", label: "Crop", enabled: true },
    { id: "eraser", icon: "eraser", label: "Eraser", enabled: true },
  ];
</script>

<div class="toolbar">
  {#each tools as t}
    <button
      class:on={$tool === t.id} disabled={!t.enabled} title={t.label}
      on:click={() => t.enabled && tool.set(t.id)}
    >
      <Icon name={t.icon} size={17} />
    </button>
  {/each}
</div>

<style>
  .toolbar { display: flex; gap: 4px; margin-bottom: 12px;
    padding-bottom: 10px; border-bottom: 1px solid var(--glass-brd); }
  button { flex: 1; display: grid; place-items: center; padding: 7px 0;
    border-radius: 8px; border: 1px solid transparent; background: transparent;
    color: var(--text-dim); cursor: pointer; }
  button.on { color: #fff; background: rgba(244,157,78,0.18);
    border-color: rgba(244,157,78,0.5); }
  button:disabled { opacity: 0.35; cursor: default; }
</style>
