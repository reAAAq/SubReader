//! Backend configuration loaded from environment variables.

use std::env;

/// Application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Database URL (SQLite connection string).
    pub database_url: String,
    /// JWT signing secret key.
    pub jwt_secret: String,
    /// Server listen address (e.g., "0.0.0.0:8080").
    pub listen_addr: String,
    /// File storage root path.
    pub storage_path: String,
    /// Database connection pool min size.
    pub db_pool_min: u32,
    /// Database connection pool max size.
    pub db_pool_max: u32,
    /// Access token expiry in seconds (default: 3600 = 1 hour).
    pub access_token_expiry_secs: u64,
    /// Refresh token expiry in seconds (default: 2592000 = 30 days).
    pub refresh_token_expiry_secs: u64,
}

impl AppConfig {
    /// Load configuration from environment variables.
    ///
    /// Required env vars:
    /// - `DATABASE_URL` — database connection string
    /// - `JWT_SECRET` — secret key for JWT signing
    ///
    /// Optional env vars:
    /// - `LISTEN_ADDR` — server listen address (default: "0.0.0.0:8080")
    /// - `STORAGE_PATH` — file storage root (default: "./storage")
    /// - `DB_POOL_MIN` — min pool connections (default: 5)
    /// - `DB_POOL_MAX` — max pool connections (default: 50)
    /// - `ACCESS_TOKEN_EXPIRY_SECS` — access token TTL (default: 3600)
    /// - `REFRESH_TOKEN_EXPIRY_SECS` — refresh token TTL (default: 2592000)
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL environment variable is required".to_string())?;
        let jwt_secret = env::var("JWT_SECRET")
            .map_err(|_| "JWT_SECRET environment variable is required".to_string())?;

        let listen_addr = env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let storage_path = env::var("STORAGE_PATH").unwrap_or_else(|_| "./storage".to_string());

        let db_pool_min = env::var("DB_POOL_MIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);
        let db_pool_max = env::var("DB_POOL_MAX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let access_token_expiry_secs = env::var("ACCESS_TOKEN_EXPIRY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);
        let refresh_token_expiry_secs = env::var("REFRESH_TOKEN_EXPIRY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2_592_000);

        Ok(Self {
            database_url,
            jwt_secret,
            listen_addr,
            storage_path,
            db_pool_min,
            db_pool_max,
            access_token_expiry_secs,
            refresh_token_expiry_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to serialize config tests that modify env vars.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env_vars() {
        // SAFETY: These tests are serialized by ENV_LOCK and run single-threaded.
        unsafe {
            env::remove_var("DATABASE_URL");
            env::remove_var("JWT_SECRET");
            env::remove_var("LISTEN_ADDR");
            env::remove_var("STORAGE_PATH");
            env::remove_var("DB_POOL_MIN");
            env::remove_var("DB_POOL_MAX");
            env::remove_var("ACCESS_TOKEN_EXPIRY_SECS");
            env::remove_var("REFRESH_TOKEN_EXPIRY_SECS");
        }
    }

    /// SAFETY: env::set_var is unsafe in Rust 2024 edition because it is not
    /// thread-safe. We serialize all config tests with ENV_LOCK to ensure
    /// no concurrent access.
    unsafe fn set_env(key: &str, value: &str) {
        env::set_var(key, value);
    }

    #[test]
    fn test_config_missing_database_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_env_vars();
        unsafe { set_env("JWT_SECRET", "secret") };

        let result = AppConfig::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("DATABASE_URL"));
    }

    #[test]
    fn test_config_missing_jwt_secret() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_env_vars();
        unsafe { set_env("DATABASE_URL", "sqlite::memory:") };

        let result = AppConfig::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("JWT_SECRET"));
    }

    #[test]
    fn test_config_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_env_vars();
        unsafe {
            set_env("DATABASE_URL", "sqlite::memory:");
            set_env("JWT_SECRET", "test-secret");
        }

        let config = AppConfig::from_env().unwrap();
        assert_eq!(config.database_url, "sqlite::memory:");
        assert_eq!(config.jwt_secret, "test-secret");
        assert_eq!(config.listen_addr, "0.0.0.0:8080");
        assert_eq!(config.storage_path, "./storage");
        assert_eq!(config.db_pool_min, 5);
        assert_eq!(config.db_pool_max, 50);
        assert_eq!(config.access_token_expiry_secs, 3600);
        assert_eq!(config.refresh_token_expiry_secs, 2_592_000);
    }

    #[test]
    fn test_config_custom_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_env_vars();
        unsafe {
            set_env("DATABASE_URL", "postgres://localhost/subreader");
            set_env("JWT_SECRET", "my-secret");
            set_env("LISTEN_ADDR", "127.0.0.1:3000");
            set_env("STORAGE_PATH", "/data/files");
            set_env("DB_POOL_MIN", "2");
            set_env("DB_POOL_MAX", "20");
            set_env("ACCESS_TOKEN_EXPIRY_SECS", "7200");
            set_env("REFRESH_TOKEN_EXPIRY_SECS", "86400");
        }

        let config = AppConfig::from_env().unwrap();
        assert_eq!(config.listen_addr, "127.0.0.1:3000");
        assert_eq!(config.storage_path, "/data/files");
        assert_eq!(config.db_pool_min, 2);
        assert_eq!(config.db_pool_max, 20);
        assert_eq!(config.access_token_expiry_secs, 7200);
        assert_eq!(config.refresh_token_expiry_secs, 86400);
    }

    #[test]
    fn test_config_invalid_numeric_uses_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_env_vars();
        unsafe {
            set_env("DATABASE_URL", "sqlite::memory:");
            set_env("JWT_SECRET", "secret");
            set_env("DB_POOL_MIN", "not_a_number");
            set_env("DB_POOL_MAX", "also_not");
        }

        let config = AppConfig::from_env().unwrap();
        assert_eq!(config.db_pool_min, 5);
        assert_eq!(config.db_pool_max, 50);
    }
}
