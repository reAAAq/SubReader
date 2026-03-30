//! HTTP-based transport implementation using reqwest.

use reqwest::Client;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::{
    FileTransport, PushOperation, RemoteFileInfo, SyncPullResponse, SyncPushResponse,
    SyncTransport, TransportError, UploadChunkResponse, UploadCompleteResponse,
    UploadInitResponse,
};

/// HTTP transport that communicates with the backend API.
pub struct HttpTransport {
    client: Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[allow(dead_code)]
    error: String,
    message: String,
}

#[derive(Debug, serde::Serialize)]
struct PushBody {
    operations: Vec<PushOperation>,
}

#[derive(Debug, serde::Serialize)]
struct UploadInitBody {
    file_name: String,
    file_size: i64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct FilesListResponse {
    files: Vec<RemoteFileInfo>,
}

impl HttpTransport {
    /// Create a new HTTP transport.
    ///
    /// # Arguments
    /// * `base_url` - Backend API base URL (e.g., "http://localhost:8080")
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Parse an error response from the server.
    async fn parse_error(response: reqwest::Response) -> TransportError {
        let status = response.status().as_u16();
        let message = response
            .json::<ErrorBody>()
            .await
            .map(|e| e.message)
            .unwrap_or_else(|_| "Unknown error".to_string());

        match status {
            401 => TransportError::Unauthorized,
            408 => TransportError::Timeout,
            _ => TransportError::ServerError {
                status_code: status,
                message,
            },
        }
    }
}

impl SyncTransport for HttpTransport {
    async fn push_operations(
        &self,
        operations: &[PushOperation],
        auth_token: &str,
    ) -> Result<SyncPushResponse, TransportError> {
        let url = format!("{}/sync/push", self.base_url);

        let body = PushBody {
            operations: operations.to_vec(),
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(auth_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    TransportError::Timeout
                } else {
                    TransportError::ConnectionFailed(e.to_string())
                }
            })?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        response
            .json()
            .await
            .map_err(|e| TransportError::DeserializationError(e.to_string()))
    }

    async fn pull_operations(
        &self,
        cursor: i64,
        limit: i64,
        auth_token: &str,
    ) -> Result<SyncPullResponse, TransportError> {
        let url = format!(
            "{}/sync/pull?cursor={}&limit={}",
            self.base_url, cursor, limit
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(auth_token)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    TransportError::Timeout
                } else {
                    TransportError::ConnectionFailed(e.to_string())
                }
            })?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        response
            .json()
            .await
            .map_err(|e| TransportError::DeserializationError(e.to_string()))
    }

    async fn health_check(&self) -> Result<bool, TransportError> {
        let url = format!("{}/health", self.base_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        Ok(response.status().is_success())
    }
}

impl FileTransport for HttpTransport {
    async fn upload_init(
        &self,
        file_name: &str,
        file_size: i64,
        sha256: &str,
        auth_token: &str,
    ) -> Result<UploadInitResponse, TransportError> {
        let url = format!("{}/files/upload/init", self.base_url);

        let body = UploadInitBody {
            file_name: file_name.to_string(),
            file_size,
            sha256: sha256.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(auth_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        response
            .json()
            .await
            .map_err(|e| TransportError::DeserializationError(e.to_string()))
    }

    async fn upload_chunk(
        &self,
        upload_id: &str,
        chunk_index: i64,
        data: &[u8],
        auth_token: &str,
    ) -> Result<UploadChunkResponse, TransportError> {
        let url = format!(
            "{}/files/upload/{}/chunk/{}",
            self.base_url, upload_id, chunk_index
        );

        let response = self
            .client
            .put(&url)
            .bearer_auth(auth_token)
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        response
            .json()
            .await
            .map_err(|e| TransportError::DeserializationError(e.to_string()))
    }

    async fn upload_complete(
        &self,
        upload_id: &str,
        auth_token: &str,
    ) -> Result<UploadCompleteResponse, TransportError> {
        let url = format!("{}/files/upload/{}/complete", self.base_url, upload_id);

        let response = self
            .client
            .post(&url)
            .bearer_auth(auth_token)
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        response
            .json()
            .await
            .map_err(|e| TransportError::DeserializationError(e.to_string()))
    }

    async fn download_file(
        &self,
        file_id: &str,
        auth_token: &str,
        writer: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> Result<(), TransportError> {
        let url = format!("{}/files/{}", self.base_url, file_id);

        let mut response = self
            .client
            .get(&url)
            .bearer_auth(auth_token)
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| TransportError::Unknown(format!("Failed to read response body: {}", e)))?
        {
            writer
                .write_all(&chunk)
                .await
                .map_err(|e| TransportError::Unknown(format!("Failed to write download chunk: {}", e)))?;
        }

        writer
            .flush()
            .await
            .map_err(|e| TransportError::Unknown(format!("Failed to flush download writer: {}", e)))?;

        Ok(())
    }

    async fn list_files(
        &self,
        auth_token: &str,
    ) -> Result<Vec<RemoteFileInfo>, TransportError> {
        let url = format!("{}/files", self.base_url);

        let response = self
            .client
            .get(&url)
            .bearer_auth(auth_token)
            .send()
            .await
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        let body: FilesListResponse = response
            .json()
            .await
            .map_err(|e| TransportError::DeserializationError(e.to_string()))?;

        Ok(body.files)
    }
}
