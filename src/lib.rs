//! Ядро `file_transfer`: архивация (параллельный zip) + перемещение в назначение.
//!
//! Слой не зависит от GUI — общение с интерфейсом идёт через каналы
//! ([`Command`]/[`Event`]) движка [`engine::Engine`].

pub mod core;

pub use crate::core::engine::{Command, Engine, Event};
pub use crate::core::error::CoreError;
pub use crate::core::model::{
    Config, DestinationSpec, JobId, JobSpec, JobStatus,
};
