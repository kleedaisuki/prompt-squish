//! Bounded invocation-scoped retention of verified immutable CAS buffers.

use std::{
    collections::HashMap,
    io::{self, Read, Write},
    sync::{Arc, Condvar, Mutex, Weak},
};

use squish_build::{BlobStore, ContentDigest, VerifiedBlob};

use crate::cas::{AcquisitionCounts, build_digest};
use crate::{BlobDigest, Cas, CasError};

/// Bound for active duplicate-read coordination, not persistent byte retention.
const MAX_IN_FLIGHTS: usize = 256;

/// Limits for hidden invocation-local retention, independent of caller-owned live handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobSessionLimits {
    /// Maximum canonical payload bytes retained; explicit compatibility views are caller costs.
    pub max_bytes: usize,
    /// Maximum count of strongly retained blobs, including empty blobs.
    pub max_entries: usize,
}

impl Default for BlobSessionLimits {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024,
            max_entries: 256,
        }
    }
}

/// Mechanism counters, not wall-time or syscall-latency measurements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BlobSessionStats {
    /// Successful physical file reads during acquisitions, including corrupt files.
    pub acquisition_reads: u64,
    /// Payload bytes read and hashed at acquisition boundaries, including corrupt files.
    pub acquisition_bytes: u64,
    /// Full-buffer BLAKE3 computations during acquisitions.
    pub acquisition_hashes: u64,
    /// Acquisitions satisfied by an immutable retained handle.
    pub memory_hits: u64,
    /// Overlapping acquisitions served by the same in-flight verified read.
    pub coalesced_hits: u64,
    /// New distinct reads executed without coordination when the bounded registry is full.
    pub coalescing_bypasses: u64,
    /// Successful durable writes/reuses through `put_verified`, excluding input hashing.
    pub verified_puts: u64,
    /// Number of handles currently retained by the bounded session cache.
    pub retained_entries: usize,
    /// Total payload bytes currently retained by the bounded session cache.
    pub retained_bytes: usize,
}

/// A verified buffer plus advisory in-memory recency used only for bounded eviction.
struct CachedBlob {
    blob: VerifiedBlob,
    used: u64,
}

/// Retained immutable snapshots and work counters; no filesystem metadata is trusted.
#[derive(Default)]
struct State {
    entries: HashMap<BlobDigest, CachedBlob>,
    clock: u64,
    bytes: usize,
    stats: BlobSessionStats,
    /// Weak active-flight references: no completed bytes are kept alive by coordination.
    flights: HashMap<BlobDigest, Weak<Flight>>,
}

/// Shares verified byte identity within one invocation, never across commands.
///
/// Fresh reads retain the CAS corruption boundary. Once acquired, a handle is an immutable
/// snapshot even if its backing file changes; a new session detects that change. Missing or
/// corrupt files are not negatively cached. Callers must create a new session for each
/// invocation and must not use it to grant trust to mutable source or generation files.
///
/// Retention is bounded by bytes and entries. Oversized/evicted outputs require explicit
/// caller-owned handles (for example `VerifiedAction.blobs`) to avoid subsequent reads.
/// Cache misses perform disk I/O outside the state mutex. Overlapping reads of one digest
/// share a bounded in-flight result; unrelated digests never wait on one global I/O lock.
/// The registry holds only weak coordination references, never persistent trust or bytes.
///
/// # Example
/// ```
/// use std::sync::Arc;
/// use squish_build::VerifiedBlob;
/// use squish_store::{Cas, CasSession};
/// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp/session-doctest");
/// let cas = Arc::new(Cas::open(&root)?);
/// let session = CasSession::new(cas);
/// let input = VerifiedBlob::from_owned(b"shared immutable bytes".to_vec());
/// let digest = session.put_verified(&input)?;
/// let output = session.acquire(digest)?.expect("published input exists");
/// assert_eq!(input.digest(), output.digest());
/// # Ok::<(), squish_store::CasError>(())
/// ```
pub struct CasSession {
    cas: Arc<Cas>,
    limits: BlobSessionLimits,
    state: Mutex<State>,
    /// A deterministic test-only gate before the real disk read, not a fake verifier.
    #[cfg(test)]
    before_read: Mutex<Option<Arc<std::sync::Barrier>>>,
}

impl CasSession {
    /// Creates a new invocation using the default 8 MiB / 256-entry retention budget.
    #[must_use]
    pub fn new(cas: Arc<Cas>) -> Self {
        Self::with_limits(cas, BlobSessionLimits::default())
    }

    /// Creates a new invocation with explicit bounds; either zero bound disables retention.
    #[must_use]
    pub fn with_limits(cas: Arc<Cas>, limits: BlobSessionLimits) -> Self {
        Self {
            cas,
            limits,
            state: Mutex::new(State::default()),
            #[cfg(test)]
            before_read: Mutex::new(None),
        }
    }

    /// Returns the authoritative persistent store without granting invocation-local trust.
    #[must_use]
    pub fn cas(&self) -> &Arc<Cas> {
        &self.cas
    }

    /// Acquires verified immutable bytes; retained hits do not read or hash a file again.
    pub fn acquire(&self, digest: BlobDigest) -> Result<Option<VerifiedBlob>, CasError> {
        match self.begin_acquisition(digest) {
            Acquisition::Cached(blob) => Ok(Some(blob)),
            Acquisition::Wait(flight) => flight.wait(),
            Acquisition::Read(leader) => self.read_acquisition(digest, leader),
        }
    }

    /// Performs the actual disk/hash boundary, then publishes its result to overlapping readers.
    fn read_acquisition(
        &self,
        digest: BlobDigest,
        mut leader: FlightLeader,
    ) -> Result<Option<VerifiedBlob>, CasError> {
        #[cfg(test)]
        {
            let gate = self.before_read.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.wait();
            }
        }
        let mut counts = AcquisitionCounts::default();
        let result = self.cas.acquire_counted(digest, &mut counts);
        self.record_acquisition(digest, &counts, &result);
        leader.complete(&result);
        result
    }

    /// Aggregates real acquisition work and retains only valid buffers within the byte budget.
    fn record_acquisition(
        &self,
        digest: BlobDigest,
        counts: &AcquisitionCounts,
        result: &Result<Option<VerifiedBlob>, CasError>,
    ) {
        let mut state = self.state.lock().expect("CAS session poisoned");
        state.stats.acquisition_reads += counts.reads;
        state.stats.acquisition_bytes += counts.bytes;
        state.stats.acquisition_hashes += counts.hashes;
        if let Ok(Some(blob)) = result {
            self.retain(&mut state, digest, blob);
        }
    }

    /// Selects retained, overlapping, or new acquisition without holding a lock across I/O.
    fn begin_acquisition(&self, digest: BlobDigest) -> Acquisition {
        let mut state = self.state.lock().expect("CAS session poisoned");
        state.clock = state.clock.saturating_add(1);
        let used = state.clock;
        if let Some(entry) = state.entries.get_mut(&digest) {
            entry.used = used;
            let blob = entry.blob.clone();
            state.stats.memory_hits += 1;
            return Acquisition::Cached(blob);
        }
        if let Some(flight) = state.flights.get(&digest).and_then(Weak::upgrade) {
            state.stats.coalesced_hits += 1;
            return Acquisition::Wait(flight);
        }
        state.flights.retain(|_, flight| flight.strong_count() > 0);
        if state.flights.len() >= MAX_IN_FLIGHTS {
            state.stats.coalescing_bypasses += 1;
            return Acquisition::Read(FlightLeader { flight: None });
        }
        let flight = Arc::new(Flight::default());
        state.flights.insert(digest, Arc::downgrade(&flight));
        Acquisition::Read(FlightLeader {
            flight: Some(flight),
        })
    }

    /// Durably stores a sealed buffer and retains its handle within the invocation budget.
    ///
    /// Durable existing-file validation and publication barriers are never bypassed. Only
    /// hashing of the already-verified input buffer is removed; underlying put I/O is not
    /// included in acquisition counters and should be measured separately by syscall probes.
    pub fn put_verified(&self, blob: &VerifiedBlob) -> Result<BlobDigest, CasError> {
        let digest = self.cas.put_verified(blob)?;
        let mut state = self.state.lock().expect("CAS session poisoned");
        state.stats.verified_puts += 1;
        self.retain(&mut state, digest, blob);
        Ok(digest)
    }

    /// Returns counters plus the current bounded retained payload size.
    #[must_use]
    pub fn stats(&self) -> BlobSessionStats {
        let state = self.state.lock().expect("CAS session poisoned");
        BlobSessionStats {
            retained_entries: state.entries.len(),
            retained_bytes: state.bytes,
            ..state.stats
        }
    }

    /// Admits a verified snapshot, evicting least-recently-used retained handles as needed.
    fn retain(&self, state: &mut State, digest: BlobDigest, blob: &VerifiedBlob) {
        let size = blob.bytes().len();
        if self.limits.max_entries == 0
            || size > self.limits.max_bytes
            || self.limits.max_bytes == 0
        {
            return;
        }
        state.clock = state.clock.saturating_add(1);
        if let Some(entry) = state.entries.get_mut(&digest) {
            entry.used = state.clock;
            return;
        }
        while state.entries.len() >= self.limits.max_entries
            || state.bytes > self.limits.max_bytes - size
        {
            let Some(key) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            let removed = state
                .entries
                .remove(&key)
                .expect("selected retained entry exists");
            state.bytes -= removed.blob.bytes().len();
        }
        state.bytes += size;
        state.entries.insert(
            digest,
            CachedBlob {
                blob: blob.clone(),
                used: state.clock,
            },
        );
    }
}

/// An immutable retained hit, an overlapping read, or responsibility for a fresh read.
enum Acquisition {
    Cached(VerifiedBlob),
    Wait(Arc<Flight>),
    Read(FlightLeader),
}

/// Cloneable I/O failure metadata; waiters preserve OS codes without sharing mutable errors.
#[derive(Clone)]
enum SharedFailure {
    Io {
        kind: io::ErrorKind,
        raw: Option<i32>,
        message: String,
    },
    InvalidDigest,
    UnsupportedAlgorithm,
}

impl SharedFailure {
    /// Captures only the immutable diagnostic identity required to reconstruct a CAS failure.
    fn capture(error: &CasError) -> Self {
        match error {
            CasError::Io(error) => Self::Io {
                kind: error.kind(),
                raw: error.raw_os_error(),
                message: error.to_string(),
            },
            CasError::InvalidDigest => Self::InvalidDigest,
            CasError::UnsupportedAlgorithm => Self::UnsupportedAlgorithm,
        }
    }

    /// Reconstructs the original error class, retaining raw OS identity when available.
    fn restore(&self) -> CasError {
        match self {
            Self::Io { kind, raw, message } => CasError::Io(raw.map_or_else(
                || io::Error::new(*kind, message.clone()),
                io::Error::from_raw_os_error,
            )),
            Self::InvalidDigest => CasError::InvalidDigest,
            Self::UnsupportedAlgorithm => CasError::UnsupportedAlgorithm,
        }
    }
}

/// An immutable byte result or cloneable failure, shared only by overlapping acquisitions.
type SharedResult = Result<Option<VerifiedBlob>, SharedFailure>;

/// Coordination owns a result only while actual overlapping acquisitions are alive.
#[derive(Default)]
struct Flight {
    result: Mutex<Option<SharedResult>>,
    ready: Condvar,
    /// Test-only evidence that interruption wakes an actually blocked condition-variable reader.
    #[cfg(test)]
    waiting: std::sync::atomic::AtomicUsize,
}

impl Flight {
    /// Publishes one complete immutable result, waking every waiter without a session lock.
    fn publish(&self, result: &Result<Option<VerifiedBlob>, CasError>) {
        let shared = result
            .as_ref()
            .map(|blob| blob.clone())
            .map_err(SharedFailure::capture);
        *self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(shared);
        self.ready.notify_all();
    }

    /// Blocks only this digest; successful waiters clone the verified authority, not bytes.
    fn wait(&self) -> Result<Option<VerifiedBlob>, CasError> {
        let mut result = self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while result.is_none() {
            #[cfg(test)]
            self.waiting
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            result = self
                .ready
                .wait(result)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            #[cfg(test)]
            self.waiting
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
        result
            .as_ref()
            .expect("completed flight has a result")
            .as_ref()
            .map(|blob| blob.clone())
            .map_err(SharedFailure::restore)
    }
}

/// Ensures a panicking acquisition never leaves waiters blocked forever.
struct FlightLeader {
    /// A registered active flight, absent only when the bounded registry is saturated.
    flight: Option<Arc<Flight>>,
}

impl FlightLeader {
    /// Completes coordination after actual verification and bounded retention are settled.
    fn complete(&mut self, result: &Result<Option<VerifiedBlob>, CasError>) {
        if let Some(flight) = self.flight.take() {
            flight.publish(result);
        }
    }
}

impl Drop for FlightLeader {
    fn drop(&mut self) {
        if let Some(flight) = self.flight.take() {
            flight.publish(&Err(CasError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "CAS acquisition leader ended before verification completed",
            ))));
        }
    }
}

impl BlobStore for CasSession {
    type Error = CasError;

    fn copy_to(&self, digest: &ContentDigest, sink: &mut dyn Write) -> Result<bool, Self::Error> {
        let Some(blob) = self.acquire(build_digest(digest)?)? else {
            return Ok(false);
        };
        sink.write_all(blob.bytes())?;
        Ok(true)
    }

    fn write_from(&self, source: &mut dyn Read) -> Result<ContentDigest, Self::Error> {
        // Streaming inputs keep the existing bounded-memory, durable publication path.
        self.cas.write_from(source)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    /// Keeps fixtures inside the repository scratch namespace.
    fn fixture() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp/squish-store-tests");
        fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(root)
            .unwrap()
    }

    #[test]
    fn acquisition_reuses_verified_bytes_without_rereading_or_rehashing() {
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let digest = cas.put(b"shared immutable input").unwrap();
        let session = CasSession::new(cas);
        let first = session.acquire(digest).unwrap().unwrap();
        let second = session.acquire(digest).unwrap().unwrap();
        assert_eq!(first.bytes().as_ptr(), second.bytes().as_ptr());
        let stats = session.stats();
        assert_eq!(stats.acquisition_reads, 1);
        assert_eq!(stats.acquisition_hashes, 1);
        assert_eq!(stats.acquisition_bytes, first.bytes().len() as u64);
        assert_eq!(stats.memory_hits, 1);
    }

    #[test]
    fn immutable_snapshot_does_not_hide_corruption_from_a_fresh_invocation() {
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let digest = cas.put(b"original").unwrap();
        let first = CasSession::new(Arc::clone(&cas));
        assert_eq!(first.acquire(digest).unwrap().unwrap().bytes(), b"original");
        fs::write(cas.path_for(digest), b"corruption").unwrap();
        assert_eq!(first.acquire(digest).unwrap().unwrap().bytes(), b"original");
        assert_eq!(cas.get(digest).unwrap(), None);
        let next = CasSession::new(Arc::clone(&cas));
        assert!(next.acquire(digest).unwrap().is_none());
        assert_eq!(next.stats().acquisition_reads, 1);
        assert_eq!(next.stats().acquisition_hashes, 1);
        assert_eq!(next.stats().acquisition_bytes, 10);
        cas.put(b"original").unwrap();
        assert_eq!(next.acquire(digest).unwrap().unwrap().bytes(), b"original");
        assert_eq!(next.stats().acquisition_reads, 2);
    }

    #[test]
    fn retention_obeys_byte_and_entry_limits_and_live_handles_survive_eviction() {
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let a = cas.put(b"aaa").unwrap();
        let b = cas.put(b"bbb").unwrap();
        let c = cas.put(b"cccc").unwrap();
        let empty = cas.put(b"").unwrap();
        let session = CasSession::with_limits(
            cas,
            BlobSessionLimits {
                max_bytes: 6,
                max_entries: 2,
            },
        );
        let live = session.acquire(a).unwrap().unwrap();
        session.acquire(b).unwrap().unwrap();
        session.acquire(c).unwrap().unwrap();
        session.acquire(empty).unwrap().unwrap();
        assert!(session.stats().retained_bytes <= 6);
        assert!(session.stats().retained_entries <= 2);
        assert_eq!(live.bytes(), b"aaa");
        assert_eq!(session.stats().acquisition_reads, 4);
        session.acquire(a).unwrap().unwrap();
        assert_eq!(session.stats().acquisition_reads, 5);
    }

    #[test]
    fn zero_retention_keeps_explicit_handles_but_does_not_retain_hidden_payload() {
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let digest = cas.put(b"large explicit caller input").unwrap();
        let session = CasSession::with_limits(
            cas,
            BlobSessionLimits {
                max_bytes: 0,
                max_entries: 0,
            },
        );
        let first = session.acquire(digest).unwrap().unwrap();
        session.acquire(digest).unwrap().unwrap();
        assert_eq!(session.stats().acquisition_reads, 2);
        assert_eq!(session.stats().retained_entries, 0);
        assert_eq!(session.stats().retained_bytes, 0);
        assert_eq!(first.bytes(), b"large explicit caller input");
    }

    #[test]
    fn verified_put_keeps_durable_corruption_repair_and_uses_the_supplied_handle() {
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let session = CasSession::new(Arc::clone(&cas));
        let input = VerifiedBlob::from_owned(b"durable".to_vec());
        let digest = session.put_verified(&input).unwrap();
        assert_eq!(cas.get(digest).unwrap().unwrap(), b"durable");
        let acquired = session.acquire(digest).unwrap().unwrap();
        assert_eq!(input.bytes().as_ptr(), acquired.bytes().as_ptr());
        fs::write(cas.path_for(digest), b"broken").unwrap();
        assert_eq!(session.put_verified(&input).unwrap(), digest);
        assert_eq!(cas.get(digest).unwrap().unwrap(), b"durable");
        assert_eq!(session.stats().verified_puts, 2);
        assert_eq!(session.stats().acquisition_hashes, 0);
    }

    #[test]
    fn publisher_copy_uses_the_same_verified_allocation() {
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let digest = cas.put(b"publication bytes").unwrap();
        let session = CasSession::new(cas);
        let acquired = session.acquire(digest).unwrap().unwrap();
        let mut destination = Vec::new();
        assert!(
            session
                .copy_to(acquired.digest(), &mut destination)
                .unwrap()
        );
        assert_eq!(destination, b"publication bytes");
        assert_eq!(session.stats().acquisition_reads, 1);
        assert_eq!(session.stats().acquisition_hashes, 1);
        assert_eq!(session.stats().memory_hits, 1);
    }
    #[test]
    fn overlapping_large_acquisitions_share_one_real_read_with_zero_retention() {
        use std::{
            sync::Barrier,
            thread,
            time::{Duration, Instant},
        };
        let root = fixture();
        let cas = Arc::new(Cas::open(root.path()).unwrap());
        let expected = vec![0x5a; 8 * 1024 * 1024 + 1];
        let digest = cas.put(&expected).unwrap();
        let session = Arc::new(CasSession::with_limits(
            cas,
            BlobSessionLimits {
                max_bytes: 0,
                max_entries: 0,
            },
        ));
        let readers = 8;
        let start = Arc::new(Barrier::new(readers + 1));
        let read_gate = Arc::new(Barrier::new(2));
        *session.before_read.lock().unwrap() = Some(Arc::clone(&read_gate));
        let workers: Vec<_> = (0..readers)
            .map(|_| {
                let session = Arc::clone(&session);
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    session.acquire(digest).unwrap().unwrap()
                })
            })
            .collect();
        start.wait();
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.stats().coalesced_hits < (readers - 1) as u64 && Instant::now() < deadline {
            thread::yield_now();
        }
        // Release even on a failed coordination assertion, so joined threads cannot hang.
        read_gate.wait();
        let blobs: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(blobs.iter().all(|blob| blob.bytes() == expected));
        assert!(
            blobs
                .iter()
                .all(|blob| blob.bytes().as_ptr() == blobs[0].bytes().as_ptr())
        );
        let stats = session.stats();
        assert_eq!(stats.acquisition_reads, 1);
        assert_eq!(stats.acquisition_hashes, 1);
        assert_eq!(stats.acquisition_bytes, expected.len() as u64);
        assert_eq!(stats.coalesced_hits, (readers - 1) as u64);
        assert_eq!(stats.retained_bytes, 0);
    }

    #[test]
    fn coordination_registry_is_bounded_and_never_keeps_completed_bytes_alive() {
        let root = fixture();
        let session = CasSession::with_limits(
            Arc::new(Cas::open(root.path()).unwrap()),
            BlobSessionLimits {
                max_bytes: 0,
                max_entries: 0,
            },
        );
        let mut leaders = Vec::new();
        for ordinal in 0..MAX_IN_FLIGHTS {
            let digest = BlobDigest::of(format!("flight-{ordinal}").as_bytes());
            let Acquisition::Read(leader) = session.begin_acquisition(digest) else {
                panic!("new flight must lead");
            };
            leaders.push(leader);
        }
        assert_eq!(session.state.lock().unwrap().flights.len(), MAX_IN_FLIGHTS);
        let Acquisition::Read(bypass) =
            session.begin_acquisition(BlobDigest::of(b"bounded overflow"))
        else {
            panic!("overflow read must remain possible");
        };
        assert!(bypass.flight.is_none());
        assert_eq!(session.stats().coalescing_bypasses, 1);
        drop(leaders);
        let Acquisition::Read(next) =
            session.begin_acquisition(BlobDigest::of(b"next invocation work"))
        else {
            panic!("expired flights must not block work");
        };
        assert!(next.flight.is_some());
        assert_eq!(session.state.lock().unwrap().flights.len(), 1);
    }

    #[test]
    fn interrupted_leaders_wake_waiters_and_preserve_error_classes() {
        let flight = Arc::new(Flight::default());
        let leader = FlightLeader {
            flight: Some(Arc::clone(&flight)),
        };
        drop(leader);
        let CasError::Io(error) = flight.wait().unwrap_err() else {
            panic!("interrupted leader must produce I/O cancellation");
        };
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        for original in [
            CasError::InvalidDigest,
            CasError::UnsupportedAlgorithm,
            CasError::Io(io::Error::from_raw_os_error(2)),
        ] {
            let result = Err(original);
            let flight = Flight::default();
            flight.publish(&result);
            assert_eq!(
                flight.wait().unwrap_err().to_string(),
                result.unwrap_err().to_string()
            );
        }
    }
    #[test]
    fn interrupted_leader_wakes_an_actually_blocked_reader() {
        use std::{
            sync::atomic::Ordering,
            thread,
            time::{Duration, Instant},
        };
        let flight = Arc::new(Flight::default());
        let leader = FlightLeader {
            flight: Some(Arc::clone(&flight)),
        };
        let waiting = Arc::clone(&flight);
        let reader = thread::spawn(move || waiting.wait());
        let deadline = Instant::now() + Duration::from_secs(10);
        while flight.waiting.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        let observed_blocked = flight.waiting.load(Ordering::Relaxed) > 0;
        // Always wake/join before asserting the observation, including overloaded CI hosts.
        drop(leader);
        let result = reader.join().unwrap();
        assert!(observed_blocked);
        let CasError::Io(error) = result.unwrap_err() else {
            panic!("interrupted leader must cancel waiter");
        };
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
}
