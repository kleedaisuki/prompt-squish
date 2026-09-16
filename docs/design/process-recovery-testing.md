# Process-Death Recovery Testing

Status: implemented acceptance contract (2026-09-15)

## 1. Decision and scope

`xmlsquish` treats recovery as an ordinary invocation path. A process may die after a durable
write and before the corresponding user-visible commit finishes; the next invocation, without a
test selector and without manual state deletion, must deterministically converge to either the
previous complete state or the newly committed complete state.

This contract tests **process death**, not arbitrary media loss or power failure. Directory
durability guarantees remain those documented by `squish-publish` for each operating system.
The design follows the same general principle used by production journaled systems: recovery is
driven by durable commit evidence rather than by guessing how far execution progressed. SQLite's
[atomic commit protocol](https://www.sqlite.org/atomiccommit.html) is a useful production reference;
ARIES provides the classic academic treatment of
[write-ahead recovery](https://research.ibm.com/publications/aries-a-transaction-recovery-method-supporting-fine-granularity-locking-and-partial-rollbacks-using-write-ahead-logging).
`xmlsquish` does not claim to implement either system; they motivate the commit-decision model.

## 2. Architecture

The three failure domains remain independent, but their injection follows adapter ownership:

```text
xmlsquish composition root
├── squish-manager::DurabilityPorts
│   └── repository          -> squish_repository::FaultInjector
└── ProductionHost / ProductionBuildRuntime
    ├── artifact_generation -> squish_publish::PublishObserver
    └── build_catalog       -> squish_publish::PublishObserver
```

The separation is semantic, not cosmetic. Repository mutation remains a manager-selected domain
capability. Target publication and build-catalog publication are concrete runtime adapters, so
their observers are installed by `squish-host` when it constructs the two corresponding
`FileArtifactPublisher` instances. The manager receives only `Arc<dyn BuildRuntime>` and never
handles publisher observers or constructs publishers.

Target publication and build-catalog publication use the same publisher event type, but they have
different recovery responsibilities. A global event occurrence counter would couple the test to
scheduler order and the number of artifacts. Separate host-composed observer inputs preserve that
scope in the composition contract. Normal composition supplies `NoFault` and no-op observers;
fault-enabled composition uses `ProductionHost::with_build_observers` to replace only the selected
publication port. No CLI, environment, or exit logic enters a domain crate.

## 3. Test-only process adapter

The root package defines a default-disabled `fault-injection` Cargo feature. Cargo features are
compile-time configuration, as described by the official
[Cargo feature documentation](https://doc.rust-lang.org/cargo/reference/features.html). Only this
feature compiles `src/fault_injection.rs` and the environment-variable literal. The normal binary
therefore has no runtime hook to activate.

At the composition boundary, the root captures the process environment once. The adapter parses
`XMLSQUISH_TEST_PROCESS_EXIT_AT` once from that snapshot and maps its closed selector language to
one repository fault port or one host runtime publisher observer. An unknown or non-Unicode value
is a deterministic `TEST_FAULT001`
configuration failure; it never silently degrades to a no-op. A matched observer calls
`std::process::exit(86)` after the durable operation has completed. It does not panic, unwind, run
destructors, or ask a domain service to emulate a crash.

Supported selectors are intentionally small:

| Selector | Completed durable boundary | State before exit | Recovery rule |
|---|---|---|---|
| `artifact-generation.generation-staged` | Immutable generation directory renamed and its parent sync attempted | No generation commit-decision journal | Keep the previous current generation |
| `artifact-generation.commit-decision` | Generation journal atomically persisted and state-directory sync attempted | Current manifest not yet switched | Roll the new generation forward |
| `build-catalog.commit-decision` | Catalog generation journal atomically persisted and synced | Catalog current manifest not yet switched | Roll the catalog forward |
| `repository.candidates-staged` | Candidate files, staged journal record, and transaction directory synchronized | No commit-decision record | Discard the transaction |
| `repository.commit-decided` | Commit-decision JSONL record appended and file synchronized | Target replacements not started | Roll manifest and lock forward together |

The process adapter deliberately does not expose selectors for every internal event. The chosen
set straddles each commit decision and covers both publisher namespaces plus the multi-file
repository transaction.

## 4. Recovery invariants

For every injected death:

1. The crashing child exits with exactly `86`; a component error or panic is not equivalent.
2. Only that child receives the selector. Every follow-up command explicitly removes it.
3. The follow-up is an ordinary public command using `CARGO_BIN_EXE_xmlsquish`.
4. No test deletes or edits a journal, current manifest, transaction directory, manifest, or lock.
5. Pre-decision death preserves the previous current generation or the byte-exact repository
   baseline.
6. Post-decision death rolls forward the whole target/catalog generation or the manifest+lock
   pair.
7. Recovery removes the completed generation journal or repository transaction state.
8. Published prompt identity is read through public `inspect artifact --format=json`, not through
   private directory or SQLite assumptions.

The assertions distinguish durable state from incidental output. The crashing process can leave a
partial terminal transcript because `process::exit` intentionally bypasses renderer completion;
that transcript is not used as recovery evidence.

## 5. Acceptance matrix

`tests/recovery_process.rs` creates every project and storage home beneath repository-local
`.temp`, launches the real binary, and covers:

| Case | Setup | Crash | Ordinary recovery | Required observation |
|---|---|---|---|---|
| Artifact pre-decision | Build source A, then change to B | generation staged | `inspect artifact` | A remains current |
| Artifact post-decision | Build A, change to B, then replace B with invalid C after death | artifact commit decision | failing `build` | Journal cleared; committed B becomes current without recomputing B |
| Catalog post-decision | Build A, change to B, then replace B with invalid C after death | catalog commit decision | failing `build` | Catalog journal cleared; committed B is inspectable without recomputing B |
| Repository pre-decision | Establish manifest+lock baseline | candidates staged during `add` | `build` | Manifest and lock equal baseline; transaction state empty |
| Repository post-decision | Prepare a local path dependency, then replace the entry with invalid XML after death | commit decided during `add` | failing `build` | Manifest and lock both contain the dependency before compilation can succeed; transaction state empty |

The invalid-selector case also proves that the selector language is closed and produces the
machine-output bootstrap contract rather than accidentally running a build.

The invalid source C in every post-decision build is a validity control: the follow-up command
must fail compilation, so it cannot recreate B and accidentally make a broken recovery path look
successful. Recovery runs during repository/publisher opening before the failing compile action;
the subsequent public inspection and byte-level assertions therefore observe recovered durable
state rather than a successful recomputation.

## 6. Reproduction and release gates

```powershell
# Real child-process deaths and automatic recovery.
cargo test --features fault-injection --test recovery_process --locked

# The default feature set must remain usable and must omit the test-only module.
cargo build --release --no-default-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.88.0 check --workspace --all-targets --all-features --locked
```

For an inspectable default-binary check, search the produced release executable for
`XMLSQUISH_TEST_PROCESS_EXIT_AT`; it must be absent. Source-level review must additionally confirm
that the literal appears only in `src/fault_injection.rs` and its feature-gated process test.

## 7. Limits and falsification

Passing this matrix is evidence for the named commit boundaries, not for every possible I/O
failure. The conclusion must be revised if any supported platform observes one of the following:

- an exit at a named point occurs before its documented synchronization operation completes;
- a normal follow-up requires a selector, repair flag, or manual deletion;
- a current manifest mixes artifacts from different generations;
- manifest and lock diverge after repository recovery;
- the default-feature binary contains or responds to the test selector; or
- concurrent recovery violates the existing publisher/repository locks.

Future durability boundaries should extend the typed port and closed selector enum only when they
represent a distinct commit-state partition. They must not introduce command-specific branches or
event occurrence counters.
