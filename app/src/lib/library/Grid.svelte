<script lang="ts">
  import { tick, onMount } from "svelte";
  import { images, activeId, selectedFolder, gridZoom } from "../store";
  let scrollEl: HTMLDivElement;
  let containerW = 800;
  $: shown = $images.filter((i) => {
    const dir = i.path.replace(/\\/g, "/").split("/").slice(0, -1).join("/");
    return dir === $selectedFolder;
  });
  const GAP = 12;
  const MIN = 130;
  // 130px at zoom 0 → full container width at zoom 100 (1 image per row).
  $: maxCol = Math.max(MIN, containerW - 4);
  $: minCol = MIN + ($gridZoom / 100) * (maxCol - MIN);

  onMount(() => {
    const measure = () => { if (scrollEl) containerW = scrollEl.clientWidth; };
    measure();
    const ro = new ResizeObserver(measure);
    if (scrollEl) ro.observe(scrollEl);
    return () => ro.disconnect();
  });

  // ctrl/cmd + scroll (and trackpad pinch) resize thumbnails; plain scroll scrolls.
  function onWheel(e: WheelEvent) {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      gridZoom.update((z) => Math.max(0, Math.min(100, z - e.deltaY * 0.5)));
    }
  }

  function colCount(): number {
    if (!scrollEl) return 1;
    const w = scrollEl.clientWidth - 4; // padding-right
    return Math.max(1, Math.floor((w + GAP) / (minCol + GAP)));
  }

  // Arrow keys navigate the grid 2-D (left/right within a row, up/down by a row).
  async function onKey(e: KeyboardEvent) {
    const arrows = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"];
    if (!arrows.includes(e.key)) return;
    const list = shown;
    if (list.length === 0) return;
    let idx = list.findIndex((i) => i.id === $activeId);
    if (idx < 0) idx = 0;
    const cols = colCount();
    e.preventDefault();
    if (e.key === "ArrowLeft") idx -= 1;
    else if (e.key === "ArrowRight") idx += 1;
    else if (e.key === "ArrowUp") idx -= cols;
    else if (e.key === "ArrowDown") idx += cols;
    idx = Math.max(0, Math.min(list.length - 1, idx));
    activeId.set(list[idx].id);
    await tick();
    scrollEl?.querySelector(`[data-id="${list[idx].id}"]`)?.scrollIntoView({ block: "nearest" });
  }
</script>

<div class="center">
  <div class="head">
    <div class="where"><b>{$selectedFolder?.split("/").pop() ?? "—"}</b> · {shown.length} image{shown.length === 1 ? "" : "s"}</div>
    <div class="right">Thumb size <input class="zoom" type="range" min="0" max="100" bind:value={$gridZoom} /></div>
  </div>
  <div class="scroll" bind:this={scrollEl} tabindex="0" role="listbox" aria-label="Folder images" on:wheel={onWheel} on:keydown={onKey}>
    <div class="grid" style="grid-template-columns:repeat(auto-fill,minmax({minCol}px,1fr))">
      {#each shown as img (img.id)}
        <button data-id={img.id} class="cell" class:sel={$activeId === img.id} on:click={() => activeId.set(img.id)}>
          <div class="ratio"><img src={img.thumbnail} alt={img.file_name} /></div>
        </button>
      {/each}
    </div>
    {#if shown.length === 0}<div class="empty">Select a folder with images</div>{/if}
  </div>
</div>

<style>
  .center { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .head { display: flex; align-items: center; gap: 12px; padding: 2px 4px 12px; }
  .where { color: var(--text-dim); } .where b { color: var(--text); }
  .right { margin-left: auto; display: flex; align-items: center; gap: 9px; color: var(--text-faint); font-size: 12px; }
  .zoom { appearance: none; width: 120px; height: 4px; border-radius: 2px; background: rgba(255,255,255,0.14); outline: 0; }
  .zoom::-webkit-slider-thumb { appearance: none; width: 13px; height: 13px; border-radius: 50%; background: var(--accent); }
  .scroll { flex: 1; overflow-y: auto; padding-right: 4px; outline: none; }
  .grid { display: grid; gap: 12px; align-content: start; }
  .cell { display: block; padding: 0; border: 1px solid var(--glass-brd); border-radius: 11px;
    overflow: hidden; background: #0d0d10; cursor: pointer; transition: transform 0.12s, box-shadow 0.12s; }
  .cell:hover { transform: translateY(-2px); box-shadow: 0 12px 26px rgba(0,0,0,0.5); }
  .cell.sel { box-shadow: 0 0 0 2px var(--accent), 0 12px 26px rgba(0,0,0,0.5); }
  .ratio { position: relative; width: 100%; height: 0; padding-bottom: 100%; }
  .ratio img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; display: block; }
  .empty { color: var(--text-faint); padding: 16px; }
</style>
