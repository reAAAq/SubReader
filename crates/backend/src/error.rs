//! Unified error types for the backend service.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// API error response body.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

/// Application-level error type.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        if is_unique_constraint_violation(&err) {
            return AppError::Conflict("Resource already exists".to_string());
        }
        AppError::Database(err)
    }
}

fn is_unique_constraint_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => {
            db_err.is_unique_violation()
                || db_err.message().contains("UNIQUE constraint failed")
        }
        _ => false,
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred".to_string(),
                )
            }
            AppError::Database(err) => {
                tracing::error!("Database error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "A database error occurred".to_string(),
                )
            }
        };

        let body = ErrorResponse {
            error: error_type.to_string(),
            message,
        };

        (status, axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse {
            error: "bad_request".to_string(),
            message: "Invalid input".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("bad_request"));
        assert!(json.contains("Invalid input"));
    }

    #[test]
    fn test_app_error_display() {
        let err = AppError::BadRequest("missing field".into());
        assert_eq!(format!("{}", err), "Bad request: missing field");

        let err = AppError::Unauthorized("invalid token".into());
        assert_eq!(format!("{}", err), "Unauthorized: invalid token");

        let err = AppError::NotFound("user not found".into());
        assert_eq!(format!("{}", err), "Not found: user not found");

        let err = AppError::Conflict("duplicate".into());
        assert_eq!(format!("{}", err), "Conflict: duplicate");

        let err = AppError::Internal("panic".into());
        assert_eq!(format!("{}", err), "Internal server error: panic");
    }

    #[test]
    fn test_bad_request_returns_400() {
        let err = AppError::BadRequest("test".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_unauthorized_returns_401() {
        let err = AppError::Unauthorized("test".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_not_found_returns_404() {
        let err = AppError::NotFound("test".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_conflict_returns_409() {
        let err = AppError::Conflict("test".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn test_internal_returns_500() {
        let err = AppError::Internal("test".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_sqlite_unique_constraint_maps_to_conflict() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT NOT NULL UNIQUE)")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO users (username) VALUES ('alice')")
            .execute(&pool)
            .await
            .unwrap();

        let err = sqlx::query("INSERT INTO users (username) VALUES ('alice')")
            .execute(&pool)
            .await
            .unwrap_err();

        let app_err = AppError::from(err);
        assert!(matches!(app_err, AppError::Conflict(_)));
    }
}
