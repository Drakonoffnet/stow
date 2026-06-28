//! Движок: очередь задач, пул воркеров, события прогресса.
//!
//! GUI-слой никогда не блокируется: вся тяжёлая работа идёт в воркерах,
//! наружу отдаются только [`Event`]. После каждого события вызывается
//! `wake` — колбэк пробуждения интерфейса (в GUI это `ctx.request_repaint`).

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::core::archive;
use crate::core::error::CoreError;
use crate::core::model::{Config, JobId, JobSpec, JobStatus};
use crate::core::naming::archive_name;
use crate::core::transfer;

/// Команда движку (для каналоориентированных вызывающих).
#[derive(Debug)]
pub enum Command {
    Submit(JobSpec),
    Cancel(JobId),
    Shutdown,
}

/// Событие движка для GUI.
#[derive(Debug, Clone)]
pub enum Event {
    Accepted { id: JobId, source: std::path::PathBuf },
    Progress { id: JobId, status: JobStatus },
    Log { id: Option<JobId>, line: String },
    Finished { id: JobId, status: JobStatus },
}

/// Внутреннее представление задачи в очереди.
struct Job {
    id: JobId,
    spec: JobSpec,
    cancel: Arc<AtomicBool>,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Engine {
    job_tx: Option<Sender<Job>>,
    evt_tx: Sender<Event>,
    wake: Wake,
    next_id: AtomicU64,
    flags: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    workers: Vec<JoinHandle<()>>,
    level: u32,
}

impl Engine {
    /// Запустить движок: пул из `cfg.jobs` воркеров. События идут в `evt_tx`,
    /// `wake` вызывается после каждого события.
    pub fn start(cfg: Config, evt_tx: Sender<Event>, wake: Wake) -> Engine {
        let (job_tx, job_rx) = unbounded::<Job>();
        let mut workers = Vec::with_capacity(cfg.jobs);

        for _ in 0..cfg.jobs.max(1) {
            let job_rx: Receiver<Job> = job_rx.clone();
            let evt_tx = evt_tx.clone();
            let wake = wake.clone();
            let level = cfg.compression_level;
            workers.push(std::thread::spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    run_job(job, &evt_tx, &wake, level);
                }
            }));
        }

        Engine {
            job_tx: Some(job_tx),
            evt_tx,
            wake,
            next_id: AtomicU64::new(1),
            flags: Arc::new(Mutex::new(HashMap::new())),
            workers,
            level: cfg.compression_level,
        }
    }

    /// Поставить задачу в очередь. Возвращает её идентификатор.
    pub fn submit(&self, spec: JobSpec) -> JobId {
        let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let cancel = Arc::new(AtomicBool::new(false));
        self.flags.lock().unwrap().insert(id.0, cancel.clone());

        emit(
            &self.evt_tx,
            &self.wake,
            Event::Accepted {
                id,
                source: spec.source.clone(),
            },
        );

        if let Some(tx) = &self.job_tx {
            let _ = tx.send(Job { id, spec, cancel });
        }
        id
    }

    /// Отменить задачу (если ещё не завершена).
    pub fn cancel(&self, id: JobId) {
        if let Some(flag) = self.flags.lock().unwrap().get(&id.0) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Унифицированная обработка команды.
    pub fn handle(&self, cmd: Command) {
        match cmd {
            Command::Submit(spec) => {
                self.submit(spec);
            }
            Command::Cancel(id) => self.cancel(id),
            Command::Shutdown => { /* выполняется в Drop */ }
        }
    }

    /// Уровень сжатия движка.
    pub fn compression_level(&self) -> u32 {
        self.level
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Закрываем очередь — воркеры до-обрабатывают и выходят.
        self.job_tx.take();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// Выполнить одну задачу целиком.
fn run_job(job: Job, evt_tx: &Sender<Event>, wake: &Wake, level: u32) {
    let id = job.id;
    let result = process(&job, evt_tx, wake, level);

    let status = match result {
        Ok((output, sha256)) => JobStatus::Done { output, sha256 },
        Err(CoreError::Canceled) => JobStatus::Canceled,
        Err(e) => JobStatus::Failed {
            error: e.to_string(),
        },
    };
    if let JobStatus::Failed { error } = &status {
        emit(
            evt_tx,
            wake,
            Event::Log {
                id: Some(id),
                line: format!("{id} ошибка: {error}"),
            },
        );
    }
    emit(evt_tx, wake, Event::Finished { id, status });
}

fn process(
    job: &Job,
    evt_tx: &Sender<Event>,
    wake: &Wake,
    level: u32,
) -> Result<(std::path::PathBuf, Option<String>), CoreError> {
    let id = job.id;
    let source = &job.spec.source;

    let total = archive::count_files(source)?;
    emit(
        evt_tx,
        wake,
        Event::Progress {
            id,
            status: JobStatus::Archiving { done: 0, total },
        },
    );

    let name = archive_name(source, chrono::Local::now());
    let dest = transfer::build(&job.spec.destination);
    let staged = dest.stage(&name)?;

    // Сжатие с прогрессом по файлам.
    let done = AtomicU64::new(0);
    let archive_result = archive::archive_dir(source, &staged, level, &job.cancel, || {
        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
        emit(
            evt_tx,
            wake,
            Event::Progress {
                id,
                status: JobStatus::Archiving { done: d, total },
            },
        );
    });

    if let Err(e) = archive_result {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    if job.cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&staged);
        return Err(CoreError::Canceled);
    }

    // Контрольная сумма (опционально) — по временно́му файлу до перемещения.
    let sha256 = if job.spec.checksum {
        Some(compute_sha256(&staged)?)
    } else {
        None
    };

    emit(
        evt_tx,
        wake,
        Event::Progress {
            id,
            status: JobStatus::Transferring,
        },
    );
    let output = dest.finalize(&staged, &name)?;

    if job.spec.remove_source {
        std::fs::remove_dir_all(source)?;
    }

    emit(
        evt_tx,
        wake,
        Event::Log {
            id: Some(id),
            line: format!("{id} готово: {}", output.display()),
        },
    );
    Ok((output, sha256))
}

fn compute_sha256(path: &Path) -> Result<String, CoreError> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn emit(evt_tx: &Sender<Event>, wake: &Wake, event: Event) {
    let _ = evt_tx.send(event);
    wake();
}
