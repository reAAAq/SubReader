//! Core authentication module.
//!
//! Provides async authentication traits and HTTP-based implementation
//! for the SubReader cloud sync service.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod http_auth;
pub mod token_store;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Authentication token pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthToken {
    /// JWT access token.
    pub access_token: String,
    /// Refresh token for obtaining new access tokens.
    pub refresh_token: String,
    /// Access token expiry duration in seconds.
    pub expires_in: u64,
    /// User ID.
    pub user_id: String,
}

/// Registration request data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

/// Login request data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub credential: String,
    pub password: String,
    pub device_id: String,
    pub device_name: Option<String>,
    pub platform: Option<String>,
}

/// Device information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub last_active_at: String,
    pub created_at: String,
}

/// Authentication state.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthState {
    /// Not logged in.
    LoggedOut,
    /// Logged in with valid tokens.
    Authenticated { user_id: String },
    /// Tokens expired, needs refresh.
    NeedsRefresh,
    /// Refresh failed, needs re-login.
    NeedsReLogin,
}

// ─── Errors ─────────────────────────────────────────────────────────────────

/// Authentication error types.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Token expired")]
    TokenExpired,

    #[error("Registration failed: {0}")]
    RegistrationFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Server error: {status_code} - {message}")]
    ServerError { status_code: u16, message: String },

    #[error("Token storage error: {0}")]
    StorageError(String),

    #[error("Not authenticated")]
    NotAuthenticated,

    #[error("Unknown auth error: {0}")]
    Unknown(String),
}

// ─── Traits ─────────────────────────────────────────────────────────────────

/// Async authentication provider trait.
#[allow(async_fn_in_trait)]
pub trait AuthProvider: Send + Sync {
    /// Register a new user account.
    async fn register(&self, req: &RegisterRequest) -> Result<String, AuthError>;

    /// Authenticate with credentials and obtain tokens.
    async fn login(&self, req: &LoginRequest) -> Result<AuthToken, AuthError>;

    /// Refresh an expired access token.
    async fn refresh_token(
        &self,
        refresh_token: &str,
        device_id: &str,
    ) -> Result<AuthToken, AuthError>;

    /// Logout and invalidate the current session.
    async fn logout(&self, access_token: &str) -> Result<(), AuthError>;

    /// Change password.
    async fn change_password(
        &self,
        access_token: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), AuthError>;

    /// List devices.
    async fn list_devices(&self, access_token: &str) -> Result<Vec<DeviceInfo>, AuthError>;

    /// Remove a device.
    async fn remove_device(
        &self,
        access_token: &str,
        device_id: &str,
    ) -> Result<(), AuthError>;
}

/// Token storage trait for persisting auth tokens locally.
pub trait TokenStore: Send + Sync {
    /// Save token pair.
    fn save_token(&self, token: &AuthToken) -> Result<(), AuthError>;

    /// Load saved token pair.
    fn load_token(&self) -> Result<Option<AuthToken>, AuthError>;

    /// Clear saved tokens.
    fn clear_token(&self) -> Result<(), AuthError>;
}

// ─── Auth Manager ───────────────────────────────────────────────────────────

/// High-level authentication manager that coordinates auth provider and token storage.
pub struct AuthManager<P: AuthProvider, S: TokenStore> {
    provider: P,
    store: S,
    state: Arc<RwLock<AuthState>>,
    current_token: Arc<RwLock<Option<AuthToken>>>,
    device_id: String,
}

impl<P: AuthProvider, S: TokenStore> AuthManager<P, S> {
    /// Create a new AuthManager.
    pub fn new(
        provider: P,
        store: S,
        device_id: String,
    ) -> Self {
        // Try to load existing token
        let existing_token = store.load_token().ok().flatten();
        let initial_state = if existing_token.is_some() {
            // We have a token, but we don't know if it's still valid
            AuthState::NeedsRefresh
        } else {
            AuthState::LoggedOut
        };

        Self {
            provider,
            store,
            state: Arc::new(RwLock::new(initial_state)),
            current_token: Arc::new(RwLock::new(existing_token)),
            device_id,
        }
    }

    /// Get current authentication state.
    pub async fn state(&self) -> AuthState {
        self.state.read().await.clone()
    }

    /// Get current access token if available.
    pub async fn access_token(&self) -> Option<String> {
        self.current_token
            .read()
            .await
            .as_ref()
            .map(|t| t.access_token.clone())
    }

    /// Get current user ID if authenticated.
    pub async fn user_id(&self) -> Option<String> {
        self.current_token
            .read()
            .await
            .as_ref()
            .map(|t| t.user_id.clone())
    }

    /// Register a new account.
    pub async fn register(&self, req: &RegisterRequest) -> Result<String, AuthError> {
        self.provider.register(req).await
    }

    /// Login with credentials.
    ///
    /// `device_name` and `platform` are optional metadata for the device list.
    pub async fn login(
        &self,
        credential: &str,
        password: &str,
        device_name: Option<&str>,
        platform: Option<&str>,
    ) -> Result<AuthToken, AuthError> {
        let req = LoginRequest {
            credential: credential.to_string(),
            password: password.to_string(),
            device_id: self.device_id.clone(),
            device_name: device_name.map(|s| s.to_string()),
            platform: platform.map(|s| s.to_string()),
        };

        let token = self.provider.login(&req).await?;

        // Save token
        self.store.save_token(&token)?;

        let user_id = token.user_id.clone();
        *self.current_token.write().await = Some(token.clone());
        *self.state.write().await = AuthState::Authenticated { user_id };

        tracing::info!("Login successful");
        Ok(token)
    }

    /// Attempt to refresh the current token.
    pub async fn refresh(&self) -> Result<AuthToken, AuthError> {
        let current = self.current_token.read().await.clone();
        let current = current.ok_or(AuthError::NotAuthenticated)?;

        match self
            .provider
            .refresh_token(&current.refresh_token, &self.device_id)
            .await
        {
            Ok(new_token) => {
                // Preserve existing user_id if the server didn't return one
                let user_id = if new_token.user_id.is_empty() {
                    current.user_id.clone()
                } else {
                    new_token.user_id.clone()
                };

                let token_to_save = AuthToken {
                    access_token: new_token.access_token.clone(),
                    refresh_token: new_token.refresh_token.clone(),
                    expires_in: new_token.expires_in,
                    user_id: user_id.clone(),
                };

                self.store.save_token(&token_to_save)?;
                *self.current_token.write().await = Some(token_to_save.clone());
                *self.state.write().await = AuthState::Authenticated { user_id };        
                tracing::info!("Token refresh successful");
                Ok(token_to_save)
            }
            Err(e) => {
                tracing::warn!("Token refresh failed: {}", e);

                match &e {
                    AuthError::InvalidCredentials
                    | AuthError::TokenExpired
                    | AuthError::NotAuthenticated => {
                        if let Err(clear_err) = self.store.clear_token() {
                            tracing::error!("Failed to clear persisted token after refresh failure: {}", clear_err);
                        }
                        *self.current_token.write().await = None;
                        *self.state.write().await = AuthState::NeedsReLogin;
                    }
                    AuthError::NetworkError(_)
                    | AuthError::ServerError { .. }
                    | AuthError::StorageError(_)
                    | AuthError::RegistrationFailed(_)
                    | AuthError::Unknown(_) => {
                        *self.state.write().await = AuthState::NeedsRefresh;
                    }
                }

                Err(e)
            }
        }
    }

    /// Get a valid access token, auto-refreshing if needed.
    pub async fn get_valid_token(&self) -> Result<String, AuthError> {
        let state = self.state.read().await.clone();
        match state {
            AuthState::Authenticated { .. } => {
                // Return current token
                self.access_token()
                    .await
                    .ok_or(AuthError::NotAuthenticated)
            }
            AuthState::NeedsRefresh => {
                // Try to refresh
                let token = self.refresh().await?;
                Ok(token.access_token)
            }
            AuthState::LoggedOut | AuthState::NeedsReLogin => Err(AuthError::NotAuthenticated),
        }
    }

    /// Logout.
    pub async fn logout(&self) -> Result<(), AuthError> {
        if let Some(token) = self.access_token().await {
            // Best effort logout on server
            let _ = self.provider.logout(&token).await;
        }

        self.store.clear_token()?;
        *self.current_token.write().await = None;
        *self.state.write().await = AuthState::LoggedOut;

        tracing::info!("Logged out");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ─── Mock Auth Provider ──────────────────────────────────────────────

    struct MockAuthProvider {
        register_result: Mutex<Result<String, AuthError>>,
        login_result: Mutex<Result<AuthToken, AuthError>>,
        refresh_result: Mutex<Result<AuthToken, AuthError>>,
        logout_called: Mutex<bool>,
    }

    impl MockAuthProvider {
        fn success() -> Self {
            Self {
                register_result: Mutex::new(Ok("user-123".to_string())),
                login_result: Mutex::new(Ok(AuthToken {
                    access_token: "access-token".to_string(),
                    refresh_token: "refresh-token".to_string(),
                    expires_in: 3600,
                    user_id: "user-123".to_string(),
                })),
                refresh_result: Mutex::new(Ok(AuthToken {
                    access_token: "new-access-token".to_string(),
                    refresh_token: "new-refresh-token".to_string(),
                    expires_in: 3600,
                    user_id: "user-123".to_string(),
                })),
                logout_called: Mutex::new(false),
            }
        }

        fn with_login_error(err: AuthError) -> Self {
            let p = Self::success();
            *p.login_result.lock().unwrap() = Err(err);
            p
        }

        fn with_refresh_error(err: AuthError) -> Self {
            let p = Self::success();
            *p.refresh_result.lock().unwrap() = Err(err);
            p
        }
    }

    impl AuthProvider for MockAuthProvider {
        async fn register(&self, _req: &RegisterRequest) -> Result<String, AuthError> {
            let result = self.register_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(_) => Err(AuthError::RegistrationFailed("mock error".into())),
            }
        }

        async fn login(&self, _req: &LoginRequest) -> Result<AuthToken, AuthError> {
            let result = self.login_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(err) => Err(err.clone()),
            }
        }

        async fn refresh_token(
            &self,
            _refresh_token: &str,
            _device_id: &str,
        ) -> Result<AuthToken, AuthError> {
            let result = self.refresh_result.lock().unwrap();
            match &*result {
                Ok(v) => Ok(v.clone()),
                Err(err) => Err(err.clone()),
            }
        }

        async fn logout(&self, _access_token: &str) -> Result<(), AuthError> {
            *self.logout_called.lock().unwrap() = true;
            Ok(())
        }

        async fn change_password(
            &self,
            _access_token: &str,
            _old_password: &str,
            _new_password: &str,
        ) -> Result<(), AuthError> {
            Ok(())
        }

        async fn list_devices(&self, _access_token: &str) -> Result<Vec<DeviceInfo>, AuthError> {
            Ok(vec![])
        }

        async fn remove_device(
            &self,
            _access_token: &str,
            _device_id: &str,
        ) -> Result<(), AuthError> {
            Ok(())
        }
    }

    // ─── Mock Token Store ────────────────────────────────────────────────

    struct MockTokenStore {
        token: Mutex<Option<AuthToken>>,
    }

    impl MockTokenStore {
        fn new() -> Self {
            Self {
                token: Mutex::new(None),
            }
        }
    }

    impl TokenStore for MockTokenStore {
        fn save_token(&self, token: &AuthToken) -> Result<(), AuthError> {
            *self.token.lock().unwrap() = Some(token.clone());
            Ok(())
        }

        fn load_token(&self) -> Result<Option<AuthToken>, AuthError> {
            Ok(self.token.lock().unwrap().clone())
        }

        fn clear_token(&self) -> Result<(), AuthError> {
            *self.token.lock().unwrap() = None;
            Ok(())
        }
    }

    // ─── AuthState Tests ─────────────────────────────────────────────────

    #[test]
    fn test_auth_state_equality() {
        assert_eq!(AuthState::LoggedOut, AuthState::LoggedOut);
        assert_eq!(AuthState::NeedsRefresh, AuthState::NeedsRefresh);
        assert_eq!(AuthState::NeedsReLogin, AuthState::NeedsReLogin);
        assert_eq!(
            AuthState::Authenticated {
                user_id: "u1".into()
            },
            AuthState::Authenticated {
                user_id: "u1".into()
            }
        );
        assert_ne!(AuthState::LoggedOut, AuthState::NeedsRefresh);
    }

    // ─── AuthManager Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_auth_manager_initial_state_logged_out() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        assert_eq!(manager.state().await, AuthState::LoggedOut);
        assert!(manager.access_token().await.is_none());
        assert!(manager.user_id().await.is_none());
    }

    #[tokio::test]
    async fn test_auth_manager_login_success() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        let token = manager.login("user@test.com", "password", None, None).await.unwrap();
        assert_eq!(token.access_token, "access-token");
        assert_eq!(token.user_id, "user-123");

        assert_eq!(
            manager.state().await,
            AuthState::Authenticated {
                user_id: "user-123".into()
            }
        );
        assert_eq!(
            manager.access_token().await,
            Some("access-token".to_string())
        );
        assert_eq!(manager.user_id().await, Some("user-123".to_string()));
    }

    #[tokio::test]
    async fn test_auth_manager_login_saves_token() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        manager.login("user@test.com", "password", None, None).await.unwrap();

        // Verify token was saved to store
        let saved = manager.store.load_token().unwrap().unwrap();
        assert_eq!(saved.access_token, "access-token");
    }

    #[tokio::test]
    async fn test_auth_manager_login_failure() {
        let provider = MockAuthProvider::with_login_error(AuthError::InvalidCredentials);
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        let result = manager.login("user@test.com", "wrong", None, None).await;
        assert!(result.is_err());
        assert_eq!(manager.state().await, AuthState::LoggedOut);
    }

    #[tokio::test]
    async fn test_auth_manager_logout() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        manager.login("user@test.com", "password", None, None).await.unwrap();
        manager.logout().await.unwrap();

        assert_eq!(manager.state().await, AuthState::LoggedOut);
        assert!(manager.access_token().await.is_none());
        assert!(manager.store.load_token().unwrap().is_none());
        assert!(*manager.provider.logout_called.lock().unwrap());
    }

    #[tokio::test]
    async fn test_auth_manager_refresh_success() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        // Login first
        manager.login("user@test.com", "password", None, None).await.unwrap();

        // Refresh
        let new_token = manager.refresh().await.unwrap();
        assert_eq!(new_token.access_token, "new-access-token");
        assert_eq!(
            manager.access_token().await,
            Some("new-access-token".to_string())
        );
    }

    #[tokio::test]
    async fn test_auth_manager_refresh_failure_sets_needs_relogin() {
        let provider = MockAuthProvider::with_refresh_error(AuthError::TokenExpired);
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        // Login first (uses success login result)
        // Need to set login to succeed first
        *manager.provider.login_result.lock().unwrap() = Ok(AuthToken {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_in: 3600,
            user_id: "user-1".into(),
        });
        manager.login("user@test.com", "password", None, None).await.unwrap();

        // Refresh should fail
        let result = manager.refresh().await;
        assert!(result.is_err());
        assert_eq!(manager.state().await, AuthState::NeedsReLogin);
        assert!(manager.access_token().await.is_none());
        assert!(manager.user_id().await.is_none());
        assert!(manager.store.load_token().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_auth_manager_refresh_network_error_preserves_token() {
        let provider = MockAuthProvider::with_refresh_error(AuthError::NetworkError("timeout".into()));
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        *manager.provider.login_result.lock().unwrap() = Ok(AuthToken {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_in: 3600,
            user_id: "user-1".into(),
        });
        manager.login("user@test.com", "password", None, None).await.unwrap();

        let result = manager.refresh().await;
        assert!(matches!(result, Err(AuthError::NetworkError(_))));
        assert_eq!(manager.state().await, AuthState::NeedsRefresh);
        assert_eq!(manager.access_token().await, Some("access".to_string()));
        assert_eq!(manager.user_id().await, Some("user-1".to_string()));
        assert!(manager.store.load_token().unwrap().is_some());
    }

    #[tokio::test]
    async fn test_auth_manager_get_valid_token_when_authenticated() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        manager.login("user@test.com", "password", None, None).await.unwrap();
        let token = manager.get_valid_token().await.unwrap();
        assert_eq!(token, "access-token");
    }

    #[tokio::test]
    async fn test_auth_manager_get_valid_token_when_logged_out() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        let result = manager.get_valid_token().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_auth_manager_register() {
        let provider = MockAuthProvider::success();
        let store = MockTokenStore::new();
        let manager = AuthManager::new(provider, store, "device-1".into());

        let req = RegisterRequest {
            username: "testuser".into(),
            email: "test@test.com".into(),
            password: "password123".into(),
        };
        let user_id = manager.register(&req).await.unwrap();
        assert_eq!(user_id, "user-123");
    }

    // ─── AuthToken Serialization Tests ───────────────────────────────────

    #[test]
    fn test_auth_token_serialization() {
        let token = AuthToken {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_in: 3600,
            user_id: "uid".into(),
        };
        let json = serde_json::to_string(&token).unwrap();
        let deserialized: AuthToken = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.access_token, "at");
        assert_eq!(deserialized.refresh_token, "rt");
        assert_eq!(deserialized.expires_in, 3600);
        assert_eq!(deserialized.user_id, "uid");
    }

    // ─── AuthError Tests ─────────────────────────────────────────────────

    #[test]
    fn test_auth_error_display() {
        let err = AuthError::InvalidCredentials;
        assert_eq!(format!("{}", err), "Invalid credentials");

        let err = AuthError::TokenExpired;
        assert_eq!(format!("{}", err), "Token expired");

        let err = AuthError::NetworkError("timeout".into());
        assert_eq!(format!("{}", err), "Network error: timeout");

        let err = AuthError::ServerError {
            status_code: 500,
            message: "internal".into(),
        };
        assert_eq!(format!("{}", err), "Server error: 500 - internal");
    }
}
