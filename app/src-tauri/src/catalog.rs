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
