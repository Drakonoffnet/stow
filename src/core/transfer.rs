//! Destination backends.
//!
//! The [`Destination`] contract is uniform across backends: first obtain a
//! local path for the temporary write ([`Destination::stage`]), then finalize
//! atomically ([`Destination::finalize`]). [`LocalDestination`] renames within
//! the target directory; [`SshDestination`] uploads over SFTP and renames on
//! the remote.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use ssh2::Session;

use crate::core::error::CoreError;
use crate::core::model::{DestinationSpec, SshAuth, SshConfig};
use crate::core::secret;

pub trait Destination: Send + Sync {
    /// Local path to write the archive to while packing.
    fn stage(&self, name: &str) -> Result<PathBuf, CoreError>;

    /// Move the finished temporary archive `staged` to its final location under
    /// the name `name`. Returns a display path. Overwriting is forbidden.
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

/// Remote directory reached over SSH/SFTP.
pub struct SshDestination {
    pub cfg: SshConfig,
}

impl Destination for SshDestination {
    fn stage(&self, name: &str) -> Result<PathBuf, CoreError> {
        // The archive is built locally first, then uploaded in `finalize`.
        let tmp = std::env::temp_dir().join(format!(".ft-{name}.part"));
        if tmp.exists() {
            std::fs::remove_file(&tmp)?;
        }
        Ok(tmp)
    }

    fn finalize(&self, staged: &Path, name: &str) -> Result<PathBuf, CoreError> {
        let session = self.connect()?;
        let sftp = session.sftp().map_err(ssh_err)?;

        let dir = self.cfg.remote_dir.trim_end_matches('/');
        ensure_remote_dir(&sftp, dir)?;

        let final_remote = format!("{dir}/{name}");
        if sftp.stat(Path::new(&final_remote)).is_ok() {
            return Err(CoreError::DestinationExists(name.to_string()));
        }

        // Upload to a temporary remote name, then rename — atomic on the remote.
        let tmp_remote = format!("{dir}/.{name}.part");
        upload(&sftp, staged, &tmp_remote)?;
        sftp.rename(Path::new(&tmp_remote), Path::new(&final_remote), None)
            .map_err(ssh_err)?;

        // The local staging file is no longer needed.
        let _ = std::fs::remove_file(staged);

        Ok(PathBuf::from(format!(
            "ssh://{}@{}:{}{}",
            self.cfg.user, self.cfg.host, self.cfg.port, final_remote
        )))
    }
}

impl SshDestination {
    /// Open a TCP connection, perform the handshake, verify the host key, and
    /// authenticate.
    fn connect(&self) -> Result<Session, CoreError> {
        let tcp = TcpStream::connect((self.cfg.host.as_str(), self.cfg.port))?;
        let mut session = Session::new().map_err(ssh_err)?;
        session.set_tcp_stream(tcp);
        session.handshake().map_err(ssh_err)?;

        self.verify_host_key(&session)?;
        self.authenticate(&session)?;

        if !session.authenticated() {
            return Err(CoreError::Auth("server rejected the credentials".into()));
        }
        Ok(session)
    }

    /// Verify the host key against `~/.ssh/known_hosts`.
    ///
    /// A mismatch is a hard error (possible man-in-the-middle). An unknown host
    /// is accepted for this session without persisting the key (trust on first
    /// use), which keeps the user's `known_hosts` untouched.
    fn verify_host_key(&self, session: &Session) -> Result<(), CoreError> {
        let (key, _kind) = match session.host_key() {
            Some(k) => k,
            None => return Err(CoreError::Ssh("no host key offered".into())),
        };

        let mut known = session.known_hosts().map_err(ssh_err)?;
        if let Some(home) = std::env::var_os("HOME") {
            let path = Path::new(&home).join(".ssh/known_hosts");
            if path.exists() {
                let _ = known.read_file(&path, ssh2::KnownHostFileKind::OpenSSH);
            }
        }

        use ssh2::CheckResult;
        match known.check_port(&self.cfg.host, self.cfg.port, key) {
            CheckResult::Match => Ok(()),
            CheckResult::NotFound | CheckResult::Failure => Ok(()), // trust on first use
            CheckResult::Mismatch => Err(CoreError::Ssh(format!(
                "host key mismatch for {} — possible man-in-the-middle",
                self.cfg.host
            ))),
        }
    }

    fn authenticate(&self, session: &Session) -> Result<(), CoreError> {
        let user = &self.cfg.user;
        match &self.cfg.auth {
            SshAuth::Agent => session.userauth_agent(user).map_err(ssh_err),
            SshAuth::Password { keychain_account } => {
                let password = secret::load(keychain_account)?;
                session.userauth_password(user, &password).map_err(ssh_err)
            }
            SshAuth::Key {
                path,
                passphrase_account,
            } => {
                let passphrase = match passphrase_account {
                    Some(acc) => Some(secret::load(acc)?),
                    None => None,
                };
                session
                    .userauth_pubkey_file(user, None, path, passphrase.as_deref())
                    .map_err(ssh_err)
            }
        }
    }
}

/// Create the remote directory tree, ignoring components that already exist.
fn ensure_remote_dir(sftp: &ssh2::Sftp, dir: &str) -> Result<(), CoreError> {
    let mut path = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        path.push('/');
        path.push_str(part);
        // mkdir fails if the directory exists; that case is fine.
        let _ = sftp.mkdir(Path::new(&path), 0o755);
    }
    // Confirm the target is reachable.
    sftp.stat(Path::new(dir))
        .map(|_| ())
        .map_err(|e| CoreError::Ssh(format!("remote directory {dir} unavailable: {e}")))
}

/// Stream a local file to the remote path over SFTP.
fn upload(sftp: &ssh2::Sftp, local: &Path, remote: &str) -> Result<(), CoreError> {
    let mut src = std::fs::File::open(local)?;
    let mut dst = sftp.create(Path::new(remote)).map_err(ssh_err)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(ssh_err)?;
    }
    Ok(())
}

fn ssh_err<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Ssh(e.to_string())
}

/// Build a destination backend from its specification.
pub fn build(spec: &DestinationSpec) -> Box<dyn Destination> {
    match spec {
        DestinationSpec::Local { dir } => Box::new(LocalDestination { dir: dir.clone() }),
        DestinationSpec::Ssh(cfg) => Box::new(SshDestination { cfg: cfg.clone() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_spec() -> DestinationSpec {
        DestinationSpec::Ssh(SshConfig {
            host: "example.com".into(),
            port: 22,
            user: "alice".into(),
            remote_dir: "/srv/archives".into(),
            auth: SshAuth::Agent,
        })
    }

    #[test]
    fn build_returns_ssh_backend() {
        // Construction must not open a connection; staging stays local.
        let dest = build(&ssh_spec());
        let staged = dest.stage("data_2026-06-28_120000.zip").unwrap();
        assert!(staged.starts_with(std::env::temp_dir()));
        assert!(staged
            .to_string_lossy()
            .ends_with("data_2026-06-28_120000.zip.part"));
    }

    #[test]
    fn local_finalize_rejects_overwrite() {
        let dir = std::env::temp_dir().join(format!("ft_local_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = LocalDestination { dir: dir.clone() };

        let staged = dest.stage("a.zip").unwrap();
        std::fs::write(&staged, b"payload").unwrap();
        let out = dest.finalize(&staged, "a.zip").unwrap();
        assert!(out.exists());

        // A second archive with the same name must not overwrite the first.
        let staged2 = dest.stage("a.zip").unwrap();
        std::fs::write(&staged2, b"other").unwrap();
        let err = dest.finalize(&staged2, "a.zip");
        assert!(matches!(err, Err(CoreError::DestinationExists(_))));

        std::fs::remove_dir_all(&dir).ok();
    }
}
