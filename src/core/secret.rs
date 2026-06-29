//! Secret storage backed by the OS keychain (macOS Keychain via `keyring`).
//!
//! Secrets (SSH passwords and key passphrases) are never kept in [`JobSpec`];
//! the spec carries only a keychain account reference, and the value is loaded
//! at upload time.

use crate::core::error::CoreError;

const SERVICE: &str = "file_transfer";

/// Store a secret under `account`, overwriting any existing value.
pub fn store(account: &str, secret: &str) -> Result<(), CoreError> {
    entry(account)?.set_password(secret).map_err(to_err)
}

/// Load the secret stored under `account`.
pub fn load(account: &str) -> Result<String, CoreError> {
    entry(account)?.get_password().map_err(to_err)
}

/// Delete the secret stored under `account`. Missing entries are ignored.
pub fn delete(account: &str) -> Result<(), CoreError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(to_err(e)),
    }
}

fn entry(account: &str) -> Result<keyring::Entry, CoreError> {
    keyring::Entry::new(SERVICE, account).map_err(to_err)
}

fn to_err(e: keyring::Error) -> CoreError {
    CoreError::Secret(e.to_string())
}
