<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { credits, GITHUB_URL } from "./credits";
  import { t } from "$lib/i18n";

  const dispatch = createEventDispatcher<{ close: void }>();
  let view: "about" | "licenses" = "about";

  // Open an external link in the user's browser; fall back silently if blocked.
  function open(url: string) {
    openUrl(url).catch(() => {});
  }
</script>

<div class="scrim" on:click|self={() => dispatch("close")} transition:fade={{ duration: 150 }}>
  <div class="card" transition:scale={{ duration: 160, start: 0.96, opacity: 0 }}>
    {#if view === "about"}
      <div class="head">
        <img class="logo" src="/favicon.png" alt="" />
        <div>
          <div class="title">{$t('about.title')}</div>
          <div class="sub">{$t('about.subtitle')}</div>
        </div>
      </div>
      <p class="body">
        {$t('about.body')}
      </p>
      <div class="row">
        <button class="link" on:click={() => (view = "licenses")}>{$t('about.licenses')}</button>
        <button class="link" on:click={() => open(GITHUB_URL)}>{$t('about.github')}</button>
        <div class="spacer"></div>
        <button class="go" on:click={() => dispatch("close")}>{$t('about.close')}</button>
      </div>
      <img class="hero" src="/about-hero.jpeg" alt="" />
      <div class="caption">New York, 2026 May</div>
    {:else}
      <div class="head">
        <button class="back" on:click={() => (view = "about")} aria-label={$t('about.back')}>←</button>
        <div>
          <div class="title">{$t('about.licensesTitle')}</div>
          <div class="sub">{$t('about.licensesSubtitle')}</div>
        </div>
      </div>
      <div class="licenses">
        {#each credits as section}
          <div class="grp">{$t(section.group)}</div>
          {#each section.items as c}
            <button class="dep" on:click={() => open(c.url)}>
              <span class="dep-name">{c.name}</span>
              <span class="dep-lic">{c.license}</span>
            </button>
          {/each}
        {/each}
      </div>
      <div class="row">
        <div class="spacer"></div>
        <button class="go" on:click={() => dispatch("close")}>{$t('about.close')}</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.5); backdrop-filter: blur(6px);
    display: grid; place-items: center; z-index: 80; }
  .card { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    padding: 22px; min-width: 380px; max-width: 480px; box-shadow: 0 20px 60px rgba(0,0,0,0.5); }
  .head { display: flex; align-items: center; gap: 12px; margin-bottom: 14px; }
  .logo { width: 44px; height: 44px; border-radius: 10px; display: block; flex: none; }
  .title { font-weight: 600; }
  .sub { color: var(--text-dim); font-size: 12px; margin-top: 2px; }
  .body { color: var(--text-dim); font-size: 13px; line-height: 1.5; margin: 0 0 18px; }
  .hero { display: block; width: 100%; height: auto; margin-top: 16px; }
  .caption { color: var(--text-dim); font-size: 11px; margin-top: 6px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  button { padding: 8px 14px; border-radius: 9px; border: 1px solid var(--glass-brd); background: transparent; color: var(--text); }
  .link { border: 0; padding: 8px 10px; color: var(--accent); font-weight: 600; }
  .link:hover { background: var(--glass-hi); }
  .go { background: var(--accent-grad); color: white; border: 0; font-weight: 600; }
  .back { border: 0; width: 32px; height: 32px; padding: 0; font-size: 18px; color: var(--text-dim); flex: none; }
  .back:hover { background: var(--glass-hi); color: var(--text); }
  .licenses { max-height: 46vh; overflow-y: auto; margin-bottom: 16px; }
  .grp { font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-dim);
    margin: 12px 2px 4px; }
  .grp:first-child { margin-top: 0; }
  .dep { display: flex; align-items: baseline; justify-content: space-between; gap: 12px;
    width: 100%; text-align: left; padding: 7px 10px; border: 0; border-radius: 8px; }
  .dep:hover { background: var(--glass-hi); }
  .dep-name { font-size: 13px; }
  .dep-lic { font-size: 11px; color: var(--text-dim); flex: none; }
</style>
