//! Core error types.

use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("source is not a directory: {0}")]
    NotADirectory(PathBuf),

    #[error("destination already contains the file: {0}")]
    DestinationExists(String),

    #[error("operation canceled")]
    Canceled,

    #[error("SSH error: {0}")]
    Ssh(String),

    #[error("SSH authentication failed: {0}")]
    Auth(String),

    #[error("keychain error: {0}")]
    Secret(String),
}
