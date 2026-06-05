<script lang="ts">
  import { images, activeId, metaById, activeMeta } from "../store";
  import type { MetaField } from "../api";
  import GlassPanel from "../glass/GlassPanel.svelte";

  $: active = $images.find((i) => i.id === $activeId);
  const fmtSize = (b: number) => (b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : (b / 1e3).toFixed(0) + " KB");

  // Free-text EXIF fields. Each input shows the user override if set, otherwise the
  // source EXIF value as a placeholder — an empty field means "keep the original".
  const TEXT_FIELDS: { key: Exclude<MetaField, "date" | "note">; label: string }[] = [
    { key: "camera", label: "Camera" },
    { key: "lens", label: "Lens" },
    { key: "iso", label: "ISO" },
    { key: "shutter", label: "Shutter" },
    { key: "aperture", label: "Aperture" },
  ];

  /** Write (or clear, when blank) one override field on the active image. */
  function setField(field: MetaField, value: string): void {
    const id = $activeId;
    if (!id) return;
    const v = value.trim();
    metaById.update((map) => {
      const cur = { ...(map[id] ?? {}) };
      if (v) cur[field] = v;
      else delete cur[field];
      return { ...map, [id]: cur };
    });
  }

  /** Drop every override for the active image (revert to source EXIF). */
  function resetAll(): void {
    const id = $activeId;
    if (!id) return;
    metaById.update((map) => {
      const next = { ...map };
      delete next[id];
      return next;
    });
  }

  $: hasOverride = Object.keys($activeMeta).length > 0;

  /** EXIF "YYYY:MM:DD HH:MM:SS" (or an HTML local string) → "YYYY-MM-DDTHH:MM". */
  function toLocalInput(s?: string): string {
    if (!s) return "";
    if (s.includes("T")) return s.slice(0, 16);
    const dt = s.match(/^(\d{4})\D(\d{2})\D(\d{2})[ T](\d{2}):(\d{2})/);
    if (dt) return `${dt[1]}-${dt[2]}-${dt[3]}T${dt[4]}:${dt[5]}`;
    const d = s.match(/^(\d{4})\D(\d{2})\D(\d{2})/);
    if (d) return `${d[1]}-${d[2]}-${d[3]}T00:00`;
    return "";
  }

  // Date input shows the override if present, else the source date.
  $: dateValue = toLocalInput($activeMeta.date ?? active?.metadata.date);
</script>

<GlassPanel>
  {#if active}
    {@const m = active.metadata}
    <header>
      <h3>{active.file_name}</h3>
      {#if hasOverride}
        <button class="reset" on:click={resetAll} title="Revert all metadata to the original EXIF">Reset</button>
      {/if}
    </header>

    <div class="fields">
      {#each TEXT_FIELDS as f (f.key)}
        <label>
          <span>{f.label}</span>
          <input
            type="text"
            value={$activeMeta[f.key] ?? ""}
            placeholder={m[f.key] ?? "—"}
            on:input={(e) => setField(f.key, e.currentTarget.value)}
          />
        </label>
      {/each}

      <label>
        <span>Date</span>
        <input
          type="datetime-local"
          value={dateValue}
          on:input={(e) => setField("date", e.currentTarget.value)}
        />
      </label>

      <label class="note">
        <span>Note</span>
        <textarea
          rows="3"
          value={$activeMeta.note ?? ""}
          placeholder="Add a note…"
          on:input={(e) => setField("note", e.currentTarget.value)}
        ></textarea>
      </label>
    </div>

    <dl class="ro">
      <dt>Dimensions</dt><dd>{m.width} × {m.height}</dd>
      <dt>Size</dt><dd>{fmtSize(m.file_size)}</dd>
    </dl>
  {:else}
    <div class="empty">No image selected</div>
  {/if}
</GlassPanel>

<style>
  header { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; margin: 0 0 12px; }
  h3 { margin: 0; font-size: 13px; word-break: break-all; }
  .reset {
    flex: none; background: transparent; border: 0; color: var(--accent);
    font-size: 11px; font-weight: 600; padding: 0; cursor: pointer; transition: opacity 0.15s;
  }
  .reset:hover { opacity: 0.8; }

  .fields { display: flex; flex-direction: column; gap: 9px; }
  label { display: grid; grid-template-columns: 64px 1fr; align-items: center; gap: 10px; }
  label span { color: var(--text-dim); font-size: 12px; }
  label.note { align-items: start; }

  input, textarea {
    width: 100%; box-sizing: border-box;
    background: var(--glass-hi); border: 1px solid var(--glass-brd); border-radius: 7px;
    color: var(--text); font-size: 12px; font-family: inherit; padding: 6px 8px;
    transition: border-color 0.15s, background 0.15s;
  }
  input::placeholder, textarea::placeholder { color: var(--text-faint); }
  input:focus, textarea:focus { outline: none; border-color: var(--accent); background: rgba(255, 255, 255, 0.06); }
  textarea { resize: vertical; min-height: 48px; line-height: 1.4; }
  /* Make the native datetime calendar icon visible on the dark theme. */
  input[type="datetime-local"]::-webkit-calendar-picker-indicator { filter: invert(0.8); cursor: pointer; }

  .ro {
    display: grid; grid-template-columns: auto 1fr; gap: 6px 12px; margin: 14px 0 0;
    padding-top: 12px; border-top: 1px solid var(--glass-brd);
  }
  .ro dt { color: var(--text-dim); font-size: 12px; }
  .ro dd { margin: 0; text-align: right; font-size: 12px; }
  .empty { color: var(--text-dim); }
</style>
