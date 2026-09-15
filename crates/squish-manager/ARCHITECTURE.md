# Squish Manager Architecture

## Ownership

`squish-manager` is the single kernel capability for build, format, add, remove,
and inspect operations. `src/lib.rs` owns static routing and invocation-scoped
settings. `src/build.rs`, `src/fmt.rs`, `src/mutation.rs`, and `src/inspect.rs`
own their domain-specific planning and workers. `src/orchestrator.rs` is the only
place that drives `squish_build::Scheduler` or translates scheduler/worker facts
into protocol lifecycle events. Workers return data and never print.

`src/services.rs` defines external ports. Concrete registry, Git, checkout,
filesystem, CAS, index, and artifact adapters remain composition-root concerns.

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

- `storage_layout`: return four absolute, normalized, non-overlapping paths for
  CAS blobs, the action-index database, generation publication, and durable
  catalogs. Production adapters must not infer these locations independently.
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
`StorageLayout::project_local_for_tests`. Every production operation obtains and
authorizes a `StorageLayout` before opening CAS, action-index, publication, or
catalog state; no worker independently derives a project-local fallback.

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

## Build record and current catalog

`BuildRecordV2` in `src/build.rs` is the durable, publicly decodable statement
of one completed or terminal build. It carries the exact `PlanInspection`
identity (`PlanId`, `PlanDigest`, and mode), stable per-action terminal facts,
and the exact target generations committed by that run. Each
`RecordedGeneration` preserves its target owner, immutable generation ID,
publisher current-manifest locator, complete artifacts, and canonical logical
destinations. Static link-map artifacts are members of the recorded generation,
so target graph inspection does not reconstruct them from transient state.

`BuildCatalogSnapshot` is the fully verified view of the current catalog. Its
loader verifies the catalog publication generation, record artifact, CAS digest
and size, typed record schema, target publication manifests, generation IDs,
destinations, and artifact membership before exposing queries. Absence means
only that the build-catalog publisher has no current generation and is returned
as `None`. `MissingCurrent` means a present catalog names a target generation
whose target publisher has no current pointer. `Historical` means that pointer
exists but has advanced to a generation different from the one recorded by the
catalog. `Corrupt` covers malformed, missing, or inconsistent catalog bytes,
CAS objects, manifests, identities, and memberships. None of these three typed
states collapses to absence.

Catalog recovery is deliberately separate from strict inspection. Inspect
preserves `Historical`, `MissingCurrent`, and `Corrupt` as observable typed
results. Build planning instead performs recovery in an observable `Recover`
step: it reads a fully verified base catalog, then validates any authoritative
newer current generation's complete manifest, artifact URIs, logical
destinations, CAS contents, and typed schemas before a typed reconciliation.
A missing or corrupt current generation may be repaired only through the
publisher generation API, atomically restoring a previously verified good
generation; storage errors are propagated rather than treated as absence.
Finalization persists a verified recovery candidate so interruption between G1
(target generation publication) and G2 (catalog publication) cannot leave a
permanent recovery deadlock. The manager currently depends on the publisher's
stable URI layout to validate generations; that parser should move behind a
public typed publisher API rather than remain a long-term layout dependency.

Successful action manifests preserve the complete declared named-output set,
including the backend result rather than only user-published files. Every
behavior-changing option participates in the corresponding action recipe, so a
cache hit cannot silently reuse different semantics. Catalog reads validate the
publisher's authoritative current-generation pointer instead of trusting only a
record that happens to exist in CAS.

Persisting the aggregate build catalog is observable terminal work after
`PlanClosed`. It runs only through `orchestrator::finalize`, which emits
`FinalizationStarted` followed by exactly one timed success or structured
failure event. Once started this finalization is intentionally non-cancellable;
a failure contributes one root failure to the caller's kernel outcome.

Artifact ID/path, plan, static link-map, and provenance queries are projections
of this snapshot. Provenance is closed and typed: prompt and target-record
relations name exact evidence, self-describing IR/debug/link evidence declares
non-applicability, and unsupported kinds remain distinct from a missing catalog.

Backend provenance closes over real static source identities rather than merely
over trace-node shapes: a bare `Expansion` node is not itself evidence of a
static source. For every dynamic producer, the manager follows its `LinkedOpRef`
through the `LinkedImage` unit slot into the compiled unit's `OriginTable` and
recovers all real `QualifiedOriginRef` values with stable deduplication. It
preserves the original provenance DAG as `Input` and appends topologically valid
`SourceSpan` and `BackendTransform` wrappers, rewiring only the artifact segment
that corresponds to that producer. Traversal may follow an existing
`Synthetic.nearest` edge, but closure never invents `Synthetic` or `Unknown`
origins, falls back to `definition_origin`, or flattens provenance to the first
source. The `semantic_example_publishes_fully_traceable_debug_bundle` regression
in `tests/build.rs` exercises this end-to-end closure contract.

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
