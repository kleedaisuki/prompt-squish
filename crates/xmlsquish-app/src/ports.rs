use std::path::Path;

use crate::{FileFailure, SquishResult};

/// Secondary port for reading and persisting XML documents.
pub trait FileStore {
    fn read(&self, path: &Path) -> Result<String, FileFailure>;

    /// Writes a completed document. Production adapters may implement this as
    /// an atomic replace to avoid exposing a partial output file.
    fn write(&self, path: &Path, contents: &str) -> Result<(), FileFailure>;

    /// Discards adapter state reserved for a future write when processing stops
    /// after a successful read. Stateless stores need no special handling.
    fn discard_pending_write(&self, _path: &Path) {}
}

/// Primary text-transformation port, normally backed by the FSM in the core crate.
pub trait Squasher {
    fn squish(&self, input: &str) -> Result<SquishResult, FileFailure>;
}

/// Token measurement port. Tokenization policy is intentionally not part of the
/// application layer and may vary with the target model.
pub trait TokenCounter {
    fn count(&self, text: &str) -> Result<u64, FileFailure>;
}
