<script lang="ts">
  export let pad = 12;
</script>

<div class="glass" style="padding:{pad}px">
  <slot />
</div>

<style>
  .glass {
    background: var(--glass-bg);
    border: 1px solid var(--glass-brd);
    border-radius: var(--radius);
    backdrop-filter: blur(22px) saturate(140%);
    -webkit-backdrop-filter: blur(22px) saturate(140%);
    box-shadow: inset 0 1px 0 var(--glass-hi), 0 8px 30px rgba(0,0,0,0.35);
    overflow: auto;
    height: 100%;
  }
</style>
