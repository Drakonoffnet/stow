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
}
