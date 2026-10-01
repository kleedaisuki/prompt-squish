//! Hosted mechanism probe: operation counts only, never normal-path latency claims.
//!
//! Run with a project-local scratch directory, for example:
//! `cargo run -p squish-store --example io_probe --release -- .temp/store-io-probe`.

use std::{path::PathBuf, sync::Arc};

use squish_build::{
    ActionIndex, ActionKey, ActionRecord, OutputName, ProducedOutput, VerifiedBlob,
};
use squish_protocol::ArtifactKind;
use squish_store::{BlobSessionLimits, Cas, CasSession, VerifiedActionIndex};

/// Compares real verified-acquisition and advisory-transaction counts with fixed byte oracles.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.temp/store-io-probe")
        });
    let cas = Arc::new(Cas::open(root.join("cas"))?);
    let small = VerifiedBlob::from_owned(b"immutable IO probe".to_vec());
    let small_digest = cas.put_verified(&small)?;
    let fresh = CasSession::with_limits(
        Arc::clone(&cas),
        BlobSessionLimits {
            max_bytes: 0,
            max_entries: 0,
        },
    );
    let cached = CasSession::new(Arc::clone(&cas));
    for _ in 0..16 {
        assert_eq!(fresh.acquire(small_digest)?.unwrap().bytes(), small.bytes());
        assert_eq!(
            cached.acquire(small_digest)?.unwrap().bytes(),
            small.bytes()
        );
    }
    let fresh_stats = fresh.stats();
    let cached_stats = cached.stats();
    assert_eq!(fresh_stats.acquisition_reads, 16);
    assert_eq!(cached_stats.acquisition_reads, 1);
    assert_eq!(cached_stats.acquisition_hashes, 1);
    assert_eq!(cached_stats.memory_hits, 15);

    let large_bytes = vec![0x5a; 8 * 1024 * 1024 + 1];
    let original_pointer = large_bytes.as_ptr();
    let large = VerifiedBlob::from_owned(large_bytes);
    assert_eq!(large.bytes().as_ptr(), original_pointer);
    cas.put_verified(&large)?;
    let no_retention = Arc::new(CasSession::with_limits(
        Arc::clone(&cas),
        BlobSessionLimits {
            max_bytes: 0,
            max_entries: 0,
        },
    ));
    let index = VerifiedActionIndex::open_in_session(
        root.join("actions.sqlite3"),
        Arc::clone(&no_retention),
    )?;
    let key = ActionKey::new(format!(
        "blake3:{}",
        blake3::hash(b"oversized explicit handoff").to_hex()
    ))?;
    let output = ProducedOutput {
        name: OutputName::new("large")?,
        kind: ArtifactKind::Prompt,
        digest: large.digest().clone(),
        size: large.bytes().len() as u64,
    };
    let mut same_bytes = output.clone();
    same_bytes.name = OutputName::new("same-bytes")?;
    index.record(&ActionRecord {
        key: key.clone(),
        outputs: vec![output, same_bytes],
    })?;
    let verified = index.lookup_verified(&key)?.unwrap();
    assert_eq!(verified.blobs[0].bytes(), large.bytes());
    assert_eq!(
        verified.blobs[0].bytes().as_ptr(),
        verified.blobs[1].bytes().as_ptr()
    );
    let lookup_stats = no_retention.stats();
    assert_eq!(lookup_stats.acquisition_reads, 1);
    assert_eq!(lookup_stats.retained_bytes, 0);
    index.flush_touches()?;

    // A separate index isolates advisory batches from output acquisition and publication.
    let touch_index = VerifiedActionIndex::open_in_session(
        root.join("touches.sqlite3"),
        Arc::new(CasSession::new(cas)),
    )?;
    let mut keys = Vec::new();
    for ordinal in 0..70 {
        let key = ActionKey::new(format!(
            "blake3:{}",
            blake3::hash(format!("touch-{ordinal}").as_bytes()).to_hex()
        ))?;
        touch_index.record(&ActionRecord {
            key: key.clone(),
            outputs: Vec::new(),
        })?;
        keys.push(key);
    }
    for key in &keys {
        assert!(touch_index.lookup_verified(key)?.is_some());
    }
    touch_index.flush_touches()?;
    let touches = touch_index.touch_stats();
    assert_eq!(touches.transactions, 2);

    println!(
        "{{\"schema\":\"xmlsquish.store-io-probe.v1\",\"fresh_acquisition_reads\":{},\"cached_acquisition_reads\":{},\"cached_acquisition_hashes\":{},\"cached_memory_hits\":{},\"zero_budget_lookup_reads\":{},\"zero_budget_retained_bytes\":{},\"oversized_bytes\":{},\"owned_allocation_preserved\":true,\"touch_transactions\":{},\"touch_changed_rows\":{}}}",
        fresh_stats.acquisition_reads,
        cached_stats.acquisition_reads,
        cached_stats.acquisition_hashes,
        cached_stats.memory_hits,
        lookup_stats.acquisition_reads,
        lookup_stats.retained_bytes,
        large.bytes().len(),
        touches.transactions,
        touches.rows
    );
    Ok(())
}
