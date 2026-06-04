<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { selectedFolder } from "../store";
  import { countImages, type FolderNode } from "./folderTree";
  export let node: FolderNode;
  export let depth = 0;
  export let isRoot = false;
  let open = true;
  $: hasChildren = node.children.length > 0;
  $: count = countImages(node);
</script>

<div class="row" class:sel={$selectedFolder === node.fullPath}
  style="padding-left:{8 + depth * 16}px"
  on:click={() => { selectedFolder.set(node.fullPath); if (hasChildren) open = !open; }}>
  <span class="chev">
    {#if hasChildren}<Icon name={open ? "chevron-down" : "chevron-right"} size={12} />{/if}
  </span>
  <Icon name={isRoot ? "hard-drive" : "folder"} />
  <span class="lbl">{node.name}</span>
  {#if count > 0}<span class="ct">{count}</span>{/if}
</div>
{#if open}
  {#each node.children as child}
    <svelte:self node={child} depth={depth + 1} />
  {/each}
{/if}

<style>
  .row { display: flex; align-items: center; gap: 7px; padding: 6px 8px; border-radius: 8px;
    color: var(--text-dim); cursor: pointer; white-space: nowrap; }
  .row:hover { background: rgba(255,255,255,0.04); }
  .row.sel { background: rgba(255,255,255,0.07); color: var(--text); }
  .chev { color: var(--text-faint); display: inline-flex; width: 12px; }
  .lbl { overflow: hidden; text-overflow: ellipsis; }
  .ct { margin-left: auto; font-size: 11px; color: var(--text-faint); padding-left: 8px; }
</style>
