# OpenEnlarge Catalog — Persistence Design

**Date:** 2026-06-04
**Branch:** `feat/catalog-persistence`
**Status:** Approved, ready for implementation planning

## Problem

OpenEnlarge holds its entire catalog in memory. The backend `Session`
(`app/src-tauri/src/session.rs`) is a `HashMap<String, CachedImage>` with
session-scoped ids (`img0`, `img1`…); the frontend keeps all per-image edits in
Svelte stores (`editsById`, `cropById`, `dustById`). **Nothing survives an app
restart** except the single `developMode` localStorage entry. Every relaunch
means re-importing every file and redoing every adjustment.

Goal: a durable, Lightroom-style **catalog** that persists everything — global
preferences through per-file states — and restores it on launch.

## Decisions (resolved during brainstorming)

| Question | Decision |
|---|---|
| Catalog model | **Single, invisible app-wide catalog** (not multi-file `.lrcat`) |
| Storage format | **SQLite** (`catalog.db` in `app_data_dir()`) |
| Thumbnails | **Stored in catalog** (instant library on launch) |
| Missing files | **Marked offline, kept** (never dropped) |
| Save timing | **Autosave, debounced (~400ms) write-through** + flush on quit |
| Session/UI state | **Restored** (selected folder, active image, grid zoom, module) |

## Architecture

The catalog **owns the durable truth**; the Svelte stores remain the live
working copy. The backend (Rust) owns the SQLite database. Data flow:

- **Launch:** frontend calls `load_catalog()` → backend returns the whole
  catalog → frontend hydrates every store.
- **Edit:** store mutates → debounced write-through → `save_*` command →
  backend persists a transactional write.
- **Quit:** Tauri `close-requested` → frontend flushes pending debounced writes.

```
┌──────────── Frontend (Svelte) ────────────┐      ┌──── Backend (Rust) ────┐
│ images / editsById / cropById / dustById   │      │  catalog.rs (rusqlite) │
│ developMode / quality / selectedFolder …   │      │  ┌──────────────────┐  │
│                                            │      │  │   catalog.db     │  │
│  catalog.ts:                               │      │  │ images           │  │
│   • hydrate()  ──── load_catalog() ───────────────►│ edits            │  │
│   • write-through subs (debounced 400ms) ──┼─save_*─►│ prefs            │  │
│   • flush on quit                          │      │  │ app_state        │  │
└────────────────────────────────────────────┘      │  └──────────────────┘  │
                                                     └────────────────────────┘
```

## Storage location

`app.path().app_data_dir()` (Tauri), e.g.
`~/Library/Application Support/OpenEnlarge/catalog.db` on macOS. Created on first
launch if absent. Opened in **WAL** mode for crash-safety; every write is a
transaction.

## Schema (v1)

Four tables. Versioned via `PRAGMA user_version`; a small migration runner
applies versioned steps (starts at v1).

### `images`
| column | type | notes |
|---|---|---|
| `id` | TEXT PRIMARY KEY | UUID, stable identity |
| `path` | TEXT **UNIQUE** | re-import of same path → existing entry (dedupe) |
| `file_name` | TEXT | |
| `metadata` | TEXT | JSON of `Metadata` |
| `thumbnail` | TEXT | base64 data-URL; updated to the inverted thumb after develop |
| `added_at` | INTEGER | unix seconds (for stable import ordering) |

### `edits`
| column | type | notes |
|---|---|---|
| `image_id` | TEXT PK / FK → images.id | one row per image |
| `params_json` | TEXT | full `InvertParams` |
| `crop_json` | TEXT | `CropRect | null` + geometry |
| `dust_json` | TEXT | `DustEdits` (strokes + IR removal) |

**JSON-blob bet (the key design choice):** edit families are stored as opaque
JSON rather than a normalized column-per-param schema. `InvertParams` evolves
constantly; serde `#[serde(default)]` already backfills missing fields when an
older JSON is deserialized, so **adding a develop param requires no schema
migration**. Only structural changes (new tables/columns) use `user_version`
migrations.

### `prefs` (key/value)
- `develop_mode` — `"b" | "c"` (replaces the localStorage entry)
- `quality` — `"performance" | "quality"`

### `app_state` (key/value)
- `selected_folder`, `active_id`, `grid_zoom`, `module`

## Identity change

The backend image key moves from the session-scoped `img{n}` counter to the
catalog **UUID**. Ids are opaque to the frontend, so the stores are unaffected.
`Session.next_id` retires. `import_image` upserts by `path`: an already-cataloged
path returns its existing id (and, at startup, its edits via `load_catalog`).

## What persists vs. what doesn't

**Persisted:** import list (by reference — *never pixels*), all per-image edits
(params/crop/dust+IR), the latest thumbnail, prefs, session state.

**Not persisted:** decoded working pixels. Per the existing memory-bounded
architecture, relaunch shows images as *undeveloped*; a re-run of "Develop all"
re-decodes. **Every adjustment is already restored**, so re-develop reproduces
the exact prior look. `developed` and `has_ir` are runtime decode flags — `false`
on load until re-develop. The stored thumbnail is the last (inverted-after-
develop) one, so the library still *looks* developed on relaunch.

## Missing files

On `load_catalog`, each `path` is checked with `exists()`. Missing → entry kept,
new boolean field **`offline`** set on `ImageEntry` (frontend type updated too).
Library shows an offline badge; Develop/Export are disabled for offline entries.
Nothing is dropped when a drive is temporarily unplugged. **Relink UI is out of
scope** (noted follow-up) — offline entries simply reactivate when their path is
valid again on a later launch.

## Backend changes (`app/src-tauri/`)

- **New module `catalog.rs`** (rusqlite): open/create db, migration runner, and
  CRUD: `upsert_image`, `save_edits`, `save_crop`, `save_dust`, `save_image`
  (thumbnail/metadata), `save_pref`, `save_app_state`, `load_all`, `delete_image`.
- **`Session`** gains a handle to the catalog (or the catalog is a separate
  Tauri-managed state). Image key becomes the catalog UUID.
- **New commands:** `load_catalog`, `save_edits`, `save_crop`, `save_dust`,
  `save_image`, `save_pref`, `save_app_state`.
- **Extended commands:** `import_image` (upsert + dedupe by path),
  `delete_image` (remove catalog rows).
- **Dependency:** add `rusqlite` (bundled SQLite) + `uuid` to
  `app/src-tauri/Cargo.toml`.

## Frontend changes (`app/src/lib/`)

- **New module `catalog.ts`:**
  - `hydrate()` — called once on mount (`+page.svelte`/`+layout`): awaits
    `load_catalog()`, populates `images`, `editsById`, `cropById`, `dustById`,
    `developMode`, `quality`, `selectedFolder`, `activeId`, `gridZoom`, `module`.
  - **Write-through layer:** subscribes to the stores and calls the `save_*`
    commands, **debounced ~400ms**, coalescing rapid edits into one write. All
    persistence wiring lives here so the stores stay pure.
  - **Flush on quit:** listens to Tauri `close-requested` (and/or
    `beforeunload`) and flushes pending writes.
- **`store.ts`:** `developMode` persistence moves from localStorage to the
  catalog. Stores otherwise unchanged.
- **`api.ts`:** new command bindings; `ImageEntry` gains `offline: boolean`.

## Testing

**Rust (temp-file SQLite):**
- `upsert_image` dedupes by path (same path → same id, edits preserved).
- `edits`/`crop`/`dust` JSON round-trip.
- `prefs` and `app_state` k/v round-trip.
- `load_all` returns every image + edits + prefs + state; sets `offline` for a
  non-existent path.
- migration runner bumps `user_version` and is idempotent on re-open.
- serde default backfills: deserializing an "old" params JSON missing a newly
  added field succeeds with the default.

**Frontend (vitest):**
- debounce coalesces N rapid edits into one save call.
- pure serialize/deserialize helpers round-trip.
- `hydrate()` populates every target store from a catalog payload.

## Out of scope (follow-ups)

- Relink UI for offline files (manual "locate file").
- Multiple catalogs / catalog switching.
- Preview-cache sidecar (previews stay inline in the thumbnail column).
- Catalog backup/export.
