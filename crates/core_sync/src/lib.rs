//! Core sync module.
//!
//! Defines the foundational types for CRDT-based synchronization,
//! HLC clock logic, and the sync engine.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod engine;
pub mod file_sync;
pub mod scheduler;

/// Sync engine error types.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Not authenticated")]
    NotAuthenticated,

    #[error("Conflict resolution error: {0}")]
    ConflictResolution(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Hybrid Logical Clock timestamp for causal ordering of operations.
///
/// Combines a physical wall-clock component with a logical counter
/// to ensure unique, monotonically increasing timestamps across devices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HlcTimestamp {
    /// Physical time component (milliseconds since Unix epoch).
    pub wall_time_ms: u64,
    /// Logical counter for disambiguation within the same millisecond.
    /// Limited to u16 (0–65535) to fit the 48+16 bit encoding in `to_u64()`.
    pub counter: u16,
    /// Device/node identifier hash (truncated to u32 for compactness).
    pub node_id: u32,
}

impl HlcTimestamp {
    /// Create a new HLC timestamp with the given wall time.
    pub fn new(wall_time_ms: u64, node_id: u32) -> Self {
        Self {
            wall_time_ms,
            counter: 0,
            node_id,
        }
    }

    /// Create a zero timestamp (used as a default/sentinel).
    pub fn zero() -> Self {
        Self {
            wall_time_ms: 0,
            counter: 0,
            node_id: 0,
        }
    }

    /// Convert to a single u64 for storage (wall_time in upper 48 bits, counter in lower 16 bits).
    ///
    /// The wall_time_ms field must fit in 48 bits (valid until ~year 10889).
    /// The counter field is u16, so no truncation occurs.
    pub fn to_u64(&self) -> u64 {
        (self.wall_time_ms << 16) | (self.counter as u64)
    }

    /// Reconstruct from a u64 value.
    pub fn from_u64(val: u64, node_id: u32) -> Self {
        Self {
            wall_time_ms: val >> 16,
            counter: (val & 0xFFFF) as u16,
            node_id,
        }
    }

    /// Tick the HLC clock: advance based on current wall time.
    /// Returns a new timestamp that is guaranteed to be greater than `self`.
    ///
    /// # Panics
    /// Panics if the counter overflows u16 (65535 ops in the same millisecond).
    pub fn tick(&self, now_ms: u64) -> Self {
        if now_ms > self.wall_time_ms {
            Self {
                wall_time_ms: now_ms,
                counter: 0,
                node_id: self.node_id,
            }
        } else {
            Self {
                wall_time_ms: self.wall_time_ms,
                counter: self.counter.checked_add(1).expect("HLC counter overflow: too many operations in the same millisecond"),
                node_id: self.node_id,
            }
        }
    }

    /// Merge with a remote HLC timestamp.
    /// Returns a new timestamp that is greater than both `self` and `remote`.
    pub fn merge(&self, remote: &HlcTimestamp, now_ms: u64) -> Self {
        let max_wall = now_ms.max(self.wall_time_ms).max(remote.wall_time_ms);

        let counter = if max_wall == self.wall_time_ms
            && max_wall == remote.wall_time_ms
        {
            self.counter.max(remote.counter).checked_add(1)
                .expect("HLC counter overflow during merge")
        } else if max_wall == self.wall_time_ms {
            self.counter.checked_add(1)
                .expect("HLC counter overflow during merge")
        } else if max_wall == remote.wall_time_ms {
            remote.counter.checked_add(1)
                .expect("HLC counter overflow during merge")
        } else {
            0
        };

        Self {
            wall_time_ms: max_wall,
            counter,
            node_id: self.node_id,
        }
    }
}

/// Types of operations that can be synchronized across devices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Operation {
    /// Update reading progress for a book.
    UpdateProgress {
        book_id: String,
        cfi_position: String,
        percentage: f64,
    },
    /// Add a bookmark.
    AddBookmark {
        bookmark_id: String,
        book_id: String,
        cfi_position: String,
        title: Option<String>,
    },
    /// Delete a bookmark.
    DeleteBookmark { bookmark_id: String },
    /// Add an annotation.
    AddAnnotation {
        annotation_id: String,
        book_id: String,
        cfi_start: String,
        cfi_end: String,
        color_rgba: String,
        note: Option<String>,
    },
    /// Delete an annotation.
    DeleteAnnotation { annotation_id: String },
    /// Update a user preference.
    UpdatePreference { key: String, value: String },
}

/// A timestamped operation ready for synchronization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncOperation {
    /// Unique operation identifier.
    pub op_id: String,
    /// The operation payload.
    pub operation: Operation,
    /// HLC timestamp when the operation was created.
    pub hlc_timestamp: HlcTimestamp,
    /// Device ID that created this operation.
    pub device_id: String,
}

impl SyncOperation {
    /// Create a new sync operation with a generated ID.
    pub fn new(operation: Operation, hlc_timestamp: HlcTimestamp, device_id: String) -> Self {
        Self {
            op_id: Uuid::new_v4().to_string(),
            operation,
            hlc_timestamp,
            device_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hlc_timestamp_ordering() {
        let ts1 = HlcTimestamp::new(1000, 1);
        let ts2 = HlcTimestamp::new(2000, 1);
        assert!(ts1 < ts2);
    }

    #[test]
    fn test_hlc_timestamp_u64_roundtrip() {
        let ts = HlcTimestamp {
            wall_time_ms: 1234567890,
            counter: 42,
            node_id: 7,
        };
        let encoded = ts.to_u64();
        let decoded = HlcTimestamp::from_u64(encoded, 7);
        assert_eq!(decoded.wall_time_ms, ts.wall_time_ms);
        assert_eq!(decoded.counter, ts.counter);
    }

    #[test]
    fn test_operation_serialization() {
        let op = Operation::UpdateProgress {
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            percentage: 42.5,
        };
        let json = serde_json::to_string(&op).unwrap();
        let deserialized: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(op, deserialized);
    }

    #[test]
    fn test_hlc_timestamp_zero() {
        let ts = HlcTimestamp::zero();
        assert_eq!(ts.wall_time_ms, 0);
        assert_eq!(ts.counter, 0);
        assert_eq!(ts.node_id, 0);

        // Zero should be less than any real timestamp
        let real_ts = HlcTimestamp::new(1, 1);
        assert!(ts < real_ts);
    }

    #[test]
    fn test_sync_operation_new() {
        let op = Operation::AddBookmark {
            bookmark_id: "bm-1".to_string(),
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            title: Some("Test".to_string()),
        };
        let hlc = HlcTimestamp::new(1000, 42);
        let sync_op = SyncOperation::new(op.clone(), hlc, "device-1".to_string());

        assert!(!sync_op.op_id.is_empty());
        assert_eq!(sync_op.operation, op);
        assert_eq!(sync_op.hlc_timestamp, hlc);
        assert_eq!(sync_op.device_id, "device-1");

        // op_id should be a valid UUID
        let parsed = uuid::Uuid::parse_str(&sync_op.op_id);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_sync_operation_serialization() {
        let op = Operation::DeleteBookmark {
            bookmark_id: "bm-1".to_string(),
        };
        let hlc = HlcTimestamp::new(5000, 7);
        let sync_op = SyncOperation::new(op, hlc, "device-2".to_string());

        let json = serde_json::to_string(&sync_op).unwrap();
        let deserialized: SyncOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(sync_op, deserialized);
    }

    #[test]
    fn test_hlc_timestamp_ordering_same_wall_time() {
        let ts1 = HlcTimestamp {
            wall_time_ms: 1000,
            counter: 1,
            node_id: 1,
        };
        let ts2 = HlcTimestamp {
            wall_time_ms: 1000,
            counter: 2,
            node_id: 1,
        };
        assert!(ts1 < ts2);
    }

    #[test]
    fn test_hlc_timestamp_ordering_same_wall_time_same_counter() {
        let ts1 = HlcTimestamp {
            wall_time_ms: 1000,
            counter: 1,
            node_id: 1,
        };
        let ts2 = HlcTimestamp {
            wall_time_ms: 1000,
            counter: 1,
            node_id: 2,
        };
        assert!(ts1 < ts2);
    }

    #[test]
    fn test_all_operation_variants_serialization() {
        let ops = vec![
            Operation::UpdateProgress {
                book_id: "b1".to_string(),
                cfi_position: "/6/4".to_string(),
                percentage: 50.0,
            },
            Operation::AddBookmark {
                bookmark_id: "bm1".to_string(),
                book_id: "b1".to_string(),
                cfi_position: "/6/4".to_string(),
                title: None,
            },
            Operation::DeleteBookmark {
                bookmark_id: "bm1".to_string(),
            },
            Operation::AddAnnotation {
                annotation_id: "ann1".to_string(),
                book_id: "b1".to_string(),
                cfi_start: "/6/4:0".to_string(),
                cfi_end: "/6/4:10".to_string(),
                color_rgba: "#FF0000FF".to_string(),
                note: Some("note".to_string()),
            },
            Operation::DeleteAnnotation {
                annotation_id: "ann1".to_string(),
            },
            Operation::UpdatePreference {
                key: "theme".to_string(),
                value: "dark".to_string(),
            },
        ];

        for op in ops {
            let json = serde_json::to_string(&op).unwrap();
            let deserialized: Operation = serde_json::from_str(&json).unwrap();
            assert_eq!(op, deserialized);
        }
    }
}
