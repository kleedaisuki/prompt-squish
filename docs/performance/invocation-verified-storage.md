# Invocation-scoped verified blob ownership and advisory LRU batching

Date: 2026-10-01. Status: implementation complete; hosted verification pending.
Follow-up to `v1.2-store-orchestration-audit.md`.

## Integrity and ownership contract

The filesystem CAS remains authoritative across invocations. A fresh acquisition reads complete
bytes, hashes them, compares the requested address, and returns an immutable digest-bearing handle.
Only that handle may bypass repeated hashing of the same immutable bytes. Mutable source files,
artifact generation files, or arbitrary caller-provided byte slices are not trusted handles.

An explicitly invocation-owned session shares those verified handles between the action-index
verifier and subsequent consumers. The ordinary CAS and index constructors retain fresh-read
semantics, including detection when an existing test mutates a CAS file between accesses. Sessions
are never persisted, never reused by a later command, and never treat path/mtime/size as integrity
proof. A file modified after acquisition does not change the already-verified immutable snapshot;
a new session detects corruption. Missing/corrupt acquisitions are not retained negatively,
so subsequent repair within an invocation remains discoverable.

Retained session state is byte- and entry-bounded (`BlobSessionLimits`). The default is 8 MiB
and 256 entries, configurable for mechanism tests. Caller-owned handles are separate live inputs, not
unbounded hidden cache state. Oversized outputs require explicit lookup-to-consumer handle
handoff; a bounded implicit cache cannot honestly promise reuse for them. Eviction may require
another verified read, but cannot weaken integrity. Concurrent cache misses perform disk I/O outside the state mutex. Overlapping first reads of
one digest share a per-digest condition-variable result. A weak coordination registry is bounded
at 256 active distinct digests; it neither retains completed bytes nor grants persistent trust.
At saturation, a new distinct digest may execute without coordination (`coalescing_bypasses`),
while already-registered digests still coalesce. Unrelated reads are never serialized behind a
global I/O lock. A leader guard wakes waiters with an interrupted I/O error if verification
unwinds before completion; successful/failing results retain immutable byte or error identity.

Input writing uses the same handle authority (`squish_build::VerifiedBlob`): hashing an immutable buffer once produces
both its bytes and expected digest. A durable writer can publish that verified buffer without
rehashing its input. Existing-file validation, synchronized temporary-file contents, atomic
publication, and parent-directory sync remain intact. The injected runtime's returned digest
must still match the handle's expected digest; optimization is not permission to trust a mock
or arbitrary adapter's claim.

## Advisory last-use timestamps

`VerifiedActionIndex::with_batched_touches()` opt-in queues at most 64 distinct action keys;
repeated touches retain the maximum timestamp. A full queue or explicit `flush_touches()`
commits updates in one immediate SQLite transaction. The queue is cleared only after successful
commit. Updates only advance timestamps (`last_used_unix_ms < new_time`) and cannot recreate
rows deleted concurrently. Drop attempts to flush remaining advisory timestamps.

Successful manifests, invalid-row deletion, and output publication keep their existing immediate
transactions and durability. A crash may lose recent LRU timestamps, not action correctness.
Separate public transaction/changed-row counters permit hosted mechanism checks. Existing index
constructors remain unbatched unless the invocation adapter explicitly selects batching.

## Discriminating tests and hosted probes

Added focused tests (not executed locally due the user's disk constraint):

- 70 unique zero-output action hits require two advisory batches instead of 70 touch commits;
  records remain visible from a separate SQLite connection before the final touch flush.
- 100 touches of one key require one row update; older timestamps cannot backdate it and
  pending touches cannot resurrect a deleted record.
- An injected SQLite trigger failure leaves the queue retryable and bounded at 64 keys;
  dropping the trigger permits a successful retry. Drop flushes pending timestamps.

Added CAS-session and handle tests cover verified read/hash counts, allocation identity, corruption
in fresh sessions, repaired negative misses, byte/entry-bound eviction, explicit zero-retention
handles, durable put corruption repair, and snapshot-backed publisher copying. Action-index
lookup tests prove duplicate >1 MiB output handles are acquired once even with zero hidden
retention, and incorrect manifest sizes remain a miss. Existing CAS publication code and its
cold-write/fsync/directory-sync tests remain unchanged.

`crates/squish-store/examples/io_probe.rs` produces a fixed numeric JSON record for hosted CI:

```text
cargo run -p squish-store --example io_probe --release -- .temp/store-io-probe
```

The expected asserted mechanism counts are 16 physical acquisitions with zero retention versus
1 acquisition/hash plus 15 immutable memory hits with retention; duplicate >8 MiB output handles
are acquired once with zero retained bytes; 70 unique advisory touches commit in 2 batches.
These are test expectations, not measured results before hosted execution. Actual changed-row
counts depend on timestamps and are reported without an unconditional 70-row claim.

The runtime bridge additionally accepts explicit handles for generation publication. Those
handles take precedence in a temporary publisher view, so oversized products do not rely on
hidden cache retention to avoid another acquisition. A weak-handle cache was considered and
rejected: explicit publication ownership expresses the required lifetime with less machinery.

Hosted Linux syscall probing must distinguish successful verified reads/hashes from cache hits
and LRU transactions; count `openat/read/write/fsync/fdatasync` by CAS, SQLite WAL, and publication
path with `strace -f -yy`. Syscall-count runs are not latency benchmarks. No measured claim is
made before CI results exist.

## External evidence

SQLite serializes writers; batching semantically advisory updates amortizes transactions without
changing `synchronous` settings or disabling barriers:
https://www.sqlite.org/lang_transaction.html
https://www.sqlite.org/atomiccommit.html
https://www.sqlite.org/wal.html

Bairavasundaram et al., FAST 2008, report real storage-stack corruption including checksum
mismatches and identity discrepancies. The engineering consequence is to retain the first
persistent-acquisition checksum boundary, not to trust filenames or timestamps:
https://www.usenix.org/conference/fast-08/analysis-data-corruption-storage-stack

## Exact APIs and trust boundaries

- `squish_build::VerifiedBlob::from_owned(Vec<u8>)` and `from_shared(Arc<[u8]>)` hash once.
  Fields are private; `digest()` and `bytes()` expose immutable zero-copy views. An owned Vec
  stays in its original allocation inside the Arc-shared authority: acquisition no longer
  performs a complete Vec-to-Arc-slice copy before hashing. Shared input preserves its supplied
  Arc. `shared_bytes()` is an explicit compatibility API: an owned input lazily materializes
  one Arc-slice copy through OnceLock, reused by subsequent calls. Actual verified manager and
  publication paths retain VerifiedBlob and borrow `bytes()`, not that compatibility view.
  The digest authority is itself Arc-shared, avoiding a new 32-byte digest Vec allocation
  at every handle clone as well as avoiding payload copies. Equality compares digest and
  byte value, not the owned/shared representation or whether compatibility materialized.
  A handle proves byte identity, not durable publication. No unchecked constructor exists.
- `VerifiedAction { record, blobs }` transfers handles in output-declaration order. The manifest
  is not sealed, allowing runtime injection, so managers still compare key/name/type/digest/size
  against the expected action contract.
- `Cas::get_verified` freshly acquires/hashes one persistent file. `Cas::get` keeps its original
  fresh-read compatibility semantics. `Cas::put_verified` shares the unchanged durable
  publication implementation but does not hash an already-sealed input again.
- `CasSession::{new,with_limits,acquire,put_verified,stats}` adds explicitly invocation-local
  verified ownership. `BlobStore::copy_to` writes the verified snapshot, rather than reopening,
  rehashing, rewinding, and rereading a mutable filesystem file. Streaming `write_from` keeps
  its original bounded-memory publication path.
- `VerifiedActionIndex::open_in_session` shares that acquisition authority and automatically
  batches advisory touches. `lookup_verified` verifies all declared members and transfers the
  actual allocations; duplicate digests in one manifest share one acquired handle even with
  zero session retention. Ordinary `lookup`/constructors remain available.
- Session stats count `acquisition_reads`, `acquisition_bytes`, `acquisition_hashes`,
  `memory_hits`, `coalesced_hits`, `coalescing_bypasses`, `verified_puts`, `retained_entries`,
  and `retained_bytes`. Acquisition counters
  include corrupt reads, but do not claim to cover existing-file validation on durable puts.
  Touch stats separately count actual successful advisory transactions and changed rows.

No new local Cargo commands were run for this work because the user prohibited local compilation
and testing after the disk-space warning. Owned files passed rustfmt and diff whitespace checks;
compilation, tests, and mechanism counts must be obtained from hosted CI.

## Follow-up discovery: full-buffer conversion and simultaneous acquisition

Source review found that `Arc::from(Vec<u8>)` can allocate and copy the entire buffer. Retaining
an Arc-slice was therefore insufficient evidence of a zero-copy acquisition. The repaired owned
representation retains Vec directly and seals it in a small Arc authority; a regression records
the original Vec pointer and checks both the handle and clone retain exactly that pointer. A
separate test preserves the original shared Arc pointer and checks value equality before/after
lazy compatibility materialization. An explicit compatibility caller may retain an additional
Arc-slice copy; the session's byte budget and `retained_bytes` count canonical payload bytes,
not these caller-requested compatibility allocations. In verified production paths this view
is not requested. This bounded compatibility cost is not claimed eliminated.

The previously documented simultaneous-read duplication was also repaired instead of deferred.
A deterministic test starts eight threads at a barrier, holds the actual leader immediately
before its real >8 MiB CAS read until all other readers join the flight, then releases it. With
zero session retention, it asserts one actual read/hash, seven coalesced hits, zero retained
bytes, and identical underlying Vec pointers across all returned handles. Separate tests bound
registry cardinality, prove expired flights are cleaned up without retaining completed payload,
and verify interrupted leaders and ordinary CAS error classes wake waiters correctly. These are
new hosted test expectations, not a claim that local execution occurred.

Independent source review identified a possible concurrency-test hang if a reader arrived after
its deadline and reused the already-released read gate. The test gate is now single-use, so late
readers complete and cause a normal count assertion failure rather than waiting forever. A
further regression observes a reader entering the actual condition-variable wait and verifies
that dropping an interrupted leader wakes and joins it before assertions. These test-only gates
and waiter observations are absent from production builds.
