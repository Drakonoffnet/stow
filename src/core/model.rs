//! Core data model: jobs, statuses, configuration.

use std::path::PathBuf;

/// Job identifier (increases monotonically in the engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Destination chosen by the user. Extended for SSH/S3 (phase 2).
#[derive(Debug, Clone)]
pub enum DestinationSpec {
    /// Local or mounted network directory (SMB/NFS as a path).
    Local { dir: PathBuf },
}

/// Description of a single job: what to archive and where to place it.
#[derive(Debug, Clone)]
pub struct JobSpec {
    /// Source directory.
    pub source: PathBuf,
    /// Where to move the finished archive.
    pub destination: DestinationSpec,
    /// Remove the source after a successful archive and move.
    pub remove_source: bool,
    /// Compute the sha256 of the finished archive.
    pub checksum: bool,
}

/// Current state of a job.
#[derive(Debug, Clone)]
pub enum JobStatus {
    Queued,
    Archiving { done: u64, total: u64 },
    Transferring,
    Done {
        output: PathBuf,
        sha256: Option<String>,
    },
    Failed {
        error: String,
    },
    Canceled,
}

/// Engine parameters.
#[derive(Debug, Clone)]
pub struct Config {
    /// How many archives to process concurrently (defaults to the CPU count).
    pub jobs: usize,
    /// Deflate compression level (0..=9). Fixed at 6 in the MVP.
    pub compression_level: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            jobs: num_cpus::get().max(1),
            compression_level: 6,
        }
    }
}
