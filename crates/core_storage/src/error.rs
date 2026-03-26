//! Storage error types.

use thiserror::Error;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database locked: {0}")]
    DatabaseLocked(String),

    #[error("Migration failed: {0}")]
    MigrationFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Disk full or I/O error: {0}")]
    DiskFull(String),

    #[error("Record not found: {0}")]
    NotFound(String),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}
