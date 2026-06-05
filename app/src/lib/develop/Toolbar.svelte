<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "$lib/i18n";
  import { tool, type Tool } from "../store";

  const tools: { id: Tool; icon: string; labelKey: string; enabled: boolean }[] = [
    { id: "edit", icon: "sliders", labelKey: "toolbar.edit", enabled: true },
    { id: "crop", icon: "crop", labelKey: "toolbar.crop", enabled: true },
    { id: "eraser", icon: "eraser", labelKey: "toolbar.eraser", enabled: true },
    { id: "base_picker", icon: "pipette", labelKey: "toolbar.basePicker", enabled: true },
  ];
</script>

<div class="toolbar">
  {#each tools as tl}
    <button
      class:on={$tool === tl.id} disabled={!tl.enabled} title={$t(tl.labelKey)}
      on:click={() => tl.enabled && tool.set(tl.id)}
    >
      <Icon name={tl.icon} size={17} />
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
