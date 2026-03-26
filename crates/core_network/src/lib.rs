//! Core network module — stub for P0.
//!
//! Defines the `SyncTransport` trait interface for network communication.
//! Full implementation is deferred to a later phase.

use core_sync::SyncOperation;

/// Network transport error types.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Request timeout")]
    Timeout,
    #[error("Server error: {status_code}")]
    ServerError { status_code: u16 },
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Unknown transport error: {0}")]
    Unknown(String),
}

/// Response from a sync push operation.
#[derive(Debug, Clone)]
pub struct SyncPushResponse {
    /// Number of operations accepted by the server.
    pub accepted_count: usize,
    /// Server-assigned timestamp for the batch.
    pub server_timestamp: u64,
}

/// Response from a sync pull operation.
#[derive(Debug, Clone)]
pub struct SyncPullResponse {
    /// Operations received from the server.
    pub operations: Vec<SyncOperation>,
    /// Server cursor for pagination.
    pub cursor: Option<String>,
}

/// Trait defining the sync transport interface.
///
/// Implementations will handle specific transport protocols (HTTP, WebSocket, etc.).
pub trait SyncTransport {
    /// Push local operations to the remote server.
    fn push_operations(
        &self,
        operations: &[SyncOperation],
        auth_token: &str,
    ) -> Result<SyncPushResponse, TransportError>;

    /// Pull remote operations since the given cursor.
    fn pull_operations(
        &self,
        since_cursor: Option<&str>,
        auth_token: &str,
    ) -> Result<SyncPullResponse, TransportError>;

    /// Check if the remote server is reachable.
    fn health_check(&self) -> Result<bool, TransportError>;
}
