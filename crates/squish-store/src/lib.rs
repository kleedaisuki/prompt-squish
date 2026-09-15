//! prompt-squish 的产物与缓存存储领域。 / Artifact and cache storage domain for prompt-squish.
//!
//! 此 crate 提供不可变内容寻址存储（CAS）以及可丢弃、可重建的动作索引适配器。
//! CAS 是 blob 的权威来源；SQLite 仅保存小型协调元数据，绝不保存大 blob。
//! This crate provides an immutable content-addressed store (CAS) and disposable,
//! rebuildable action-index adapters. The CAS is authoritative for blobs; SQLite
//! contains only small coordination metadata and never large blobs.

#![forbid(unsafe_code)]

mod action;
mod cas;

pub use action::{
    ActionEntry, ActionKey, BuildActionIndex, IndexError, MemoryActionIndex, RunEvent,
    SqliteActionIndex, VerifiedActionIndex,
};
pub use cas::{BlobDigest, Cas, CasError, CasEvent, CasEventKind, CasObserver, NoopObserver};
