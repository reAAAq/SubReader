//! Token storage implementations.

use std::path::PathBuf;

use crate::{AuthError, AuthToken, TokenStore};

/// Set restrictive file permissions (owner read/write only) on Unix systems.
#[cfg(unix)]
fn set_owner_read_write(path: &std::path::Path) -> Result<(), AuthError> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
        .map_err(|e| AuthError::StorageError(format!("Failed to set file permissions: {}", e)))
}

#[cfg(not(unix))]
fn set_owner_read_write(_path: &std::path::Path) -> Result<(), AuthError> {
    // No-op on non-Unix platforms; rely on OS-level ACLs or Keychain.
    Ok(())
}

#[cfg(unix)]
fn write_token_file(path: &std::path::Path, json: &str) -> Result<(), AuthError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| AuthError::StorageError(format!("Failed to write token file: {}", e)))?;

    file.write_all(json.as_bytes())
        .map_err(|e| AuthError::StorageError(format!("Failed to write token file: {}", e)))?;

    file.sync_all()
        .map_err(|e| AuthError::StorageError(format!("Failed to flush token file: {}", e)))?;

    set_owner_read_write(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_token_file(path: &std::path::Path, json: &str) -> Result<(), AuthError> {
    std::fs::write(path, json)
        .map_err(|e| AuthError::StorageError(format!("Failed to write token file: {}", e)))
}

/// File-based token store that persists tokens to a JSON file.
///
/// For production use on macOS/iOS, the Swift layer should inject
/// a Keychain-backed implementation via FFI callback.
pub struct FileTokenStore {
    path: PathBuf,
}

impl FileTokenStore {
    /// Create a new file-based token store.
    ///
    /// # Arguments
    /// * `path` - Path to the token storage file.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl TokenStore for FileTokenStore {
    fn save_token(&self, token: &AuthToken) -> Result<(), AuthError> {
        let json = serde_json::to_string_pretty(token)
            .map_err(|e| AuthError::StorageError(format!("Serialization failed: {}", e)))?;

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AuthError::StorageError(format!("Failed to create directory: {}", e)))?;
        }

        write_token_file(&self.path, &json)?;

        // WARNING: Tokens are stored as plaintext JSON. For production use on
        // macOS/iOS, use a Keychain-backed TokenStore instead.
        set_owner_read_write(&self.path)?;

        Ok(())
    }

    fn load_token(&self) -> Result<Option<AuthToken>, AuthError> {
        if !self.path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&self.path)
            .map_err(|e| AuthError::StorageError(format!("Failed to read token file: {}", e)))?;

        let token: AuthToken = serde_json::from_str(&json)
            .map_err(|e| AuthError::StorageError(format!("Failed to parse token file: {}", e)))?;

        Ok(Some(token))
    }

    fn clear_token(&self) -> Result<(), AuthError> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)
                .map_err(|e| AuthError::StorageError(format!("Failed to remove token file: {}", e)))?;
        }
        Ok(())
    }
}

/// In-memory token store for testing.
pub struct MemoryTokenStore {
    token: std::sync::Mutex<Option<AuthToken>>,
}

impl MemoryTokenStore {
    /// Create a new in-memory token store.
    pub fn new() -> Self {
        Self {
            token: std::sync::Mutex::new(None),
        }
    }
}

impl Default for MemoryTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Concrete sync token provider backed by a TokenStore.
pub struct StoredTokenProvider<S: TokenStore> {
    store: S,
}

impl<S: TokenStore> StoredTokenProvider<S> {
    /// Create a new stored token provider.
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S: TokenStore + 'static> core_sync::scheduler::TokenProvider for StoredTokenProvider<S> {
    fn get_token(&self) -> Option<String> {
        self.store
            .load_token()
            .ok()
            .flatten()
            .map(|token| token.access_token)
    }
}

impl TokenStore for MemoryTokenStore {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_token() -> AuthToken {
        AuthToken {
            access_token: "access-123".to_string(),
            refresh_token: "refresh-456".to_string(),
            expires_in: 3600,
            user_id: "user-789".to_string(),
        }
    }

    // ─── MemoryTokenStore Tests ──────────────────────────────────────────

    #[test]
    fn test_memory_store_initially_empty() {
        let store = MemoryTokenStore::new();
        let token = store.load_token().unwrap();
        assert!(token.is_none());
    }

    #[test]
    fn test_memory_store_save_and_load() {
        let store = MemoryTokenStore::new();
        let token = make_test_token();

        store.save_token(&token).unwrap();
        let loaded = store.load_token().unwrap().unwrap();

        assert_eq!(loaded.access_token, "access-123");
        assert_eq!(loaded.refresh_token, "refresh-456");
        assert_eq!(loaded.expires_in, 3600);
        assert_eq!(loaded.user_id, "user-789");
    }

    #[test]
    fn test_memory_store_clear() {
        let store = MemoryTokenStore::new();
        store.save_token(&make_test_token()).unwrap();

        store.clear_token().unwrap();
        assert!(store.load_token().unwrap().is_none());
    }

    #[test]
    fn test_memory_store_overwrite() {
        let store = MemoryTokenStore::new();
        store.save_token(&make_test_token()).unwrap();

        let new_token = AuthToken {
            access_token: "new-access".to_string(),
            refresh_token: "new-refresh".to_string(),
            expires_in: 7200,
            user_id: "new-user".to_string(),
        };
        store.save_token(&new_token).unwrap();

        let loaded = store.load_token().unwrap().unwrap();
        assert_eq!(loaded.access_token, "new-access");
    }

    #[test]
    fn test_memory_store_default() {
        let store = MemoryTokenStore::default();
        assert!(store.load_token().unwrap().is_none());
    }

    #[test]
    fn test_stored_token_provider_reads_access_token() {
        let store = MemoryTokenStore::new();
        store.save_token(&make_test_token()).unwrap();

        let provider = StoredTokenProvider::new(store);
        assert_eq!(
            core_sync::scheduler::TokenProvider::get_token(&provider),
            Some("access-123".to_string())
        );
    }

    #[test]
    fn test_stored_token_provider_returns_none_when_empty() {
        let provider = StoredTokenProvider::new(MemoryTokenStore::new());
        assert_eq!(core_sync::scheduler::TokenProvider::get_token(&provider), None);
    }

    // ─── FileTokenStore Tests ────────────────────────────────────────────

    #[test]
    fn test_file_store_save_and_load() {
        let tmp_dir = std::env::temp_dir().join("subreader_test_token_store");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("token.json");

        let store = FileTokenStore::new(path.clone());
        let token = make_test_token();

        store.save_token(&token).unwrap();
        let loaded = store.load_token().unwrap().unwrap();

        assert_eq!(loaded.access_token, "access-123");
        assert_eq!(loaded.refresh_token, "refresh-456");
        assert_eq!(loaded.user_id, "user-789");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_file_store_load_nonexistent() {
        let path = std::env::temp_dir().join("subreader_nonexistent_token.json");
        let _ = std::fs::remove_file(&path); // Ensure it doesn't exist

        let store = FileTokenStore::new(path);
        let result = store.load_token().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_file_store_clear() {
        let tmp_dir = std::env::temp_dir().join("subreader_test_token_clear");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("token.json");

        let store = FileTokenStore::new(path.clone());
        store.save_token(&make_test_token()).unwrap();
        assert!(path.exists());

        store.clear_token().unwrap();
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_file_store_creates_parent_dirs() {
        let tmp_dir = std::env::temp_dir().join("subreader_test_nested_token");
        let path = tmp_dir.join("deep").join("nested").join("token.json");

        let store = FileTokenStore::new(path.clone());
        store.save_token(&make_test_token()).unwrap();
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_file_store_clear_nonexistent_is_ok() {
        let path = std::env::temp_dir().join("subreader_clear_nonexistent.json");
        let _ = std::fs::remove_file(&path);

        let store = FileTokenStore::new(path);
        let result = store.clear_token();
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_file_store_sets_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp_dir = std::env::temp_dir().join("subreader_test_token_permissions");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("token.json");

        let store = FileTokenStore::new(path.clone());
        store.save_token(&make_test_token()).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
