//! Database migrations.
//!
//! Version-based migration system for SQLite schema management.

use rusqlite::Connection;

use crate::StorageError;

/// Current schema version.
pub const CURRENT_VERSION: u32 = 1;

/// Run all pending migrations to bring the database up to date.
pub fn run_migrations(conn: &Connection) -> Result<(), StorageError> {
    // Create the migrations tracking table if it doesn't exist
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .map_err(|e| StorageError::MigrationFailed(e.to_string()))?;

    let current_version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_version < 1 {
        v001_init(conn)?;
    }

    Ok(())
}

/// V001: Initial schema — creates all required tables.
fn v001_init(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "
        -- Books table
        CREATE TABLE IF NOT EXISTS books (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            author TEXT NOT NULL DEFAULT '',
            format TEXT NOT NULL,
            file_hash TEXT,
            file_size INTEGER,
            cover_path TEXT,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- Reading progress table
        CREATE TABLE IF NOT EXISTS reading_progress (
            book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
            cfi_position TEXT NOT NULL,
            percentage REAL NOT NULL DEFAULT 0.0,
            hlc_ts INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- Bookmarks table
        CREATE TABLE IF NOT EXISTS bookmarks (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            cfi_position TEXT NOT NULL,
            title TEXT,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_bookmarks_book_id ON bookmarks(book_id);

        -- Annotations table
        CREATE TABLE IF NOT EXISTS annotations (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            cfi_start TEXT NOT NULL,
            cfi_end TEXT NOT NULL,
            color_rgba TEXT NOT NULL DEFAULT '#FFFF00FF',
            note TEXT,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_annotations_book_id ON annotations(book_id);

        -- User preferences table
        CREATE TABLE IF NOT EXISTS preferences (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            hlc_ts INTEGER NOT NULL DEFAULT 0
        );

        -- Operation log for CRDT sync
        CREATE TABLE IF NOT EXISTS op_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            op_type TEXT NOT NULL,
            op_data TEXT NOT NULL,
            hlc_ts INTEGER NOT NULL,
            device_id TEXT NOT NULL,
            synced INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_op_log_synced ON op_log(synced, hlc_ts);

        -- Sync metadata table
        CREATE TABLE IF NOT EXISTS sync_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- Reading statistics table
        CREATE TABLE IF NOT EXISTS reading_stats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            session_start TEXT NOT NULL,
            session_end TEXT,
            pages_read INTEGER DEFAULT 0,
            duration_secs INTEGER DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_reading_stats_book_id ON reading_stats(book_id);

        -- Record migration
        INSERT INTO _migrations (version) VALUES (1);
        ",
    )
    .map_err(|e| StorageError::MigrationFailed(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_run_successfully() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"books".to_string()));
        assert!(tables.contains(&"reading_progress".to_string()));
        assert!(tables.contains(&"bookmarks".to_string()));
        assert!(tables.contains(&"annotations".to_string()));
        assert!(tables.contains(&"preferences".to_string()));
        assert!(tables.contains(&"op_log".to_string()));
        assert!(tables.contains(&"sync_metadata".to_string()));
        assert!(tables.contains(&"reading_stats".to_string()));
    }

    #[test]
    fn test_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Running again should not fail
        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }
}
