//! Database connection and CRUD operations.
//!
//! Provides the main `Database` handle and all table-specific operations.

use rusqlite::{params, Connection, OptionalExtension};

use shared_types::{Annotation, Bookmark, ReadingProgress, UserPreference};

use crate::StorageError;

fn init_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "
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

        CREATE TABLE IF NOT EXISTS reading_progress (
            book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
            cfi_position TEXT NOT NULL,
            percentage REAL NOT NULL DEFAULT 0.0,
            hlc_ts INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS bookmarks (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            cfi_position TEXT NOT NULL,
            title TEXT,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_bookmarks_book_id ON bookmarks(book_id);

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

        CREATE TABLE IF NOT EXISTS preferences (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            hlc_ts INTEGER NOT NULL DEFAULT 0
        );

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

        CREATE TABLE IF NOT EXISTS sync_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS reading_stats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            session_start TEXT NOT NULL,
            session_end TEXT,
            pages_read INTEGER DEFAULT 0,
            duration_secs INTEGER DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_reading_stats_book_id ON reading_stats(book_id);
        ",
    )
    .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

    Ok(())
}

/// Main database handle.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create a database at the given path.
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Get a reference to the underlying connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // ─── Books CRUD ──────────────────────────────────────────────────────────

    /// Insert or replace a book record.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_book(
        &self,
        id: &str,
        title: &str,
        author: &str,
        format: &str,
        file_hash: Option<&str>,
        file_size: Option<u64>,
        cover_path: Option<&str>,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO books (id, title, author, format, file_hash, file_size, cover_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, title, author, format, file_hash, file_size.map(|s| s as i64), cover_path],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    /// Get a book by ID.
    pub fn get_book(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, String, String)>, StorageError> {
        let result = self
            .conn
            .query_row(
                "SELECT id, title, author, format FROM books WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(result)
    }

    /// Delete a book by ID.
    pub fn delete_book(&self, id: &str) -> Result<bool, StorageError> {
        let rows = self
            .conn
            .execute("DELETE FROM books WHERE id = ?1", params![id])
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(rows > 0)
    }

    // ─── Reading Progress CRUD ───────────────────────────────────────────────

    /// Update reading progress for a book.
    pub fn upsert_progress(&self, progress: &ReadingProgress) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO reading_progress (book_id, cfi_position, percentage, hlc_ts)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    progress.book_id,
                    progress.cfi_position,
                    progress.percentage,
                    progress.hlc_timestamp as i64,
                ],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    /// Get reading progress for a book.
    pub fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StorageError> {
        let result = self
            .conn
            .query_row(
                "SELECT book_id, cfi_position, percentage, hlc_ts FROM reading_progress WHERE book_id = ?1",
                params![book_id],
                |row| {
                    Ok(ReadingProgress {
                        book_id: row.get(0)?,
                        cfi_position: row.get(1)?,
                        percentage: row.get(2)?,
                        hlc_timestamp: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .optional()
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(result)
    }

    // ─── Bookmarks CRUD ──────────────────────────────────────────────────────

    /// Add a bookmark.
    pub fn add_bookmark(&self, bookmark: &Bookmark) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO bookmarks (id, book_id, cfi_position, title, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    bookmark.id,
                    bookmark.book_id,
                    bookmark.cfi_position,
                    bookmark.title,
                    bookmark.created_at as i64,
                ],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    /// Delete a bookmark by ID.
    pub fn delete_bookmark(&self, id: &str) -> Result<bool, StorageError> {
        let rows = self
            .conn
            .execute("DELETE FROM bookmarks WHERE id = ?1", params![id])
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(rows > 0)
    }

    /// List bookmarks for a book, ordered by CFI position.
    pub fn list_bookmarks(&self, book_id: &str) -> Result<Vec<Bookmark>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, book_id, cfi_position, title, created_at
                 FROM bookmarks WHERE book_id = ?1 ORDER BY cfi_position",
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let bookmarks = stmt
            .query_map(params![book_id], |row| {
                Ok(Bookmark {
                    id: row.get(0)?,
                    book_id: row.get(1)?,
                    cfi_position: row.get(2)?,
                    title: row.get(3)?,
                    created_at: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(bookmarks)
    }

    // ─── Annotations CRUD ────────────────────────────────────────────────────

    /// Add an annotation.
    pub fn add_annotation(&self, annotation: &Annotation) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO annotations (id, book_id, cfi_start, cfi_end, color_rgba, note, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    annotation.id,
                    annotation.book_id,
                    annotation.cfi_start,
                    annotation.cfi_end,
                    annotation.color_rgba,
                    annotation.note,
                    annotation.created_at as i64,
                ],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    /// Delete an annotation by ID.
    pub fn delete_annotation(&self, id: &str) -> Result<bool, StorageError> {
        let rows = self
            .conn
            .execute("DELETE FROM annotations WHERE id = ?1", params![id])
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(rows > 0)
    }

    /// List annotations for a book, ordered by CFI start position.
    pub fn list_annotations(&self, book_id: &str) -> Result<Vec<Annotation>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, book_id, cfi_start, cfi_end, color_rgba, note, created_at
                 FROM annotations WHERE book_id = ?1 ORDER BY cfi_start",
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let annotations = stmt
            .query_map(params![book_id], |row| {
                Ok(Annotation {
                    id: row.get(0)?,
                    book_id: row.get(1)?,
                    cfi_start: row.get(2)?,
                    cfi_end: row.get(3)?,
                    color_rgba: row.get(4)?,
                    note: row.get(5)?,
                    created_at: row.get::<_, i64>(6)? as u64,
                })
            })
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(annotations)
    }

    // ─── Preferences CRUD ────────────────────────────────────────────────────

    /// Set a user preference.
    pub fn set_preference(&self, pref: &UserPreference) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO preferences (key, value, hlc_ts) VALUES (?1, ?2, ?3)",
                params![pref.key, pref.value, pref.hlc_timestamp as i64],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    /// Get a user preference by key.
    pub fn get_preference(&self, key: &str) -> Result<Option<UserPreference>, StorageError> {
        let result = self
            .conn
            .query_row(
                "SELECT key, value, hlc_ts FROM preferences WHERE key = ?1",
                params![key],
                |row| {
                    Ok(UserPreference {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        hlc_timestamp: row.get::<_, i64>(2)? as u64,
                    })
                },
            )
            .optional()
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(result)
    }

    // ─── Op Log ──────────────────────────────────────────────────────────────

    /// Write an operation to the op_log.
    pub fn write_op_log(
        &self,
        op_type: &str,
        op_data: &str,
        hlc_ts: u64,
        device_id: &str,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT INTO op_log (op_type, op_data, hlc_ts, device_id, synced)
                 VALUES (?1, ?2, ?3, ?4, 0)",
                params![op_type, op_data, hlc_ts as i64, device_id],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    /// Query unsynced operations, ordered by HLC timestamp ascending.
    /// Returns tuples of (id, op_type, op_data, hlc_ts, device_id).
    #[allow(clippy::type_complexity)]
    pub fn get_unsynced_ops(
        &self,
    ) -> Result<Vec<(i64, String, String, u64, String)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, op_type, op_data, hlc_ts, device_id
                 FROM op_log WHERE synced = 0 ORDER BY hlc_ts ASC",
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let ops = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(ops)
    }

    /// Mark operations as synced.
    pub fn mark_ops_synced(&self, op_ids: &[i64]) -> Result<(), StorageError> {
        if op_ids.is_empty() {
            return Ok(());
        }
        let placeholders: Vec<String> = op_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "UPDATE op_log SET synced = 1 WHERE id IN ({})",
            placeholders.join(",")
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let params: Vec<Box<dyn rusqlite::types::ToSql>> = op_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect();

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        stmt.execute(param_refs.as_slice())
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        // Insert a test book
        db.upsert_book(
            "book-1",
            "Test Book",
            "Author",
            "epub",
            Some("abc123"),
            Some(1024),
            None,
        )
        .unwrap();
        db
    }

    #[test]
    fn test_open_in_memory() {
        let db = Database::open_in_memory();
        assert!(db.is_ok());
    }

    #[test]
    fn test_book_crud() {
        let db = Database::open_in_memory().unwrap();

        // Insert
        db.upsert_book(
            "b1",
            "My Book",
            "Author A",
            "epub",
            Some("hash1"),
            Some(2048),
            None,
        )
        .unwrap();

        // Read
        let book = db.get_book("b1").unwrap();
        assert!(book.is_some());
        let (id, title, author, format) = book.unwrap();
        assert_eq!(id, "b1");
        assert_eq!(title, "My Book");
        assert_eq!(author, "Author A");
        assert_eq!(format, "epub");

        // Not found
        let missing = db.get_book("nonexistent").unwrap();
        assert!(missing.is_none());

        // Delete
        let deleted = db.delete_book("b1").unwrap();
        assert!(deleted);
        let after_delete = db.get_book("b1").unwrap();
        assert!(after_delete.is_none());
    }

    #[test]
    fn test_reading_progress_crud() {
        let db = setup_db();

        let progress = ReadingProgress {
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            percentage: 42.5,
            hlc_timestamp: 1000,
        };

        // Upsert
        db.upsert_progress(&progress).unwrap();

        // Read
        let loaded = db.get_progress("book-1").unwrap().unwrap();
        assert_eq!(loaded.cfi_position, "/6/4!/4/2:0");
        assert!((loaded.percentage - 42.5).abs() < f64::EPSILON);
        assert_eq!(loaded.hlc_timestamp, 1000);

        // Update
        let updated = ReadingProgress {
            percentage: 75.0,
            hlc_timestamp: 2000,
            ..progress
        };
        db.upsert_progress(&updated).unwrap();
        let reloaded = db.get_progress("book-1").unwrap().unwrap();
        assert!((reloaded.percentage - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_bookmark_crud() {
        let db = setup_db();

        let bm1 = Bookmark {
            id: "bm-1".to_string(),
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            title: Some("Chapter 1".to_string()),
            created_at: 1000,
        };
        let bm2 = Bookmark {
            id: "bm-2".to_string(),
            book_id: "book-1".to_string(),
            cfi_position: "/6/8!/4/2:0".to_string(),
            title: None,
            created_at: 2000,
        };

        db.add_bookmark(&bm1).unwrap();
        db.add_bookmark(&bm2).unwrap();

        let bookmarks = db.list_bookmarks("book-1").unwrap();
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].id, "bm-1"); // Sorted by CFI

        // Delete
        let deleted = db.delete_bookmark("bm-1").unwrap();
        assert!(deleted);
        let remaining = db.list_bookmarks("book-1").unwrap();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_annotation_crud() {
        let db = setup_db();

        let ann = Annotation {
            id: "ann-1".to_string(),
            book_id: "book-1".to_string(),
            cfi_start: "/6/4!/4/2:0".to_string(),
            cfi_end: "/6/4!/4/2:10".to_string(),
            color_rgba: "#FFFF00FF".to_string(),
            note: Some("Important note".to_string()),
            created_at: 1000,
        };

        db.add_annotation(&ann).unwrap();

        let annotations = db.list_annotations("book-1").unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].note, Some("Important note".to_string()));

        let deleted = db.delete_annotation("ann-1").unwrap();
        assert!(deleted);
        let empty = db.list_annotations("book-1").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_preferences_crud() {
        let db = Database::open_in_memory().unwrap();

        let pref = UserPreference {
            key: "font_size".to_string(),
            value: "16".to_string(),
            hlc_timestamp: 1000,
        };

        db.set_preference(&pref).unwrap();

        let loaded = db.get_preference("font_size").unwrap().unwrap();
        assert_eq!(loaded.value, "16");
        assert_eq!(loaded.hlc_timestamp, 1000);

        // Update
        let updated = UserPreference {
            value: "18".to_string(),
            hlc_timestamp: 2000,
            ..pref
        };
        db.set_preference(&updated).unwrap();
        let reloaded = db.get_preference("font_size").unwrap().unwrap();
        assert_eq!(reloaded.value, "18");

        // Not found
        let missing = db.get_preference("nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_op_log_write_and_query() {
        let db = Database::open_in_memory().unwrap();

        db.write_op_log("UpdateProgress", r#"{"book_id":"b1"}"#, 1000, "device-1")
            .unwrap();
        db.write_op_log("AddBookmark", r#"{"id":"bm1"}"#, 2000, "device-1")
            .unwrap();
        db.write_op_log("UpdateProgress", r#"{"book_id":"b2"}"#, 3000, "device-2")
            .unwrap();

        let unsynced = db.get_unsynced_ops().unwrap();
        assert_eq!(unsynced.len(), 3);
        // Should be ordered by hlc_ts ascending
        assert_eq!(unsynced[0].3, 1000);
        assert_eq!(unsynced[1].3, 2000);
        assert_eq!(unsynced[2].3, 3000);

        // Mark first two as synced
        let ids: Vec<i64> = unsynced[..2].iter().map(|op| op.0).collect();
        db.mark_ops_synced(&ids).unwrap();

        let remaining = db.get_unsynced_ops().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].3, 3000);
    }

    #[test]
    fn test_op_log_empty_mark() {
        let db = Database::open_in_memory().unwrap();
        // Should not fail with empty list
        db.mark_ops_synced(&[]).unwrap();
    }

    // ─── Additional Boundary Tests ───────────────────────────────────────────

    #[test]
    fn test_upsert_book_overwrites_existing() {
        let db = Database::open_in_memory().unwrap();

        // Insert initial
        db.upsert_book("b1", "Original Title", "Author A", "epub", Some("hash1"), Some(1024), None)
            .unwrap();

        // Overwrite with same ID
        db.upsert_book("b1", "Updated Title", "Author B", "epub", Some("hash2"), Some(2048), Some("/covers/b1.jpg"))
            .unwrap();

        let book = db.get_book("b1").unwrap().unwrap();
        assert_eq!(book.1, "Updated Title");
        assert_eq!(book.2, "Author B");
    }

    #[test]
    fn test_delete_nonexistent_bookmark_returns_false() {
        let db = setup_db();
        let deleted = db.delete_bookmark("nonexistent-bm").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_delete_nonexistent_annotation_returns_false() {
        let db = setup_db();
        let deleted = db.delete_annotation("nonexistent-ann").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_delete_nonexistent_book_returns_false() {
        let db = Database::open_in_memory().unwrap();
        let deleted = db.delete_book("nonexistent-book").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_mark_ops_synced_multiple() {
        let db = Database::open_in_memory().unwrap();

        // Write 5 operations
        for i in 0..5 {
            db.write_op_log(
                "UpdateProgress",
                &format!(r#"{{"book_id":"b{}"}}"#, i),
                (i + 1) * 1000,
                "device-1",
            )
            .unwrap();
        }

        let unsynced = db.get_unsynced_ops().unwrap();
        assert_eq!(unsynced.len(), 5);

        // Mark first 3 as synced
        let ids: Vec<i64> = unsynced[..3].iter().map(|op| op.0).collect();
        db.mark_ops_synced(&ids).unwrap();

        let remaining = db.get_unsynced_ops().unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].3, 4000);
        assert_eq!(remaining[1].3, 5000);

        // Mark remaining 2
        let ids: Vec<i64> = remaining.iter().map(|op| op.0).collect();
        db.mark_ops_synced(&ids).unwrap();

        let final_remaining = db.get_unsynced_ops().unwrap();
        assert!(final_remaining.is_empty());
    }

    #[test]
    fn test_get_progress_nonexistent_book() {
        let db = setup_db();
        let progress = db.get_progress("nonexistent-book").unwrap();
        assert!(progress.is_none());
    }

    #[test]
    fn test_get_preference_nonexistent_key() {
        let db = Database::open_in_memory().unwrap();
        let pref = db.get_preference("nonexistent-key").unwrap();
        assert!(pref.is_none());
    }

    #[test]
    fn test_list_bookmarks_empty() {
        let db = setup_db();
        let bookmarks = db.list_bookmarks("book-1").unwrap();
        assert!(bookmarks.is_empty());
    }

    #[test]
    fn test_list_annotations_empty() {
        let db = setup_db();
        let annotations = db.list_annotations("book-1").unwrap();
        assert!(annotations.is_empty());
    }

    #[test]
    fn test_upsert_progress_overwrites() {
        let db = setup_db();

        let p1 = ReadingProgress {
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            percentage: 25.0,
            hlc_timestamp: 1000,
        };
        db.upsert_progress(&p1).unwrap();

        let p2 = ReadingProgress {
            book_id: "book-1".to_string(),
            cfi_position: "/6/8!/4/2:0".to_string(),
            percentage: 75.0,
            hlc_timestamp: 2000,
        };
        db.upsert_progress(&p2).unwrap();

        let loaded = db.get_progress("book-1").unwrap().unwrap();
        assert_eq!(loaded.cfi_position, "/6/8!/4/2:0");
        assert!((loaded.percentage - 75.0).abs() < f64::EPSILON);
        assert_eq!(loaded.hlc_timestamp, 2000);
    }
}
