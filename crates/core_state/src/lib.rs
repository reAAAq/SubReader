//! Core state management module.
//!
//! Provides the `StateManager` as the unified entry point for managing
//! reading progress, bookmarks, annotations, and user preferences.
//! Every state mutation is persisted to `core_storage` and generates
//! a CRDT `Operation` written to the `op_log`.

use core_storage::Database;
use core_sync::Operation;
use shared_types::{Annotation, Bookmark, ReadingProgress, UserPreference};
use thiserror::Error;

/// Errors from state management operations.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("Storage error: {0}")]
    Storage(#[from] core_storage::StorageError),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Unified state manager that wraps `core_storage::Database`.
///
/// All state mutations are persisted and generate CRDT operations
/// for future synchronization.
pub struct StateManager {
    db: Database,
    device_id: String,
}

impl StateManager {
    /// Create a new StateManager with a database path.
    pub fn new(db_path: &str, device_id: impl Into<String>) -> Result<Self, StateError> {
        let db = Database::open(db_path)?;
        Ok(Self {
            db,
            device_id: device_id.into(),
        })
    }

    /// Create a new StateManager with an in-memory database (for testing).
    pub fn new_in_memory(device_id: impl Into<String>) -> Result<Self, StateError> {
        let db = Database::open_in_memory()?;
        Ok(Self {
            db,
            device_id: device_id.into(),
        })
    }

    /// Get a reference to the underlying database.
    pub fn database(&self) -> &Database {
        &self.db
    }

    // ─── Book Management ─────────────────────────────────────────────────────

    /// Register a book in the database.
    pub fn register_book(
        &self,
        id: &str,
        title: &str,
        author: &str,
        format: &str,
        file_hash: Option<&str>,
        file_size: Option<u64>,
    ) -> Result<(), StateError> {
        self.db
            .upsert_book(id, title, author, format, file_hash, file_size, None)?;
        Ok(())
    }

    // ─── Reading Progress ────────────────────────────────────────────────────

    /// Update reading progress for a book.
    pub fn update_progress(
        &self,
        book_id: &str,
        cfi_position: &str,
        percentage: f64,
        hlc_timestamp: u64,
    ) -> Result<(), StateError> {
        let progress = ReadingProgress {
            book_id: book_id.to_string(),
            cfi_position: cfi_position.to_string(),
            percentage,
            hlc_timestamp,
        };

        self.db.upsert_progress(&progress)?;

        // Generate CRDT operation
        let op = Operation::UpdateProgress {
            book_id: book_id.to_string(),
            cfi_position: cfi_position.to_string(),
            percentage,
        };
        self.write_operation(&op, hlc_timestamp)?;

        Ok(())
    }

    /// Get current reading progress for a book.
    pub fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StateError> {
        Ok(self.db.get_progress(book_id)?)
    }

    // ─── Bookmarks ───────────────────────────────────────────────────────────

    /// Add a bookmark.
    pub fn add_bookmark(&self, bookmark: &Bookmark) -> Result<(), StateError> {
        self.db.add_bookmark(bookmark)?;

        let op = Operation::AddBookmark {
            bookmark_id: bookmark.id.clone(),
            book_id: bookmark.book_id.clone(),
            cfi_position: bookmark.cfi_position.clone(),
            title: bookmark.title.clone(),
        };
        self.write_operation(&op, bookmark.created_at)?;

        Ok(())
    }

    /// Delete a bookmark by ID.
    pub fn delete_bookmark(
        &self,
        bookmark_id: &str,
        hlc_timestamp: u64,
    ) -> Result<bool, StateError> {
        let deleted = self.db.delete_bookmark(bookmark_id)?;

        if deleted {
            let op = Operation::DeleteBookmark {
                bookmark_id: bookmark_id.to_string(),
            };
            self.write_operation(&op, hlc_timestamp)?;
        }

        Ok(deleted)
    }

    /// List bookmarks for a book, ordered by CFI position.
    pub fn list_bookmarks(&self, book_id: &str) -> Result<Vec<Bookmark>, StateError> {
        Ok(self.db.list_bookmarks(book_id)?)
    }

    // ─── Annotations ─────────────────────────────────────────────────────────

    /// Add an annotation.
    pub fn add_annotation(&self, annotation: &Annotation) -> Result<(), StateError> {
        self.db.add_annotation(annotation)?;

        let op = Operation::AddAnnotation {
            annotation_id: annotation.id.clone(),
            book_id: annotation.book_id.clone(),
            cfi_start: annotation.cfi_start.clone(),
            cfi_end: annotation.cfi_end.clone(),
            color_rgba: annotation.color_rgba.clone(),
            note: annotation.note.clone(),
        };
        self.write_operation(&op, annotation.created_at)?;

        Ok(())
    }

    /// Delete an annotation by ID.
    pub fn delete_annotation(
        &self,
        annotation_id: &str,
        hlc_timestamp: u64,
    ) -> Result<bool, StateError> {
        let deleted = self.db.delete_annotation(annotation_id)?;

        if deleted {
            let op = Operation::DeleteAnnotation {
                annotation_id: annotation_id.to_string(),
            };
            self.write_operation(&op, hlc_timestamp)?;
        }

        Ok(deleted)
    }

    /// List annotations for a book, ordered by CFI start position.
    pub fn list_annotations(&self, book_id: &str) -> Result<Vec<Annotation>, StateError> {
        Ok(self.db.list_annotations(book_id)?)
    }

    // ─── User Preferences ────────────────────────────────────────────────────

    /// Set a user preference.
    pub fn set_preference(
        &self,
        key: &str,
        value: &str,
        hlc_timestamp: u64,
    ) -> Result<(), StateError> {
        let pref = UserPreference {
            key: key.to_string(),
            value: value.to_string(),
            hlc_timestamp,
        };

        self.db.set_preference(&pref)?;

        let op = Operation::UpdatePreference {
            key: key.to_string(),
            value: value.to_string(),
        };
        self.write_operation(&op, hlc_timestamp)?;

        Ok(())
    }

    /// Get a user preference by key.
    pub fn get_preference(&self, key: &str) -> Result<Option<UserPreference>, StateError> {
        Ok(self.db.get_preference(key)?)
    }

    // ─── Internal Helpers ────────────────────────────────────────────────────

    /// Serialize an operation and write it to the op_log.
    fn write_operation(&self, op: &Operation, hlc_timestamp: u64) -> Result<(), StateError> {
        let op_type = match op {
            Operation::UpdateProgress { .. } => "UpdateProgress",
            Operation::AddBookmark { .. } => "AddBookmark",
            Operation::DeleteBookmark { .. } => "DeleteBookmark",
            Operation::AddAnnotation { .. } => "AddAnnotation",
            Operation::DeleteAnnotation { .. } => "DeleteAnnotation",
            Operation::UpdatePreference { .. } => "UpdatePreference",
        };

        let op_data =
            serde_json::to_string(op).map_err(|e| StateError::Serialization(e.to_string()))?;

        self.db
            .write_op_log(op_type, &op_data, hlc_timestamp, &self.device_id)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> StateManager {
        let sm = StateManager::new_in_memory("test-device-1").unwrap();
        sm.register_book(
            "book-1",
            "Test Book",
            "Author",
            "epub",
            Some("hash"),
            Some(1024),
        )
        .unwrap();
        sm
    }

    #[test]
    fn test_create_state_manager() {
        let sm = StateManager::new_in_memory("device-1");
        assert!(sm.is_ok());
    }

    #[test]
    fn test_progress_update_and_query() {
        let sm = setup();

        sm.update_progress("book-1", "/6/4!/4/2:0", 42.5, 1000)
            .unwrap();

        let progress = sm.get_progress("book-1").unwrap().unwrap();
        assert_eq!(progress.cfi_position, "/6/4!/4/2:0");
        assert!((progress.percentage - 42.5).abs() < f64::EPSILON);

        // Verify op_log was written
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].1, "UpdateProgress");
    }

    #[test]
    fn test_bookmark_crud_with_op_log() {
        let sm = setup();

        let bm = Bookmark {
            id: "bm-1".to_string(),
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            title: Some("My Bookmark".to_string()),
            created_at: 1000,
        };

        sm.add_bookmark(&bm).unwrap();

        let bookmarks = sm.list_bookmarks("book-1").unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].title, Some("My Bookmark".to_string()));

        // Delete
        let deleted = sm.delete_bookmark("bm-1", 2000).unwrap();
        assert!(deleted);

        let empty = sm.list_bookmarks("book-1").unwrap();
        assert!(empty.is_empty());

        // Verify op_log: should have AddBookmark + DeleteBookmark
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].1, "AddBookmark");
        assert_eq!(ops[1].1, "DeleteBookmark");
    }

    #[test]
    fn test_annotation_crud_with_op_log() {
        let sm = setup();

        let ann = Annotation {
            id: "ann-1".to_string(),
            book_id: "book-1".to_string(),
            cfi_start: "/6/4!/4/2:0".to_string(),
            cfi_end: "/6/4!/4/2:10".to_string(),
            color_rgba: "#FF0000FF".to_string(),
            note: Some("Important".to_string()),
            created_at: 1000,
        };

        sm.add_annotation(&ann).unwrap();

        let annotations = sm.list_annotations("book-1").unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].color_rgba, "#FF0000FF");

        let deleted = sm.delete_annotation("ann-1", 2000).unwrap();
        assert!(deleted);

        let empty = sm.list_annotations("book-1").unwrap();
        assert!(empty.is_empty());

        // Verify op_log
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].1, "AddAnnotation");
        assert_eq!(ops[1].1, "DeleteAnnotation");
    }

    #[test]
    fn test_preference_set_and_get() {
        let sm = StateManager::new_in_memory("device-1").unwrap();

        sm.set_preference("font_size", "16", 1000).unwrap();
        sm.set_preference("theme", "dark", 1001).unwrap();

        let font = sm.get_preference("font_size").unwrap().unwrap();
        assert_eq!(font.value, "16");

        let theme = sm.get_preference("theme").unwrap().unwrap();
        assert_eq!(theme.value, "dark");

        // Update
        sm.set_preference("font_size", "18", 2000).unwrap();
        let updated = sm.get_preference("font_size").unwrap().unwrap();
        assert_eq!(updated.value, "18");

        // Verify op_log: 3 UpdatePreference operations
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert_eq!(ops.len(), 3);
        assert!(ops.iter().all(|op| op.1 == "UpdatePreference"));
    }

    #[test]
    fn test_op_log_serialization() {
        let sm = setup();

        sm.update_progress("book-1", "/6/4!/4/2:0", 50.0, 1000)
            .unwrap();

        let ops = sm.database().get_unsynced_ops().unwrap();
        assert_eq!(ops.len(), 1);

        // Verify the op_data is valid JSON
        let op_data = &ops[0].2;
        let parsed: Operation = serde_json::from_str(op_data).unwrap();
        match parsed {
            Operation::UpdateProgress {
                book_id,
                percentage,
                ..
            } => {
                assert_eq!(book_id, "book-1");
                assert!((percentage - 50.0).abs() < f64::EPSILON);
            }
            _ => panic!("Expected UpdateProgress operation"),
        }
    }

    // ─── Additional Boundary Tests ───────────────────────────────────────────

    #[test]
    fn test_register_book_standalone() {
        let sm = StateManager::new_in_memory("device-1").unwrap();
        let result = sm.register_book("book-99", "Standalone Book", "Author X", "epub", None, None);
        assert!(result.is_ok());

        // Verify the book was stored
        let book = sm.database().get_book("book-99").unwrap();
        assert!(book.is_some());
        let (id, title, author, format) = book.unwrap();
        assert_eq!(id, "book-99");
        assert_eq!(title, "Standalone Book");
        assert_eq!(author, "Author X");
        assert_eq!(format, "epub");
    }

    #[test]
    fn test_delete_nonexistent_bookmark_returns_false() {
        let sm = setup();
        let deleted = sm.delete_bookmark("nonexistent-bm", 1000).unwrap();
        assert!(!deleted);

        // Verify no op_log entry was created for failed delete
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_delete_nonexistent_annotation_returns_false() {
        let sm = setup();
        let deleted = sm.delete_annotation("nonexistent-ann", 1000).unwrap();
        assert!(!deleted);

        // Verify no op_log entry was created for failed delete
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_get_progress_nonexistent_book() {
        let sm = setup();
        let progress = sm.get_progress("nonexistent-book").unwrap();
        assert!(progress.is_none());
    }

    #[test]
    fn test_get_preference_nonexistent_key() {
        let sm = StateManager::new_in_memory("device-1").unwrap();
        let pref = sm.get_preference("nonexistent-key").unwrap();
        assert!(pref.is_none());
    }

    #[test]
    fn test_list_bookmarks_empty_book() {
        let sm = setup();
        let bookmarks = sm.list_bookmarks("book-1").unwrap();
        assert!(bookmarks.is_empty());
    }

    #[test]
    fn test_list_annotations_empty_book() {
        let sm = setup();
        let annotations = sm.list_annotations("book-1").unwrap();
        assert!(annotations.is_empty());
    }

    #[test]
    fn test_update_progress_overwrites_previous() {
        let sm = setup();

        sm.update_progress("book-1", "/6/4!/4/2:0", 25.0, 1000)
            .unwrap();
        sm.update_progress("book-1", "/6/8!/4/2:0", 75.0, 2000)
            .unwrap();

        let progress = sm.get_progress("book-1").unwrap().unwrap();
        assert_eq!(progress.cfi_position, "/6/8!/4/2:0");
        assert!((progress.percentage - 75.0).abs() < f64::EPSILON);
        assert_eq!(progress.hlc_timestamp, 2000);
    }

    #[test]
    fn test_delete_nonexistent_bookmark() {
        let sm = setup();
        let deleted = sm.delete_bookmark("nonexistent", 1000).unwrap();
        assert!(!deleted);

        // No op_log entry should be created for failed delete
        let ops = sm.database().get_unsynced_ops().unwrap();
        assert!(ops.is_empty());
    }
}
