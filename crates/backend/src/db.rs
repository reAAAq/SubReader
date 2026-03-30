//! Database initialization, connection pool, and schema setup.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::str::FromStr;

use crate::config::AppConfig;

/// Database pool type alias.
pub type DbPool = Pool<Sqlite>;

/// Initialize the database connection pool and ensure schema exists.
pub async fn init_pool(config: &AppConfig) -> Result<DbPool, sqlx::Error> {
    let connect_options = SqliteConnectOptions::from_str(&config.database_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .min_connections(config.db_pool_min)
        .max_connections(config.db_pool_max)
        .connect_with(connect_options)
        .await?;

    init_schema(&pool).await?;

    Ok(pool)
}

/// Ensure the full database schema exists.
async fn init_schema(pool: &DbPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            email TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            deleted_at TEXT,
            token_valid_after INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
        CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

        CREATE TABLE IF NOT EXISTS refresh_tokens (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            token_hash TEXT NOT NULL UNIQUE,
            device_id TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            revoked INTEGER NOT NULL DEFAULT 0,
            revoked_reason TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user_id ON refresh_tokens(user_id);
        CREATE INDEX IF NOT EXISTS idx_refresh_tokens_token_hash ON refresh_tokens(token_hash);

        CREATE TABLE IF NOT EXISTS user_devices (
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            device_id TEXT NOT NULL,
            device_name TEXT NOT NULL DEFAULT '',
            platform TEXT NOT NULL DEFAULT '',
            token_valid_after INTEGER NOT NULL DEFAULT 0,
            is_active INTEGER NOT NULL DEFAULT 1,
            last_active_at TEXT NOT NULL DEFAULT (datetime('now')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, device_id)
        );
        CREATE INDEX IF NOT EXISTS idx_user_devices_user_id ON user_devices(user_id);

        CREATE TABLE IF NOT EXISTS sync_operations (
            server_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            op_id TEXT NOT NULL,
            op_type TEXT NOT NULL,
            op_data TEXT NOT NULL,
            hlc_ts INTEGER NOT NULL,
            device_id TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(user_id, op_id)
        );
        CREATE INDEX IF NOT EXISTS idx_sync_ops_user_seq ON sync_operations(user_id, server_seq);
        CREATE INDEX IF NOT EXISTS idx_sync_ops_user_device ON sync_operations(user_id, device_id);

        CREATE TABLE IF NOT EXISTS upload_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            chunk_size INTEGER NOT NULL,
            total_chunks INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_upload_sessions_user ON upload_sessions(user_id);

        CREATE TABLE IF NOT EXISTS file_chunks (
            upload_id TEXT NOT NULL REFERENCES upload_sessions(id) ON DELETE CASCADE,
            chunk_index INTEGER NOT NULL,
            size INTEGER NOT NULL,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (upload_id, chunk_index)
        );

        CREATE TABLE IF NOT EXISTS user_files (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            file_name TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            storage_path TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            deleted_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_user_files_user ON user_files(user_id);
        CREATE INDEX IF NOT EXISTS idx_user_files_sha256 ON user_files(user_id, sha256);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_user_files_active_sha256
        ON user_files(user_id, sha256)
        WHERE deleted_at IS NULL;
        ",
    )
    .execute(pool)
    .await?;

    tracing::info!("Database schema is ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_pool_and_schema() {
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

        let pool = init_pool(&config).await.expect("Failed to init pool");

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        let table_names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        assert!(table_names.contains(&"users"));
        assert!(table_names.contains(&"refresh_tokens"));
        assert!(table_names.contains(&"user_devices"));
        assert!(table_names.contains(&"sync_operations"));
        assert!(table_names.contains(&"upload_sessions"));
        assert!(table_names.contains(&"file_chunks"));
        assert!(table_names.contains(&"user_files"));
    }

    #[tokio::test]
    async fn test_schema_init_idempotent() {
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

        let pool = init_pool(&config).await.expect("Failed to init pool");
        init_schema(&pool).await.expect("Idempotent schema init failed");

        let token_valid_after: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM pragma_table_info('users') WHERE name = 'token_valid_after'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(token_valid_after.is_some());

        let device_token_valid_after: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM pragma_table_info('user_devices') WHERE name = 'token_valid_after'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(device_token_valid_after.is_some());

        let device_is_active: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM pragma_table_info('user_devices') WHERE name = 'is_active'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(device_is_active.is_some());

        let revoked_reason: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM pragma_table_info('refresh_tokens') WHERE name = 'revoked_reason'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(revoked_reason.is_some());

        let dedupe_index: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_user_files_active_sha256'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(dedupe_index.is_some());
    }

    #[tokio::test]
    async fn test_user_devices_allows_same_device_id_for_different_users() {
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

        let pool = init_pool(&config).await.expect("Failed to init pool");

        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES ('u1', 'user1', 'u1@example.com', 'hash')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash) VALUES ('u2', 'user2', 'u2@example.com', 'hash')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO user_devices (user_id, device_id, device_name, platform) VALUES ('u1', 'shared-device', 'Laptop', 'macos')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO user_devices (user_id, device_id, device_name, platform) VALUES ('u2', 'shared-device', 'Tablet', 'ios')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM user_devices WHERE device_id = 'shared-device'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(count, 2);
    }
}
