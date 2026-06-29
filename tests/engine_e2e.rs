//! End-to-end engine test: submit a job → progress events → finished archive
//! in the destination. Exercises the threading wiring that the GUI drives.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use file_transfer::core::engine::Event;
use file_transfer::{Config, DestinationSpec, Engine, JobSpec, JobStatus};

fn unique_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ft_e2e_{tag}_{}", std::process::id()))
}

#[test]
fn submit_produces_archive_in_destination() {
    let base = unique_dir("ok");
    let src = base.join("payload");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("one.txt"), "alpha").unwrap();
    std::fs::write(src.join("two.txt"), "beta").unwrap();
    let dest = base.join("out");

    let (tx, rx) = unbounded::<Event>();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let engine = Engine::start(Config::default(), tx, wake);

    let id = engine.submit(JobSpec {
        source: src.clone(),
        destination: DestinationSpec::Local { dir: dest.clone() },
        remove_source: false,
        checksum: true,
    });

    // Wait for Finished for our job (with a timeout).
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = None;
    let mut sha = None;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Event::Finished { id: fid, status }) if fid == id => {
                match status {
                    JobStatus::Done { output: o, sha256 } => {
                        output = Some(o);
                        sha = sha256;
                    }
                    other => panic!("expected Done, got: {other:?}"),
                }
                break;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }

    let output = output.expect("the job did not finish within the timeout");
    assert!(output.exists(), "archive was not created: {}", output.display());
    assert!(output.extension().unwrap() == "zip");
    assert!(src.exists(), "the source must not be removed");
    let sha = sha.expect("sha256 was requested but is missing");
    assert_eq!(sha.len(), 64, "sha256 must be 64 hex characters");

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn missing_source_reports_failure() {
    let base = unique_dir("fail");
    let dest = base.join("out");

    let (tx, rx) = unbounded::<Event>();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let engine = Engine::start(Config::default(), tx, wake);

    let id = engine.submit(JobSpec {
        source: base.join("does_not_exist"),
        destination: DestinationSpec::Local { dir: dest },
        remove_source: false,
        checksum: false,
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut failed = false;
    while Instant::now() < deadline {
        if let Ok(Event::Finished { id: fid, status }) =
            rx.recv_timeout(Duration::from_millis(500))
        {
            if fid == id {
                failed = matches!(status, JobStatus::Failed { .. });
                break;
            }
        }
    }
    assert!(failed, "a missing source must produce Failed");

    std::fs::remove_dir_all(&base).ok();
}
