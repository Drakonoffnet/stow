//! Core of `file_transfer`: archiving (parallel zip) plus delivery to a destination.
//!
//! The layer does not depend on the GUI — it communicates with the interface
//! through the channels ([`Command`]/[`Event`]) of the [`engine::Engine`].

pub mod core;

pub use crate::core::engine::{Command, Engine, Event};
pub use crate::core::error::CoreError;
pub use crate::core::model::{
    Config, DestinationSpec, JobId, JobSpec, JobStatus,
};
