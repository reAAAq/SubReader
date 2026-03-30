//! Sync engine — core push/pull logic with conflict resolution.
//!
//! The engine coordinates between local storage (core_storage) and
//! remote transport (core_network) to synchronize operations.

use crate::{HlcTimestamp, SyncError};

/// Trait abstracting local storage operations needed by the sync engine.
///
/// This avoids a direct dependency on core_storage, allowing the engine
/// to be tested with mock implementations.
pub trait SyncStorage: Send + Sync {
    /// Get unsynced operations: (id, op_id, op_type, op_data, hlc_ts, device_id).
    fn get_unsynced_ops(&self) -> Result<Vec<UnsyncedOp>, SyncError>;

    /// Mark operations as synced by their local IDs.
    fn mark_ops_synced(&self, local_ids: &[i64]) -> Result<(), SyncError>;

    /// Get a sync metadata value by key.
    fn get_sync_meta(&self, key: &str) -> Result<Option<String>, SyncError>;

    /// Set a sync metadata value.
    fn set_sync_meta(&self, key: &str, value: &str) -> Result<(), SyncError>;

    /// Apply a remote operation to local storage.
    /// Returns true if the operation was applied (not a duplicate/conflict loser).
    fn apply_remote_op(&self, op: &RemoteOp) -> Result<bool, SyncError>;
}

/// An unsynced operation from local storage.
#[derive(Debug, Clone)]
pub struct UnsyncedOp {
    pub local_id: i64,
    pub op_id: String,
    pub op_type: String,
    pub op_data: String,
    pub hlc_ts: u64,
    pub device_id: String,
}

/// A remote operation received from the server.
#[derive(Debug, Clone)]
pub struct RemoteOp {
    pub server_seq: i64,
    pub op_id: String,
    pub op_type: String,
    pub op_data: String,
    pub hlc_ts: i64,
    pub device_id: String,
}

/// Trait abstracting the network transport for sync operations.
pub trait SyncTransportAdapter: Send + Sync {
    /// Push operations to the server.
    /// Returns the number of accepted operations.
    fn push_ops(
        &self,
        ops: &[PushOpPayload],
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<usize, SyncError>> + Send;

    /// Pull operations from the server since the given cursor.
    /// Returns (operations, next_cursor, has_more).
    fn pull_ops(
        &self,
        cursor: i64,
        limit: i64,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<PullResult, SyncError>> + Send;
}

/// Payload for pushing an operation.
#[derive(Debug, Clone)]
pub struct PushOpPayload {
    pub op_id: String,
    pub op_type: String,
    pub op_data: String,
    pub hlc_ts: i64,
    pub device_id: String,
}

/// Result of a pull operation.
#[derive(Debug, Clone)]
pub struct PullResult {
    pub operations: Vec<RemoteOp>,
    pub next_cursor: i64,
    pub has_more: bool,
}

/// The sync engine that coordinates push and pull operations.
pub struct SyncEngine<S: SyncStorage, T: SyncTransportAdapter> {
    storage: S,
    transport: T,
    local_device_id: String,
    hlc: std::sync::Mutex<HlcTimestamp>,
}

/// Sync metadata keys.
const META_PULL_CURSOR: &str = "sync_pull_cursor";

impl<S: SyncStorage, T: SyncTransportAdapter> SyncEngine<S, T> {
    /// Create a new sync engine.
    pub fn new(storage: S, transport: T, device_id: String, node_id: u32) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            storage,
            transport,
            local_device_id: device_id,
            hlc: std::sync::Mutex::new(HlcTimestamp::new(now_ms, node_id)),
        }
    }

    /// Get the current HLC timestamp and advance the clock.
    pub fn tick(&self) -> HlcTimestamp {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut hlc = self.hlc.lock().unwrap();
        *hlc = hlc.tick(now_ms);
        *hlc
    }

    /// Push all pending local operations to the server.
    ///
    /// Reads unsynced ops from storage, batches them (max 500 per request),
    /// pushes to server, and marks as synced.
    pub async fn push_pending(&self, auth_token: &str) -> Result<usize, SyncError> {
        let unsynced = self.storage.get_unsynced_ops()?;

        if unsynced.is_empty() {
            tracing::debug!("No pending operations to push");
            return Ok(0);
        }

        let mut total_pushed = 0usize;
        let batch_size = 500;

        for chunk in unsynced.chunks(batch_size) {
            let payloads: Vec<PushOpPayload> = chunk
                .iter()
                .map(|op| PushOpPayload {
                    op_id: op.op_id.clone(),
                    op_type: op.op_type.clone(),
                    op_data: op.op_data.clone(),
                    hlc_ts: op.hlc_ts as i64,
                    device_id: op.device_id.clone(),
                })
                .collect();

            let accepted = self.transport.push_ops(&payloads, auth_token).await?;

            // Mark all ops in this batch as synced (server handles dedup)
            let local_ids: Vec<i64> = chunk.iter().map(|op| op.local_id).collect();
            self.storage.mark_ops_synced(&local_ids)?;

            total_pushed += accepted;

            tracing::info!(
                batch_size = chunk.len(),
                accepted = accepted,
                "Pushed batch of operations"
            );
        }

        tracing::info!(total = total_pushed, "Push completed");
        Ok(total_pushed)
    }

    /// Pull remote operations from the server and apply them locally.
    ///
    /// Uses the stored cursor to resume from where we left off.
    /// Applies HLC LWW conflict resolution.
    pub async fn pull_remote(&self, auth_token: &str) -> Result<usize, SyncError> {
        let cursor_str = self.storage.get_sync_meta(META_PULL_CURSOR)?;
        let mut cursor: i64 = cursor_str
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let mut total_applied = 0usize;
        let page_size = 100i64;

        loop {
            let result = self.transport.pull_ops(cursor, page_size, auth_token).await?;

            if result.operations.is_empty() {
                break;
            }

            let mut page_failed = false;

            for remote_op in &result.operations {
                // Skip operations from this device (we already have them locally).
                // NOTE: If the process crashes mid-page, the cursor saved at the end
                // of the previous page will cause these ops to be re-fetched on restart.
                // This is safe because own-device ops are always skipped (idempotent).
                if remote_op.device_id == self.local_device_id {
                    cursor = remote_op.server_seq;
                    continue;
                }

                // Merge HLC clock with remote timestamp
                {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    let remote_hlc = HlcTimestamp::from_u64(remote_op.hlc_ts as u64, 0);
                    let mut hlc = self.hlc.lock().unwrap();
                    *hlc = hlc.merge(&remote_hlc, now_ms);
                }

                // Apply the operation (storage handles LWW conflict resolution)
                match self.storage.apply_remote_op(remote_op) {
                    Ok(applied) => {
                        if applied {
                            total_applied += 1;
                        }
                        // Only advance cursor after successful application
                        cursor = remote_op.server_seq;
                    }
                    Err(e) => {
                        tracing::error!(
                            op_id = %remote_op.op_id,
                            server_seq = remote_op.server_seq,
                            error = %e,
                            "Failed to apply remote operation, stopping pull to prevent data loss"
                        );
                        page_failed = true;
                        break;
                    }
                }
            }

            // Save cursor after each page (only up to last successfully applied op)
            if page_failed {
                self.storage
                    .set_sync_meta(META_PULL_CURSOR, &cursor.to_string())?;
                return Err(SyncError::Storage(
                    "Pull stopped: failed to apply remote operation".to_string(),
                ));
            }

            cursor = result.next_cursor;
            self.storage
                .set_sync_meta(META_PULL_CURSOR, &cursor.to_string())?;

            if !result.has_more {
                break;
            }
        }

        tracing::info!(total = total_applied, cursor = cursor, "Pull completed");
        Ok(total_applied)
    }

    /// Perform a full sync cycle: push then pull.
    pub async fn sync(&self, auth_token: &str) -> Result<(usize, usize), SyncError> {
        let pushed = self.push_pending(auth_token).await?;
        let pulled = self.pull_remote(auth_token).await?;
        Ok((pushed, pulled))
    }

    /// Get the local device ID.
    pub fn device_id(&self) -> &str {
        &self.local_device_id
    }
}

/// Resolve a Last-Writer-Wins conflict between local and remote operations.
///
/// Returns true if the remote operation should win (i.e., has a higher HLC timestamp).
pub fn lww_resolve(local_hlc_ts: u64, remote_hlc_ts: u64) -> bool {
    remote_hlc_ts > local_hlc_ts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // ─── Mock Storage ────────────────────────────────────────────────────

    struct MockStorage {
        unsynced_ops: Mutex<Vec<UnsyncedOp>>,
        synced_ids: Mutex<Vec<i64>>,
        meta: Mutex<HashMap<String, String>>,
        applied_ops: Mutex<Vec<RemoteOp>>,
        apply_result: Mutex<Result<bool, SyncError>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                unsynced_ops: Mutex::new(Vec::new()),
                synced_ids: Mutex::new(Vec::new()),
                meta: Mutex::new(HashMap::new()),
                applied_ops: Mutex::new(Vec::new()),
                apply_result: Mutex::new(Ok(true)),
            }
        }

        fn with_unsynced(ops: Vec<UnsyncedOp>) -> Self {
            let s = Self::new();
            *s.unsynced_ops.lock().unwrap() = ops;
            s
        }
    }

    impl SyncStorage for MockStorage {
        fn get_unsynced_ops(&self) -> Result<Vec<UnsyncedOp>, SyncError> {
            Ok(self.unsynced_ops.lock().unwrap().clone())
        }

        fn mark_ops_synced(&self, local_ids: &[i64]) -> Result<(), SyncError> {
            self.synced_ids.lock().unwrap().extend_from_slice(local_ids);
            Ok(())
        }

        fn get_sync_meta(&self, key: &str) -> Result<Option<String>, SyncError> {
            Ok(self.meta.lock().unwrap().get(key).cloned())
        }

        fn set_sync_meta(&self, key: &str, value: &str) -> Result<(), SyncError> {
            self.meta
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn apply_remote_op(&self, op: &RemoteOp) -> Result<bool, SyncError> {
            self.applied_ops.lock().unwrap().push(op.clone());
            let result = self.apply_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(*v),
                Err(e) => Err(SyncError::Storage(format!("{}", e))),
            }
        }
    }

    // ─── Mock Transport ──────────────────────────────────────────────────

    struct MockTransport {
        push_result: Mutex<Result<usize, SyncError>>,
        pull_result: Mutex<Result<PullResult, SyncError>>,
        pushed_ops: Mutex<Vec<PushOpPayload>>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                push_result: Mutex::new(Ok(0)),
                pull_result: Mutex::new(Ok(PullResult {
                    operations: vec![],
                    next_cursor: 0,
                    has_more: false,
                })),
                pushed_ops: Mutex::new(Vec::new()),
            }
        }

        fn with_push_result(result: Result<usize, SyncError>) -> Self {
            let t = Self::new();
            *t.push_result.lock().unwrap() = result;
            t
        }

        fn with_pull_result(result: PullResult) -> Self {
            let t = Self::new();
            *t.pull_result.lock().unwrap() = Ok(result);
            t
        }
    }

    impl SyncTransportAdapter for MockTransport {
        async fn push_ops(
            &self,
            ops: &[PushOpPayload],
            _auth_token: &str,
        ) -> Result<usize, SyncError> {
            self.pushed_ops.lock().unwrap().extend(ops.iter().cloned());
            let result = self.push_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(*v),
                Err(e) => Err(SyncError::Transport(format!("{}", e))),
            }
        }

        async fn pull_ops(
            &self,
            _cursor: i64,
            _limit: i64,
            _auth_token: &str,
        ) -> Result<PullResult, SyncError> {
            let result = self.pull_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(SyncError::Transport(format!("{}", e))),
            }
        }
    }

    // ─── LWW Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_lww_resolve_remote_wins() {
        assert!(lww_resolve(1000, 2000));
    }

    #[test]
    fn test_lww_resolve_local_wins() {
        assert!(!lww_resolve(2000, 1000));
    }

    #[test]
    fn test_lww_resolve_tie_local_wins() {
        assert!(!lww_resolve(1000, 1000));
    }

    // ─── Push Tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_push_empty_returns_zero() {
        let storage = MockStorage::new();
        let transport = MockTransport::new();
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.push_pending("token").await.unwrap();
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_push_sends_ops_and_marks_synced() {
        let ops = vec![
            UnsyncedOp {
                local_id: 1,
                op_id: "op-1".into(),
                op_type: "UpdateProgress".into(),
                op_data: "{}".into(),
                hlc_ts: 1000,
                device_id: "dev-1".into(),
            },
            UnsyncedOp {
                local_id: 2,
                op_id: "op-2".into(),
                op_type: "AddBookmark".into(),
                op_data: "{}".into(),
                hlc_ts: 2000,
                device_id: "dev-1".into(),
            },
        ];
        let storage = MockStorage::with_unsynced(ops);
        let transport = MockTransport::with_push_result(Ok(2));
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.push_pending("token").await.unwrap();
        assert_eq!(result, 2);

        // Verify ops were marked as synced
        let synced = engine.storage.synced_ids.lock().unwrap();
        assert_eq!(*synced, vec![1, 2]);

        // Verify ops were sent to transport
        let pushed = engine.transport.pushed_ops.lock().unwrap();
        assert_eq!(pushed.len(), 2);
        assert_eq!(pushed[0].op_id, "op-1");
        assert_eq!(pushed[1].op_id, "op-2");
    }

    #[tokio::test]
    async fn test_push_transport_error() {
        let ops = vec![UnsyncedOp {
            local_id: 1,
            op_id: "op-1".into(),
            op_type: "UpdateProgress".into(),
            op_data: "{}".into(),
            hlc_ts: 1000,
            device_id: "dev-1".into(),
        }];
        let storage = MockStorage::with_unsynced(ops);
        let transport =
            MockTransport::with_push_result(Err(SyncError::Transport("network down".into())));
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.push_pending("token").await;
        assert!(result.is_err());
    }

    // ─── Pull Tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_pull_empty_returns_zero() {
        let storage = MockStorage::new();
        let transport = MockTransport::new();
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.pull_remote("token").await.unwrap();
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_pull_applies_remote_ops() {
        let storage = MockStorage::new();
        let pull_result = PullResult {
            operations: vec![
                RemoteOp {
                    server_seq: 1,
                    op_id: "remote-1".into(),
                    op_type: "UpdateProgress".into(),
                    op_data: "{}".into(),
                    hlc_ts: 5000,
                    device_id: "dev-2".into(), // Different device
                },
                RemoteOp {
                    server_seq: 2,
                    op_id: "remote-2".into(),
                    op_type: "AddBookmark".into(),
                    op_data: "{}".into(),
                    hlc_ts: 6000,
                    device_id: "dev-2".into(),
                },
            ],
            next_cursor: 2,
            has_more: false,
        };
        let transport = MockTransport::with_pull_result(pull_result);
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.pull_remote("token").await.unwrap();
        assert_eq!(result, 2);

        // Verify ops were applied
        let applied = engine.storage.applied_ops.lock().unwrap();
        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0].op_id, "remote-1");
        assert_eq!(applied[1].op_id, "remote-2");

        // Verify cursor was saved
        let cursor = engine.storage.get_sync_meta("sync_pull_cursor").unwrap();
        assert_eq!(cursor, Some("2".to_string()));
    }

    #[tokio::test]
    async fn test_pull_skips_own_device_ops() {
        let storage = MockStorage::new();
        let pull_result = PullResult {
            operations: vec![RemoteOp {
                server_seq: 1,
                op_id: "own-op".into(),
                op_type: "UpdateProgress".into(),
                op_data: "{}".into(),
                hlc_ts: 5000,
                device_id: "dev-1".into(), // Same device
            }],
            next_cursor: 1,
            has_more: false,
        };
        let transport = MockTransport::with_pull_result(pull_result);
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.pull_remote("token").await.unwrap();
        assert_eq!(result, 0); // Skipped own op

        let applied = engine.storage.applied_ops.lock().unwrap();
        assert_eq!(applied.len(), 0);

        let cursor = engine.storage.get_sync_meta("sync_pull_cursor").unwrap();
        assert_eq!(cursor, Some("1".to_string()));
    }

    #[tokio::test]
    async fn test_pull_uses_server_next_cursor() {
        let storage = MockStorage::new();
        let pull_result = PullResult {
            operations: vec![RemoteOp {
                server_seq: 2,
                op_id: "remote-1".into(),
                op_type: "UpdateProgress".into(),
                op_data: "{}".into(),
                hlc_ts: 5000,
                device_id: "dev-2".into(),
            }],
            next_cursor: 10,
            has_more: false,
        };
        let transport = MockTransport::with_pull_result(pull_result);
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let result = engine.pull_remote("token").await.unwrap();
        assert_eq!(result, 1);

        let cursor = engine.storage.get_sync_meta("sync_pull_cursor").unwrap();
        assert_eq!(cursor, Some("10".to_string()));
    }

    // ─── Full Sync Tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_full_sync_push_then_pull() {
        let ops = vec![UnsyncedOp {
            local_id: 1,
            op_id: "local-1".into(),
            op_type: "UpdateProgress".into(),
            op_data: "{}".into(),
            hlc_ts: 1000,
            device_id: "dev-1".into(),
        }];
        let storage = MockStorage::with_unsynced(ops);
        let transport = MockTransport::with_push_result(Ok(1));
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let (pushed, pulled) = engine.sync("token").await.unwrap();
        assert_eq!(pushed, 1);
        assert_eq!(pulled, 0);
    }

    // ─── Engine Utility Tests ────────────────────────────────────────────

    #[test]
    fn test_engine_device_id() {
        let storage = MockStorage::new();
        let transport = MockTransport::new();
        let engine = SyncEngine::new(storage, transport, "my-device".into(), 42);

        assert_eq!(engine.device_id(), "my-device");
    }

    #[test]
    fn test_engine_tick_advances_clock() {
        let storage = MockStorage::new();
        let transport = MockTransport::new();
        let engine = SyncEngine::new(storage, transport, "dev-1".into(), 1);

        let ts1 = engine.tick();
        let ts2 = engine.tick();
        assert!(ts2 > ts1);
    }
}
