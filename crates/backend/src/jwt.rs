//! JWT token generation and validation utilities.

use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::middleware::Claims;

/// Token pair returned after successful authentication.
#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

/// Generate a JWT access token for the given user and device.
pub fn generate_access_token(
    config: &AppConfig,
    user_id: &str,
    device_id: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    generate_access_token_at(config, user_id, device_id, Utc::now().timestamp_millis())
}

/// Generate a JWT access token using an explicit issued-at timestamp in milliseconds.
pub fn generate_access_token_at(
    config: &AppConfig,
    user_id: &str,
    device_id: &str,
    issued_at_ms: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: user_id.to_string(),
        exp: (issued_at_ms / 1000) as u64 + config.access_token_expiry_secs,
        iat: issued_at_ms as u64,
        device_id: device_id.to_string(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
}

/// Generate a random refresh token string.
pub fn generate_refresh_token() -> String {
    Uuid::new_v4().to_string()
}

/// Hash a refresh token for secure storage using SHA-256.
pub fn hash_refresh_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    fn test_config() -> AppConfig {
        AppConfig {
            database_url: "sqlite::memory:".to_string(),
            jwt_secret: "test-secret-key-for-unit-tests".to_string(),
            listen_addr: "127.0.0.1:0".to_string(),
            storage_path: "/tmp/test".to_string(),
            db_pool_min: 1,
            db_pool_max: 5,
            access_token_expiry_secs: 3600,
            refresh_token_expiry_secs: 2_592_000,
        }
    }

    #[test]
    fn test_generate_access_token_is_valid_jwt() {
        let config = test_config();
        let token = generate_access_token(&config, "user-1", "device-1").unwrap();

        // Should be a valid JWT with 3 parts
        assert_eq!(token.split('.').count(), 3);

        // Should be decodable
        let decoded = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .unwrap();

        assert_eq!(decoded.claims.sub, "user-1");
        assert_eq!(decoded.claims.device_id, "device-1");
        assert!(decoded.claims.iat > 0);
        assert_eq!(
            decoded.claims.exp,
            decoded.claims.iat / 1000 + config.access_token_expiry_secs
        );
    }

    #[test]
    fn test_generate_access_token_at_uses_exact_iat() {
        let config = test_config();
        let issued_at_ms = 1_700_000_123_456i64;
        let token = generate_access_token_at(&config, "user-1", "device-1", issued_at_ms).unwrap();

        let decoded = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .unwrap();

        assert_eq!(decoded.claims.iat, issued_at_ms as u64);
        assert_eq!(
            decoded.claims.exp,
            issued_at_ms as u64 / 1000 + config.access_token_expiry_secs
        );
    }

    #[test]
    fn test_generate_access_token_different_users_different_tokens() {
        let config = test_config();
        let token1 = generate_access_token(&config, "user-1", "device-1").unwrap();
        let token2 = generate_access_token(&config, "user-2", "device-1").unwrap();
        assert_ne!(token1, token2);
    }

    #[test]
    fn test_generate_refresh_token_is_uuid() {
        let token = generate_refresh_token();
        // Should be a valid UUID
        assert!(uuid::Uuid::parse_str(&token).is_ok());
    }

    #[test]
    fn test_generate_refresh_token_unique() {
        let t1 = generate_refresh_token();
        let t2 = generate_refresh_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_hash_refresh_token_deterministic() {
        let hash1 = hash_refresh_token("my-token");
        let hash2 = hash_refresh_token("my-token");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_refresh_token_different_inputs() {
        let hash1 = hash_refresh_token("token-a");
        let hash2 = hash_refresh_token("token-b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_refresh_token_is_hex_sha256() {
        let hash = hash_refresh_token("test");
        // SHA-256 produces 64 hex characters
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
