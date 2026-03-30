//! File sync module — handles chunked upload/download of book files.
//!
//! Provides file synchronization with SHA-256 integrity verification,
//! chunked transfers, and progress reporting.

use crate::SyncError;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Progress callback type for file transfers.
pub type ProgressCallback = Box<dyn Fn(FileProgress) + Send + Sync>;

/// File transfer progress information.
#[derive(Debug, Clone)]
pub struct FileProgress {
    /// File name being transferred.
    pub file_name: String,
    /// Current chunk index (0-based).
    pub current_chunk: i64,
    /// Total number of chunks.
    pub total_chunks: i64,
    /// Bytes transferred so far.
    pub bytes_transferred: i64,
    /// Total file size in bytes.
    pub total_bytes: i64,
    /// Whether this is an upload (true) or download (false).
    pub is_upload: bool,
}

impl FileProgress {
    /// Get the progress percentage (0.0 to 100.0).
    pub fn percentage(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.bytes_transferred as f64 / self.total_bytes as f64) * 100.0
    }
}

/// Trait abstracting file transport operations.
pub trait FileTransportAdapter: Send + Sync {
    /// Initialize an upload session.
    fn upload_init(
        &self,
        file_name: &str,
        file_size: i64,
        sha256: &str,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<UploadSession, SyncError>> + Send;

    /// Upload a single chunk.
    fn upload_chunk(
        &self,
        upload_id: &str,
        chunk_index: i64,
        data: &[u8],
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<i64, SyncError>> + Send;

    /// Complete an upload.
    fn upload_complete(
        &self,
        upload_id: &str,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<String, SyncError>> + Send;

    /// Download a file by ID.
    fn download_file(
        &self,
        file_id: &str,
        auth_token: &str,
        writer: &mut (dyn AsyncWrite + Unpin + Send),
    ) -> impl std::future::Future<Output = Result<(), SyncError>> + Send;

    /// List remote files.
    fn list_remote_files(
        &self,
        auth_token: &str,
    ) -> impl std::future::Future<Output = Result<Vec<RemoteFileInfo>, SyncError>> + Send;
}

/// Upload session information.
#[derive(Debug, Clone)]
pub struct UploadSession {
    pub upload_id: String,
    pub chunk_size: i64,
    pub total_chunks: i64,
}

/// Remote file information.
#[derive(Debug, Clone)]
pub struct RemoteFileInfo {
    pub file_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub sha256: String,
}

/// File sync engine that handles upload and download of book files.
pub struct FileSyncEngine<T: FileTransportAdapter> {
    transport: T,
    progress_callback: Option<ProgressCallback>,
}

impl<T: FileTransportAdapter> FileSyncEngine<T> {
    /// Create a new file sync engine.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            progress_callback: None,
        }
    }

    /// Set a progress callback for file transfers.
    pub fn set_progress_callback(&mut self, callback: ProgressCallback) {
        self.progress_callback = Some(callback);
    }

    /// Upload a file from the local filesystem.
    ///
    /// 1. Compute SHA-256 hash
    /// 2. Initialize upload session
    /// 3. Upload chunks
    /// 4. Complete upload
    pub async fn upload_file(
        &self,
        file_path: &Path,
        auth_token: &str,
    ) -> Result<String, SyncError> {
        // Stream file: compute SHA-256 without loading entire file into memory
        let mut file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| SyncError::Storage(format!("Failed to open file: {}", e)))?;

        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let metadata = file
            .metadata()
            .await
            .map_err(|e| SyncError::Storage(format!("Failed to read file metadata: {}", e)))?;
        let file_size = metadata.len() as i64;

        // Compute SHA-256 by streaming through the file
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024]; // 64KB read buffer
        loop {
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| SyncError::Storage(format!("Failed to read file: {}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let sha256 = format!("{:x}", hasher.finalize());

        tracing::info!(
            file_name = %file_name,
            file_size = file_size,
            sha256 = %sha256,
            "Starting file upload"
        );

        // Initialize upload session
        let session = self
            .transport
            .upload_init(&file_name, file_size, &sha256, auth_token)
            .await?;

        // Re-open file for chunked upload
        let mut file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| SyncError::Storage(format!("Failed to reopen file: {}", e)))?;

        let chunk_size = session.chunk_size as usize;
        let mut bytes_transferred: i64 = 0;
        let mut chunk_buf = vec![0u8; chunk_size];

        for i in 0..session.total_chunks {
            // Read one chunk from file
            let mut chunk_len = 0;
            while chunk_len < chunk_size {
                let n = file
                    .read(&mut chunk_buf[chunk_len..])
                    .await
                    .map_err(|e| SyncError::Storage(format!("Failed to read chunk: {}", e)))?;
                if n == 0 {
                    break;
                }
                chunk_len += n;
            }

            let chunk_data = &chunk_buf[..chunk_len];
            let mut retries = 0;
            let max_retries = 3;

            loop {
                match self
                    .transport
                    .upload_chunk(&session.upload_id, i, chunk_data, auth_token)
                    .await
                {
                    Ok(received) => {
                        bytes_transferred += received;
                        break;
                    }
                    Err(e) if retries < max_retries => {
                        retries += 1;
                        tracing::warn!(
                            chunk_index = i,
                            retry = retries,
                            error = %e,
                            "Chunk upload failed, retrying"
                        );
                        // Exponential backoff
                        let delay = std::time::Duration::from_secs(1 << retries);
                        tokio::time::sleep(delay).await;
                    }
                    Err(e) => return Err(e),
                }
            }

            // Report progress
            if let Some(ref cb) = self.progress_callback {
                cb(FileProgress {
                    file_name: file_name.clone(),
                    current_chunk: i + 1,
                    total_chunks: session.total_chunks,
                    bytes_transferred,
                    total_bytes: file_size,
                    is_upload: true,
                });
            }
        }

        // Complete upload
        let file_id = self
            .transport
            .upload_complete(&session.upload_id, auth_token)
            .await?;

        tracing::info!(
            file_id = %file_id,
            file_name = %file_name,
            "File upload completed"
        );

        Ok(file_id)
    }

    /// Download a file from the server and save to local path.
    ///
    /// 1. Download file data
    /// 2. Verify SHA-256 hash
    /// 3. Write to local filesystem
    pub async fn download_file(
        &self,
        file_id: &str,
        expected_sha256: &str,
        dest_path: &Path,
        auth_token: &str,
    ) -> Result<(), SyncError> {
        tracing::info!(
            file_id = %file_id,
            dest = %dest_path.display(),
            "Starting file download"
        );

        let mut retries = 0;
        let max_retries = 3;
        let temp_path = temporary_download_path(dest_path);

        loop {
            // Ensure parent directory exists using async fs
            if let Some(parent) = dest_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| SyncError::Storage(format!("Failed to create directory: {}", e)))?;
            }

            let mut temp_file = tokio::fs::File::create(&temp_path)
                .await
                .map_err(|e| SyncError::Storage(format!("Failed to create temp file: {}", e)))?;

            match self
                .transport
                .download_file(file_id, auth_token, &mut temp_file)
                .await
            {
                Ok(()) => {
                    temp_file
                        .flush()
                        .await
                        .map_err(|e| SyncError::Storage(format!("Failed to flush temp file: {}", e)))?;
                    break;
                }
                Err(e) if retries < max_retries => {
                    retries += 1;
                    tracing::warn!(
                        file_id = %file_id,
                        retry = retries,
                        error = %e,
                        "Download failed, retrying"
                    );
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    let delay = std::time::Duration::from_secs(1 << retries);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    return Err(e);
                }
            }
        }

        // Verify SHA-256 by streaming from disk
        let mut downloaded = tokio::fs::File::open(&temp_path)
            .await
            .map_err(|e| SyncError::Storage(format!("Failed to reopen temp file: {}", e)))?;
        let mut hasher = Sha256::new();
        let mut total_bytes: i64 = 0;
        let mut buf = vec![0u8; 64 * 1024];

        loop {
            let n = downloaded
                .read(&mut buf)
                .await
                .map_err(|e| SyncError::Storage(format!("Failed to read downloaded file: {}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            total_bytes += n as i64;
        }

        let actual_sha256 = format!("{:x}", hasher.finalize());

        if actual_sha256 != expected_sha256 {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(SyncError::Storage(format!(
                "SHA-256 mismatch: expected {}, got {}",
                expected_sha256, actual_sha256
            )));
        }

        tokio::fs::rename(&temp_path, dest_path)
            .await
            .map_err(|e| SyncError::Storage(format!("Failed to finalize downloaded file: {}", e)))?;

        tracing::info!(
            file_id = %file_id,
            size = total_bytes,
            "File download completed"
        );

        Ok(())
    }

    /// List remote files.
    pub async fn list_remote_files(
        &self,
        auth_token: &str,
    ) -> Result<Vec<RemoteFileInfo>, SyncError> {
        self.transport.list_remote_files(auth_token).await
    }
}

fn temporary_download_path(dest_path: &Path) -> PathBuf {
    let file_name = dest_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    dest_path.with_file_name(format!("{}.part", file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::Mutex;

    // ─── Mock File Transport ─────────────────────────────────────────────

    struct MockFileTransport {
        upload_init_result: Mutex<Result<UploadSession, SyncError>>,
        upload_chunk_results: Mutex<Vec<Result<i64, SyncError>>>,
        upload_complete_result: Mutex<Result<String, SyncError>>,
        download_result: Mutex<Result<Vec<u8>, SyncError>>,
        list_result: Mutex<Result<Vec<RemoteFileInfo>, SyncError>>,
        uploaded_chunks: Mutex<Vec<(String, i64, Vec<u8>)>>,
    }

    impl MockFileTransport {
        fn success_for_data(data: &[u8], chunk_size: i64) -> Self {
            let total_chunks = ((data.len() as i64 + chunk_size - 1) / chunk_size).max(1);
            let chunk_results: Vec<Result<i64, SyncError>> = data
                .chunks(chunk_size as usize)
                .map(|c| Ok(c.len() as i64))
                .collect();

            Self {
                upload_init_result: Mutex::new(Ok(UploadSession {
                    upload_id: "upload-123".into(),
                    chunk_size,
                    total_chunks,
                })),
                upload_chunk_results: Mutex::new(chunk_results),
                upload_complete_result: Mutex::new(Ok("file-abc".into())),
                download_result: Mutex::new(Ok(data.to_vec())),
                list_result: Mutex::new(Ok(vec![])),
                uploaded_chunks: Mutex::new(Vec::new()),
            }
        }
    }

    impl FileTransportAdapter for MockFileTransport {
        async fn upload_init(
            &self,
            _file_name: &str,
            _file_size: i64,
            _sha256: &str,
            _auth_token: &str,
        ) -> Result<UploadSession, SyncError> {
            let result = self.upload_init_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(SyncError::Transport(format!("{}", e))),
            }
        }

        async fn upload_chunk(
            &self,
            upload_id: &str,
            chunk_index: i64,
            data: &[u8],
            _auth_token: &str,
        ) -> Result<i64, SyncError> {
            self.uploaded_chunks.lock().unwrap().push((
                upload_id.to_string(),
                chunk_index,
                data.to_vec(),
            ));
            let mut results = self.upload_chunk_results.lock().unwrap();
            if results.is_empty() {
                Ok(data.len() as i64)
            } else {
                let r = results.remove(0);
                match r {
                    Ok(v) => Ok(v),
                    Err(e) => Err(SyncError::Transport(format!("{}", e))),
                }
            }
        }

        async fn upload_complete(
            &self,
            _upload_id: &str,
            _auth_token: &str,
        ) -> Result<String, SyncError> {
            let result = self.upload_complete_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(SyncError::Transport(format!("{}", e))),
            }
        }

        async fn download_file(
            &self,
            _file_id: &str,
            _auth_token: &str,
            writer: &mut (dyn AsyncWrite + Unpin + Send),
        ) -> Result<(), SyncError> {
            let data = {
                let result = self.download_result.lock().unwrap();
                match &*result {
                    Ok(v) => v.clone(),
                    Err(e) => return Err(SyncError::Transport(format!("{}", e))),
                }
            };

            writer
                .write_all(&data)
                .await
                .map_err(|e| SyncError::Storage(format!("Failed to write mock download: {}", e)))?;
            writer
                .flush()
                .await
                .map_err(|e| SyncError::Storage(format!("Failed to flush mock download: {}", e)))?;
            Ok(())
        }

        async fn list_remote_files(
            &self,
            _auth_token: &str,
        ) -> Result<Vec<RemoteFileInfo>, SyncError> {
            let result = self.list_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(SyncError::Transport(format!("{}", e))),
            }
        }
    }

    // ─── FileProgress Tests ──────────────────────────────────────────────

    #[test]
    fn test_file_progress_percentage() {
        let p = FileProgress {
            file_name: "test.epub".into(),
            current_chunk: 5,
            total_chunks: 10,
            bytes_transferred: 500,
            total_bytes: 1000,
            is_upload: true,
        };
        assert!((p.percentage() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_file_progress_percentage_zero_total() {
        let p = FileProgress {
            file_name: "empty.epub".into(),
            current_chunk: 0,
            total_chunks: 0,
            bytes_transferred: 0,
            total_bytes: 0,
            is_upload: true,
        };
        assert!((p.percentage() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_file_progress_percentage_complete() {
        let p = FileProgress {
            file_name: "done.epub".into(),
            current_chunk: 10,
            total_chunks: 10,
            bytes_transferred: 1000,
            total_bytes: 1000,
            is_upload: false,
        };
        assert!((p.percentage() - 100.0).abs() < f64::EPSILON);
    }

    // ─── Upload Tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_upload_file_success() {
        let test_data = b"Hello, SubReader!";
        let tmp_dir = std::env::temp_dir().join("subreader_test_upload");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let file_path = tmp_dir.join("test_book.epub");
        std::fs::write(&file_path, test_data).unwrap();

        let transport = MockFileTransport::success_for_data(test_data, 1024);
        let engine = FileSyncEngine::new(transport);

        let result = engine.upload_file(&file_path, "token").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "file-abc");

        // Verify chunks were uploaded
        let chunks = engine.transport.uploaded_chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1); // Small file = 1 chunk
        assert_eq!(chunks[0].0, "upload-123");
        assert_eq!(chunks[0].1, 0);
        assert_eq!(chunks[0].2, test_data.to_vec());

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_upload_file_with_progress_callback() {
        let test_data = vec![0u8; 2048]; // 2KB
        let tmp_dir = std::env::temp_dir().join("subreader_test_upload_progress");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let file_path = tmp_dir.join("big_book.epub");
        std::fs::write(&file_path, &test_data).unwrap();

        let transport = MockFileTransport::success_for_data(&test_data, 1024);
        let progress_reports = std::sync::Arc::new(Mutex::new(Vec::new()));
        let reports_clone = progress_reports.clone();

        let mut engine = FileSyncEngine::new(transport);
        engine.set_progress_callback(Box::new(move |p| {
            reports_clone.lock().unwrap().push(p);
        }));

        let result = engine.upload_file(&file_path, "token").await;
        assert!(result.is_ok());

        let reports = progress_reports.lock().unwrap();
        assert_eq!(reports.len(), 2); // 2KB / 1024 = 2 chunks
        assert!(reports[0].is_upload);
        assert_eq!(reports[0].current_chunk, 1);
        assert_eq!(reports[1].current_chunk, 2);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_upload_file_not_found() {
        let transport = MockFileTransport::success_for_data(b"", 1024);
        let engine = FileSyncEngine::new(transport);

        let result = engine
            .upload_file(Path::new("/nonexistent/file.epub"), "token")
            .await;
        assert!(result.is_err());
    }

    // ─── Download Tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_download_file_success() {
        let test_data = b"Downloaded book content";
        let mut hasher = Sha256::new();
        hasher.update(test_data);
        let expected_sha = format!("{:x}", hasher.finalize());

        let transport = MockFileTransport::success_for_data(test_data, 1024);
        let engine = FileSyncEngine::new(transport);

        let tmp_dir = std::env::temp_dir().join("subreader_test_download");
        let dest_path = tmp_dir.join("downloaded.epub");

        let result = engine
            .download_file("file-1", &expected_sha, &dest_path, "token")
            .await;
        assert!(result.is_ok());

        // Verify file was written
        let written = std::fs::read(&dest_path).unwrap();
        assert_eq!(written, test_data);
        assert!(!dest_path.with_file_name("downloaded.epub.part").exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_download_file_sha256_mismatch() {
        let test_data = b"Some content";
        let transport = MockFileTransport::success_for_data(test_data, 1024);
        let engine = FileSyncEngine::new(transport);

        let tmp_dir = std::env::temp_dir().join("subreader_test_download_bad_sha");
        let dest_path = tmp_dir.join("bad.epub");

        let result = engine
            .download_file("file-1", "wrong_sha256_hash", &dest_path, "token")
            .await;
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("SHA-256 mismatch"));
        assert!(!dest_path.exists());
        assert!(!dest_path.with_file_name("bad.epub.part").exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    // ─── List Remote Files Tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_list_remote_files() {
        let transport = MockFileTransport::success_for_data(b"", 1024);
        *transport.list_result.lock().unwrap() = Ok(vec![
            RemoteFileInfo {
                file_id: "f1".into(),
                file_name: "book1.epub".into(),
                file_size: 1024,
                sha256: "abc123".into(),
            },
            RemoteFileInfo {
                file_id: "f2".into(),
                file_name: "book2.epub".into(),
                file_size: 2048,
                sha256: "def456".into(),
            },
        ]);

        let engine = FileSyncEngine::new(transport);
        let files = engine.list_remote_files("token").await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_name, "book1.epub");
        assert_eq!(files[1].file_name, "book2.epub");
    }
}
