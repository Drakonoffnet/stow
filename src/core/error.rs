//! Типы ошибок ядра.

use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error("источник не является папкой: {0}")]
    NotADirectory(PathBuf),

    #[error("назначение уже содержит файл: {0}")]
    DestinationExists(String),

    #[error("операция отменена")]
    Canceled,
}
