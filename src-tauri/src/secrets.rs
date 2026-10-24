use crate::domain::*;
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc};

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> AppResult<Option<String>>;
    fn set(&self, name: &str, secret: &str) -> AppResult<()>;
    fn delete(&self, name: &str) -> AppResult<()>;
}

pub struct OsSecrets;

impl OsSecrets {
    pub fn initialize() -> Arc<dyn SecretStore> {
        #[cfg(target_os = "macos")]
        if let Ok(store) = apple_native_keyring_store::keychain::Store::new() {
            keyring_core::set_default_store(store);
        }
        #[cfg(target_os = "windows")]
        if let Ok(store) = windows_native_keyring_store::Store::new() {
            keyring_core::set_default_store(store);
        }
        #[cfg(target_os = "linux")]
        if let Ok(store) = dbus_secret_service_keyring_store::Store::new() {
            keyring_core::set_default_store(store);
        }
        Arc::new(Self)
    }

    fn entry(name: &str) -> AppResult<keyring_core::Entry> {
        keyring_core::Entry::new("com.quiinthieu.joinery", name).map_err(store_error)
    }
}

impl SecretStore for OsSecrets {
    fn get(&self, name: &str) -> AppResult<Option<String>> {
        match Self::entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(store_error(error)),
        }
    }
    fn set(&self, name: &str, secret: &str) -> AppResult<()> {
        Self::entry(name)?.set_password(secret).map_err(store_error)
    }
    fn delete(&self, name: &str) -> AppResult<()> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(store_error(error)),
        }
    }
}

fn store_error(_: keyring_core::Error) -> AppError {
    AppError::new(
        ErrorCode::CredentialStore,
        "Secure credential storage is unavailable or locked. Enter a password for this session, or unlock the system credential store.",
    )
}

// Explicit dependency injection for tests; this store is never selected by the desktop app.
pub struct MemorySecrets(Mutex<HashMap<String, String>>);
impl Default for MemorySecrets {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}
impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> AppResult<Option<String>> {
        Ok(self.0.lock().get(name).cloned())
    }
    fn set(&self, name: &str, secret: &str) -> AppResult<()> {
        self.0.lock().insert(name.into(), secret.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> AppResult<()> {
        self.0.lock().remove(name);
        Ok(())
    }
}
