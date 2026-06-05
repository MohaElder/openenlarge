<script lang="ts">
  import type { SpinRect } from "./spin";
  let active = false;
  let src = "";
  let style = "";
  let timer: ReturnType<typeof setTimeout> | null = null;

  /** Spin a snapshot from `rect` through `dir·90°` + `scale(k)`, then remove. */
  export function spin(snapshot: string, rect: SpinRect, dir: number, k: number) {
    if (timer) clearTimeout(timer);
    src = snapshot;
    active = true;
    const base = `left:${rect.left}px; top:${rect.top}px; width:${rect.width}px; height:${rect.height}px;`;
    style = `${base} transform:none; transition:none;`;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      style = `${base} transform:rotate(${dir * 90}deg) scale(${k}); transition:transform 260ms cubic-bezier(0.4,0,0.2,1);`;
    }));
    timer = setTimeout(() => { active = false; src = ""; }, 300);
  }
</script>

{#if active}<img class="spin" {src} alt="" style={style} />{/if}

<style>
  .spin { position: absolute; z-index: 5; pointer-events: none; transform-origin: center center; display: block; }
</style>
