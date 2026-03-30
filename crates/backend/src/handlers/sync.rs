//! Sync handlers: push and pull operations.

use axum::extract::{Extension, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::AppState;

// ─── Request / Response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PushRequest {
    /// Batch of operations to push (max 500 per request).
    pub operations: Vec<PushOperation>,
}

#[derive(Debug, Deserialize)]
pub struct PushOperation {
    /// Client-generated operation ID.
    pub op_id: String,
    /// Operation type (e.g., "UpdateProgress", "AddBookmark").
    pub op_type: String,
    /// JSON-serialized operation data.
    pub op_data: String,
    /// HLC timestamp as u64.
    pub hlc_ts: i64,
}

#[derive(Debug, Serialize)]
pub struct PushResponse {
    pub accepted_count: usize,
    pub server_timestamp: String,
}

#[derive(Debug, Deserialize)]
pub struct PullQuery {
    /// Server sequence cursor (0 = from beginning).
    pub cursor: Option<i64>,
    /// Number of operations to return (default 100, max 1000).
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PullResponse {
    pub operations: Vec<PullOperation>,
    /// Next cursor value for pagination.
    pub next_cursor: i64,
    /// Whether there are more operations to pull.
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
pub struct PullOperation {
    pub server_seq: i64,
    pub op_id: String,
    pub op_type: String,
    pub op_data: String,
    pub hlc_ts: i64,
    pub device_id: String,
    pub created_at: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// POST /sync/push — push a batch of operations to the server.
pub async fn push(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, AppError> {
    // Validate batch size
    if req.operations.is_empty() {
        return Err(AppError::BadRequest("No operations to push".to_string()));
    }
    if req.operations.len() > 500 {
        return Err(AppError::BadRequest(
            "Maximum 500 operations per push request".to_string(),
        ));
    }

    let mut accepted = 0usize;

    // Use a transaction for atomicity
    let mut tx = state.pool.begin().await?;

    for op in &req.operations {
        // Skip duplicates (idempotent push)
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT server_seq FROM sync_operations WHERE op_id = ? AND user_id = ?",
        )
        .bind(&op.op_id)
        .bind(&auth.user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if existing.is_some() {
            // Already pushed, skip
            continue;
        }

        sqlx::query(
            "INSERT INTO sync_operations (user_id, op_id, op_type, op_data, hlc_ts, device_id)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&auth.user_id)
        .bind(&op.op_id)
        .bind(&op.op_type)
        .bind(&op.op_data)
        .bind(op.hlc_ts)
        .bind(&auth.device_id)
        .execute(&mut *tx)
        .await?;

        accepted += 1;
    }

    tx.commit().await?;

    let server_timestamp = chrono::Utc::now().to_rfc3339();

    tracing::info!(
        user_id = %auth.user_id,
        accepted = accepted,
        total = req.operations.len(),
        "Sync push completed"
    );

    Ok(Json(PushResponse {
        accepted_count: accepted,
        server_timestamp,
    }))
}

/// GET /sync/pull?cursor=N&limit=M — pull operations from the server.
pub async fn pull(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(query): Query<PullQuery>,
) -> Result<Json<PullResponse>, AppError> {
    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).min(1000).max(1);

    // Fetch limit+1 to detect if there are more
    let rows: Vec<(i64, String, String, String, i64, String, String)> = sqlx::query_as(
        "SELECT server_seq, op_id, op_type, op_data, hlc_ts, device_id, created_at
         FROM sync_operations
         WHERE user_id = ? AND server_seq > ?
         ORDER BY server_seq ASC
         LIMIT ?",
    )
    .bind(&auth.user_id)
    .bind(cursor)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;

    let has_more = rows.len() as i64 > limit;
    let operations: Vec<PullOperation> = rows
        .into_iter()
        .take(limit as usize)
        .map(|(seq, op_id, op_type, op_data, hlc_ts, device_id, created_at)| PullOperation {
            server_seq: seq,
            op_id,
            op_type,
            op_data,
            hlc_ts,
            device_id,
            created_at,
        })
        .collect();

    let next_cursor = operations.last().map(|op| op.server_seq).unwrap_or(cursor);

    tracing::debug!(
        user_id = %auth.user_id,
        cursor = cursor,
        returned = operations.len(),
        has_more = has_more,
        "Sync pull completed"
    );

    Ok(Json(PullResponse {
        operations,
        next_cursor,
        has_more,
    }))
}
