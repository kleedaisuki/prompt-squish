//! Application-layer orchestration for `xmlsquish`.
//!
//! This crate deliberately knows nothing about the host filesystem or a
//! particular tokenizer. Those details are supplied through ports, leaving the
//! batch-processing policy deterministic and easy to test.

mod batch;
mod model;
mod ports;

pub use batch::BatchProcessor;
pub use model::{
    BatchReport, BatchStats, FailedFile, FileFailure, FileReport, ProcessingStage, SquishResult,
    output_path_for,
};
pub use ports::{FileStore, Squasher, TokenCounter};
