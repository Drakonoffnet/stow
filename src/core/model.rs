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

/// Destination chosen by the user. Extended for S3 (phase 2).
#[derive(Debug, Clone)]
pub enum DestinationSpec {
    /// Local or mounted network directory (SMB/NFS as a path).
    Local { dir: PathBuf },
    /// Remote directory reached over SSH/SFTP.
    Ssh(SshConfig),
}

/// Connection details for an SSH/SFTP destination.
#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Absolute remote directory for the finished archives.
    pub remote_dir: String,
    pub auth: SshAuth,
}

/// SSH authentication method. Secrets live in the keychain, not here — the
/// variants carry only the keychain account reference.
#[derive(Debug, Clone)]
pub enum SshAuth {
    /// Use the running SSH agent.
    Agent,
    /// Password authentication; the password is stored under this account.
    Password { keychain_account: String },
    /// Public-key authentication from a key file, with an optional passphrase
    /// stored under `passphrase_account`.
    Key {
        path: PathBuf,
        passphrase_account: Option<String>,
    },
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

impl JobStatus {
    /// Whether the job has reached a terminal state.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            JobStatus::Done { .. } | JobStatus::Failed { .. } | JobStatus::Canceled
        )
    }

    /// Whether the job is still in progress and can be canceled.
    pub fn is_active(&self) -> bool {
        !self.is_finished()
    }
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
