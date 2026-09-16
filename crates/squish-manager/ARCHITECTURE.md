# Squish Manager Architecture

## Ownership

`squish-manager` is the single kernel capability for new, build, format, add,
remove, and inspect operations. `src/lib.rs` owns static routing and invocation-scoped
settings. `src/new.rs`, `src/build.rs`, `src/fmt.rs`, `src/mutation.rs`, and `src/inspect.rs`
own their domain-specific planning and workers. `src/orchestrator.rs` is the only
place that drives `squish_build::Scheduler` or translates scheduler/worker facts
into protocol lifecycle events. Workers return data and never print.

`new` keeps prospective destinations out of existing-project discovery and
storage bootstrap. A read-only `Locate` port freezes the absolute destination,
effective Git placement, and optional workspace membership. `PrepareCandidate`
generates canonical scaffold bytes, after which `ValidatePlan` seals exactly one
truthful `CreateProject` `WriteEffect`. Cancellation returned before the durable
commit decision is acknowledged through the scheduler and is not a root failure;
publication that crossed the decision emits `CancellationDeferred` immediately
before its successful action terminal, preserving both the created result and
the invocation's interrupted status.

`src/services.rs` defines external ports. `src/runtime.rs` defines the
object-safe `BuildRuntime` and `BuildRuntimeProvider` boundary. Concrete
registry, Git, checkout, filesystem, CAS, index, publisher, frontend, linker,
evaluator, and backend adapters remain host/composition-root concerns.

## Injected build runtime

`Services::open_build_runtime` is called once during the recorded planning
recovery step. The blanket `BuildRuntimeProvider` implementation for `Services`
keeps command services and runtime acquisition on one injected capability while
still making the build runtime explicit. Opening it there, rather than during
process bootstrap, preserves planning failure and event ordering.

The returned `Arc<dyn BuildRuntime>` lives for the complete invocation and is
shared across scheduler workers. Its `BuildRuntimeDescriptor` freezes
`frontend_abi`, `linker_abi`, `evaluator_abi`, and `document_abi`; planning also
queries that runtime's option-dependent backend identity before plan sealing.
Planning and execution therefore cannot accidentally use different toolchain
identities.

The boundary divides responsibility as follows:

| Manager | Runtime implementation |
| --- | --- |
| Build requests, action keys, cache eligibility, named output contracts | Blob and action-index effects |
| Digest/size/schema/result validation and cache hydration | Target and build-catalog generation I/O |
| Reconciliation, cancellation accounting, lifecycle events, finalization policy | Frontend, linker, evaluator, and backend implementation selection |
| Typed `GenerationSpace` choice (`TargetArtifacts` or `BuildCatalog`) | Publisher locks, recovery, and physical layout |

Runtime methods return domain values or `BuildRuntimeError`. They never emit
kernel events, print, or choose process exits. `BuildExecutor` owns the runtime
plus invocation-local semantic state; it does not name or open `Cas`,
`VerifiedActionIndex`, or `FileArtifactPublisher`.

`squish-host::ProductionHost` owns the production implementation. It opens one
coherent CAS/action-index graph, distinct target and catalog publishers backed
by that graph, and the selected compiler toolchain. Publisher observers are
installed with `ProductionHost::with_build_observers` because they are adapter
construction inputs. Manager
`DurabilityPorts` retains only repository mutation fault injection.

## Plan invariant

`PreparedPlan<W>` in `src/model.rs` contains an immutable `BuildPlan` and a
`BTreeMap<ActionId, W>`. Construction rejects a missing work item, an extra work
item, or a mismatch between `Action.kind` and `PlannedWork::kind()`. Therefore
graph nodes and executable domain work have an exact one-to-one identity and
kind relationship. `tests/core.rs` covers each rejected shape.

Plan identity is behavioral rather than invocation-specific. The build graph's
canonical key recipes include the requested emit set, backend/result schema,
named output schemas, and logical publication destinations. Formatting recipes
include selected source identity and digest, style edition, diff policy, and the
Commit action's check/diff/write behavior. These recipes enter
`BuildPlan::semantic_digest()` and therefore `PlanDigest`; invocation, job,
attempt, timing, and presentation identities do not.

## Effects and caching

`Effect` distinguishes pure `Transform` work from `ReadEffect`, `WriteEffect`,
and `Coordination`. Only `Transform` is persistently cacheable. The orchestrator
does not call `WorkExecutor::lookup` or `record` for effectful work. Action keys
describe semantic computation and must not be polluted with target identity to
defeat legitimate single-flight reuse.

At dispatch, `ResolvedInputs` freezes outputs for the complete transitive
prerequisite closure, not merely direct dependencies. A worker can consequently
hydrate target-local state from CAS even when any producer was a persistent
cache hit or a same-run single-flight follower. `tests/core.rs` exercises a
same-key follower whose dependent receives the follower's logical outputs.

A Publish follower restores its owner only from that owner's exact, complete
named Backend outputs: both `prompt` and `backend-result`, with each output's
kind, digest, and size verified. It also uses that same owner's link map. A
prompt digest alone is insufficient because it does not identify the complete
backend result or the owner-specific link state.

## Bounded orchestration and lifecycle

`PlanningRecorder` records real locate, fetch, resolve, snapshot, scan, candidate,
and validation steps before sealing. It computes one canonical `PlanDigest` from
the frozen snapshot digest, `BuildPlan::semantic_digest()`, canonically sorted
planning issues, and `PlanMode`; job and attempt IDs are deliberately excluded.
It then emits `PlanReady` and the complete `ActionDeclared` set.

`orchestrator::run` performs stable Kahn topological ordering before declarations.
It asks the scheduler for at most `InvocationSettings.jobs` dispatches per wave,
executes them with scoped threads and a typed channel, and sorts completions by
`ActionId` before returning them to the scheduler. Resource capacity and the job
bound both constrain admission. Cancellation tokens remain shared with running
workers. Worker panics become structured failures rather than unwinding through
the manager.

The orchestrator exclusively maps planning, declaration, plan-scoped action, and
`PlanClosed` events. Planning failure and cancellation close the attempt with
`PlanningFailed` or `PlanningCancelled` and declare no execution actions.
`WorkDisposition::Superseded` maps an authoritative-revision race to
`ActionSuperseded` and `PlanClosed(Superseded)`; the caller then begins a fresh
attempt. Kernel-valid success, failure, cancellation, supersede/replan, reverse
lexical dependencies, real parallelism, stable plan identity, and deterministic
completion events are covered by `tests/core.rs`.

`ExecutionReport.actions` is built from scheduler state rather than replaying
events. Each stable-ID fact carries kind, canonical dependencies, terminal
state, optional materialized action key, verified outputs, and result source.
Build aggregation can therefore record failed and cancelled plans without
inventing keys or assuming missing outputs exist.

## Production Services checklist

Default `service_unavailable` methods exist only for command-scoped test doubles.
A production host must implement every raw port and validate its adapter inputs:

- `locate_project_creation`: perform no writes and return a normalized absolute
  destination plus VCS/workspace placement consistent with the requested policy.
- `create_project`: publish the exact candidate through the repository recovery
  transaction and return `Cancelled` only before its commit decision.
- `open_build_runtime`: open or return one coherent invocation runtime whose
  descriptor and adapters remain stable through planning, execution, and
  finalization.
- `storage_layout`: return four absolute, normalized, non-overlapping paths for
  CAS blobs, the action-index database, generation publication, and durable
  catalogs. This path contract remains for repository mutation and inspection;
  build and format workers must not use it to construct storage adapters.
- `materialize_locked`: materialize registry/Git packages pinned by an existing
  lock before the repository freezes the full candidate snapshot.
- `resolve`: resolve the complete manifest snapshot, canonical digest, prior
  lock, and access mode into an exact lock plus package locations.
- `cache_records`: return typed raw action records for manager-side validation.
- `read_blob`: return raw CAS bytes; the manager verifies digest and size.
- `artifact` and `artifact_at`: query artifact records by typed ID or validated
  path locator.
- `link_map`: query the persistent static-link-map artifact for a target.
- `provenance_evidence`: return either provenance candidates for manager-side
  decoding and validation or a typed closed non-applicability reason; an empty
  vector can no longer masquerade as a missing catalog relation.
- `planned_actions`: return the persisted build-record plan projection for
  manager-side graph validation.

Hard-coded storage paths exist only in
`StorageLayout::project_local_for_tests`. `ProductionHost` validates and
authorizes the layout before composing the runtime; no manager worker opens an
adapter or independently derives a project-local fallback.

## External artifact contracts

Format diff artifacts are a deterministic public representation, not an
implementation-private debug payload. Their bytes are UTF-8 unified diff, their
kind is `Other("text/x-diff")`, and their `--- a/` and `+++ b/` headers use the
logical `OpaqueSourceId` rather than a host filesystem path. Lines are
normalized to LF in the diff representation, while an unterminated input or
output line is represented by the standard no-final-newline marker. Non-UTF-8
input produces a structured formatting failure and no diff artifact or source
write.

On Windows, an absolute inspect artifact path is accepted only after both the
project root and the existing artifact resolve to canonical native filesystem
identities. The artifact must be contained by that canonical project identity;
the manager then passes only its canonical project-relative `ArtifactLocator`
to `Services`. Nonexistent absolute paths, paths outside the project, and path
traversal are rejected. Already-relative catalog locators retain their catalog
identity after validation rather than being replaced by host-absolute paths.

## Build record and typed publication catalog

`BuildRecordV3` is the durable semantic statement of the sealed plan, terminal
action facts, and retained target generations. It stores typed publication target
and generation identities plus `ArtifactDescriptor` and `PublicationPath`; it does
not store publisher manifests, target hashes, generation-directory URIs, or physical
paths.

The inward-facing `squish-build::GenerationRepository` port owns the complete
publication conversation: atomic generation publication, current-generation query,
and verified member reads. `squish-publish::FileArtifactPublisher` implements that
port and exclusively interprets journals, locks, current pointers, target hashes,
generation directories, filesystem aliases, and digest verification. A read returns
`NotFound` only when the logical member is absent from the manifest; a promised but
missing or mismatched member is an integrity error.

Manager reconciliation remains semantic. It compares typed `GenerationId` values,
validates plan/action coverage and required artifact membership, compares verified
published bytes with CAS, and may adopt or restore a generation through the typed
API. It never joins a publication root with an artifact locator or parses an adapter
URI. `BuildCatalogSnapshot` exposes queries by immutable artifact ID and stable
logical locator. Successful build results report `(target_id, generation_id,
locator, descriptor)`; the locator is stable across generations and is accepted
verbatim by `inspect artifact`. The artifact inspection result carries bytes that
the manager has checked against the descriptor; `--format=raw` writes those bytes
without exposing the publisher's physical location. Provenance remains a separate
identity-based query.

Absence of the build-catalog current generation is `None`. A recorded target with no
current generation is `MissingCurrent`; a different current generation is
`Historical`; malformed semantic records or publisher integrity failures are
`Corrupt`. Storage availability failures remain distinct. Target publication (G1)
and catalog publication (G2) retain separate crash-safe commit decisions, so an
interrupted finalization is recovered during ordinary publisher opening without
user repair.

## Reviewer issues resolved

- Graph/work drift is structurally rejected (`src/model.rs`).
- Effectful work bypasses persistent cache (`src/orchestrator.rs`).
- Child IDs sorting before parent IDs no longer creates illegal kernel events;
  declarations are topological (`src/orchestrator.rs`, `tests/core.rs`).
- `jobs` controls real concurrent worker count rather than only resource-vector
  arithmetic (`src/orchestrator.rs`, `tests/core.rs`).
- Registry/Git checkout and raw inspection data are explicit ports rather than
  successful stubs (`src/services.rs`).
- Inspection subjects are typed as cache keys or validated artifact locators
  (`src/model.rs`, `src/inspect.rs`).
- Cache and single-flight followers do not depend on ephemeral leader state;
  workers receive the transitive prerequisite output closure.
- Compile keys include the canonical logical `SourceKey` and frontend ABI, so
  byte-identical sources with different semantic identities never alias. Link
  keys additionally include the complete resolution fingerprint rather than a
  target-name workaround (`src/build.rs`, `tests/build.rs`).
- Compile, Link, Instantiate, and Backend persist typed action results and
  hydrate their state from verified CAS bytes. The fresh-manager cache test in
  `tests/build.rs` proves that no executor-local map is required across an
  invocation boundary.
- Build and format effects use the injected `BuildRuntime`; concrete CAS,
  action-index, publisher, and toolchain construction lives in `squish-host`.
  Runtime descriptors are frozen before plan sealing, and target/catalog
  publisher observers are composed at that host boundary.
- A target's prompt, debug companion, emitted IR, and target record commit as
  one publisher generation. The aggregate `BuildRecord` is post-generation
  coordination describing completed target generations; it is not falsely
  presented as a member of an already committed target generation. Failed or
  cancelled runs aggregate only actually materialized keys and terminal facts,
  without `expect` on absent predecessor outputs (`src/build.rs`).
- Formatting computation runs inside per-source workers. A final transaction
  barrier commits the batch only after every semantic preflight succeeds, so
  `jobs` and cancellation govern the real work (`src/fmt.rs`, `tests/fmt.rs`).
- Add/remove perform observation, snapshot, and candidate resolution as real
  planning steps. The sealed execution plan contains only the recoverable Commit
  action. Contention supersedes that plan and starts a complete fresh attempt
  rather than hiding a partial retry (`src/mutation.rs`, `tests/mutation.rs`).
- Execution plans no longer replay already-completed planning as fake
  Resolve/Snapshot/Scan actions. Protocol-v2 adaptation is isolated at the
  orchestrator event boundary described below.

## V2 lifecycle and current compatibility

ADR commit `c690a9d` and protocol v2 implement
`Job -> PlanningAttempt -> immutable Plan`. Every expensive or fallible planning
operation is represented by a real typed planning step. A successful attempt
atomically seals one immutable plan; a failed or cancelled attempt produces no
plan. Every action event carries that plan's identity, and every sealed plan ends
with `PlanClosed` as Executed, Reported, or Superseded.

Execution `PreparedPlan` values must not contain fake `Resolve`, `Snapshot`, or
`Scan` placeholders for planning work that already happened. Doing so would make
events look observable while merely replaying successful no-op actions. Legacy
v1 events are decode-only compatibility projections in `squish-protocol`; the
manager emits native v2 lifecycle events exclusively.
