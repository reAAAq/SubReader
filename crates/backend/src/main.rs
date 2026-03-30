//! Reader Backend — cloud sync service.
//!
//! A high-performance backend service for the SubReader platform,
//! providing account management, data synchronization, and file storage.

use axum::Router;
use std::sync::Arc;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::EnvFilter;

mod config;
mod db;
mod error;
mod handlers;
mod jwt;
mod middleware;

pub use config::AppConfig;
pub use db::DbPool;
pub use error::AppError;

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub config: Arc<AppConfig>,
}

#[tokio::main]
async fn main() {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Initialize tracing/logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    // Load configuration
    let config = AppConfig::from_env().expect("Failed to load configuration");
    let listen_addr = config.listen_addr.clone();

    tracing::info!(
        listen_addr = %listen_addr,
        "Starting reader-backend"
    );

    // Initialize database pool and ensure schema exists
    let pool = db::init_pool(&config).await.expect("Failed to initialize database");

    tracing::info!("Database initialized and schema is ready");

    // Build application state
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config),
    };

    // Spawn background task to clean up stale upload sessions (every hour)
    {
        let cleanup_pool = pool.clone();
        let storage_path = state.config.storage_path.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if let Err(e) = cleanup_stale_uploads(&cleanup_pool, &storage_path).await {
                    tracing::error!(error = %e, "Failed to clean up stale upload sessions");
                }
            }
        });
    }

    // Build router
    let auth_protected = Router::new()
        .route("/auth/logout", axum::routing::post(handlers::auth::logout))
        .route("/auth/password", axum::routing::put(handlers::auth::change_password))
        .route("/auth/account", axum::routing::delete(handlers::auth::delete_account))
        .route("/auth/devices", axum::routing::get(handlers::auth::list_devices))
.route("/auth/devices/:device_id", axum::routing::delete(handlers::auth::remove_device))
        .route("/sync/push", axum::routing::post(handlers::sync::push))
        .route("/sync/pull", axum::routing::get(handlers::sync::pull))
        .route("/files/upload/init", axum::routing::post(handlers::files::upload_init))
.route("/files/upload/:upload_id/chunk/:index", axum::routing::put(handlers::files::upload_chunk))
        .route("/files/upload/:upload_id/complete", axum::routing::post(handlers::files::upload_complete))
        .route("/files/:file_id", axum::routing::get(handlers::files::download))
        .route("/files/:file_id", axum::routing::delete(handlers::files::delete_file))
        .route("/files", axum::routing::get(handlers::files::list_files))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ));

    let app = Router::new()
        .route("/health", axum::routing::get(handlers::health::health_check))
        .route("/auth/register", axum::routing::post(handlers::auth::register))
        .route("/auth/login", axum::routing::post(handlers::auth::login))
        .route("/auth/refresh", axum::routing::post(handlers::auth::refresh))
        .merge(auth_protected)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(CatchPanicLayer::new())
        .layer({
            // CORS: configurable via CORS_ALLOWED_ORIGINS env var
            // Default: allow localhost origins for development
            let allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:3000,http://localhost:8080".to_string());
            let origins: Vec<_> = allowed_origins
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods(AllowMethods::any())
                .allow_headers(AllowHeaders::any())
        })
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind to address");

    tracing::info!("Server listening on {}", listen_addr);

    axum::serve(listener, app)
        .await
        .expect("Server error");
}

/// Clean up upload sessions that have been pending for more than 6 hours.
///
/// Removes both the database records and the on-disk chunk files.
async fn cleanup_stale_uploads(pool: &db::DbPool, storage_path: &str) -> Result<(), sqlx::Error> {
    let stale_sessions: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM upload_sessions
         WHERE status = 'pending'
         AND created_at < datetime('now', '-6 hours')",
    )
    .fetch_all(pool)
    .await?;

    if stale_sessions.is_empty() {
        return Ok(());
    }

    let count = stale_sessions.len();

    for (session_id,) in &stale_sessions {
        // Remove chunk files from disk
        let upload_dir = std::path::PathBuf::from(storage_path)
            .join("uploads")
            .join(session_id);
        let _ = tokio::fs::remove_dir_all(&upload_dir).await;

        // Remove chunk records
        sqlx::query("DELETE FROM file_chunks WHERE upload_id = ?")
            .bind(session_id)
            .execute(pool)
            .await?;

        // Mark session as expired
        sqlx::query("UPDATE upload_sessions SET status = 'expired' WHERE id = ?")
            .bind(session_id)
            .execute(pool)
            .await?;
    }

    tracing::info!(count = count, "Cleaned up stale upload sessions");
    Ok(())
}
