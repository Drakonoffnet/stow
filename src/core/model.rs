//! Модель данных ядра: задачи, статусы, конфигурация.

use std::path::PathBuf;

/// Идентификатор задачи (монотонно растёт в движке).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Назначение, выбранное пользователем. Расширяется под SSH/S3 (этап 2).
#[derive(Debug, Clone)]
pub enum DestinationSpec {
    /// Локальная или смонтированная сетевая папка (SMB/NFS как путь).
    Local { dir: PathBuf },
}

/// Описание одной задачи: что упаковать и куда положить.
#[derive(Debug, Clone)]
pub struct JobSpec {
    /// Папка-источник.
    pub source: PathBuf,
    /// Куда переместить готовый архив.
    pub destination: DestinationSpec,
    /// Удалить источник после успешной упаковки и перемещения.
    pub remove_source: bool,
    /// Посчитать sha256 готового архива.
    pub checksum: bool,
}

/// Текущее состояние задачи.
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

/// Параметры движка.
#[derive(Debug, Clone)]
pub struct Config {
    /// Сколько архивов обрабатывать одновременно (по умолчанию = число ядер).
    pub jobs: usize,
    /// Уровень сжатия Deflate (0..=9). Фиксировано 6 в MVP.
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
