//! File storage handlers: upload (init, chunk, complete) and download.

use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::header;
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::AppState;

// ─── Request / Response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UploadInitRequest {
    /// Original file name.
    pub file_name: String,
    /// Total file size in bytes.
    pub file_size: i64,
    /// Expected SHA-256 hash of the complete file.
    pub sha256: String,
    /// Chunk size in bytes (default: 4MB).
    pub chunk_size: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct UploadInitResponse {
    pub upload_id: String,
    pub chunk_size: i64,
    pub total_chunks: i64,
}

#[derive(Debug, Serialize)]
pub struct UploadChunkResponse {
    pub chunk_index: i64,
    pub received_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct UploadCompleteResponse {
    pub file_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub file_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub sha256: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct FilesListResponse {
    pub files: Vec<FileInfo>,
}

// ─── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_CHUNK_SIZE: i64 = 4 * 1024 * 1024; // 4MB
const MAX_CHUNK_SIZE: i64 = 8 * 1024 * 1024; // 8MB
const MAX_FILE_SIZE: i64 = 500 * 1024 * 1024; // 500MB

// ─── Handlers ───────────────────────────────────────────────────────────────

/// POST /files/upload/init — initialize a chunked upload session.
pub async fn upload_init(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<UploadInitRequest>,
) -> Result<Json<UploadInitResponse>, AppError> {
    // Validate file size
    if req.file_size <= 0 || req.file_size > MAX_FILE_SIZE {
        return Err(AppError::BadRequest(format!(
            "File size must be between 1 byte and {} bytes",
            MAX_FILE_SIZE
        )));
    }

    // Validate file name
    if req.file_name.is_empty() || req.file_name.len() > 255 {
        return Err(AppError::BadRequest(
            "File name must be between 1 and 255 characters".to_string(),
        ));
    }

    // Check if file with same hash already exists for this user (dedup)
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM user_files WHERE user_id = ? AND sha256 = ? AND deleted_at IS NULL",
    )
    .bind(&auth.user_id)
    .bind(&req.sha256)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((file_id,)) = existing {
        return Err(AppError::Conflict(format!(
            "File with same hash already exists: {}",
            file_id
        )));
    }

    let chunk_size = req.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);
    if !(1024..=MAX_CHUNK_SIZE).contains(&chunk_size) {
        return Err(AppError::BadRequest(format!(
            "Chunk size must be between 1024 and {} bytes",
            MAX_CHUNK_SIZE
        )));
    }
    let total_chunks = (req.file_size + chunk_size - 1) / chunk_size;

    let upload_id = Uuid::new_v4().to_string();

    // Create upload session directory
    let upload_dir = PathBuf::from(&state.config.storage_path)
        .join("uploads")
        .join(&upload_id);
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create upload directory: {}", e)))?;

    // Record upload session
    sqlx::query(
        "INSERT INTO upload_sessions (id, user_id, file_name, file_size, sha256, chunk_size, total_chunks, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending')",
    )
    .bind(&upload_id)
    .bind(&auth.user_id)
    .bind(&req.file_name)
    .bind(req.file_size)
    .bind(&req.sha256)
    .bind(chunk_size)
    .bind(total_chunks)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        user_id = %auth.user_id,
        upload_id = %upload_id,
        file_name = %req.file_name,
        file_size = req.file_size,
        total_chunks = total_chunks,
        "Upload session initialized"
    );

    Ok(Json(UploadInitResponse {
        upload_id,
        chunk_size,
        total_chunks,
    }))
}

/// PUT /files/upload/:upload_id/chunk/:index — upload a single chunk.
pub async fn upload_chunk(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path((upload_id, chunk_index)): Path<(String, i64)>,
    body: Body,
) -> Result<Json<UploadChunkResponse>, AppError> {
    // Verify upload session belongs to user and is pending
    let session: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT user_id, total_chunks, chunk_size FROM upload_sessions WHERE id = ? AND status = 'pending'",
    )
    .bind(&upload_id)
    .fetch_optional(&state.pool)
    .await?;

    let (session_user_id, total_chunks, chunk_size) = session.ok_or_else(|| {
        AppError::NotFound("Upload session not found or already completed".to_string())
    })?;

    if session_user_id != auth.user_id {
        return Err(AppError::Unauthorized(
            "Upload session belongs to another user".to_string(),
        ));
    }

    if chunk_index < 0 || chunk_index >= total_chunks {
        return Err(AppError::BadRequest(format!(
            "Chunk index must be between 0 and {}",
            total_chunks - 1
        )));
    }

    let max_chunk_bytes = usize::try_from(chunk_size)
        .map_err(|_| AppError::Internal("Configured chunk size exceeds platform limits".to_string()))?;

    let bytes = axum::body::to_bytes(body, max_chunk_bytes)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read chunk data: {}", e)))?;

    let received_bytes = bytes.len() as i64;

    // Write chunk to disk
    let chunk_path = PathBuf::from(&state.config.storage_path)
        .join("uploads")
        .join(&upload_id)
        .join(format!("chunk_{:06}", chunk_index));

    let mut file = tokio::fs::File::create(&chunk_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create chunk file: {}", e)))?;

    file.write_all(&bytes)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write chunk data: {}", e)))?;

    // Record chunk in database
    sqlx::query(
        "INSERT INTO file_chunks (upload_id, chunk_index, size, received_at)
         VALUES (?, ?, ?, datetime('now'))
         ON CONFLICT(upload_id, chunk_index) DO UPDATE SET
            size = excluded.size,
            received_at = datetime('now')",
    )
    .bind(&upload_id)
    .bind(chunk_index)
    .bind(received_bytes)
    .execute(&state.pool)
    .await?;

    tracing::debug!(
        upload_id = %upload_id,
        chunk_index = chunk_index,
        received_bytes = received_bytes,
        "Chunk uploaded"
    );

    Ok(Json(UploadChunkResponse {
        chunk_index,
        received_bytes,
    }))
}

/// POST /files/upload/:upload_id/complete — finalize the upload.
pub async fn upload_complete(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(upload_id): Path<String>,
) -> Result<Json<UploadCompleteResponse>, AppError> {
    // Verify upload session
    let session: Option<(String, String, i64, String, i64)> = sqlx::query_as(
        "SELECT user_id, file_name, file_size, sha256, total_chunks
         FROM upload_sessions WHERE id = ? AND status = 'pending'",
    )
    .bind(&upload_id)
    .fetch_optional(&state.pool)
    .await?;

    let (session_user_id, file_name, file_size, expected_sha256, total_chunks) =
        session.ok_or_else(|| {
            AppError::NotFound("Upload session not found or already completed".to_string())
        })?;

    if session_user_id != auth.user_id {
        return Err(AppError::Unauthorized(
            "Upload session belongs to another user".to_string(),
        ));
    }

    // Verify all chunks are present
    let chunk_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM file_chunks WHERE upload_id = ?",
    )
    .bind(&upload_id)
    .fetch_one(&state.pool)
    .await?;

    if chunk_count != total_chunks {
        return Err(AppError::BadRequest(format!(
            "Expected {} chunks, but only {} received",
            total_chunks, chunk_count
        )));
    }

    // Merge chunks and compute SHA-256
    let upload_dir = PathBuf::from(&state.config.storage_path)
        .join("uploads")
        .join(&upload_id);

    let file_id = Uuid::new_v4().to_string();
    let files_dir = PathBuf::from(&state.config.storage_path).join("files");
    tokio::fs::create_dir_all(&files_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create files directory: {}", e)))?;

    let final_path = files_dir.join(&file_id);
    let mut final_file = tokio::fs::File::create(&final_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create final file: {}", e)))?;

    let mut hasher = Sha256::new();
    let mut actual_size: i64 = 0;

    for i in 0..total_chunks {
        let chunk_path = upload_dir.join(format!("chunk_{:06}", i));
        let mut chunk_file = tokio::fs::File::open(&chunk_path)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to open chunk {}: {}", i, e)))?;

        let mut buf = vec![0u8; 64 * 1024]; // 64KB read buffer
        loop {
            let n = chunk_file
                .read(&mut buf)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to read chunk {}: {}", i, e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            actual_size += n as i64;
            final_file
                .write_all(&buf[..n])
                .await
                .map_err(|e| AppError::Internal(format!("Failed to write to final file: {}", e)))?;
        }
    }

    let actual_sha256 = format!("{:x}", hasher.finalize());

    // Verify SHA-256
    if actual_sha256 != expected_sha256 {
        // Clean up the merged file
        let _ = tokio::fs::remove_file(&final_path).await;
        return Err(AppError::BadRequest(format!(
            "SHA-256 mismatch: expected {}, got {}",
            expected_sha256, actual_sha256
        )));
    }

    // Verify file size
    if actual_size != file_size {
        let _ = tokio::fs::remove_file(&final_path).await;
        return Err(AppError::BadRequest(format!(
            "File size mismatch: expected {}, got {}",
            file_size, actual_size
        )));
    }

    // Record file metadata
    let mut tx = state.pool.begin().await?;
    let db_result: Result<(), AppError> = async {
        sqlx::query(
            "INSERT INTO user_files (id, user_id, file_name, file_size, sha256, storage_path)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&file_id)
        .bind(&auth.user_id)
        .bind(&file_name)
        .bind(file_size)
        .bind(&actual_sha256)
        .bind(final_path.to_string_lossy().as_ref())
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE upload_sessions SET status = 'completed' WHERE id = ?")
            .bind(&upload_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM file_chunks WHERE upload_id = ?")
            .bind(&upload_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
    .await;

    if let Err(err) = db_result {
        let _ = tokio::fs::remove_file(&final_path).await;
        return Err(map_user_file_insert_error(err, &actual_sha256));
    }

    if let Err(err) = tokio::fs::remove_dir_all(&upload_dir).await {
        tracing::warn!(
            upload_id = %upload_id,
            error = %err,
            "Failed to clean up upload directory after completion"
        );
    }

    tracing::info!(
        user_id = %auth.user_id,
        file_id = %file_id,
        file_name = %file_name,
        file_size = file_size,
        "Upload completed"
    );

    Ok(Json(UploadCompleteResponse {
        file_id,
        file_name,
        file_size,
        sha256: actual_sha256,
    }))
}

/// GET /files/:file_id — download a file.
pub async fn download(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(file_id): Path<String>,
) -> Result<Response, AppError> {
    // Find file metadata
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT file_name, storage_path, file_size, sha256
         FROM user_files WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(&file_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let (file_name, storage_path, file_size, _sha256) = row.ok_or_else(|| {
        AppError::NotFound("File not found".to_string())
    })?;

    // Stream file to client instead of reading entire file into memory
    let file = tokio::fs::File::open(&storage_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open file: {}", e)))?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            encode_content_disposition(&file_name),
        )
        .header(header::CONTENT_LENGTH, file_size.to_string())
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

/// Encode a filename for the Content-Disposition header using RFC 5987.
///
/// Falls back to ASCII-safe `filename` and adds `filename*=UTF-8''...` for
/// non-ASCII characters or characters that need escaping.
fn encode_content_disposition(file_name: &str) -> String {
    // Check if the filename is simple ASCII without special chars
    let is_simple_ascii = file_name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' || b == b' ');

    if is_simple_ascii {
        format!("attachment; filename=\"{}\"" , file_name)
    } else {
        // Percent-encode the filename for filename* parameter
        let encoded: String = file_name
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' {
                    String::from(b as char)
                } else {
                    format!("%{:02X}", b)
                }
            })
            .collect();

        // Provide a fallback ASCII filename and the UTF-8 encoded version
        let ascii_fallback: String = file_name
            .chars()
            .map(|c| if c.is_ascii() && c != '"' { c } else { '_' })
            .collect();

        format!(
            "attachment; filename=\"{}\"; filename*=UTF-8''{}",
            ascii_fallback, encoded
        )
    }
}

/// GET /files — list user's files.
pub async fn list_files(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<FilesListResponse>, AppError> {
    let rows: Vec<(String, String, i64, String, String)> = sqlx::query_as(
        "SELECT id, file_name, file_size, sha256, created_at
         FROM user_files WHERE user_id = ? AND deleted_at IS NULL
         ORDER BY created_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let files = rows
        .into_iter()
        .map(|(id, name, size, sha256, created)| FileInfo {
            file_id: id,
            file_name: name,
            file_size: size,
            sha256,
            created_at: created,
        })
        .collect();

    Ok(Json(FilesListResponse { files }))
}

/// DELETE /files/:file_id — soft-delete a file.
pub async fn delete_file(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(file_id): Path<String>,
) -> Result<Json<crate::handlers::auth::MessageResponse>, AppError> {
    let result = sqlx::query(
        "UPDATE user_files SET deleted_at = datetime('now') WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(&file_id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("File not found".to_string()));
    }

    tracing::info!(user_id = %auth.user_id, file_id = %file_id, "File soft-deleted");

    Ok(Json(crate::handlers::auth::MessageResponse {
        message: "File deleted successfully".to_string(),
    }))
}

fn map_user_file_insert_error(err: AppError, sha256: &str) -> AppError {
    if let AppError::Database(sqlx::Error::Database(db_err)) = &err {
        let message = db_err.message();
        if message.contains("idx_user_files_active_sha256")
            || message.contains("user_files.user_id")
            || message.contains("user_files.sha256")
        {
            return AppError::Conflict(format!(
                "File with same hash already exists: {}",
                sha256
            ));
        }
    }

    err
}
