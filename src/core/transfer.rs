//! Destination backends. MVP: local/network directory.
//!
//! The [`Destination`] contract is the same for future SSH/S3 backends
//! (phase 2): first obtain a path for the temporary write
//! ([`Destination::stage`]), then finalize atomically
//! ([`Destination::finalize`]).

use std::path::{Path, PathBuf};

use crate::core::error::CoreError;
use crate::core::model::DestinationSpec;

pub trait Destination: Send + Sync {
    /// Path to write the archive to while packing (next to the final path so
    /// that finalization is an atomic rename).
    fn stage(&self, name: &str) -> Result<PathBuf, CoreError>;

    /// Move the finished temporary archive `staged` to its final location under
    /// the name `name`. Returns the final path. Overwriting is forbidden.
    fn finalize(&self, staged: &Path, name: &str) -> Result<PathBuf, CoreError>;
}

/// Local or mounted network directory.
pub struct LocalDestination {
    pub dir: PathBuf,
}

impl Destination for LocalDestination {
    fn stage(&self, name: &str) -> Result<PathBuf, CoreError> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".{name}.part"));
        if tmp.exists() {
            std::fs::remove_file(&tmp)?;
        }
        Ok(tmp)
    }

    fn finalize(&self, staged: &Path, name: &str) -> Result<PathBuf, CoreError> {
        let final_path = self.dir.join(name);
        if final_path.exists() {
            return Err(CoreError::DestinationExists(name.to_string()));
        }
        // On a single volume the rename is atomic — the archive appears whole.
        std::fs::rename(staged, &final_path)?;
        Ok(final_path)
    }
}

/// Build a destination backend from its specification.
pub fn build(spec: &DestinationSpec) -> Box<dyn Destination> {
    match spec {
        DestinationSpec::Local { dir } => {
            Box::new(LocalDestination { dir: dir.clone() })
        }
    }
}
