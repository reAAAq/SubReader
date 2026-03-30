//! Core network module.
//!
//! Provides async network transport traits and HTTP-based implementation
//! for sync operations and file transfers.

use core_sync::SyncOperation;
use serde::{Deserialize, Serialize};

pub mod http_transport;

// ─── Error types ────────────────────────────────────────────────────────────

/// Network transport error types.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Request timeout")]
    Timeout,

    #[error("Server error: {status_code} - {message}")]
    ServerError { status_code: u16, message: String },

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Unknown transport error: {0}")]
    Unknown(String),
}

// ─── Sync transport types ───────────────────────────────────────────────────

/// A single operation to push to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushOperation {
    pub op_id: String,
    pub op_type: String,
    pub op_data: String,
    pub hlc_ts: i64,
}

/// Response from a sync push operation.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncPushResponse {
    /// Number of operations accepted by the server.
    pub accepted_count: usize,
    /// Server timestamp.
    pub server_timestamp: String,
}

/// A single operation pulled from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct PulledOperation {
    pub server_seq: i64,
    pub op_id: String,
    pub op_type: String,
    pub op_data: String,
    pub hlc_ts: i64,
    pub device_id: String,
    pub created_at: String,
}

/// Response from a sync pull operation.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncPullResponse {
    /// Operations received from the server.
    pub operations: Vec<PulledOperation>,
    /// Next cursor for pagination.
    pub next_cursor: i64,
    /// Whether there are more operations.
    pub has_more: bool,
}

// ─── File transport types ───────────────────────────────────────────────────

/// Upload session initialization response.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadInitResponse {
    pub upload_id: String,
    pub chunk_size: i64,
    pub total_chunks: i64,
}

/// Upload chunk response.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadChunkResponse {
    pub chunk_index: i64,
    pub received_bytes: i64,
}

/// Upload complete response.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadCompleteResponse {
    pub file_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub sha256: String,
}

/// File info from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteFileInfo {
    pub file_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub sha256: String,
    pub created_at: String,
}

// ─── Traits ─────────────────────────────────────────────────────────────────

/// Async sync transport trait.
#[allow(async_fn_in_trait)]
pub trait SyncTransport: Send + Sync {
    /// Push local operations to the remote server.
    fn push_operations(
        &self,
        operations: &[PushOperation],
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<SyncPushResponse, TransportError>> + Send;

    /// Pull remote operations since the given cursor.
    fn pull_operations(
        &self,
        cursor: i64,
        limit: i64,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<SyncPullResponse, TransportError>> + Send;

    /// Check if the remote server is reachable.
    fn health_check(&self) -> impl std::future::Future<Output = Result<bool, TransportError>> + Send;
}

/// Async file transport trait.
#[allow(async_fn_in_trait)]
pub trait FileTransport: Send + Sync {
    /// Initialize a chunked upload session.
    fn upload_init(
        &self,
        file_name: &str,
        file_size: i64,
        sha256: &str,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<UploadInitResponse, TransportError>> + Send;

    /// Upload a single chunk.
    fn upload_chunk(
        &self,
        upload_id: &str,
        chunk_index: i64,
        data: &[u8],
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<UploadChunkResponse, TransportError>> + Send;

    /// Complete the upload.
    fn upload_complete(
        &self,
        upload_id: &str,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<UploadCompleteResponse, TransportError>> + Send;

    /// Download a file by ID.
    fn download_file(
        &self,
        file_id: &str,
        auth_token: &str,
        writer: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send;

    /// List user's remote files.
    fn list_files(
        &self,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<Vec<RemoteFileInfo>, TransportError>> + Send;
}

/// Convert a SyncOperation to a PushOperation for transport.
impl From<&SyncOperation> for PushOperation {
    fn from(op: &SyncOperation) -> Self {
        let op_type = match &op.operation {
            core_sync::Operation::UpdateProgress { .. } => "UpdateProgress",
            core_sync::Operation::AddBookmark { .. } => "AddBookmark",
            core_sync::Operation::DeleteBookmark { .. } => "DeleteBookmark",
            core_sync::Operation::AddAnnotation { .. } => "AddAnnotation",
            core_sync::Operation::DeleteAnnotation { .. } => "DeleteAnnotation",
            core_sync::Operation::UpdatePreference { .. } => "UpdatePreference",
        };

        PushOperation {
            op_id: op.op_id.clone(),
            op_type: op_type.to_string(),
            op_data: serde_json::to_string(&op.operation).unwrap_or_default(),
            hlc_ts: op.hlc_timestamp.to_u64() as i64,
        }
    }
}

/// Concrete sync transport adapter backed by a core_network transport.
pub struct NetworkSyncAdapter<T> {
    transport: T,
}

impl<T> NetworkSyncAdapter<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> core_sync::engine::SyncTransportAdapter for NetworkSyncAdapter<T>
where
    T: SyncTransport,
{
    async fn push_ops(
        &self,
        ops: &[core_sync::engine::PushOpPayload],
        auth_token: &str,
    ) -> Result<usize, core_sync::SyncError> {
        let payloads: Vec<PushOperation> = ops
            .iter()
            .map(|op| PushOperation {
                op_id: op.op_id.clone(),
                op_type: op.op_type.clone(),
                op_data: op.op_data.clone(),
                hlc_ts: op.hlc_ts,
            })
            .collect();

        self.transport
            .push_operations(&payloads, auth_token)
            .await
            .map(|response| response.accepted_count)
            .map_err(map_transport_error_to_sync_error)
    }

    async fn pull_ops(
        &self,
        cursor: i64,
        limit: i64,
        auth_token: &str,
    ) -> Result<core_sync::engine::PullResult, core_sync::SyncError> {
        self.transport
            .pull_operations(cursor, limit, auth_token)
            .await
            .map(|response| core_sync::engine::PullResult {
                operations: response
                    .operations
                    .into_iter()
                    .map(|op| core_sync::engine::RemoteOp {
                        server_seq: op.server_seq,
                        op_id: op.op_id,
                        op_type: op.op_type,
                        op_data: op.op_data,
                        hlc_ts: op.hlc_ts,
                        device_id: op.device_id,
                    })
                    .collect(),
                next_cursor: response.next_cursor,
                has_more: response.has_more,
            })
            .map_err(map_transport_error_to_sync_error)
    }
}

/// Concrete file transport adapter backed by a core_network transport.
pub struct NetworkFileAdapter<T> {
    transport: T,
}

impl<T> NetworkFileAdapter<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> core_sync::file_sync::FileTransportAdapter for NetworkFileAdapter<T>
where
    T: FileTransport,
{
    async fn upload_init(
        &self,
        file_name: &str,
        file_size: i64,
        sha256: &str,
        auth_token: &str,
    ) -> Result<core_sync::file_sync::UploadSession, core_sync::SyncError> {
        self.transport
            .upload_init(file_name, file_size, sha256, auth_token)
            .await
            .map(|response| core_sync::file_sync::UploadSession {
                upload_id: response.upload_id,
                chunk_size: response.chunk_size,
                total_chunks: response.total_chunks,
            })
            .map_err(map_transport_error_to_sync_error)
    }

    async fn upload_chunk(
        &self,
        upload_id: &str,
        chunk_index: i64,
        data: &[u8],
        auth_token: &str,
    ) -> Result<i64, core_sync::SyncError> {
        self.transport
            .upload_chunk(upload_id, chunk_index, data, auth_token)
            .await
            .map(|response| response.received_bytes)
            .map_err(map_transport_error_to_sync_error)
    }

    async fn upload_complete(
        &self,
        upload_id: &str,
        auth_token: &str,
    ) -> Result<String, core_sync::SyncError> {
        self.transport
            .upload_complete(upload_id, auth_token)
            .await
            .map(|response| response.file_id)
            .map_err(map_transport_error_to_sync_error)
    }

    async fn download_file(
        &self,
        file_id: &str,
        auth_token: &str,
        writer: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> Result<(), core_sync::SyncError> {
        self.transport
            .download_file(file_id, auth_token, writer)
            .await
            .map_err(map_transport_error_to_sync_error)
    }

    async fn list_remote_files(
        &self,
        auth_token: &str,
    ) -> Result<Vec<core_sync::file_sync::RemoteFileInfo>, core_sync::SyncError> {
        self.transport
            .list_files(auth_token)
            .await
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| core_sync::file_sync::RemoteFileInfo {
                        file_id: file.file_id,
                        file_name: file.file_name,
                        file_size: file.file_size,
                        sha256: file.sha256,
                    })
                    .collect()
            })
            .map_err(map_transport_error_to_sync_error)
    }
}

fn map_transport_error_to_sync_error(err: TransportError) -> core_sync::SyncError {
    match err {
        TransportError::Unauthorized => core_sync::SyncError::NotAuthenticated,
        other => core_sync::SyncError::Transport(other.to_string()),
    }
}
