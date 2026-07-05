//! Durable SQLite catalog: image references, per-image edits, prefs, session state.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// One catalog image as sent to the frontend. `offline` is computed at load time.
/// `developed` and `has_ir` are set to false here; `load_catalog` in commands.rs
/// overrides them from cache-file presence.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogImage {
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub metadata: Value,
    pub thumbnail: String,
    pub offline: bool,
    pub developed: bool,
    pub has_ir: bool,
    /// Develop-time negative/positive classification (true = positive). Not stored in
    /// the catalog DB; defaults to false on reload and is reseeded when the image develops.
    #[serde(default)]
    pub positive: bool,
    /// True when the baked `thumbnail` was rendered by an older engine version than the
    /// current `film_core::ENGINE_VERSION` — the grid lazily regenerates these.
    #[serde(default)]
    pub thumb_stale: bool,
}

/// One image's stored edits. Stored as opaque JSON blobs; deserialized to `Value`
/// on load so the frontend receives structured data without a per-field schema.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEdits {
    pub image_id: String,
    pub params: Option<Value>,
    pub crop: Option<Value>,
    pub dust: Option<Value>,
    pub meta: Option<Value>,
}

/// The full catalog handed to the frontend on launch.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogSnapshot {
    pub images: Vec<CatalogImage>,
    pub edits: Vec<CatalogEdits>,
    pub prefs: HashMap<String, String>,
    pub app_state: HashMap<String, String>,
}

/// The on-disk catalog. Wraps a single SQLite connection behind a Mutex
/// (rusqlite Connection is not Sync).
pub struct Catalog {
    conn: Mutex<Connection>,
}

const SCHEMA_VERSION: i64 = 6;

impl Catalog {
    /// Open (creating if absent) the catalog at `db_path`. Enables WAL and migrates.
    pub fn open(db_path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        Self::init(conn)
    }

    /// In-memory catalog for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

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
            .query_row("SELECT id FROM images WHERE path = ?1", [path], |r| {
                r.get(0)
            })
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
            "UPDATE images SET thumbnail = ?2, metadata = ?3, thumb_version = ?4 WHERE id = ?1",
            rusqlite::params![id, thumbnail, metadata_json, film_core::ENGINE_VERSION as i64],
        )?;
        Ok(())
    }

    /// Persist just the thumbnail (the frontend's edited-look render). Lets the strip
    /// thumbnail survive relaunch instead of reverting to the develop-time default.
    /// Stamps the current engine version, so a freshly rendered thumbnail is no longer
    /// flagged stale.
    pub fn update_thumbnail(&self, id: &str, thumbnail: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE images SET thumbnail = ?2, thumb_version = ?3 WHERE id = ?1",
            rusqlite::params![id, thumbnail, film_core::ENGINE_VERSION as i64],
        )?;
        Ok(())
    }

    /// Invalidate the baked thumbnails for `ids` by stamping a version below the
    /// current engine version, so `load_images` reports them `thumb_stale` and the
    /// frontend regen worker rebakes them — even after a relaunch that interrupts an
    /// in-session regen. Mirror of `update_thumbnail` (which stamps the current version).
    pub fn invalidate_thumbnails(&self, ids: &[String]) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for id in ids {
            tx.execute("UPDATE images SET thumb_version = 0 WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove an image and its edits (atomically).
    pub fn delete_image(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM edits WHERE image_id = ?1", [id])?;
        tx.execute("DELETE FROM images WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Wipe catalog content — `images`, `edits`, and `app_state` — but preserve
    /// `prefs` (language, telemetry, API key, hotkeys). Backs the Settings
    /// "Reset all data" action; the app relaunches afterward. Truncates rows
    /// (never deletes the DB file) so the live connection stays valid on Windows.
    pub fn reset_content(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM edits", [])?;
        tx.execute("DELETE FROM images", [])?;
        tx.execute("DELETE FROM app_state", [])?;
        tx.commit()?;
        // VACUUM reclaims the freed pages; it cannot run inside a transaction.
        conn.execute("VACUUM", [])?;
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
            "SELECT id, path, file_name, metadata, thumbnail, thumb_version \
             FROM images ORDER BY added_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            let path: String = r.get(1)?;
            let metadata: String = r.get(3)?;
            let thumb_version: i64 = r.get(5)?;
            Ok(CatalogImage {
                id: r.get(0)?,
                offline: !exists(&path),
                path,
                file_name: r.get(2)?,
                metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
                thumbnail: r.get(4)?,
                developed: false,
                has_ir: false,
                positive: false,
                thumb_stale: thumb_version != film_core::ENGINE_VERSION as i64,
            })
        })?;
        rows.collect()
    }

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

    /// Upsert the metadata-override JSON for an image's edits row.
    pub fn save_meta(&self, image_id: &str, meta_json: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO edits (image_id, meta_json) VALUES (?1, ?2)
             ON CONFLICT(image_id) DO UPDATE SET meta_json = excluded.meta_json",
            rusqlite::params![image_id, meta_json],
        )?;
        Ok(())
    }

    /// Load every image's edits row.
    pub fn load_edits(&self) -> rusqlite::Result<Vec<CatalogEdits>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT image_id, params_json, crop_json, dust_json, meta_json FROM edits")?;
        let parse = |s: Option<String>| s.and_then(|t| serde_json::from_str(&t).ok());
        let rows = stmt.query_map([], |r| {
            Ok(CatalogEdits {
                image_id: r.get(0)?,
                params: parse(r.get(1)?),
                crop: parse(r.get(2)?),
                dust: parse(r.get(3)?),
                meta: parse(r.get(4)?),
            })
        })?;
        rows.collect()
    }

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

    // `table` must be one of the two hardcoded names; never call with user input.
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

    /// Aggregate everything for launch. `exists` decides each image's offline flag.
    pub fn snapshot(&self, exists: &dyn Fn(&str) -> bool) -> rusqlite::Result<CatalogSnapshot> {
        Ok(CatalogSnapshot {
            images: self.load_images(exists)?,
            edits: self.load_edits()?,
            prefs: self.load_prefs()?,
            app_state: self.load_app_state()?,
        })
    }

    /// Current schema version (for tests).
    #[cfg(test)]
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
            "BEGIN;
             CREATE TABLE images (
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
             );
             COMMIT;",
        )?;
    }
    if version < 2 {
        // Per-image editable metadata overrides (camera/lens/iso/shutter/aperture/
        // date/note), stored as one opaque JSON blob alongside the other edits.
        add_column_if_missing(conn, "edits", "meta_json", "TEXT")?;
    }
    if version < 3 {
        // Render-engine version the baked `thumbnail` was rendered with. 0 = "before
        // versioning" (always stale vs the current engine). Lets the grid lazily
        // regenerate thumbnails whose look predates an engine change (e.g. the filmic
        // curve) instead of showing the old look until each image is opened.
        add_column_if_missing(conn, "images", "thumb_version", "INTEGER NOT NULL DEFAULT 0")?;
    }
    if version < 4 {
        // Reset every stored `exposure` to 0. The Faithful exposure response changed from
        // the weak shared EXPO_K (0.14) to a photographic FAITHFUL_EXPO_K (1.0), so the old
        // auto-seeded slider values (mostly the −3 clamp) now mean ~8× too dark. Zeroing them
        // makes each image re-solve auto-exposure at the new strength (grid sweep on entry +
        // seedExposure on develop). Other params are preserved. JSON1 is built into SQLite.
        conn.execute_batch(
            "UPDATE edits
             SET params_json = json_set(params_json, '$.exposure', 0)
             WHERE params_json IS NOT NULL
               AND json_valid(params_json)
               AND json_extract(params_json, '$.exposure') IS NOT NULL;",
        )?;
    }
    if version < 5 {
        // Absolute white balance: fold the hidden `wb_baseline` gains into the visible Temp/Tint
        // so the sliders carry the white balance (the absolute-WB model). Old frames stored WB in
        // wb_baseline with temp=5500/tint=0; the render — wb_baseline × wb_from_kelvin(temp,tint) —
        // is unchanged because the folded Temp/Tint reproduce the same gains. Folding makes every
        // frame new-model (identity baseline) so copy/apply can't double-apply a baseline. Done in
        // Rust because it needs gains_to_cct, not SQL.
        migrate_wb_to_absolute(conn)?;
    }
    if version < 6 {
        // #41 exposure-baseline anchor: engine.rs folded FAITHFUL_BASELINE_EV
        // (−2.25 EV) into the Faithful gain so a negative at base+0.6D density
        // renders at mid-gray (8-bit 118) by default. Stored exposures predate
        // that anchor; adding the OPPOSITE shift reproduces each edit's old
        // appearance exactly (the baseline enters the render identically to the
        // slider, so engine_new(ev + shift) == engine_old(ev)) — baked thumbnails
        // stay visually valid. Positives skip the shift: their render path never
        // passes the negative baseline. The `baseline_v` stamp + IS NULL guard
        // make each statement idempotent (a crash before the version bump re-runs
        // migrate; stamped rows are never re-shifted — same philosophy as
        // add_column_if_missing). Shifted values may pass the ±5 slider range;
        // the engine renders raw values, the UI merely pins the thumb.
        let shift = -film_core::engine::FAITHFUL_BASELINE_EV;
        conn.execute_batch(&format!(
            "UPDATE edits
             SET params_json = json_set(params_json,
                 '$.exposure', json_extract(params_json, '$.exposure') + {shift},
                 '$.baseline_v', 2)
             WHERE params_json IS NOT NULL
               AND json_valid(params_json)
               AND json_extract(params_json, '$.baseline_v') IS NULL
               AND json_extract(params_json, '$.exposure') IS NOT NULL
               AND COALESCE(json_extract(params_json, '$.positive'), 0) = 0;
             UPDATE edits
             SET params_json = json_set(params_json, '$.baseline_v', 2)
             WHERE params_json IS NOT NULL
               AND json_valid(params_json)
               AND json_extract(params_json, '$.baseline_v') IS NULL;",
        ))?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// v5 fold: rewrite each edit's `wb_baseline`/`temp`/`tint` so the visible Temp/Tint carry the
/// full white balance and `wb_baseline` is identity, preserving the rendered gains. Frames already
/// at an identity baseline are left untouched (no round-trip drift).
fn migrate_wb_to_absolute(conn: &Connection) -> rusqlite::Result<()> {
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT image_id, params_json FROM edits \
             WHERE params_json IS NOT NULL AND json_valid(params_json)",
        )?;
        let it = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        it.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let rgb = |v: &Value, k: &str| -> Option<[f32; 3]> {
        let a = v.get(k)?.as_array()?;
        if a.len() != 3 { return None; }
        Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
    };
    for (id, json) in rows {
        let Ok(mut v) = serde_json::from_str::<Value>(&json) else { continue };
        let base = rgb(&v, "wb_baseline").unwrap_or([1.0, 1.0, 1.0]);
        // Already identity (new-model frame) → skip, so we don't introduce round-trip drift.
        if base.iter().all(|c| (c - 1.0).abs() < 1e-4) { continue; }
        let temp = v.get("temp").and_then(|x| x.as_f64()).unwrap_or(5500.0) as f32;
        let tint = v.get("tint").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        let slider = film_core::wb::wb_from_kelvin(temp, tint / 150.0);
        let total = [base[0] * slider[0], base[1] * slider[1], base[2] * slider[2]];
        let (t2, ti2) = film_core::wb::gains_to_cct(total);
        v["temp"] = serde_json::json!(t2);
        v["tint"] = serde_json::json!(ti2 * 150.0);
        v["wb_baseline"] = serde_json::json!([1.0, 1.0, 1.0]);
        if let Ok(out) = serde_json::to_string(&v) {
            conn.execute(
                "UPDATE edits SET params_json = ?2 WHERE image_id = ?1",
                rusqlite::params![id, out],
            )?;
        }
    }
    Ok(())
}

/// Idempotent `ALTER TABLE ADD COLUMN`. A bare ALTER auto-commits immediately, so a
/// run that added the column but crashed before bumping `user_version` would re-run
/// the ALTER on next launch and fail with "duplicate column". Guarding on
/// `PRAGMA table_info` makes the migration safe to re-apply.
fn add_column_if_missing(conn: &Connection, table: &str, col: &str, decl: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))? // column 1 = name
        .filter_map(Result::ok)
        .any(|name| name == col);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl};"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_resets_stale_exposure_but_keeps_other_params() {
        let cat = Catalog::open_in_memory().unwrap();
        {
            let conn = cat.conn.lock().unwrap();
            // Simulate a pre-v4 edit carrying a stale auto-seeded exposure plus other params.
            conn.execute(
                "INSERT INTO edits (image_id, params_json) VALUES ('x', ?1)",
                [r#"{"exposure":-3.0,"black":0.1,"temp":5500}"#],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 3i64).unwrap(); // pretend we're pre-v4
            migrate(&conn).unwrap(); // the v<4 step resets exposure
        }
        let edits = cat.load_edits().unwrap();
        let p = edits.iter().find(|e| e.image_id == "x").unwrap().params.clone().unwrap();
        // v<4 resets the stale exposure to 0; the v<6 baseline anchor then shifts
        // it by −FAITHFUL_BASELINE_EV (= +2.25) into the anchored space.
        let shift = -film_core::engine::FAITHFUL_BASELINE_EV as f64;
        assert!(
            (p.get("exposure").unwrap().as_f64().unwrap() - shift).abs() < 1e-6,
            "stale exposure reset to 0 then anchored (+{shift})"
        );
        assert_eq!(p.get("black").unwrap().as_f64().unwrap(), 0.1, "other params preserved");
        assert_eq!(p.get("temp").unwrap().as_f64().unwrap(), 5500.0, "other params preserved");
    }

    #[test]
    fn migration_v6_shifts_negatives_into_anchored_baseline_space() {
        let cat = Catalog::open_in_memory().unwrap();
        {
            let conn = cat.conn.lock().unwrap();
            // Pre-v6 edits: a negative with a manual −2 EV correction, and a positive.
            conn.execute(
                "INSERT INTO edits (image_id, params_json) VALUES ('neg', ?1)",
                [r#"{"exposure":-2.0,"contrast":15}"#],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO edits (image_id, params_json) VALUES ('pos', ?1)",
                [r#"{"exposure":1.2,"positive":true}"#],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 5i64).unwrap(); // pre-v6
            migrate(&conn).unwrap();
            // Crash-window idempotence: re-running migrate (version rolled back as if
            // the bump never landed) must NOT double-shift stamped rows.
            conn.pragma_update(None, "user_version", 5i64).unwrap();
            migrate(&conn).unwrap();
        }
        let edits = cat.load_edits().unwrap();
        let p = |id: &str| edits.iter().find(|e| e.image_id == id).unwrap().params.clone().unwrap();
        let shift = -film_core::engine::FAITHFUL_BASELINE_EV as f64;
        let neg = p("neg");
        assert!(
            (neg.get("exposure").unwrap().as_f64().unwrap() - (-2.0 + shift)).abs() < 1e-6,
            "negative shifted once into the anchored space: {neg:?}"
        );
        assert_eq!(neg.get("contrast").unwrap().as_f64().unwrap(), 15.0, "other params preserved");
        assert_eq!(neg.get("baseline_v").unwrap().as_i64().unwrap(), 2);
        let pos = p("pos");
        assert!(
            (pos.get("exposure").unwrap().as_f64().unwrap() - 1.2).abs() < 1e-6,
            "positive exposure untouched (no negative baseline in its path): {pos:?}"
        );
        assert_eq!(pos.get("baseline_v").unwrap().as_i64().unwrap(), 2);
    }

    #[test]
    fn migration_folds_wb_baseline_into_temp_tint() {
        let cat = Catalog::open_in_memory().unwrap();
        {
            let conn = cat.conn.lock().unwrap();
            // Pre-v5 old-model edit: WB lives in the hidden baseline, sliders neutral.
            conn.execute(
                "INSERT INTO edits (image_id, params_json) VALUES ('x', ?1)",
                [r#"{"wb_baseline":[1.2,1.0,0.8],"temp":5500,"tint":0,"contrast":15}"#],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 4i64).unwrap(); // pre-v5
            migrate(&conn).unwrap();
        }
        let edits = cat.load_edits().unwrap();
        let p = edits.iter().find(|e| e.image_id == "x").unwrap().params.clone().unwrap();
        // Baseline reset to identity; other params preserved.
        let b = p.get("wb_baseline").unwrap().as_array().unwrap();
        assert!((b[0].as_f64().unwrap() - 1.0).abs() < 1e-6 && (b[2].as_f64().unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(p.get("contrast").unwrap().as_f64().unwrap(), 15.0);
        // The folded Temp/Tint reproduce the original WB. The Planckian-locus model
        // (issue #14) snaps an arbitrary baseline onto the blackbody curve, so it
        // preserves the perceptual WB *direction* (R/B ratio) exactly while the
        // absolute per-channel magnitudes settle onto the nearest on-locus white
        // (~3 % here) — the render is visually unchanged.
        let temp = p.get("temp").unwrap().as_f64().unwrap() as f32;
        let tint = p.get("tint").unwrap().as_f64().unwrap() as f32;
        let g = film_core::wb::wb_from_kelvin(temp, tint / 150.0);
        assert!((g[0] / g[2] - 1.2 / 0.8).abs() < 0.02, "R/B direction not preserved: {g:?}");
        for (c, want) in [1.2f32, 1.0, 0.8].iter().enumerate() {
            assert!((g[c] / g[1] - want / 1.0).abs() < 0.05, "ch {c}: gains {g:?} vs [1.2,1,0.8]");
        }
    }

    #[test]
    fn open_creates_schema_at_current_version() {
        let cat = Catalog::open_in_memory().unwrap();
        assert_eq!(cat.user_version(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_is_idempotent_on_reopen() {
        let dir = std::env::temp_dir().join(format!("oe-cat-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("catalog.db");
        let _ = std::fs::remove_file(&db);
        {
            let cat = Catalog::open(&db).unwrap();
            assert_eq!(cat.user_version(), SCHEMA_VERSION);
        }
        // Reopen: should not error and stay at the current version.
        let cat = Catalog::open(&db).unwrap();
        assert_eq!(cat.user_version(), SCHEMA_VERSION);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn meta_override_round_trips() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.save_params("img-1", r#"{"exposure":1.0}"#).unwrap();
        cat.save_meta("img-1", r#"{"camera":"Leica M6","note":"roll 12"}"#)
            .unwrap();
        let edits = cat.load_edits().unwrap();
        assert_eq!(edits.len(), 1);
        let m = edits[0].meta.as_ref().unwrap();
        assert_eq!(m["camera"], "Leica M6");
        assert_eq!(m["note"], "roll 12");
        // Co-stored families are untouched by a meta save.
        assert_eq!(edits[0].params.as_ref().unwrap()["exposure"], 1.0);
    }

    #[test]
    fn upsert_dedupes_by_path_and_keeps_id() {
        let cat = Catalog::open_in_memory().unwrap();
        let id1 = cat
            .upsert_image("/x/a.dng", "a.dng", "{}", "thumb1", 100)
            .unwrap();
        // Re-import the same path with a new thumbnail → same id, updated row.
        let id2 = cat
            .upsert_image("/x/a.dng", "a.dng", "{}", "thumb2", 200)
            .unwrap();
        assert_eq!(id1, id2);
        let imgs = cat.load_images(&|_| true).unwrap();
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].thumbnail, "thumb2");
    }

    #[test]
    fn load_images_sets_offline_when_missing() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.upsert_image("/x/here.dng", "here.dng", "{}", "t", 1)
            .unwrap();
        cat.upsert_image("/x/gone.dng", "gone.dng", "{}", "t", 2)
            .unwrap();
        let imgs = cat.load_images(&|p| p == "/x/here.dng").unwrap();
        assert_eq!(imgs.len(), 2);
        assert!(!imgs[0].offline); // here.dng exists
        assert!(imgs[1].offline); // gone.dng missing
    }

    #[test]
    fn delete_image_removes_row() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/x/a.dng", "a.dng", "{}", "t", 1).unwrap();
        cat.save_params(&id, r#"{"exposure":1.0}"#).unwrap();
        cat.delete_image(&id).unwrap();
        assert!(cat.load_images(&|_| true).unwrap().is_empty());
        assert!(cat.load_edits().unwrap().is_empty());
    }

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

    #[test]
    fn prefs_round_trip_and_overwrite() {
        let cat = Catalog::open_in_memory().unwrap();
        cat.save_pref("develop_mode", "c").unwrap();
        cat.save_pref("quality", "performance").unwrap();
        cat.save_pref("develop_mode", "b").unwrap(); // overwrite
        let prefs = cat.load_prefs().unwrap();
        assert_eq!(prefs.get("develop_mode").map(String::as_str), Some("b"));
        assert_eq!(
            prefs.get("quality").map(String::as_str),
            Some("performance")
        );
    }

    #[test]
    fn migration_idempotent_after_partial_apply() {
        // Reproduce the crash: a prior run added a column (bare ALTER auto-commits)
        // but didn't reach pragma_update(user_version), leaving the column present at
        // an older version. Re-running migrate must NOT panic on "duplicate column".
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap(); // full migrate → current schema, columns present
        conn.pragma_update(None, "user_version", 1i64).unwrap(); // simulate stuck version
        migrate(&conn).unwrap(); // must succeed (idempotent), not duplicate-column panic
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn thumb_version_tracks_engine_and_clears_on_render() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/p.raw", "p.raw", "{}", "data:thumb", 1).unwrap();
        // A freshly imported row is at thumb_version 0 → stale vs the current engine.
        let before = cat.load_images(&|_| true).unwrap();
        assert!(before[0].thumb_stale, "v0 thumbnail must read stale");
        // Rendering (update_thumbnail) stamps the current version → no longer stale.
        cat.update_thumbnail(&id, "data:new").unwrap();
        let after = cat.load_images(&|_| true).unwrap();
        assert!(!after[0].thumb_stale, "rendered thumbnail must read current");
        assert_eq!(after[0].thumbnail, "data:new");
    }

    #[test]
    fn old_engine_version_thumbnail_loads_stale() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/x/a.dng", "a.dng", "{}", "thumb", 0).unwrap();
        // Stamp a render at an OLDER engine version (current - 1) directly.
        {
            let conn = cat.conn.lock().unwrap();
            conn.execute(
                "UPDATE images SET thumb_version = ?2 WHERE id = ?1",
                rusqlite::params![id, (film_core::ENGINE_VERSION as i64) - 1],
            ).unwrap();
        }
        let imgs = cat.load_images(&|_| true).unwrap();
        let me = imgs.iter().find(|i| i.id == id).unwrap();
        assert!(me.thumb_stale, "thumbnail rendered by an older engine must load stale");
    }

    #[test]
    fn invalidate_thumbnails_marks_stale() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/x/a.dng", "a.dng", "{}", "thumb", 0).unwrap();
        // A render stamps the current engine version → not stale.
        cat.update_thumbnail(&id, "data:fresh").unwrap();
        assert!(!cat.load_images(&|_| true).unwrap()[0].thumb_stale);
        // Invalidation drops it below the current version → loads stale again.
        cat.invalidate_thumbnails(&[id.clone()]).unwrap();
        let imgs = cat.load_images(&|_| true).unwrap();
        assert!(
            imgs.iter().find(|i| i.id == id).unwrap().thumb_stale,
            "invalidated thumbnail must load stale"
        );
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

    #[test]
    fn snapshot_aggregates_everything() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat
            .upsert_image("/x/a.dng", "a.dng", r#"{"width":100}"#, "t", 1)
            .unwrap();
        cat.save_params(&id, r#"{"exposure":1.0}"#).unwrap();
        cat.save_pref("develop_mode", "c").unwrap();
        cat.save_app_state("module", "library").unwrap();
        let snap = cat.snapshot(&|_| true).unwrap();
        assert_eq!(snap.images.len(), 1);
        assert_eq!(snap.edits.len(), 1);
        assert_eq!(snap.edits[0].image_id, id);
        assert_eq!(
            snap.prefs.get("develop_mode").map(String::as_str),
            Some("c")
        );
        assert_eq!(
            snap.app_state.get("module").map(String::as_str),
            Some("library")
        );
    }

    #[test]
    fn reset_content_clears_catalog_but_keeps_prefs() {
        let cat = Catalog::open_in_memory().unwrap();
        let id = cat.upsert_image("/x/a.dng", "a.dng", "{}", "t", 1).unwrap();
        cat.save_params(&id, r#"{"exposure":1.0}"#).unwrap();
        cat.save_app_state("grid_zoom", "3").unwrap();
        cat.save_pref("telemetry", "on").unwrap();

        cat.reset_content().unwrap();

        // Content gone…
        assert!(cat.load_images(&|_| true).unwrap().is_empty());
        assert!(cat.load_edits().unwrap().is_empty());
        assert!(cat.load_app_state().unwrap().is_empty());
        // …but prefs preserved.
        assert_eq!(cat.load_prefs().unwrap().get("telemetry").map(String::as_str), Some("on"));

        // Idempotent: a second call is a no-op.
        cat.reset_content().unwrap();
        assert!(cat.load_images(&|_| true).unwrap().is_empty());
    }
}
