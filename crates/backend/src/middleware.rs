//! Middleware for JWT authentication and request logging.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// JWT claims payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID).
    pub sub: String,
    /// Expiration time (Unix timestamp).
    pub exp: u64,
    /// Issued at (Unix timestamp).
    pub iat: u64,
    /// Device ID.
    pub device_id: String,
}

/// Extension type to pass authenticated user info to handlers.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub device_id: String,
}

/// JWT authentication middleware.
///
/// Validates the `Authorization: Bearer <token>` header and injects
/// `AuthUser` as a request extension.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let claims = token_data.claims;
    let auth_row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT users.token_valid_after, user_devices.token_valid_after
         FROM users
         JOIN user_devices
           ON user_devices.user_id = users.id
          AND user_devices.device_id = ?
         WHERE users.id = ?
           AND users.deleted_at IS NULL
           AND user_devices.is_active = 1",
    )
    .bind(&claims.device_id)
    .bind(&claims.sub)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (user_token_valid_after, device_token_valid_after) = auth_row.ok_or(StatusCode::UNAUTHORIZED)?;
    if claims.iat < user_token_valid_after as u64 || claims.iat < device_token_valid_after as u64 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let auth_user = AuthUser {
        user_id: claims.sub,
        device_id: claims.device_id,
    };

    request.extensions_mut().insert(auth_user);

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_serialization() {
        let claims = Claims {
            sub: "user-123".to_string(),
            exp: 1700000000,
            iat: 1699996400,
            device_id: "device-abc".to_string(),
        };

        let json = serde_json::to_string(&claims).unwrap();
        let deserialized: Claims = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.sub, "user-123");
        assert_eq!(deserialized.exp, 1700000000);
        assert_eq!(deserialized.iat, 1699996400);
        assert_eq!(deserialized.device_id, "device-abc");
    }

    #[test]
    fn test_auth_user_debug() {
        let user = AuthUser {
            user_id: "u1".to_string(),
            device_id: "d1".to_string(),
        };
        let debug = format!("{:?}", user);
        assert!(debug.contains("u1"));
        assert!(debug.contains("d1"));
    }

    #[test]
    fn test_auth_user_clone() {
        let user = AuthUser {
            user_id: "u1".to_string(),
            device_id: "d1".to_string(),
        };
        let cloned = user.clone();
        assert_eq!(cloned.user_id, "u1");
        assert_eq!(cloned.device_id, "d1");
    }
}
