<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { developProgress } from "../store";
  import { t } from "$lib/i18n";
  $: p = $developProgress;
  $: pct = p.total ? Math.round((p.done / p.total) * 100) : 0;
  $: current = p.done + 1 > p.total ? p.total : p.done + 1;
</script>

{#if p.active}
  <div class="scrim" transition:fade={{ duration: 150 }}>
    <div class="card" transition:scale={{ duration: 160, start: 0.96, opacity: 0 }}>
      <div class="title">{$t('progress.developing', { current, total: p.total })}</div>
      <div class="bar"><div class="fill" style="width:{pct}%"></div></div>
    </div>
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.55); backdrop-filter: blur(8px);
    display: grid; place-items: center; z-index: 50; }
  .card { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    padding: 22px 26px; min-width: 320px; box-shadow: 0 20px 60px rgba(0,0,0,0.5); }
  .title { margin-bottom: 14px; }
  .bar { height: 6px; border-radius: 3px; background: rgba(255,255,255,0.1); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); transition: width 0.2s; }
</style>
