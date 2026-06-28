//! Сквозной тест движка: постановка задачи → события прогресса → готовый
//! архив в назначении. Проверяет потоковую обвязку, которую запускает GUI.

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

    // Ждём Finished для нашей задачи (с таймаутом).
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
                    other => panic!("ожидался Done, получено: {other:?}"),
                }
                break;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }

    let output = output.expect("задача не завершилась за таймаут");
    assert!(output.exists(), "архив не создан: {}", output.display());
    assert!(output.extension().unwrap() == "zip");
    assert!(src.exists(), "источник не должен удаляться");
    let sha = sha.expect("sha256 запрошен, но отсутствует");
    assert_eq!(sha.len(), 64, "sha256 должен быть 64 hex-символа");

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
    assert!(failed, "несуществующий источник должен дать Failed");

    std::fs::remove_dir_all(&base).ok();
}
