# Issue #1 Runtime Port Map

## Question and scope

Issue #1 requires build execution to consume injected runtime capabilities instead of opening
production adapters inside `squish-manager`. In particular, `BuildExecutor` must no longer name
`Cas`, `VerifiedActionIndex`, or `FileArtifactPublisher`; the production implementation must be
assembled by `squish-host`/the executable composition root. The injected runtime must cover blob
I/O, action-cache lookup/recording, artifact generations, compilation, linking, instantiation, and
rendering. Existing determinism, cache verification, cancellation/event ordering, and crash-safe
publication remain behavioral invariants.

This note maps the current implementation and proposes the smallest coherent seam. It is based on
the repository at `3964a83` and does not prescribe unrelated source/repository or formatter work.

## Current construction and call chain

```text
src/main.rs::execute
  -> compose_host(root, config, environment)
       -> StorageLayout::new(...)
       -> ProductionHost::open(HostConfig { storage, fetch/resolver adapters, ... })
  -> dispatch_operation(host, optional Cas, DurabilityPorts)
       -> ManagerCapability::with_durability(host, settings, durability)
       -> Kernel::dispatch
          -> ManagerCapability<S>::execute
             -> build::execute_with_durability
                -> prepare_recorded
                   -> Services::storage_layout
                   -> recover_build_catalog                 [opens publisher + CAS in manager]
                   -> repository snapshot / resolve / source freeze / build_plan
                -> BuildExecutor::new                       [opens CAS handles, 2 publishers, index]
                -> planning.seal                            [freezes plan/runtime identity]
                -> orchestrator::run(..., &BuildExecutor, ...)
                   -> WorkExecutor::lookup                  [VerifiedActionIndex]
                   -> WorkExecutor::execute
                      -> compile/link/instantiate/backend/publish_target
                   -> WorkExecutor::record                  [VerifiedActionIndex]
                -> orchestrator::finalize
                   -> BuildExecutor::persist_terminal_record [CAS + catalog publisher]
```

The executable is already the process composition root, but only fetch/resolution and inspection
services are composed there. Build storage and the complete compiler toolchain are still selected
inside the manager.

## Concrete-symbol inventory

| Concern | Current symbol and evidence | Consequence |
|---|---|---|
| Process composition | `src/main.rs::compose_host` (lines 635-687) constructs `StorageLayout` and `ProductionHost`; `dispatch_operation` (499-601) constructs `ManagerCapability` | This is the correct place to choose production runtime adapters. `compose_host` currently does not construct a build runtime. |
| Manager routing | `crates/squish-manager/src/lib.rs::ManagerCapability<S>` and its `Capability::execute` implementation (117-215) | `Services` is the existing injection path. Adding a build-runtime capability here avoids a second service locator. |
| External ports | `crates/squish-manager/src/services.rs::Services` (261-355) | It exposes storage paths and fetch/query methods, but no executable build runtime. `storage_layout` leaks adapter construction material into the manager. |
| Planning | `build.rs::prepare_recorded` (1379-1496) | Calls `Services::storage_layout`, then manager-owned `recover_build_catalog`. The runtime must be available before `build_plan`, because backend identity contributes to action keys. |
| Executor construction | `build.rs::BuildExecutor::new` (1685-1713) | Calls `Cas::open` four times (the retained main handle, one per publisher, and one for the index), and opens two `FileArtifactPublisher<Cas>` instances plus one `VerifiedActionIndex`. This is the primary acceptance-criteria violation. |
| Blob I/O | `compile` (1877-1918), `link` (1920-1969), `instantiate` (1971-2007), `backend` (2009-2065), `publish_target` (2067-2249), `cached_bytes` (2385-2407), and `persist_terminal_record` (1723-1788) | Every build phase is coupled to `squish-store::Cas`. Returned digests and fetched bytes are still semantically verified/decoded by manager code and should remain so. |
| Action cache | `WorkExecutor::lookup` and `record` for `BuildExecutor` (2410-2481) | Coupled to `VerifiedActionIndex`; only `Effect::Transform` is looked up/recorded by `orchestrator`, an invariant that must remain above the port. |
| Frontend | `BuildExecutor::compile` calls `squish_xml_front::compile` with `FrontendSourceContext` | Frontend selection is static but not injected. The manager should construct the semantic request and validate/encode its result, while the runtime selects the implementation. |
| Linker | `BuildExecutor::link` calls `StaticLinker::link` | Same problem. Import binding and `UnitClosure` assembly are manager policy and should not move into the host. |
| Instantiator | `BuildExecutor::instantiate` calls `Instantiator::instantiate` and writes `squish_backend::DOCUMENT_ABI` into the document (1971-1987) | Arguments and budgets are manager policy; evaluation implementation and its document ABI are runtime capabilities. The document ABI must be frozen into the plan rather than selected after a cache key is computed. |
| Backend | `BuildExecutor::backend`, `hydrate_cached`, and `backend_options` name `SquishBackend` | The runtime must provide both rendering and the cache identity used while planning. Cache identity cannot be recomputed from a different backend instance after the plan is sealed. |
| Target generations | `publish_target`, free function `publish_generation` (2483-2502), and `restore_recorded_generation` (972-1010) | Generation publication/recovery is richer than `squish_build::ArtifactPublisher::publish`, so the existing single-publication trait is insufficient. Retry/locking/crash recovery belong to the production adapter, not the manager. |
| Catalog generation | `persist_terminal_record`, `read_base_build_catalog` (487-594), `read_current_build_catalog_with` (631-673), `recover_build_catalog` (703-749) | Target and catalog generation spaces must remain distinct typed capabilities; mixing them would weaken the existing fault partition. |
| Catalog verification | `verify_catalog_blob`, `recover_current_generation`, `validate_generation_target_record`, and schema validators in `build.rs` | These are domain validation, not adapter construction. Keep them in manager code, but feed them bytes/generation snapshots through ports. |
| Production inspection | `squish-host/src/lib.rs::ProductionHost::current_catalog` (1357-1362) calls `squish_manager::build::read_current_build_catalog(&StorageLayout)` | After inversion, this helper should accept a runtime/catalog view rather than open storage. The host may still reuse manager's pure validation/query projection. |
| Reopened host storage | `squish-host/src/lib.rs::cache_records` (1385-1428) reopens CAS/index and `read_blob` (1430-1436) reopens CAS per query | Production inspection should share the composed runtime handles too; otherwise adapter lifecycle remains duplicated even after build execution is injected. |
| Durability injection | `squish-manager::DurabilityPorts` stores repository `FaultInjector` plus two `squish_publish::PublishObserver`s; `src/main.rs::durability_ports` creates them | Publication observers are adapter construction inputs currently passed through manager solely so manager can construct publishers. Once the host constructs publishers, those two observers should be supplied to the host/runtime at composition. Repository faults remain a repository capability. |
| Adjacent manager CAS opening | `crates/squish-manager/src/fmt.rs::store_diffs` (745-771) opens `squish_store::Cas` directly | If “manager no longer opens CAS” is interpreted crate-wide, this must migrate too. Prefer a shared blob sub-port used by both format and build rather than making formatting depend on a misleadingly named build-only service. |

## Existing lower-level traits are not the complete seam

`squish-build/src/ports.rs` defines `BlobStore`, `ActionIndex`, and
`ArtifactPublisher`. They are useful adapter contracts, but they do not by themselves satisfy the
issue:

* associated error types make a heterogeneous runtime aggregate awkward as a trait object;
* `ArtifactPublisher` publishes one object and has no current-generation, recovery, or atomic
  multi-artifact-generation operation;
* there are no frontend/link/instantiate/backend ports;
* exposing several independently opened objects would permit mismatched CAS/index/publisher
  instances and duplicate lifecycle management.

The manager needs one invocation-scoped, thread-safe runtime aggregate whose production
implementation may delegate to those existing traits.

## Recommended minimal API

Define the port in `squish-manager` (most naturally in a new `src/runtime.rs`, re-exported from
`lib.rs`). `squish-host` already depends on `squish-manager`; defining it in `squish-host` would
reverse that edge, while defining semantic compile/link types in `squish-build` would make the
low-level action crate depend upward on frontend/link/backend crates.

The minimal shape is one object-safe aggregate, not a service locator and not a high-level
`execute(BuildWork)` callback:

```rust,ignore
pub trait BuildRuntime: Send + Sync {
    fn descriptor(&self) -> BuildRuntimeDescriptor;
    fn render_identity(&self, options: &RenderOptions) -> Result<BackendIdentity, BuildRuntimeError>;

    fn read_blob(&self, digest: &Digest) -> Result<Option<Vec<u8>>, BuildRuntimeError>;
    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError>;

    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError>;
    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError>;

    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &str,
    ) -> Result<Option<Generation>, BuildRuntimeError>;
    fn read_generation_artifact(
        &self,
        space: GenerationSpace,
        locator: &GenerationArtifactLocator,
    ) -> Result<Option<Vec<u8>>, BuildRuntimeError>;
    fn publish_generation(
        &self,
        space: GenerationSpace,
        target: &str,
        publications: &[Publication],
    ) -> Result<Generation, BuildRuntimeError>;

    fn compile(&self, request: CompileRequest<'_>) -> Result<FrontendOutput, BuildRuntimeError>;
    fn link(&self, request: LinkRequest) -> Result<LinkOutput, BuildRuntimeError>;
    fn instantiate(
        &self,
        request: InstantiateRequest<'_>,
    ) -> Result<InstantiateOutput, BuildRuntimeError>;
    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError>;
}

pub enum GenerationSpace { TargetArtifacts, BuildCatalog }

pub struct BuildRuntimeDescriptor {
    pub frontend_abi: String,
    pub linker_abi: String,
    pub evaluator_abi: String,
    pub document_abi: String,
}
```

`Generation` should be a neutral data value (`target_id`, typed `generation_id`, and logical
artifacts) rather than `FileArtifactPublisher` itself. It should live in `squish-build` (the
publisher port contract) or, less ideally, `squish-manager`; it must not alias or expose
`squish_publish::PublishedGeneration`, because that would preserve the concrete dependency.
Coordinate this type with issue #2 so no physical manifest/URI layout enters the runtime port.
`GenerationSpace` makes the existing durability partition part of the type contract without
fragile event counters; two separately injected generation sub-ports would enforce that partition
even more strongly.

Add one acquisition method to the existing service boundary:

```rust,ignore
pub trait Services: Send + Sync {
    fn build_runtime(&self, project_root: &Path)
        -> Result<Arc<dyn BuildRuntime>, ServiceError>;
    // existing resolver/repository/query capabilities ...
}
```

The runtime should be acquired once in a recorded `open-build-runtime` planning step, before
catalog recovery and `build_plan`, and stored as `Arc<dyn BuildRuntime>` in `PreparedBuild` or
`BuildExecutor`. It must not be reopened per action. `render_identity` is option-sensitive (the
current squish identity includes `max_output_bytes`), so planning calls it for each target and
execution retains the same runtime object. `BuildExecutor` then owns only immutable prepared
domain data, the injected runtime, and its invocation-local mutex-protected intermediate state.
Injectability makes frontend, linker, evaluator, document, and backend identities mandatory action
key inputs now: postponing any of them permits one implementation to reuse another implementation's
cache entry.

### Boundary rules

1. **Manager retains policy and validation.** It builds action requests, binds imports, owns named
   output sets, verifies returned digest/size, decodes cached bytes, enforces schemas, decides what
   is cacheable, and emits no events from runtime adapters.
2. **Runtime owns effects and implementation selection.** It owns CAS/index handles, publisher
   locks/recovery, frontend/link/evaluator/backend instances, and maps adapter errors to a stable
   typed error.
3. **Returned identity is distrusted.** `write_blob` results must equal the manager-computed digest;
   `read_blob` bytes must match the requested digest **and declared size**; cache outputs remain
   decoded and checked. Today digest validation comes from `Cas::get` and output-size validation
   from `VerifiedActionIndex::lookup`; a generic runtime must not accidentally drop either check.
   A compiled unit's `header.frontend_abi` must also equal the frontend ABI frozen into the plan.
   Catalog verification also compares `read_generation_artifact` bytes with independently read CAS
   bytes, preserving the current check that immutable publication and CAS content agree.
4. **Publication is non-cancellable after its commit decision.** Cancellation is checked during
   scheduler admission and again at the start of `BuildExecutor::execute`. Current synchronous
   compile/link/instantiate/backend/publish calls do not observe cancellation that arrives after
   entry, and the initial runtime API should not pretend otherwise. The generation adapter
   preserves the existing crash-recovery contract and reports success once committed.
5. **The runtime emits no kernel events.** `orchestrator` remains the only scheduler-to-protocol
   event mapper, preserving deterministic declaration and completion order.
6. **Descriptor identity is frozen before sealing.** The exact runtime descriptor used in action
   key recipes is retained with the prepared plan; execution must use the same runtime object.

## Migration sequence

1. Introduce neutral `BuildRuntimeError`, `GenerationSpace`, `Generation`, request wrappers, and
   `BuildRuntime`; add `Services::build_runtime`. Do not yet alter scheduling or action data.
2. Add `squish-host::ProductionBuildRuntime`. It owns `Cas`, `VerifiedActionIndex`, target and
   catalog `FileArtifactPublisher`s, plus the current XML frontend/static linker/instantiator/squish
   backend adapters. Construct it from the already validated `StorageLayout` in
   `ProductionHost::open` or `src/main.rs::compose_host`.
3. Move the artifact-generation and build-catalog observers from manager-side publisher
   construction into `ProductionBuildRuntime` construction. Keep repository fault injection
   separate. This preserves the three durability domains documented in
   `docs/design/process-recovery-testing.md`.
4. Acquire the runtime in `prepare_recorded`; pass its descriptor to `build_plan`. Replace
   `XML_FRONTEND_ABI`/`SquishBackend.cache_identity` selection with the frozen descriptor.
5. Replace `BuildExecutor`'s four concrete adapter fields with one runtime. Route blob, action,
   compile, link, instantiate, render, and generation calls through it. Keep state hydration and
   semantic validation in `BuildExecutor`.
6. Convert catalog read/recovery helpers to accept `&dyn BuildRuntime` (or a narrower borrowed
   `BuildCatalogRuntime` subtrait). Remove all `Cas::open` and `FileArtifactPublisher::...` calls
   from `squish-manager`.
7. Update `ProductionHost` inspection methods to query the same composed runtime and reuse the
   manager's pure verified `BuildCatalogSnapshot` projection. Route `cache_records` enumeration and
   `read_blob` through those same retained CAS/index handles rather than reopening them per query.
   Do not create a second catalog reader in the host.
8. Remove direct `squish-store` and `squish-publish` dependencies from `squish-manager` once no
   other manager module uses them. In particular, route `fmt.rs::store_diffs` through the shared
   blob capability if the issue's no-manager-CAS criterion is crate-wide. Add direct dependencies
   to `squish-host` for every concrete adapter it constructs; do not rely on transitive
   dependencies.
9. Only after behavior is green, consider splitting the aggregate into narrower subtraits. Doing
   so initially adds generic/lifetime machinery without improving the issue's ownership boundary.

## Test injection map

| Existing coverage | Current injection/problem | Port-based replacement or addition |
|---|---|---|
| `crates/squish-manager/tests/build.rs::prepared_plan_has_one_compile_per_sealed_source_and_effectful_publish` and plan-digest tests | `LocalServices` supplies only filesystem layout; planning silently selects `SquishBackend` | Inject a deterministic runtime descriptor and assert descriptor changes affect only the appropriate action key/plan digest. |
| `complete_build_publishes_prompt_debug_and_ir_from_cas` and semantic debug-bundle test | Opens real `Cas`/`FileArtifactPublisher` from manager tests | Use a deterministic in-memory blob/action/generation runtime, with real leaf compiler delegates where semantic output is under test. Assert the same published membership and byte/digest validation through the port. |
| `successful_execute_returns_the_persisted_build_record` (warm-cache assertions around lines 488-567) | Persists a real SQLite index and filesystem CAS | Reuse one in-memory runtime across cold/warm manager instances; record calls and corruptible returned manifests. Assert four transform hits and no publish hit exactly as today. |
| `compile_failure_persists_terminal_action_facts` | Failure is induced through malformed source and real compiler | Keep the semantic fixture, and add a faulting `compile` port case to prove adapter errors become one structured action failure and catalog finalization still records terminal facts. |
| cancellation tests around lines 610-688 | Real workers plus cancellation token | Add a runtime method controlled by a barrier. Cancel before worker entry to verify the executor-start check; cancel after entry to document current non-cooperative synchronous behavior and let orchestrator account for completion. The runtime must not emit events. |
| recovery tests around lines 690-935 | Manipulate real publisher lock/generations directly | Inject a generation runtime that can fail before/after an explicit commit decision. Assert prior generation retention, roll-forward, catalog reconciliation, and non-cancellable post-decision success. Keep production publisher recovery as a `squish-host` integration suite. |
| `crates/squish-host/tests/composition.rs` | Covers fetch/resolver composition only | Add a composition test proving `ProductionHost::build_runtime` shares the configured CAS/index, uses distinct target/catalog roots, and selects the expected toolchain descriptor. |
| `tests/process.rs` build/cache/inspect cases | Real end-to-end binary | Retain unchanged as the external compatibility gate for cache reuse, artifacts, structured events, and queries. |
| `tests/recovery_process.rs` | Real process death at publisher observers | Retain unchanged. It is the decisive evidence that moving observer wiring to host composition did not weaken crash safety. |
| `crates/squish-manager/tests/core.rs` orchestrator tests | Already inject `WorkExecutor` directly | No migration required. These remain the focused proof of deterministic event ordering, single-flight, keep-going, and cancellation accounting. |

The in-memory runtime should expose an operation log and independently configurable faults for
blob read/write, cache lookup/record, each generation space, and each toolchain phase. Avoid one
global occurrence counter: faults must identify the capability and boundary, mirroring the current
durability design.

## Dependency and cycle analysis

Current relevant edges are:

```text
xmlsquish(root) -> squish-host -> squish-manager
                         |             |
                         v             +-> squish-store
                  squish-store         +-> squish-publish -> squish-build/squish-protocol
                                       +-> squish-xml-front/squish-link/squish-backend
```

Desired edges are:

```text
xmlsquish(root) -> squish-host -> squish-manager (port + orchestration)
                         |
                         +-> squish-store / squish-publish
                         +-> squish-xml-front / squish-link / squish-backend

squish-manager -> domain data crates (protocol/build/IR/source/link/backend types as needed)
squish-manager -X-> concrete store/publisher adapters
```

Potential cycles and traps:

* **Do not put `BuildRuntime` in `squish-host`.** Manager would need to depend on host while host
  already implements `Services` and depends on manager.
* **Do not put the full semantic runtime trait in `squish-build`.** Its compile/link signatures
  would pull frontend/link/backend upward into the low-level graph/action crate and risk cycles.
* **Do not pass `PreparedBuild` or `BuildWork` to host.** Those manager-owned types would turn the
  adapter into a second executor and make host behavior depend on manager internals.
* **Do not retain `Services::storage_layout` as the build factory API.** A path bundle is not a
  capability; it lets every caller reopen inconsistent handles. It may remain temporarily for
  formatting/inspection migration, but build execution should receive the already composed
  runtime.
* **Avoid host-to-manager-to-host recursion in inspection.** `ProductionHost::current_catalog`
  may call a pure manager verifier with `&dyn BuildRuntime`; that verifier must not call back into
  `Services::artifact`/`planned_actions`.
* **Keep `GenerationSpace` typed.** A single string root or publisher selected by caller would
  allow target and catalog state, locks, or fault observers to alias.

## Acceptance checklist

The refactor is complete only when all of the following hold:

- `rg "Cas|VerifiedActionIndex|FileArtifactPublisher" crates/squish-manager/src/build.rs` has no
  concrete adapter construction or field type (neutral domain aliases excepted).
- No `Cas::open`, `VerifiedActionIndex::open`, or `FileArtifactPublisher::{open,with_observer}`
  remains anywhere in `squish-manager` (including `fmt.rs::store_diffs`).
- One production runtime is composed by `squish-host`/root and one deterministic runtime is used by
  manager tests through the same `BuildRuntime` API.
- Backend/frontend identity in the sealed action plan comes from the exact injected runtime used
  for execution.
- Manager still validates cache bytes, declared output sets, digests, IR/debug/link schemas, and
  generation/catalog membership.
- Transform-only persistent caching, scheduler cancellation, single-flight behavior, and event
  ordering remain owned by `orchestrator` and pass existing tests.
- Real-process recovery tests still demonstrate both pre-decision retention and post-decision
  roll-forward for target and catalog generations.
- `docs/adr/0009-microkernel-manager-and-reusable-ir.md` and
  `crates/squish-manager/ARCHITECTURE.md` no longer claim that concrete CAS/index/publisher
  adapters are manager concerns; `docs/design/process-recovery-testing.md` documents observer
  composition at the host runtime, and `crates/squish-host/IMPLEMENTATION.md` documents the
  production runtime assembly.
