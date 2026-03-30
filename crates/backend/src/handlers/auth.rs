//! Authentication handlers: register, login, refresh, logout, password change,
//! account deletion, device management.

use axum::extract::{Extension, Path, State};
use axum::http::{Request, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::jwt;
use crate::middleware::AuthUser;
use crate::AppState;

// ─── Request / Response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Username or email.
    pub credential: String,
    pub password: String,
    /// Client-generated device ID.
    pub device_id: String,
    /// Human-readable device name (e.g., "iPhone 15").
    pub device_name: Option<String>,
    /// Platform (e.g., "ios", "macos", "android").
    pub platform: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
    pub device_id: String,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub last_active_at: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct DevicesResponse {
    pub devices: Vec<DeviceInfo>,
}

async fn revoke_all_user_device_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    now_ts: i64,
    revoked_reason: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE refresh_tokens
         SET revoked = 1,
             revoked_reason = CASE
                 WHEN revoked_reason = '' THEN ?
                 ELSE revoked_reason
             END
         WHERE user_id = ?",
    )
    .bind(revoked_reason)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE user_devices
         SET is_active = 0,
             token_valid_after = MAX(token_valid_after, ?)
         WHERE user_id = ?",
    )
    .bind(now_ts)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// POST /auth/register
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    // Validate username length
    if req.username.len() < 3 || req.username.len() > 32 {
        return Err(AppError::BadRequest(
            "Username must be between 3 and 32 characters".to_string(),
        ));
    }

    // Validate email format (basic check)
    if !req.email.contains('@') || !req.email.contains('.') {
        return Err(AppError::BadRequest("Invalid email format".to_string()));
    }

    // Validate password strength
    if req.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".to_string(),
        ));
    }

    // Check username uniqueness
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM users WHERE username = ? AND deleted_at IS NULL",
    )
    .bind(&req.username)
    .fetch_optional(&state.pool)
    .await?;

    if existing.is_some() {
        return Err(AppError::Conflict("Username already taken".to_string()));
    }

    // Check email uniqueness
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM users WHERE email = ? AND deleted_at IS NULL",
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await?;

    if existing.is_some() {
        return Err(AppError::Conflict("Email already registered".to_string()));
    }

    // Hash password with argon2
    let password_hash = hash_password(&req.password)?;

    let user_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
    )
    .bind(&user_id)
    .bind(&req.username)
    .bind(&req.email)
    .bind(&password_hash)
    .execute(&state.pool)
    .await
    .map_err(map_register_insert_error)?;

    tracing::info!(user_id = %user_id, username = %req.username, "User registered");

    Ok(Json(RegisterResponse {
        user_id,
        message: "Registration successful".to_string(),
    }))
}

/// POST /auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // Find user by username or email
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE (username = ? OR email = ?) AND deleted_at IS NULL",
    )
    .bind(&req.credential)
    .bind(&req.credential)
    .fetch_optional(&state.pool)
    .await?;

    let (user_id, password_hash) = row.ok_or_else(|| {
        AppError::Unauthorized("Invalid credentials".to_string())
    })?;

    // Verify password
    verify_password(&req.password, &password_hash)?;

    let issued_at_ms = chrono::Utc::now().timestamp_millis();

    // Generate refresh token; access token is generated after the device cutoff is finalized.
    let refresh_token = jwt::generate_refresh_token();
    let refresh_token_hash = jwt::hash_refresh_token(&refresh_token);

    let refresh_token_id = Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.refresh_token_expiry_secs as i64);
    let expires_at_str = expires_at.format("%Y-%m-%d %H:%M:%S").to_string();

    let device_name = req.device_name.unwrap_or_default();
    let platform = req.platform.unwrap_or_default();

    // Atomic transaction: store refresh token + upsert device
    let mut tx = state.pool.begin().await?;

    sqlx::query(
        "UPDATE refresh_tokens
         SET revoked = 1,
             revoked_reason = CASE
                 WHEN revoked_reason = '' THEN 'replaced'
                 ELSE revoked_reason
             END
         WHERE user_id = ? AND device_id = ? AND revoked = 0",
    )
    .bind(&user_id)
    .bind(&req.device_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO refresh_tokens (id, user_id, token_hash, device_id, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&refresh_token_id)
    .bind(&user_id)
    .bind(&refresh_token_hash)
    .bind(&req.device_id)
    .bind(&expires_at_str)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO user_devices (user_id, device_id, device_name, platform, token_valid_after, is_active)
         VALUES (?, ?, ?, ?, ?, 1)
         ON CONFLICT(user_id, device_id) DO UPDATE SET
            device_name = excluded.device_name,
            platform = excluded.platform,
            is_active = 1,
            token_valid_after = MAX(user_devices.token_valid_after, excluded.token_valid_after),
            last_active_at = datetime('now')",
    )
    .bind(&user_id)
    .bind(&req.device_id)
    .bind(&device_name)
    .bind(&platform)
    .bind(issued_at_ms)
    .execute(&mut *tx)
    .await?;

    let effective_issued_at_ms: i64 = sqlx::query_scalar(
        "SELECT token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
    )
    .bind(&user_id)
    .bind(&req.device_id)
    .fetch_one(&mut *tx)
    .await?;

    let access_token = jwt::generate_access_token_at(
        &state.config,
        &user_id,
        &req.device_id,
        effective_issued_at_ms,
    )
    .map_err(|e| AppError::Internal(format!("Token generation failed: {}", e)))?;

    tx.commit().await?;

    tracing::info!(user_id = %user_id, device_id = %req.device_id, "User logged in");

    Ok(Json(LoginResponse {
        access_token,
        refresh_token,
        expires_in: state.config.access_token_expiry_secs,
        user_id,
    }))
}

/// POST /auth/refresh
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
    let token_hash = jwt::hash_refresh_token(&req.refresh_token);

    let new_refresh_token = jwt::generate_refresh_token();
    let new_refresh_hash = jwt::hash_refresh_token(&new_refresh_token);
    let new_token_id = Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.refresh_token_expiry_secs as i64);
    let expires_at_str = expires_at.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut tx = state.pool.begin().await?;

    let row: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM refresh_tokens
          WHERE token_hash = ? AND device_id = ? AND revoked = 0 AND expires_at > datetime('now')",
    )
    .bind(&token_hash)
    .bind(&req.device_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (user_id,) = match row {
        Some(r) => r,
        None => {
            let revoked: Option<(String, String)> = sqlx::query_as(
                "SELECT user_id, revoked_reason FROM refresh_tokens WHERE token_hash = ? AND revoked = 1",
            )
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some((compromised_user_id, revoked_reason)) = revoked {
                if revoked_reason == "rotated" {
                    let now_ts = chrono::Utc::now().timestamp_millis();

                    sqlx::query(
                        "UPDATE users
                         SET token_valid_after = MAX(token_valid_after, ?),
                             updated_at = datetime('now')
                         WHERE id = ?",
                    )
                    .bind(now_ts)
                    .bind(&compromised_user_id)
                    .execute(&mut *tx)
                    .await?;

                    revoke_all_user_device_sessions(
                        &mut tx,
                        &compromised_user_id,
                        now_ts,
                        "compromised",
                    )
                    .await?;

                    tx.commit().await?;

                    tracing::warn!(
                        user_id = %compromised_user_id,
                        device_id = %req.device_id,
                        "Refresh token reuse detected — all tokens revoked for user"
                    );
                } else {
                    tx.rollback().await?;
                }
            } else {
                tx.rollback().await?;
            }

            return Err(AppError::Unauthorized(
                "Invalid or expired refresh token".to_string(),
            ));
        }
    };

    let consume_result = sqlx::query(
        "UPDATE refresh_tokens
         SET revoked = 1,
             revoked_reason = 'rotated'
         WHERE token_hash = ? AND device_id = ? AND revoked = 0 AND user_id = ?",
    )
    .bind(&token_hash)
    .bind(&req.device_id)
    .bind(&user_id)
    .execute(&mut *tx)
    .await?;

    if consume_result.rows_affected() != 1 {
        let now_ts = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE users
             SET token_valid_after = MAX(token_valid_after, ?),
                 updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(now_ts)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;

        revoke_all_user_device_sessions(&mut tx, &user_id, now_ts, "compromised").await?;

        tx.commit().await?;

        tracing::warn!(
            user_id = %user_id,
            device_id = %req.device_id,
            "Refresh token double-spend detected — token family revoked"
        );

        return Err(AppError::Unauthorized(
            "Invalid or expired refresh token".to_string(),
        ));
    }

    let device_row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT token_valid_after, is_active FROM user_devices WHERE user_id = ? AND device_id = ?",
    )
    .bind(&user_id)
    .bind(&req.device_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (device_token_valid_after, is_active) = match device_row {
        Some(row) => row,
        None => {
            tx.rollback().await?;
            return Err(AppError::Unauthorized(
                "Device is no longer authorized".to_string(),
            ));
        }
    };

    if is_active == 0 {
        tx.rollback().await?;
        return Err(AppError::Unauthorized(
            "Device is no longer authorized".to_string(),
        ));
    }

    let access_token = jwt::generate_access_token(&state.config, &user_id, &req.device_id)
        .map_err(|e| AppError::Internal(format!("Token generation failed: {}", e)))?;

    let access_claims = jsonwebtoken::decode::<crate::middleware::Claims>(
        &access_token,
        &jsonwebtoken::DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .map_err(|e| AppError::Internal(format!("Token validation failed: {}", e)))?
    .claims;

    if access_claims.iat < device_token_valid_after as u64 {
        tx.rollback().await?;
        return Err(AppError::Unauthorized(
            "Device is no longer authorized".to_string(),
        ));
    }

    sqlx::query(
        "INSERT INTO refresh_tokens (id, user_id, token_hash, device_id, expires_at, revoked_reason) VALUES (?, ?, ?, ?, ?, '')",
    )
    .bind(&new_token_id)
    .bind(&user_id)
    .bind(&new_refresh_hash)
    .bind(&req.device_id)
    .bind(&expires_at_str)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE user_devices
         SET is_active = 1,
             token_valid_after = MAX(token_valid_after, ?),
             last_active_at = datetime('now')
         WHERE user_id = ? AND device_id = ?",
    )
    .bind(access_claims.iat as i64)
    .bind(&user_id)
    .bind(&req.device_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(user_id = %user_id, device_id = %req.device_id, "Token refreshed");

    Ok(Json(RefreshResponse {
        access_token,
        refresh_token: new_refresh_token,
        expires_in: state.config.access_token_expiry_secs,
        user_id,
    }))
}

/// POST /auth/logout (requires auth)
pub async fn logout(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<MessageResponse>, AppError> {
    let mut tx = state.pool.begin().await?;
    let now_ts = chrono::Utc::now().timestamp_millis();

    sqlx::query(
        "UPDATE refresh_tokens
         SET revoked = 1,
             revoked_reason = 'logout'
         WHERE user_id = ? AND device_id = ? AND revoked = 0",
    )
    .bind(&auth.user_id)
    .bind(&auth.device_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE user_devices
         SET is_active = 0,
             token_valid_after = MAX(token_valid_after, ?),
             last_active_at = datetime('now')
         WHERE user_id = ? AND device_id = ?",
    )
    .bind(now_ts)
    .bind(&auth.user_id)
    .bind(&auth.device_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(user_id = %auth.user_id, device_id = %auth.device_id, "User logged out");

    Ok(Json(MessageResponse {
        message: "Logged out successfully".to_string(),
    }))
}

/// PUT /auth/password (requires auth)
pub async fn change_password(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    // Validate new password
    if req.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "New password must be at least 8 characters".to_string(),
        ));
    }

    // Get current password hash
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT password_hash FROM users WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await?;

    let (current_hash,) = row.ok_or_else(|| {
        AppError::NotFound("User not found".to_string())
    })?;

    // Verify old password
    verify_password(&req.old_password, &current_hash)?;

    // Hash new password
    let new_hash = hash_password(&req.new_password)?;

    // Atomic transaction: update password + revoke all tokens
    let mut tx = state.pool.begin().await?;
    let now_ts = chrono::Utc::now().timestamp_millis();

    sqlx::query(
        "UPDATE users
         SET password_hash = ?,
             token_valid_after = MAX(token_valid_after, ?),
             updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(&new_hash)
    .bind(now_ts)
    .bind(&auth.user_id)
    .execute(&mut *tx)
    .await?;

    revoke_all_user_device_sessions(&mut tx, &auth.user_id, now_ts, "password_changed").await?;

    tx.commit().await?;

    tracing::info!(user_id = %auth.user_id, "Password changed, all tokens revoked");

    Ok(Json(MessageResponse {
        message: "Password changed successfully. Please log in again on all devices.".to_string(),
    }))
}

/// DELETE /auth/account (requires auth)
pub async fn delete_account(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<MessageResponse>, AppError> {
    // Atomic transaction: soft delete user + revoke all tokens
    let mut tx = state.pool.begin().await?;
    let now_ts = chrono::Utc::now().timestamp_millis();

    sqlx::query(
        "UPDATE users
         SET deleted_at = datetime('now'),
             token_valid_after = MAX(token_valid_after, ?),
             updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(now_ts)
    .bind(&auth.user_id)
    .execute(&mut *tx)
    .await?;

    revoke_all_user_device_sessions(&mut tx, &auth.user_id, now_ts, "account_deleted").await?;

    tx.commit().await?;

    tracing::info!(user_id = %auth.user_id, "Account soft-deleted");

    Ok(Json(MessageResponse {
        message: "Account deleted successfully".to_string(),
    }))
}

/// GET /auth/devices (requires auth)
pub async fn list_devices(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<DevicesResponse>, AppError> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT device_id, device_name, platform, last_active_at, created_at
         FROM user_devices
         WHERE user_id = ? AND is_active = 1
         ORDER BY last_active_at DESC",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let devices = rows
        .into_iter()
        .map(|(device_id, name, platform, last_active, created)| DeviceInfo {
            device_id,
            device_name: name,
            platform,
            last_active_at: last_active,
            created_at: created,
        })
        .collect();

    Ok(Json(DevicesResponse { devices }))
}

/// DELETE /auth/devices/:device_id (requires auth)
pub async fn remove_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(device_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let mut tx = state.pool.begin().await?;
    let now_ts = chrono::Utc::now().timestamp_millis();

    sqlx::query(
        "UPDATE refresh_tokens
         SET revoked = 1,
             revoked_reason = 'removed'
         WHERE user_id = ? AND device_id = ? AND revoked = 0",
    )
    .bind(&auth.user_id)
    .bind(&device_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE user_devices
         SET is_active = 0,
             token_valid_after = MAX(token_valid_after, ?),
             last_active_at = datetime('now')
         WHERE user_id = ? AND device_id = ?",
    )
    .bind(now_ts)
    .bind(&auth.user_id)
    .bind(&device_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(
        user_id = %auth.user_id,
        target_device = %device_id,
        "Device removed"
    );

    Ok(Json(MessageResponse {
        message: "Device removed successfully".to_string(),
    }))
}

// ─── Password helpers ───────────────────────────────────────────────────────

fn hash_password(password: &str) -> Result<String, AppError> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))
}

fn verify_password(password: &str, hash: &str) -> Result<(), AppError> {
    use argon2::{
        password_hash::PasswordHash, Argon2, PasswordVerifier,
    };

    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("Invalid password hash: {}", e)))?;

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid credentials".to_string()))
}

fn map_register_insert_error(err: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db_err) = &err {
        let message = db_err.message();
        if message.contains("users.username") {
            return AppError::Conflict("Username already taken".to_string());
        }
        if message.contains("users.email") {
            return AppError::Conflict("Email already registered".to_string());
        }
    }

    AppError::from(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::middleware;
    use crate::AppConfig;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn test_state() -> AppState {
        let config = AppConfig {
            database_url: "sqlite::memory:".to_string(),
            jwt_secret: "test-secret".to_string(),
            listen_addr: "127.0.0.1:0".to_string(),
            storage_path: "/tmp/test-storage".to_string(),
            db_pool_min: 1,
            db_pool_max: 5,
            access_token_expiry_secs: 3600,
            refresh_token_expiry_secs: 2_592_000,
        };

        let pool = db::init_pool(&config).await.expect("Failed to init pool");

        AppState {
            pool,
            config: Arc::new(config),
        }
    }

    async fn seed_user_and_devices(state: &AppState) {
        seed_user_and_devices_with_password_hash(state, "hash").await;
    }

    async fn seed_user_and_devices_with_password_hash(
        state: &AppState,
        password_hash: &str,
    ) {
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
        )
        .bind("user-1")
        .bind("user1")
        .bind("user1@example.com")
        .bind(password_hash)
        .execute(&state.pool)
        .await
        .unwrap();

        for (device_id, refresh_token) in [("device-a", "token-a"), ("device-b", "token-b")] {
            sqlx::query(
                "INSERT INTO user_devices (user_id, device_id, device_name, platform, token_valid_after, is_active) VALUES (?, ?, ?, ?, 0, 1)",
            )
            .bind("user-1")
            .bind(device_id)
            .bind(device_id)
            .bind("test")
            .execute(&state.pool)
            .await
            .unwrap();

            sqlx::query(
                "INSERT INTO refresh_tokens (id, user_id, token_hash, device_id, expires_at, revoked, revoked_reason) VALUES (?, ?, ?, ?, datetime('now', '+1 day'), 0, '')",
            )
            .bind(format!("refresh-{device_id}"))
            .bind("user-1")
            .bind(crate::jwt::hash_refresh_token(refresh_token))
            .bind(device_id)
            .execute(&state.pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn test_logout_only_removes_current_device_session() {
        let state = test_state().await;
        seed_user_and_devices(&state).await;

        let auth = AuthUser {
            user_id: "user-1".to_string(),
            device_id: "device-a".to_string(),
        };

        let _ = logout(State(state.clone()), Extension(auth)).await.unwrap();

        let current_device: (i64, i64) = sqlx::query_as(
            "SELECT is_active, token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(current_device.0, 0);
        assert!(current_device.1 > 0);

        let remaining_device: (i64, i64) = sqlx::query_as(
            "SELECT is_active, token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-b")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(remaining_device.0, 1);
        assert_eq!(remaining_device.1, 0);

        let revoked_current: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND device_id = ? AND revoked = 1",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(revoked_current, 1);

        let current_reason: String = sqlx::query_scalar(
            "SELECT revoked_reason FROM refresh_tokens WHERE user_id = ? AND device_id = ? LIMIT 1",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(current_reason, "logout");

        let unaffected_other: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND device_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .bind("device-b")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(unaffected_other, 1);

        let token_valid_after: i64 = sqlx::query_scalar(
            "SELECT token_valid_after FROM users WHERE id = ?",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(token_valid_after, 0);
    }

    #[tokio::test]
    async fn test_remove_device_only_affects_target_device() {
        let state = test_state().await;
        seed_user_and_devices(&state).await;

        let auth = AuthUser {
            user_id: "user-1".to_string(),
            device_id: "device-b".to_string(),
        };

        let response = remove_device(
            State(state.clone()),
            Extension(auth),
            Path("device-a".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(response.0.message, "Device removed successfully");

        let target_device: (i64, i64) = sqlx::query_as(
            "SELECT is_active, token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(target_device.0, 0);
        assert!(target_device.1 > 0);

        let remaining_device: (i64, i64) = sqlx::query_as(
            "SELECT is_active, token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-b")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(remaining_device.0, 1);
        assert_eq!(remaining_device.1, 0);

        let revoked_target: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND device_id = ? AND revoked = 1",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(revoked_target, 1);

        let target_reason: String = sqlx::query_scalar(
            "SELECT revoked_reason FROM refresh_tokens WHERE user_id = ? AND device_id = ? LIMIT 1",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(target_reason, "removed");

        let unaffected_other: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND device_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .bind("device-b")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(unaffected_other, 1);

        let token_valid_after: i64 = sqlx::query_scalar(
            "SELECT token_valid_after FROM users WHERE id = ?",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(token_valid_after, 0);
    }

    #[tokio::test]
    async fn test_refresh_with_logged_out_token_does_not_revoke_other_devices() {
        let state = test_state().await;
        seed_user_and_devices(&state).await;

        let auth = AuthUser {
            user_id: "user-1".to_string(),
            device_id: "device-a".to_string(),
        };
        let _ = logout(State(state.clone()), Extension(auth)).await.unwrap();

        let result = refresh(
            State(state.clone()),
            Json(RefreshRequest {
                refresh_token: "token-a".to_string(),
                device_id: "device-a".to_string(),
            }),
        )
        .await;
        assert!(result.is_err());

        let other_device_token: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND device_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .bind("device-b")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(other_device_token, 1);

        let user_cutoff: i64 = sqlx::query_scalar(
            "SELECT token_valid_after FROM users WHERE id = ?",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(user_cutoff, 0);
    }

    #[tokio::test]
    async fn test_login_same_device_id_does_not_revive_old_access_token() {
        let state = test_state().await;

        let password_hash = hash_password("password123").unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
        )
        .bind("user-1")
        .bind("user1")
        .bind("user1@example.com")
        .bind(&password_hash)
        .execute(&state.pool)
        .await
        .unwrap();

        let login_request = LoginRequest {
            credential: "user1".to_string(),
            password: "password123".to_string(),
            device_id: "device-a".to_string(),
            device_name: Some("Laptop".to_string()),
            platform: Some("macos".to_string()),
        };

        let first_login = login(State(state.clone()), Json(login_request)).await.unwrap();
        let first_claims = jsonwebtoken::decode::<crate::middleware::Claims>(
            &first_login.0.access_token,
            &jsonwebtoken::DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
            &jsonwebtoken::Validation::default(),
        )
        .unwrap()
        .claims;

        let _ = logout(
            State(state.clone()),
            Extension(AuthUser {
                user_id: "user-1".to_string(),
                device_id: "device-a".to_string(),
            }),
        )
        .await
        .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let second_login = login(
            State(state.clone()),
            Json(LoginRequest {
                credential: "user1".to_string(),
                password: "password123".to_string(),
                device_id: "device-a".to_string(),
                device_name: Some("Laptop".to_string()),
                platform: Some("macos".to_string()),
            }),
        )
        .await
        .unwrap();

        let second_claims = jsonwebtoken::decode::<crate::middleware::Claims>(
            &second_login.0.access_token,
            &jsonwebtoken::DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
            &jsonwebtoken::Validation::default(),
        )
        .unwrap()
        .claims;

        assert!(second_claims.iat >= first_claims.iat);

        let device_cutoff: i64 = sqlx::query_scalar(
            "SELECT token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(device_cutoff > first_claims.iat as i64);
        assert_eq!(second_claims.iat as i64, device_cutoff);
    }

    #[tokio::test]
    async fn test_login_same_device_id_revokes_old_refresh_token() {
        let state = test_state().await;

        let password_hash = hash_password("password123").unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
        )
        .bind("user-1")
        .bind("user1")
        .bind("user1@example.com")
        .bind(&password_hash)
        .execute(&state.pool)
        .await
        .unwrap();

        let first_login = login(
            State(state.clone()),
            Json(LoginRequest {
                credential: "user1".to_string(),
                password: "password123".to_string(),
                device_id: "device-a".to_string(),
                device_name: Some("Laptop".to_string()),
                platform: Some("macos".to_string()),
            }),
        )
        .await
        .unwrap();

        let second_login = login(
            State(state.clone()),
            Json(LoginRequest {
                credential: "user1".to_string(),
                password: "password123".to_string(),
                device_id: "device-a".to_string(),
                device_name: Some("Laptop".to_string()),
                platform: Some("macos".to_string()),
            }),
        )
        .await
        .unwrap();

        let first_token_status: (i64, String) = sqlx::query_as(
            "SELECT revoked, revoked_reason FROM refresh_tokens WHERE user_id = ? AND token_hash = ?",
        )
        .bind("user-1")
        .bind(jwt::hash_refresh_token(&first_login.0.refresh_token))
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(first_token_status.0, 1);
        assert_eq!(first_token_status.1, "replaced");

        let active_tokens: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND device_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(active_tokens, 1);

        let second_token_status: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND token_hash = ? AND revoked = 0",
        )
        .bind("user-1")
        .bind(jwt::hash_refresh_token(&second_login.0.refresh_token))
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(second_token_status, 1);

        let result = refresh(
            State(state.clone()),
            Json(RefreshRequest {
                refresh_token: first_login.0.refresh_token,
                device_id: "device-a".to_string(),
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::Unauthorized(message)) if message == "Invalid or expired refresh token"
        ));
    }

    #[tokio::test]
    async fn test_login_uses_effective_device_cutoff_for_access_token() {
        let state = test_state().await;
        let password_hash = hash_password("password123").unwrap();
        let future_cutoff = chrono::Utc::now().timestamp_millis() + 5_000;

        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
        )
        .bind("user-1")
        .bind("user1")
        .bind("user1@example.com")
        .bind(&password_hash)
        .execute(&state.pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO user_devices (user_id, device_id, device_name, platform, token_valid_after, is_active) VALUES (?, ?, ?, ?, ?, 1)",
        )
        .bind("user-1")
        .bind("device-a")
        .bind("Laptop")
        .bind("macos")
        .bind(future_cutoff)
        .execute(&state.pool)
        .await
        .unwrap();

        let login_response = login(
            State(state.clone()),
            Json(LoginRequest {
                credential: "user1".to_string(),
                password: "password123".to_string(),
                device_id: "device-a".to_string(),
                device_name: Some("Laptop".to_string()),
                platform: Some("macos".to_string()),
            }),
        )
        .await
        .unwrap();

        let claims = jsonwebtoken::decode::<crate::middleware::Claims>(
            &login_response.0.access_token,
            &jsonwebtoken::DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
            &jsonwebtoken::Validation::default(),
        )
        .unwrap()
        .claims;

        let stored_cutoff: i64 = sqlx::query_scalar(
            "SELECT token_valid_after FROM user_devices WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-a")
        .fetch_one(&state.pool)
        .await
        .unwrap();

        assert_eq!(stored_cutoff, future_cutoff);
        assert_eq!(claims.iat as i64, stored_cutoff);

        let app = axum::Router::new()
            .route(
                "/protected",
                axum::routing::get(|| async { StatusCode::NO_CONTENT }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth_middleware,
            ))
            .with_state(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header(
                        "Authorization",
                        format!("Bearer {}", login_response.0.access_token),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_login_token_is_immediately_authorized_by_middleware() {
        let state = test_state().await;
        let password_hash = hash_password("password123").unwrap();

        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
        )
        .bind("user-1")
        .bind("user1")
        .bind("user1@example.com")
        .bind(&password_hash)
        .execute(&state.pool)
        .await
        .unwrap();

        let login_response = login(
            State(state.clone()),
            Json(LoginRequest {
                credential: "user1".to_string(),
                password: "password123".to_string(),
                device_id: "device-a".to_string(),
                device_name: Some("Laptop".to_string()),
                platform: Some("macos".to_string()),
            }),
        )
        .await
        .unwrap();

        let app = axum::Router::new()
            .route(
                "/protected",
                axum::routing::get(|| async { StatusCode::NO_CONTENT }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth_middleware,
            ))
            .with_state(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header(
                        "Authorization",
                        format!("Bearer {}", login_response.0.access_token),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_list_devices_only_returns_active_devices() {
        let state = test_state().await;
        seed_user_and_devices(&state).await;

        let _ = remove_device(
            State(state.clone()),
            Extension(AuthUser {
                user_id: "user-1".to_string(),
                device_id: "device-b".to_string(),
            }),
            Path("device-a".to_string()),
        )
        .await
        .unwrap();

        let response = list_devices(
            State(state.clone()),
            Extension(AuthUser {
                user_id: "user-1".to_string(),
                device_id: "device-b".to_string(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.0.devices.len(), 1);
        assert_eq!(response.0.devices[0].device_id, "device-b");
    }

    #[tokio::test]
    async fn test_refresh_token_reuse_deactivates_all_devices() {
        let state = test_state().await;
        seed_user_and_devices(&state).await;

        sqlx::query(
            "UPDATE refresh_tokens
             SET revoked = 1,
                 revoked_reason = 'rotated'
             WHERE user_id = ? AND device_id = ?",
        )
        .bind("user-1")
        .bind("device-a")
        .execute(&state.pool)
        .await
        .unwrap();

        let result = refresh(
            State(state.clone()),
            Json(RefreshRequest {
                refresh_token: "token-a".to_string(),
                device_id: "device-a".to_string(),
            }),
        )
        .await;

        assert!(matches!(
            result,
            Err(AppError::Unauthorized(message)) if message == "Invalid or expired refresh token"
        ));

        let user_cutoff: i64 = sqlx::query_scalar(
            "SELECT token_valid_after FROM users WHERE id = ?",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(user_cutoff > 0);

        let devices: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT device_id, is_active, token_valid_after
             FROM user_devices
             WHERE user_id = ?
             ORDER BY device_id",
        )
        .bind("user-1")
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(devices.len(), 2);
        for (_, is_active, token_valid_after) in devices {
            assert_eq!(is_active, 0);
            assert!(token_valid_after >= user_cutoff);
        }

        let active_refresh_tokens: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(active_refresh_tokens, 0);

        let other_reason: String = sqlx::query_scalar(
            "SELECT revoked_reason FROM refresh_tokens WHERE user_id = ? AND device_id = ? LIMIT 1",
        )
        .bind("user-1")
        .bind("device-b")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(other_reason, "compromised");
    }

    #[tokio::test]
    async fn test_change_password_deactivates_all_devices() {
        let state = test_state().await;
        let password_hash = hash_password("old-password-123").unwrap();
        seed_user_and_devices_with_password_hash(&state, &password_hash).await;

        let response = change_password(
            State(state.clone()),
            Extension(AuthUser {
                user_id: "user-1".to_string(),
                device_id: "device-a".to_string(),
            }),
            Json(ChangePasswordRequest {
                old_password: "old-password-123".to_string(),
                new_password: "new-password-456".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            response.0.message,
            "Password changed successfully. Please log in again on all devices."
        );

        let updated_hash: String = sqlx::query_scalar(
            "SELECT password_hash FROM users WHERE id = ?",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        verify_password("new-password-456", &updated_hash).unwrap();

        let devices: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT is_active, token_valid_after FROM user_devices WHERE user_id = ?",
        )
        .bind("user-1")
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(devices.len(), 2);
        for (is_active, token_valid_after) in devices {
            assert_eq!(is_active, 0);
            assert!(token_valid_after > 0);
        }

        let active_refresh_tokens: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(active_refresh_tokens, 0);

        let refresh_reasons: Vec<String> = sqlx::query_scalar(
            "SELECT revoked_reason FROM refresh_tokens WHERE user_id = ? ORDER BY device_id",
        )
        .bind("user-1")
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(refresh_reasons, vec!["password_changed", "password_changed"]);
    }

    #[tokio::test]
    async fn test_delete_account_deactivates_all_devices() {
        let state = test_state().await;
        seed_user_and_devices(&state).await;

        let response = delete_account(
            State(state.clone()),
            Extension(AuthUser {
                user_id: "user-1".to_string(),
                device_id: "device-a".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.0.message, "Account deleted successfully");

        let deleted_at: Option<String> = sqlx::query_scalar(
            "SELECT deleted_at FROM users WHERE id = ?",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(deleted_at.is_some());

        let devices: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT is_active, token_valid_after FROM user_devices WHERE user_id = ?",
        )
        .bind("user-1")
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(devices.len(), 2);
        for (is_active, token_valid_after) in devices {
            assert_eq!(is_active, 0);
            assert!(token_valid_after > 0);
        }

        let active_refresh_tokens: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND revoked = 0",
        )
        .bind("user-1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(active_refresh_tokens, 0);

        let refresh_reasons: Vec<String> = sqlx::query_scalar(
            "SELECT revoked_reason FROM refresh_tokens WHERE user_id = ? ORDER BY device_id",
        )
        .bind("user-1")
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(refresh_reasons, vec!["account_deleted", "account_deleted"]);
    }
}
