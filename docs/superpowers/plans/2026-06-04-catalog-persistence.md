# Catalog Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the entire OpenEnlarge catalog — imported images (by reference), every per-image edit, global preferences, and session/UI state — in a single SQLite database, and restore it on launch.

**Architecture:** A new Rust `catalog.rs` module owns a SQLite database (`catalog.db` in the Tauri app-data dir, WAL mode). The backend upserts image rows on import/develop/delete; the frontend writes edits/prefs/session-state through debounced `save_*` commands. On launch `load_catalog` returns the whole catalog and rehydrates both the in-memory `Session` (lightweight, undeveloped) and the Svelte stores. Edit families are stored as opaque JSON so adding a develop param needs no schema migration (serde `#[serde(default)]` backfills).

**Tech Stack:** Rust, rusqlite (bundled SQLite), uuid, Tauri 2, serde_json; frontend Svelte stores + TypeScript + vitest.

**Spec:** `docs/superpowers/specs/2026-06-04-catalog-persistence-design.md`

**Build note:** cargo is not on PATH in this env — every cargo command must be prefixed with `source "$HOME/.cargo/env" &&`. Run cargo commands from `app/src-tauri/`. Run frontend commands from `app/`.

---

## File Structure

**Backend (`app/src-tauri/src/`):**
- Create: `catalog.rs` — SQLite open/migrate + all CRUD + `CatalogSnapshot`. One responsibility: durable storage.
- Modify: `session.rs` — `ImageEntry` gains `offline`; `Session.insert` takes an explicit id; `Metadata` gains `Deserialize`; drop `next_id`.
- Modify: `commands.rs` — extend `import_image`/`develop_image`/`delete_image`; add `load_catalog`, `save_edits`, `save_crop`, `save_dust`, `save_pref`, `save_app_state`.
- Modify: `lib.rs` — register `catalog` module, open the DB in `setup`, manage `Catalog` state, register new commands.
- Modify: `Cargo.toml` — add `rusqlite` + `uuid`.

**Frontend (`app/src/lib/`):**
- Create: `catalog.ts` — pure serialize/deserialize + debounce helpers, `hydrate()`, `initPersistence()`.
- Create: `catalog.test.ts` — vitest for the pure helpers + hydrate.
- Modify: `api.ts` — new command bindings; `ImageEntry.offline`.
- Modify: `store.ts` — `developMode` persistence moves from localStorage to the catalog.
- Modify: `routes/+page.svelte` — call `hydrate()` + `initPersistence()` on mount.

---

## Task 1: Add Rust dependencies

**Files:**
- Modify: `app/src-tauri/Cargo.toml`

- [ ] **Step 1: Add deps**

In `app/src-tauri/Cargo.toml`, under `[dependencies]` (after `trash = "5"`), add:

```toml
rusqlite = { version = "0.31", features = ["bundled"] }
uuid = { version = "1", features = ["v4"] }
```

The `bundled` feature compiles SQLite from source — no system dependency, fully portable.

- [ ] **Step 2: Verify it builds**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo build 2>&1 | tail -5`
Expected: builds successfully (first build downloads + compiles SQLite, may take a minute).

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock
git commit -m "build(catalog): add rusqlite (bundled) + uuid deps"
```

---

## Task 2: Catalog module — open, migrate, schema v1

**Files:**
- Create: `app/src-tauri/src/catalog.rs`
- Modify: `app/src-tauri/src/lib.rs` (register module only)

- [ ] **Step 1: Register the module**

In `app/src-tauri/src/lib.rs`, add `mod catalog;` to the top module list (after `mod commands;`).

- [ ] **Step 2: Write the failing test**

Create `app/src-tauri/src/catalog.rs` with the test first:

```rust
//! Durable SQLite catalog: image references, per-image edits, prefs, session state.

use rusqlite::Connection;
use std::sync::Mutex;

/// The on-disk catalog. Wraps a single SQLite connection behind a Mutex
/// (rusqlite Connection is not Sync).
pub struct Catalog {
    conn: Mutex<Connection>,
}

const SCHEMA_VERSION: i64 = 1;

impl Catalog {
    /// Open (creating if absent) the catalog at `db_path`. Enables WAL and migrates.
    pub fn open(db_path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        Self::init(conn)
    }

    /// In-memory catalog for tests.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Current schema version (for tests).
    pub fn user_version(&self) -> i64 {
        self.conn
            .lock()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }
}

/// Apply versioned migrations. Idempotent: only runs steps above the current
/// `user_version`. v1 creates the four base tables.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE images (
                id        TEXT PRIMARY KEY,
                path      TEXT UNIQUE NOT NULL,
                file_name TEXT NOT NULL,
                metadata  TEXT NOT NULL,
                thumbnail TEXT NOT NULL,
                added_at  INTEGER NOT NULL
             );
             CREATE TABLE edits (
                image_id    TEXT PRIMARY KEY,
                params_json TEXT,
                crop_json   TEXT,
                dust_json   TEXT
             );
             CREATE TABLE prefs (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );
             CREATE TABLE app_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_schema_at_version_1() {
        let cat = Catalog::open_in_memory().unwrap();
        assert_eq!(cat.user_version(), 1);
    }

    #[test]
    fn migrate_is_idempotent_on_reopen() {
        let dir = std::env::temp_dir().join(format!("oe-cat-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("catalog.db");
        let _ = std::fs::remove_file(&db);
        {
            let cat = Catalog::open(&db).unwrap();
            assert_eq!(cat.user_version(), 1);
        }
        // Reopen: should not error and stay at version 1.
        let cat = Catalog::open(&db).unwrap();
        assert_eq!(cat.user_version(), 1);
        let _ = std::fs::remove_file(&db);
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib catalog:: 2>&1 | tail -15`
Expected: both `open_creates_schema_at_version_1` and `migrate_is_idempotent_on_reopen` PASS.

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/catalog.rs app/src-tauri/src/lib.rs
git commit -m "feat(catalog): SQLite open + migration runner + schema v1"
```

---

## Task 3: Image upsert + load + offline flag

**Files:**
- Modify: `app/src-tauri/src/catalog.rs`

- [ ] **Step 1: Write the failing test**

Add to `catalog.rs` (above `#[cfg(test)]`), the snapshot types and image methods:

```rust
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// One catalog image as sent to the frontend. `offline` is computed at load time.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogImage {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub metadata: Value,
    pub thumbnail: String,
    pub offline: bool,
}

impl Catalog {
    /// Insert a new image or update the existing row with the same `path`.
    /// Returns the stable id (a new UUID for new paths, the existing id otherwise),
    /// so re-importing a file preserves its id — and therefore its edits.
    pub fn upsert_image(
        &self,
        path: &str,
        file_name: &str,
        metadata_json: &str,
        thumbnail: &str,
        now: i64,
    ) -> rusqlite::Result<String> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn
            .query_row("SELECT id FROM images WHERE path = ?1", [path], |r| r.get(0))
            .ok();
        if let Some(id) = existing {
            conn.execute(
                "UPDATE images SET file_name = ?2, metadata = ?3, thumbnail = ?4 WHERE id = ?1",
                rusqlite::params![id, file_name, metadata_json, thumbnail],
            )?;
            Ok(id)
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO images (id, path, file_name, metadata, thumbnail, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![id, path, file_name, metadata_json, thumbnail, now],
            )?;
            Ok(id)
        }
    }

    /// Update an image's thumbnail + metadata (called after develop).
    pub fn update_image_render(
        &self,
        id: &str,
        thumbnail: &str,
        metadata_json: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE images SET thumbnail = ?2, metadata = ?3 WHERE id = ?1",
            rusqlite::params![id, thumbnail, metadata_json],
        )?;
        Ok(())
    }

    /// Remove an image and its edits.
    pub fn delete_image(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM edits WHERE image_id = ?1", [id])?;
        conn.execute("DELETE FROM images WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Load all images, ordered by import time. `exists` decides the offline flag
    /// (injected so tests don't touch the filesystem).
    pub fn load_images(
        &self,
        exists: &dyn Fn(&str) -> bool,
    ) -> rusqlite::Result<Vec<CatalogImage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, file_name, metadata, thumbnail FROM images ORDER BY added_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            let path: String = r.get(1)?;
            let metadata: String = r.get(3)?;
            Ok(CatalogImage {
                id: r.get(0)?,
                offline: !exists(&path),
                path,
                file_name: r.get(2)?,
                metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
                thumbnail: r.get(4)?,
            })
        })?;
        rows.collect()
    }
}
```

Add these tests inside `mod tests`:

```rust
    #[test]
    fn upsert_dedupes_by_path_and_keeps_id() {
        let cat = Catalog::open_in_memory().unwrap();
        let id1 = cat.upsert_image("/x/a.dng", "a.dng", "{}", "thumb1", 100).unwrap();
        // Re-import the same path with a new thumbnail → same id, updated row.
        let id2 = cat.upsert_image("/x/a.dng", "a.dng", "{}", "thumb2", 200).unwrap();
        assert_eq!(id1, id2);
        let imgs = cat.load_images(&|_| true).unwrap();
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].thumbnail, "thumb2");
    }

    #[test]
    fn load_images_sets_offline_when_missing() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.upsert_image("/x/here.dng", "here.dng", "{}", "t", 1).unwrap();
        cat.upsert_image("/x/gone.dng", "gone.dng", "{}", "t", 2).unwrap();
        let imgs = cat.load_images(&|p| p == "/x/here.dng").unwrap();
        assert_eq!(imgs.len(), 2);
        assert!(!imgs[0].offline); // here.dng exists
        assert!(imgs[1].offline);  // gone.dng missing
    }

    #[test]
    fn delete_image_removes_row() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/x/a.dng", "a.dng", "{}", "t", 1).unwrap();
        cat.delete_image(&id).unwrap();
        assert!(cat.load_images(&|_| true).unwrap().is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib catalog:: 2>&1 | tail -15`
Expected: all catalog tests PASS (5 total now).

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/catalog.rs
git commit -m "feat(catalog): image upsert/update/delete/load with offline flag"
```

---

## Task 4: Per-image edits save/load (JSON blobs)

**Files:**
- Modify: `app/src-tauri/src/catalog.rs`
- Modify: `app/src-tauri/src/session.rs` (serde-default backfill test)

- [ ] **Step 1: Write the failing test**

Add to `catalog.rs` `impl Catalog` block:

```rust
    /// Upsert the params JSON for an image's edits row.
    pub fn save_params(&self, image_id: &str, params_json: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO edits (image_id, params_json) VALUES (?1, ?2)
             ON CONFLICT(image_id) DO UPDATE SET params_json = excluded.params_json",
            rusqlite::params![image_id, params_json],
        )?;
        Ok(())
    }

    /// Upsert the crop JSON for an image's edits row.
    pub fn save_crop(&self, image_id: &str, crop_json: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO edits (image_id, crop_json) VALUES (?1, ?2)
             ON CONFLICT(image_id) DO UPDATE SET crop_json = excluded.crop_json",
            rusqlite::params![image_id, crop_json],
        )?;
        Ok(())
    }

    /// Upsert the dust JSON for an image's edits row.
    pub fn save_dust(&self, image_id: &str, dust_json: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO edits (image_id, dust_json) VALUES (?1, ?2)
             ON CONFLICT(image_id) DO UPDATE SET dust_json = excluded.dust_json",
            rusqlite::params![image_id, dust_json],
        )?;
        Ok(())
    }
```

Add the `CatalogEdits` type next to `CatalogImage`:

```rust
/// One image's stored edits. Each field is an opaque JSON string (or null).
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEdits {
    pub image_id: String,
    pub params: Option<Value>,
    pub crop: Option<Value>,
    pub dust: Option<Value>,
}
```

And the loader in `impl Catalog`:

```rust
    /// Load every image's edits row.
    pub fn load_edits(&self) -> rusqlite::Result<Vec<CatalogEdits>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT image_id, params_json, crop_json, dust_json FROM edits")?;
        let parse = |s: Option<String>| s.and_then(|t| serde_json::from_str(&t).ok());
        let rows = stmt.query_map([], |r| {
            Ok(CatalogEdits {
                image_id: r.get(0)?,
                params: parse(r.get(1)?),
                crop: parse(r.get(2)?),
                dust: parse(r.get(3)?),
            })
        })?;
        rows.collect()
    }
```

Add tests inside `mod tests`:

```rust
    #[test]
    fn edits_round_trip_each_family_independently() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.save_params("img-1", r#"{"exposure":1.5}"#).unwrap();
        cat.save_crop("img-1", r#"{"angle":2.0}"#).unwrap();
        cat.save_dust("img-1", r#"{"strokes":[]}"#).unwrap();
        let edits = cat.load_edits().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].image_id, "img-1");
        assert_eq!(edits[0].params.as_ref().unwrap()["exposure"], 1.5);
        assert_eq!(edits[0].crop.as_ref().unwrap()["angle"], 2.0);
        assert!(edits[0].dust.is_some());
    }

    #[test]
    fn save_params_twice_updates_in_place() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.save_params("img-1", r#"{"exposure":0.0}"#).unwrap();
        cat.save_params("img-1", r#"{"exposure":2.0}"#).unwrap();
        let edits = cat.load_edits().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].params.as_ref().unwrap()["exposure"], 2.0);
    }
```

- [ ] **Step 2: Run catalog tests**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib catalog:: 2>&1 | tail -15`
Expected: all catalog tests PASS (7 total).

- [ ] **Step 3: Write the serde-default backfill test**

This validates the JSON-blob bet: an old params JSON missing a newly-added field deserializes via `#[serde(default)]`. Add to `session.rs` `mod tests`:

```rust
    #[test]
    fn invert_params_backfills_missing_fields_via_serde_default() {
        // An "old" catalog blob saved before color-grading/tone-curve fields existed.
        let old = r#"{
            "mode":"b","stock":"none","base_rect":null,
            "exposure":0.0,"black":0.0,"gamma":0.4545,"auto_wb":true,
            "temp":5500.0,"tint":0.0,"contrast":0.0,"highlights":0.0,
            "shadows":0.0,"whites":0.0,"blacks":0.0,"texture":0.0,
            "vibrance":0.0,"saturation":0.0
        }"#;
        let p: InvertParams = serde_json::from_str(old).unwrap();
        assert_eq!(p.cg_blending, 50.0); // defaulted
        assert_eq!(p.tc_curve, super::identity_curve()); // defaulted
    }
```

- [ ] **Step 4: Run session tests**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib session:: 2>&1 | tail -15`
Expected: PASS, including `invert_params_backfills_missing_fields_via_serde_default`.

- [ ] **Step 5: Commit**

```bash
git add app/src-tauri/src/catalog.rs app/src-tauri/src/session.rs
git commit -m "feat(catalog): per-image edits JSON save/load + serde backfill test"
```

---

## Task 5: Prefs + app_state key/value

**Files:**
- Modify: `app/src-tauri/src/catalog.rs`

- [ ] **Step 1: Write the failing test**

Add to `impl Catalog`:

```rust
    /// Upsert a preference (e.g. develop_mode, quality).
    pub fn save_pref(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO prefs (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Upsert a session/UI state value (selected_folder, active_id, grid_zoom, module).
    pub fn save_app_state(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    fn load_kv(&self, table: &str) -> rusqlite::Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT key, value FROM {table}"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }

    pub fn load_prefs(&self) -> rusqlite::Result<HashMap<String, String>> {
        self.load_kv("prefs")
    }

    pub fn load_app_state(&self) -> rusqlite::Result<HashMap<String, String>> {
        self.load_kv("app_state")
    }
```

Note: `load_kv` interpolates a hardcoded table name (`prefs`/`app_state`) from internal callers only — never user input — so there's no injection surface.

Add tests:

```rust
    #[test]
    fn prefs_round_trip_and_overwrite() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.save_pref("develop_mode", "c").unwrap();
        cat.save_pref("quality", "performance").unwrap();
        cat.save_pref("develop_mode", "b").unwrap(); // overwrite
        let prefs = cat.load_prefs().unwrap();
        assert_eq!(prefs.get("develop_mode").map(String::as_str), Some("b"));
        assert_eq!(prefs.get("quality").map(String::as_str), Some("performance"));
    }

    #[test]
    fn app_state_round_trip() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.save_app_state("grid_zoom", "55").unwrap();
        cat.save_app_state("module", "develop").unwrap();
        let st = cat.load_app_state().unwrap();
        assert_eq!(st.get("grid_zoom").map(String::as_str), Some("55"));
        assert_eq!(st.get("module").map(String::as_str), Some("develop"));
    }
```

- [ ] **Step 2: Run tests**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib catalog:: 2>&1 | tail -15`
Expected: all PASS (9 catalog tests).

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/catalog.rs
git commit -m "feat(catalog): prefs + app_state key/value store"
```

---

## Task 6: `CatalogSnapshot` aggregator

**Files:**
- Modify: `app/src-tauri/src/catalog.rs`

- [ ] **Step 1: Write the failing test**

Add the snapshot type (next to `CatalogImage`):

```rust
/// The full catalog handed to the frontend on launch.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogSnapshot {
    pub images: Vec<CatalogImage>,
    pub edits: Vec<CatalogEdits>,
    pub prefs: HashMap<String, String>,
    pub app_state: HashMap<String, String>,
}
```

Add to `impl Catalog`:

```rust
    /// Aggregate everything for launch. `exists` decides each image's offline flag.
    pub fn snapshot(&self, exists: &dyn Fn(&str) -> bool) -> rusqlite::Result<CatalogSnapshot> {
        Ok(CatalogSnapshot {
            images: self.load_images(exists)?,
            edits: self.load_edits()?,
            prefs: self.load_prefs()?,
            app_state: self.load_app_state()?,
        })
    }
```

Add test:

```rust
    #[test]
    fn snapshot_aggregates_everything() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/x/a.dng", "a.dng", r#"{"width":100}"#, "t", 1).unwrap();
        cat.save_params(&id, r#"{"exposure":1.0}"#).unwrap();
        cat.save_pref("develop_mode", "c").unwrap();
        cat.save_app_state("module", "library").unwrap();
        let snap = cat.snapshot(&|_| true).unwrap();
        assert_eq!(snap.images.len(), 1);
        assert_eq!(snap.edits.len(), 1);
        assert_eq!(snap.edits[0].image_id, id);
        assert_eq!(snap.prefs.get("develop_mode").map(String::as_str), Some("c"));
        assert_eq!(snap.app_state.get("module").map(String::as_str), Some("library"));
    }
```

- [ ] **Step 2: Run tests**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib catalog:: 2>&1 | tail -15`
Expected: all PASS (10 catalog tests).

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/catalog.rs
git commit -m "feat(catalog): CatalogSnapshot aggregator for launch"
```

---

## Task 7: Session changes — `offline` field, explicit id, `Metadata: Deserialize`

**Files:**
- Modify: `app/src-tauri/src/session.rs`
- Modify: `app/src-tauri/src/metadata.rs`

- [ ] **Step 1: Add `Deserialize` to `Metadata`**

In `app/src-tauri/src/metadata.rs` line 7, change:

```rust
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
```
to:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
```

Ensure `use serde::{Deserialize, Serialize};` at the top of `metadata.rs` (add `Deserialize` if only `Serialize` is imported).

- [ ] **Step 2: Add `offline` to `ImageEntry` and change `Session.insert`**

In `app/src-tauri/src/session.rs`:

Add the field to `ImageEntry` (after `pub has_ir: bool,`):

```rust
    /// True when the referenced file is missing on disk (restored from catalog).
    #[serde(default)]
    pub offline: bool,
```

Replace the `Session` struct + `impl Session` to take an explicit id and drop `next_id`:

```rust
#[derive(Default)]
pub struct Session {
    pub images: Mutex<HashMap<String, CachedImage>>,
    pub quality: Mutex<Quality>,
}

impl Session {
    /// Insert a cached image under an explicit (catalog-assigned) id.
    pub fn insert_with_id(&self, id: String, img: CachedImage) -> ImageEntry {
        let entry = ImageEntry {
            id: id.clone(),
            path: img.path.clone(),
            file_name: img.file_name.clone(),
            thumbnail: img.thumbnail.clone(),
            metadata: img.metadata.clone(),
            developed: img.developed.is_some(),
            has_ir: img.developed.as_ref().map(|d| d.working.ir.is_some()).unwrap_or(false),
            offline: false,
        };
        self.images.lock().unwrap().insert(id, img);
        entry
    }
}
```

- [ ] **Step 3: Fix the existing session tests**

In `session.rs` `mod tests`, replace the two `insert`-based tests so they use `insert_with_id` and a known id (the old `img0` assumption is gone):

```rust
    #[test]
    fn insert_reports_undeveloped() {
        let s = Session::default();
        let img = CachedImage {
            path: "/x/a.dng".into(),
            file_name: "a.dng".into(),
            metadata: Metadata::default(),
            thumbnail: "data:,".into(),
            developed: None,
        };
        let e = s.insert_with_id("abc".into(), img);
        assert_eq!(e.id, "abc");
        assert!(!e.developed);
        assert!(!e.offline);
    }

    #[test]
    fn insert_reports_has_ir_false_when_undeveloped() {
        let s = Session::default();
        let img = CachedImage {
            path: "/x/a.tif".into(), file_name: "a.tif".into(),
            metadata: Metadata::default(), thumbnail: "data:,".into(), developed: None,
        };
        let e = s.insert_with_id("xyz".into(), img);
        assert!(!e.has_ir);
    }
```

- [ ] **Step 4: Run session tests**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib session:: 2>&1 | tail -15`
Expected: PASS. (Compilation will still fail elsewhere because `commands.rs` calls `session.insert` / builds `ImageEntry` without `offline` — fixed in Task 8. If `cargo test` won't compile the whole crate, that's expected; proceed to Task 8 and run the full build there.)

- [ ] **Step 5: Commit**

```bash
git add app/src-tauri/src/session.rs app/src-tauri/src/metadata.rs
git commit -m "feat(session): offline field, explicit-id insert, Metadata Deserialize"
```

---

## Task 8: Wire Catalog into commands + lib.rs

**Files:**
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Open + manage the Catalog in `lib.rs`**

In `app/src-tauri/src/lib.rs`, inside the `.setup(|app| { ... })` closure, BEFORE `Ok(())`, open the catalog in the app-data dir and manage it:

```rust
            use tauri::Manager;
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir).ok();
            let catalog = catalog::Catalog::open(&dir.join("catalog.db"))
                .expect("open catalog db");
            app.manage(catalog);
```

(There is already a `use tauri::Manager;` inside the closure — keep just one.)

- [ ] **Step 2: Extend `import_image` to upsert into the catalog**

In `app/src-tauri/src/commands.rs`, change the signature and body of `import_image` (currently lines ~184-194):

```rust
#[tauri::command]
pub fn import_image(
    path: String,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<ImageEntry, String> {
    let p = Path::new(&path);
    let thumbnail = match decode_tiff(p) {
        Ok(prev) => to_png_b64(&proxy(&prev, THUMB_EDGE), true)?,
        Err(_) => "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
    };
    let metadata = extract(p, 0, 0);
    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("image").to_string();
    let metadata_json = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let id = catalog
        .upsert_image(&path, &file_name, &metadata_json, &thumbnail, now)
        .map_err(|e| e.to_string())?;
    let cached = CachedImage { path, file_name, metadata, thumbnail, developed: None };
    Ok(session.insert_with_id(id, cached))
}
```

- [ ] **Step 3: Persist thumbnail+metadata after develop**

In `develop_image` (commands.rs ~200), add a `catalog` param and write back the new thumbnail/metadata. Change the signature to:

```rust
pub fn develop_image(
    id: String,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<ImageEntry, String> {
```

After the block that sets `img.thumbnail = thumbnail.clone();` and before building the returned `ImageEntry`, add (still inside the function, after `img.metadata.height = h;`):

```rust
    let metadata_json = serde_json::to_string(&img.metadata).map_err(|e| e.to_string())?;
    let _ = catalog.update_image_render(&id, &thumbnail, &metadata_json);
```

Then add `offline: false,` to the `ImageEntry { ... }` returned at the end of `develop_image`.

- [ ] **Step 4: Extend `delete_image` to remove the catalog row**

Change `delete_image` (commands.rs ~248) to:

```rust
#[tauri::command]
pub fn delete_image(
    id: String,
    delete_file: bool,
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    let removed = session.images.lock().unwrap().remove(&id);
    let _ = catalog.delete_image(&id);
    if delete_file {
        let img = removed.ok_or_else(|| "unknown image".to_string())?;
        trash::delete(&img.path).map_err(|e| format!("{e}"))?;
    }
    Ok(())
}
```

- [ ] **Step 5: Build (compile-check the whole crate)**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo build 2>&1 | tail -15`
Expected: compiles. (If `as_shot_wb`/`render_view`/`thumbnail`/`export_image` build `ImageEntry` literals, none do except develop/insert — but if the compiler flags a missing `offline` anywhere, add `offline: false`.)

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/lib.rs app/src-tauri/src/commands.rs
git commit -m "feat(catalog): upsert on import, persist on develop, remove on delete"
```

---

## Task 9: New persistence commands + registration

**Files:**
- Modify: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs`

- [ ] **Step 1: Add the `load_catalog` command (returns snapshot + rehydrates Session)**

Append to `commands.rs`:

```rust
/// Load the whole catalog at launch: return the snapshot to the frontend AND
/// repopulate the in-memory Session with lightweight (undeveloped) records so
/// `develop_image`/`render_view` can find each image by id.
#[tauri::command]
pub fn load_catalog(
    session: State<Session>,
    catalog: State<crate::catalog::Catalog>,
) -> Result<crate::catalog::CatalogSnapshot, String> {
    let snap = catalog
        .snapshot(&|p| Path::new(p).exists())
        .map_err(|e| e.to_string())?;
    let mut imgs = session.images.lock().unwrap();
    imgs.clear();
    for ci in &snap.images {
        let metadata = serde_json::from_value(ci.metadata.clone()).unwrap_or_default();
        imgs.insert(
            ci.id.clone(),
            CachedImage {
                path: ci.path.clone(),
                file_name: ci.file_name.clone(),
                metadata,
                thumbnail: ci.thumbnail.clone(),
                developed: None,
            },
        );
    }
    Ok(snap)
}
```

- [ ] **Step 2: Add the write-through commands**

Append to `commands.rs`:

```rust
#[tauri::command]
pub fn save_edits(
    id: String,
    params_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_params(&id, &params_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_crop(
    id: String,
    crop_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_crop(&id, &crop_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_dust(
    id: String,
    dust_json: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_dust(&id, &dust_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_pref(
    key: String,
    value: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_pref(&key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_app_state(
    key: String,
    value: String,
    catalog: State<crate::catalog::Catalog>,
) -> Result<(), String> {
    catalog.save_app_state(&key, &value).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register all new commands**

In `app/src-tauri/src/lib.rs`, extend the `tauri::generate_handler![ ... ]` list (after `commands::as_shot_wb,`):

```rust
            commands::load_catalog,
            commands::save_edits,
            commands::save_crop,
            commands::save_dust,
            commands::save_pref,
            commands::save_app_state,
```

- [ ] **Step 4: Build + run the full backend test suite**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib 2>&1 | tail -20`
Expected: compiles and all tests PASS (existing + new catalog tests).

- [ ] **Step 5: Clippy clean**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo clippy --lib 2>&1 | tail -15`
Expected: no warnings. (If `set_quality` should also persist, leave it — quality is persisted from the frontend via `save_pref` in Task 12.)

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(catalog): load_catalog + save_* commands, registered"
```

---

## Task 10: Frontend API bindings + `offline` field

**Files:**
- Modify: `app/src/lib/api.ts`

- [ ] **Step 1: Add `offline` to `ImageEntry`**

In `app/src/lib/api.ts`, change the `ImageEntry` interface to add `offline`:

```typescript
export interface ImageEntry {
  id: string; path: string; file_name: string; thumbnail: string;
  metadata: Metadata; developed: boolean; has_ir: boolean; offline: boolean;
}
```

- [ ] **Step 2: Add the snapshot types + command bindings**

In `api.ts`, add these types (after `ExportFormat`):

```typescript
/** One image's stored edits as returned by load_catalog (JSON already parsed). */
export interface CatalogEdits {
  image_id: string;
  params: InvertParams | null;
  crop: import("./crop/types").CropRect | null;
  dust: import("./develop/dust").DustEdits | null;
}
/** The whole catalog returned at launch. */
export interface CatalogSnapshot {
  images: ImageEntry[];
  edits: CatalogEdits[];
  prefs: Record<string, string>;
  app_state: Record<string, string>;
}
```

Note: the backend serializes `CatalogImage` with the same field names as `ImageEntry` (`id`, `path`, `file_name`, `thumbnail`, `metadata`, `offline`) but WITHOUT `developed`/`has_ir`. Add `developed: false` / `has_ir: false` defaults when hydrating (Task 12) — the snapshot `images` are undeveloped on load. To keep the wire type honest, declare the snapshot image shape separately:

```typescript
export interface CatalogImage {
  id: string; path: string; file_name: string; thumbnail: string;
  metadata: Metadata; offline: boolean;
}
export interface CatalogSnapshot {
  images: CatalogImage[];
  edits: CatalogEdits[];
  prefs: Record<string, string>;
  app_state: Record<string, string>;
}
```

(Use this `CatalogImage` version — replace the `images: ImageEntry[]` line above.)

Add to the `api` object (after `asShotWb`):

```typescript
  loadCatalog: () => invoke<CatalogSnapshot>("load_catalog"),
  saveEdits: (id: string, paramsJson: string) =>
    invoke<void>("save_edits", { id, paramsJson }),
  saveCrop: (id: string, cropJson: string) =>
    invoke<void>("save_crop", { id, cropJson }),
  saveDust: (id: string, dustJson: string) =>
    invoke<void>("save_dust", { id, dustJson }),
  savePref: (key: string, value: string) =>
    invoke<void>("save_pref", { key, value }),
  saveAppState: (key: string, value: string) =>
    invoke<void>("save_app_state", { key, value }),
```

- [ ] **Step 3: Typecheck**

Run: `cd app && npm run check 2>&1 | tail -20`
Expected: no NEW errors from `api.ts`. (Other files that build `ImageEntry` literals — e.g. tests/mocks — may now flag the missing `offline`; fix those in Task 13's typecheck step. If `svelte-check` fails only on test mocks, note them and continue.)

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/api.ts
git commit -m "feat(catalog): frontend api bindings + offline field"
```

---

## Task 11: `catalog.ts` pure helpers (debounce + serialize)

**Files:**
- Create: `app/src/lib/catalog.ts`
- Create: `app/src/lib/catalog.test.ts`

- [ ] **Step 1: Write the failing test**

Create `app/src/lib/catalog.test.ts`:

```typescript
import { describe, it, expect, vi } from "vitest";
import { debounce } from "./catalog";

describe("debounce", () => {
  it("coalesces rapid calls into one trailing invocation", async () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const d = debounce(fn, 400);
    d("a"); d("b"); d("c");
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(400);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith("c"); // last args win
    vi.useRealTimers();
  });

  it("flush() invokes the pending call immediately", () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const d = debounce(fn, 400);
    d("x");
    d.flush();
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith("x");
    vi.useRealTimers();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd app && npx vitest run src/lib/catalog.test.ts 2>&1 | tail -15`
Expected: FAIL — cannot import `debounce` from `./catalog` (module/file doesn't exist).

- [ ] **Step 3: Implement the helper**

Create `app/src/lib/catalog.ts`:

```typescript
import { api, type InvertParams } from "./api";
import type { CropRect } from "./crop/types";
import type { DustEdits } from "./develop/dust";

/** A debounced function with a `flush()` that fires any pending call now. */
export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  flush(): void;
}

/** Trailing-edge debounce: coalesce rapid calls; last args win. `flush()` fires now. */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: A | null = null;
  const wrapped = ((...args: A) => {
    pending = args;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const p = pending; pending = null;
      if (p) fn(...p);
    }, ms);
  }) as Debounced<A>;
  wrapped.flush = () => {
    if (timer) { clearTimeout(timer); timer = null; }
    const p = pending; pending = null;
    if (p) fn(...p);
  };
  return wrapped;
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd app && npx vitest run src/lib/catalog.test.ts 2>&1 | tail -15`
Expected: PASS (both debounce tests).

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/catalog.ts app/src/lib/catalog.test.ts
git commit -m "feat(catalog): debounce helper for write-through"
```

---

## Task 12: `hydrate()` + `initPersistence()`

**Files:**
- Modify: `app/src/lib/catalog.ts`
- Modify: `app/src/lib/catalog.test.ts`

- [ ] **Step 1: Write the failing test for `applySnapshot`**

`hydrate()` itself calls the Tauri `invoke` (hard to unit-test), so factor the pure store-population step into `applySnapshot(snap)` and test that. Add to `catalog.test.ts`:

```typescript
import { applySnapshot } from "./catalog";
import { get } from "svelte/store";
import {
  images, editsById, cropById, dustById, developMode, quality,
  selectedFolder, gridZoom, module as moduleStore, activeId,
} from "./store";
import type { CatalogSnapshot } from "./api";

describe("applySnapshot", () => {
  it("populates every store from a catalog snapshot", () => {
    const snap: CatalogSnapshot = {
      images: [{
        id: "a", path: "/x/a.dng", file_name: "a.dng", thumbnail: "t",
        metadata: { width: 100, height: 100, file_size: 0 }, offline: false,
      }],
      edits: [{
        image_id: "a",
        params: { ...({} as any), exposure: 1.5 } as any,
        crop: null,
        dust: { strokes: [], irRemoval: { enabled: false, sensitivity: 50 } },
      }],
      prefs: { develop_mode: "c", quality: "quality" },
      app_state: { selected_folder: "/x", grid_zoom: "70", module: "develop", active_id: "a" },
    };
    applySnapshot(snap);
    expect(get(images).length).toBe(1);
    expect(get(images)[0].developed).toBe(false);
    expect(get(editsById)["a"].exposure).toBe(1.5);
    expect(get(dustById)["a"].irRemoval.sensitivity).toBe(50);
    expect(get(developMode)).toBe("c");
    expect(get(quality)).toBe("quality");
    expect(get(selectedFolder)).toBe("/x");
    expect(get(gridZoom)).toBe(70);
    expect(get(moduleStore)).toBe("develop");
    expect(get(activeId)).toBe("a");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd app && npx vitest run src/lib/catalog.test.ts 2>&1 | tail -15`
Expected: FAIL — `applySnapshot` not exported.

- [ ] **Step 3: Implement `applySnapshot`, `hydrate`, `initPersistence`**

Append to `app/src/lib/catalog.ts`:

```typescript
import {
  images, editsById, cropById, dustById, developMode, quality,
  selectedFolder, gridZoom, module as moduleStore, activeId,
} from "./store";
import type { CatalogSnapshot, ImageEntry } from "./api";

/** Populate every store from a snapshot. Pure w.r.t. the stores (no IO). */
export function applySnapshot(snap: CatalogSnapshot): void {
  const editsMap: Record<string, InvertParams> = {};
  const cropMap: Record<string, CropRect | null> = {};
  const dustMap: Record<string, DustEdits> = {};
  for (const e of snap.edits) {
    if (e.params) editsMap[e.image_id] = e.params;
    if (e.crop !== undefined) cropMap[e.image_id] = e.crop;
    if (e.dust) dustMap[e.image_id] = e.dust;
  }
  const entries: ImageEntry[] = snap.images.map((ci) => ({
    id: ci.id, path: ci.path, file_name: ci.file_name, thumbnail: ci.thumbnail,
    metadata: ci.metadata, developed: false, has_ir: false, offline: ci.offline,
  }));
  images.set(entries);
  editsById.set(editsMap);
  cropById.set(cropMap);
  dustById.set(dustMap);

  if (snap.prefs.develop_mode === "b" || snap.prefs.develop_mode === "c")
    developMode.set(snap.prefs.develop_mode);
  if (snap.prefs.quality === "performance" || snap.prefs.quality === "quality")
    quality.set(snap.prefs.quality);

  const st = snap.app_state;
  if (st.selected_folder !== undefined)
    selectedFolder.set(st.selected_folder === "" ? null : st.selected_folder);
  if (st.grid_zoom !== undefined) {
    const z = Number(st.grid_zoom);
    if (Number.isFinite(z)) gridZoom.set(z);
  }
  if (st.module === "library" || st.module === "develop") moduleStore.set(st.module);
  if (st.active_id) activeId.set(st.active_id);
}

/** Load the catalog from the backend and populate the stores. Call once on mount. */
export async function hydrate(): Promise<void> {
  try {
    const snap = await api.loadCatalog();
    applySnapshot(snap);
  } catch (e) {
    console.error("catalog hydrate failed", e);
  }
}

// --- Write-through (debounced) ---------------------------------------------

const saveEdits = debounce((id: string, json: string) => { void api.saveEdits(id, json); }, 400);
const saveCrop = debounce((id: string, json: string) => { void api.saveCrop(id, json); }, 400);
const saveDust = debounce((id: string, json: string) => { void api.saveDust(id, json); }, 400);
const savePref = debounce((k: string, v: string) => { void api.savePref(k, v); }, 400);
const saveState = debounce((k: string, v: string) => { void api.saveAppState(k, v); }, 400);

/** Persist whichever entries changed (by reference) since the last snapshot. */
function wireRecord<T>(
  store: { subscribe: (cb: (v: Record<string, T>) => void) => () => void },
  save: (id: string, json: string) => void,
): () => void {
  let prev: Record<string, T> = {};
  let first = true;
  return store.subscribe((map) => {
    if (first) { prev = map; first = false; return; } // skip hydration's initial set
    for (const id in map) {
      if (map[id] !== prev[id]) save(id, JSON.stringify(map[id]));
    }
    prev = map;
  });
}

let started = false;

/** Wire all stores to debounced write-through. Idempotent. Returns a flush fn. */
export function initPersistence(): () => void {
  if (started) return () => {};
  started = true;

  wireRecord(editsById, saveEdits);
  wireRecord(cropById, saveCrop);
  wireRecord(dustById, saveDust);

  let first = { dm: true, q: true, sf: true, gz: true, mod: true, aid: true };
  developMode.subscribe((m) => { if (first.dm) { first.dm = false; return; } savePref("develop_mode", m); });
  quality.subscribe((q) => { if (first.q) { first.q = false; return; } savePref("quality", q); });
  selectedFolder.subscribe((p) => { if (first.sf) { first.sf = false; return; } saveState("selected_folder", p ?? ""); });
  gridZoom.subscribe((z) => { if (first.gz) { first.gz = false; return; } saveState("grid_zoom", String(z)); });
  moduleStore.subscribe((m) => { if (first.mod) { first.mod = false; return; } saveState("module", m); });
  activeId.subscribe((a) => { if (first.aid) { first.aid = false; return; } saveState("active_id", a ?? ""); });

  const flush = () => {
    saveEdits.flush(); saveCrop.flush(); saveDust.flush();
    savePref.flush(); saveState.flush();
  };
  if (typeof window !== "undefined")
    window.addEventListener("beforeunload", flush);
  return flush;
}
```

Note: the `first`-guard pattern skips the initial value every Svelte store emits on subscribe, so `initPersistence()` (called right after `hydrate()`) doesn't immediately re-save the just-loaded values. `wireRecord` skips its first emission for the same reason.

- [ ] **Step 4: Run to verify it passes**

Run: `cd app && npx vitest run src/lib/catalog.test.ts 2>&1 | tail -20`
Expected: PASS (debounce + applySnapshot tests).

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/catalog.ts app/src/lib/catalog.test.ts
git commit -m "feat(catalog): hydrate + initPersistence write-through"
```

---

## Task 13: Wire into app mount + drop localStorage developMode

**Files:**
- Modify: `app/src/lib/store.ts`
- Modify: `app/src/routes/+page.svelte`

- [ ] **Step 1: Remove the localStorage developMode persistence**

In `app/src/lib/store.ts`, replace the `developMode` block (the `storedMode` read + the `developMode.subscribe(... localStorage.setItem ...)`) with a plain store — the catalog now owns this pref:

```typescript
/** Global develop mode (B·density / C·per-chan). Set once in Settings; applies to
 * every image. Persisted via the catalog (see catalog.ts). */
export const developMode = writable<"b" | "c">("b");
```

Leave the `developMode.subscribe(...)` that re-applies mode to every image's params (lines ~27-37) intact.

- [ ] **Step 2: Call hydrate + initPersistence on mount**

In `app/src/routes/+page.svelte`, add to the `<script>` block:

```typescript
  import { onMount } from "svelte";
  import { hydrate, initPersistence } from "$lib/catalog";

  onMount(() => {
    let flush: (() => void) | undefined;
    hydrate().finally(() => { flush = initPersistence(); });
    return () => flush?.();
  });
```

(Order matters: `initPersistence` runs only after `hydrate` resolves, so the first-emission guards align with the loaded values.)

- [ ] **Step 3: Typecheck the whole frontend**

Run: `cd app && npm run check 2>&1 | tail -25`
Expected: 0 errors. If any test mock or fixture constructs an `ImageEntry` without `offline`, add `offline: false` to it. Search for offenders:

Run: `cd app && grep -rn "developed:" src --include="*.ts" --include="*.svelte" | grep -v "developed: false\|\.developed\|developed:true" | head`
Fix each `ImageEntry` literal to include `offline: false` (or `has_ir`/`offline` as needed).

- [ ] **Step 4: Run the full frontend test suite**

Run: `cd app && npx vitest run 2>&1 | tail -20`
Expected: all PASS (existing + catalog tests).

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/store.ts app/src/routes/+page.svelte
git commit -m "feat(catalog): hydrate on mount, retire localStorage developMode"
```

---

## Task 14: End-to-end verification

**Files:** none (manual/observational)

- [ ] **Step 1: Full backend suite + clippy**

Run: `cd app/src-tauri && source "$HOME/.cargo/env" && cargo test --lib 2>&1 | tail -10 && cargo clippy --lib 2>&1 | tail -5`
Expected: all tests PASS, clippy clean.

- [ ] **Step 2: Full frontend suite + typecheck**

Run: `cd app && npx vitest run 2>&1 | tail -10 && npm run check 2>&1 | tail -5`
Expected: all PASS, 0 type errors.

- [ ] **Step 3: Manual smoke test (user-driven via `/run` or `npm run tauri dev`)**

Verify, in a running app:
1. Import a few images, adjust develop sliders, set a crop, draw an eraser stroke, change develop mode + quality in Settings, select a folder.
2. **Quit and relaunch.** Confirm: the library shows the same images with their (inverted) thumbnails; the selected folder, active image, grid zoom, and module are restored; opening Develop and re-running "Develop all" reproduces every adjustment exactly.
3. Rename/move one source file on disk, relaunch → that image shows as **offline** (badge), others load normally.
4. Re-import an already-cataloged file → no duplicate appears; its edits are intact.

- [ ] **Step 4: Final commit (if any fixes were needed)**

```bash
git add -A app/   # only app/ — never the whole tree (concurrent session shares it)
git commit -m "test(catalog): end-to-end verification fixes"
```

Note: per the project's concurrent-session convention, stage only files you changed — never `git add -A` at repo root.

---

## Self-Review Notes (author)

- **Spec coverage:** single auto-library (Task 8 lib.rs), SQLite (Tasks 1-2), thumbnails stored (images.thumbnail, Tasks 3/8), offline (Tasks 3/7/12), autosave debounced + flush (Tasks 11-12), session restore (Tasks 5/12), JSON-blob edits + serde backfill (Tasks 4), identity→UUID (Tasks 3/7/8), dedupe by path (Task 3). All spec sections map to a task.
- **Param drift:** edits stored as opaque JSON; new `InvertParams` fields need no migration (Task 4 backfill test proves it).
- **Out of scope (per spec):** relink UI, multi-catalog, preview sidecar, backup.
