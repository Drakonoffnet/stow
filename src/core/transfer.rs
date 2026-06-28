//! Бэкенды назначения. MVP: локальная/сетевая папка.
//!
//! Контракт [`Destination`] одинаков для будущих SSH/S3 (этап 2): сначала
//! получить путь для временной записи ([`Destination::stage`]), затем
//! атомарно финализировать ([`Destination::finalize`]).

use std::path::{Path, PathBuf};

use crate::core::error::CoreError;
use crate::core::model::DestinationSpec;

pub trait Destination: Send + Sync {
    /// Путь, куда писать архив на время упаковки (рядом с финальным —
    /// чтобы финализация была атомарным переименованием).
    fn stage(&self, name: &str) -> Result<PathBuf, CoreError>;

    /// Переместить готовый временный архив `staged` в финальное место под
    /// именем `name`. Возвращает финальный путь. Перезапись запрещена.
    fn finalize(&self, staged: &Path, name: &str) -> Result<PathBuf, CoreError>;
}

/// Локальная или смонтированная сетевая папка.
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
        // На одном томе rename атомарен — архив появляется целиком.
        std::fs::rename(staged, &final_path)?;
        Ok(final_path)
    }
}

/// Построить бэкенд назначения по спецификации.
pub fn build(spec: &DestinationSpec) -> Box<dyn Destination> {
    match spec {
        DestinationSpec::Local { dir } => {
            Box::new(LocalDestination { dir: dir.clone() })
        }
    }
}
