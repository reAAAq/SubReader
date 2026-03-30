//! HTTP-based authentication provider implementation.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{AuthError, AuthProvider, AuthToken, DeviceInfo, LoginRequest, RegisterRequest};

/// HTTP authentication provider that communicates with the backend API.
pub struct HttpAuthProvider {
    client: Client,
    base_url: String,
}

// ─── API response types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    user_id: String,
    #[allow(dead_code)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    user_id: String,
}

#[derive(Debug, Serialize)]
struct RefreshBody {
    refresh_token: String,
    device_id: String,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    user_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChangePasswordBody {
    old_password: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[allow(dead_code)]
    error: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct DevicesResponse {
    devices: Vec<DeviceInfo>,
}

// ─── Implementation ─────────────────────────────────────────────────────────

impl HttpAuthProvider {
    /// Create a new HTTP auth provider.
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

    fn classify_unauthorized_message(message: &str) -> AuthError {
        let normalized = message.trim().to_ascii_lowercase();

        if normalized.contains("device is no longer authorized") {
            return AuthError::NotAuthenticated;
        }

        if normalized.contains("invalid or expired refresh token") {
            return AuthError::TokenExpired;
        }

        if normalized.contains("invalid credentials") {
            return AuthError::InvalidCredentials;
        }

        AuthError::NotAuthenticated
    }

    /// Parse an error response from the server.
    async fn parse_error(response: reqwest::Response) -> AuthError {
        let status = response.status().as_u16();
        let message = response
            .json::<ErrorBody>()
            .await
            .map(|e| e.message)
            .unwrap_or_else(|_| "Unknown error".to_string());

        match status {
            401 => Self::classify_unauthorized_message(&message),
            409 => AuthError::RegistrationFailed(message),
            _ => AuthError::ServerError {
                status_code: status,
                message,
            },
        }
    }
}

impl AuthProvider for HttpAuthProvider {
    async fn register(&self, req: &RegisterRequest) -> Result<String, AuthError> {
        let url = format!("{}/auth/register", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        let body: RegisterResponse = response
            .json()
            .await
            .map_err(|e| AuthError::Unknown(format!("Failed to parse response: {}", e)))?;

        Ok(body.user_id)
    }

    async fn login(&self, req: &LoginRequest) -> Result<AuthToken, AuthError> {
        let url = format!("{}/auth/login", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        let body: LoginResponse = response
            .json()
            .await
            .map_err(|e| AuthError::Unknown(format!("Failed to parse response: {}", e)))?;

        Ok(AuthToken {
            access_token: body.access_token,
            refresh_token: body.refresh_token,
            expires_in: body.expires_in,
            user_id: body.user_id,
        })
    }

    async fn refresh_token(
        &self,
        refresh_token: &str,
        device_id: &str,
    ) -> Result<AuthToken, AuthError> {
        let url = format!("{}/auth/refresh", self.base_url);

        let body = RefreshBody {
            refresh_token: refresh_token.to_string(),
            device_id: device_id.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        let resp: RefreshResponse = response
            .json()
            .await
            .map_err(|e| AuthError::Unknown(format!("Failed to parse response: {}", e)))?;

        // Use user_id from server response if available, otherwise empty
        // The AuthManager will fill in the existing user_id if empty
        Ok(AuthToken {
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            expires_in: resp.expires_in,
            user_id: resp.user_id.unwrap_or_default(),
        })
    }

    async fn logout(&self, access_token: &str) -> Result<(), AuthError> {
        let url = format!("{}/auth/logout", self.base_url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        Ok(())
    }

    async fn change_password(
        &self,
        access_token: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        let url = format!("{}/auth/password", self.base_url);

        let body = ChangePasswordBody {
            old_password: old_password.to_string(),
            new_password: new_password.to_string(),
        };

        let response = self
            .client
            .put(&url)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        Ok(())
    }

    async fn list_devices(&self, access_token: &str) -> Result<Vec<DeviceInfo>, AuthError> {
        let url = format!("{}/auth/devices", self.base_url);

        let response = self
            .client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        let body: DevicesResponse = response
            .json()
            .await
            .map_err(|e| AuthError::Unknown(format!("Failed to parse response: {}", e)))?;

        Ok(body.devices)
    }

    async fn remove_device(
        &self,
        access_token: &str,
        device_id: &str,
    ) -> Result<(), AuthError> {
        let url = format!("{}/auth/devices/{}", self.base_url, device_id);

        let response = self
            .client
            .delete(&url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(Self::parse_error(response).await);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_unauthorized_invalid_credentials() {
        let err = HttpAuthProvider::classify_unauthorized_message("Invalid credentials");
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn test_classify_unauthorized_expired_refresh_token() {
        let err = HttpAuthProvider::classify_unauthorized_message("Invalid or expired refresh token");
        assert!(matches!(err, AuthError::TokenExpired));
    }

    #[test]
    fn test_classify_unauthorized_removed_device() {
        let err = HttpAuthProvider::classify_unauthorized_message("Device is no longer authorized");
        assert!(matches!(err, AuthError::NotAuthenticated));
    }

    #[test]
    fn test_classify_unauthorized_unknown_message_defaults_to_not_authenticated() {
        let err = HttpAuthProvider::classify_unauthorized_message("invalid token");
        assert!(matches!(err, AuthError::NotAuthenticated));
    }
}
