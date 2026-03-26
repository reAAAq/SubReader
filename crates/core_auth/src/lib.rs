//! Core auth module — stub for P0.
//!
//! Defines the `AuthProvider` trait interface for authentication.
//! Full implementation is deferred to a later phase.

use serde::{Deserialize, Serialize};

/// Authentication token returned after successful login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthToken {
    /// The access token string.
    pub access_token: String,
    /// The refresh token string.
    pub refresh_token: Option<String>,
    /// Token expiry time in seconds since epoch.
    pub expires_at: u64,
}

/// Authentication error types.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Token expired")]
    TokenExpired,
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Unknown auth error: {0}")]
    Unknown(String),
}

/// Trait defining the authentication provider interface.
///
/// Implementations will handle specific auth backends (e.g., JWT, OAuth).
pub trait AuthProvider {
    /// Authenticate with username and password.
    fn login(&self, username: &str, password: &str) -> Result<AuthToken, AuthError>;

    /// Refresh an expired access token.
    fn refresh_token(&self, refresh_token: &str) -> Result<AuthToken, AuthError>;

    /// Validate whether a token is still valid.
    fn validate_token(&self, token: &str) -> Result<bool, AuthError>;

    /// Logout and invalidate the token.
    fn logout(&self, token: &str) -> Result<(), AuthError>;
}
