# Historical Compiler-to-Manager Cutover Map

## Status and reading rules

This is the **completed, historical cutover map** for the compiler-to-project-
manager rework accepted on 2026-09-15. It is not an active migration plan and
does not authorize a second compiler route. The current implementation is the
workspace-crate architecture described below; the acceptance ledger is
[`project-manager-execution-status.md`](project-manager-execution-status.md).

Paths under the heading **Historical pre-cutover sources** are deliberately
retained only to explain where responsibilities came from. Those files were
deleted during the cutover. Every path used to describe the current
implementation exists in the accepted tree.

## Current system

`xmlsquish` is a microkernel-style executable. The root composition adapter
parses configuration, wires one manager capability into the kernel, connects
host services and presentation, and translates the terminal outcome to a
process exit status:

```text
argv
  -> squish-cli + squish-config
  -> squish-protocol request
  -> squish-kernel lifecycle
  -> squish-manager planning and orchestration
  -> repository / resolver / fetch / source snapshot
  -> XML frontend -> canonical XSIR -> static link -> instantiation
  -> squish backend -> .prompt
  -> debug bundle + recoverable publication -> .psdbg + build catalogue
  -> presentation
```

The composition root is [`../../src/main.rs`](../../src/main.rs). Cooperative
and emergency interrupt coordination is isolated in
[`../../src/interrupt.rs`](../../src/interrupt.rs). The root contains no legacy
`cli`, `compiler`, or `squish` module.

### Current ownership map

| Responsibility | Current owner and evidence |
|---|---|
| Filesystem-free argument parsing | [`../../crates/squish-cli/src/lib.rs`](../../crates/squish-cli/src/lib.rs) converts `argv` into typed requests and invocation settings. |
| Layered configuration | [`../../crates/squish-config/src/lib.rs`](../../crates/squish-config/src/lib.rs) loads configuration independently of command execution. |
| Wire and lifecycle vocabulary | [`../../crates/squish-protocol/src/lib.rs`](../../crates/squish-protocol/src/lib.rs) owns operation requests, events, typed IDs, diagnostics, artifacts, plans, and finalization records. |
| Microkernel dispatch and validation | [`../../crates/squish-kernel/src/lib.rs`](../../crates/squish-kernel/src/lib.rs) routes capabilities and validates planning, plan, action, finalization, cancellation, and completion transitions. |
| Unified manager capability | [`../../crates/squish-manager/src/lib.rs`](../../crates/squish-manager/src/lib.rs) is the sole capability for `fmt`, `build`, `add`, `remove`, and `inspect`. |
| Shared manager orchestration | [`../../crates/squish-manager/src/orchestrator.rs`](../../crates/squish-manager/src/orchestrator.rs) records planning, seals plans, drives the scheduler, maps worker facts to events, and coordinates finalization without printing from workers. |
| Domain-specific plans and workers | [`../../crates/squish-manager/src/build.rs`](../../crates/squish-manager/src/build.rs), [`../../crates/squish-manager/src/fmt.rs`](../../crates/squish-manager/src/fmt.rs), [`../../crates/squish-manager/src/mutation.rs`](../../crates/squish-manager/src/mutation.rs), and [`../../crates/squish-manager/src/inspect.rs`](../../crates/squish-manager/src/inspect.rs). |
| Typed plan graph and bounded scheduler | [`../../crates/squish-build/src/plan.rs`](../../crates/squish-build/src/plan.rs) and [`../../crates/squish-build/src/scheduler.rs`](../../crates/squish-build/src/scheduler.rs). |
| Project manifests, locks, and mutation transactions | [`../../crates/squish-project/src/manifest.rs`](../../crates/squish-project/src/manifest.rs), [`../../crates/squish-project/src/lock.rs`](../../crates/squish-project/src/lock.rs), and [`../../crates/squish-project/src/transaction.rs`](../../crates/squish-project/src/transaction.rs). |
| Frozen repository observations | [`../../crates/squish-repository/src/repository.rs`](../../crates/squish-repository/src/repository.rs), [`../../crates/squish-repository/src/snapshot.rs`](../../crates/squish-repository/src/snapshot.rs), and [`../../crates/squish-source/src/lib.rs`](../../crates/squish-source/src/lib.rs). |
| Dependency resolution and acquisition | [`../../crates/squish-resolver/src/lib.rs`](../../crates/squish-resolver/src/lib.rs) and [`../../crates/squish-fetch/src/lib.rs`](../../crates/squish-fetch/src/lib.rs). |
| XML syntax and semantic lowering | [`../../crates/squish-format/src/lib.rs`](../../crates/squish-format/src/lib.rs) owns loss-preserving formatting; [`../../crates/squish-xml-front/src/lib.rs`](../../crates/squish-xml-front/src/lib.rs) owns semantic compilation to relocatable IR. |
| Reusable binary IR and inspection | [`../../crates/squish-ir/src/lib.rs`](../../crates/squish-ir/src/lib.rs), with codecs, validation, persistent schemas, link/document wire formats, trace encoding, and debug bundles in the same crate. |
| Static linking and instantiation | [`../../crates/squish-link/src/linker.rs`](../../crates/squish-link/src/linker.rs), [`../../crates/squish-link/src/program.rs`](../../crates/squish-link/src/program.rs), and [`../../crates/squish-link/src/instantiate.rs`](../../crates/squish-link/src/instantiate.rs). |
| Output backend | [`../../crates/squish-backend/src/lib.rs`](../../crates/squish-backend/src/lib.rs) consumes linked document IR and expansion trace; the squish backend is the first implementation. |
| Content and action storage | [`../../crates/squish-store/src/cas.rs`](../../crates/squish-store/src/cas.rs) and [`../../crates/squish-store/src/action.rs`](../../crates/squish-store/src/action.rs). |
| Recoverable artifact generations | [`../../crates/squish-publish/src/lib.rs`](../../crates/squish-publish/src/lib.rs). |
| Concrete production adapters | [`../../crates/squish-host/src/lib.rs`](../../crates/squish-host/src/lib.rs) composes filesystem, registry, Git, credentials, CAS, action-index, publisher, and catalogue services. |
| Human, short, and NDJSON presentation | [`../../crates/squish-presentation/src/lib.rs`](../../crates/squish-presentation/src/lib.rs) owns color, progress, terminal-width observation, stable non-TTY output, inspection rendering, and final summaries. |

The crate boundary is the enforcement mechanism: semantic crates do not own
CLI parsing or terminal output, and workers return typed facts instead of
printing or choosing process exits.

## Current representation and build flow

### Source representations

Formatting and semantic compilation intentionally use different
representations. [`../../crates/squish-format/src/syntax.rs`](../../crates/squish-format/src/syntax.rs)
retains the lexical information needed for conservative rewrites. The semantic
frontend in [`../../crates/squish-xml-front/src/parser.rs`](../../crates/squish-xml-front/src/parser.rs)
and [`../../crates/squish-xml-front/src/lower.rs`](../../crates/squish-xml-front/src/lower.rs)
produces relocatable units with source identities, spans, imports, definitions,
operations, and provenance. Formatting never round-trips through semantic IR.

### Binary IR, link, instantiate, backend

The canonical reusable representation is the versioned binary XSIR contract in
[`../../crates/squish-ir/src/codec.rs`](../../crates/squish-ir/src/codec.rs),
[`../../crates/squish-ir/src/wire.rs`](../../crates/squish-ir/src/wire.rs), and
[`../../crates/squish-ir/src/validate.rs`](../../crates/squish-ir/src/validate.rs).
It retains information required for machine reuse rather than trimming the
program to one rendered output.

Static linking in [`../../crates/squish-link/src/linker.rs`](../../crates/squish-link/src/linker.rs)
resolves a frozen unit closure into a `LinkedProgram`. Instantiation in
[`../../crates/squish-link/src/instantiate.rs`](../../crates/squish-link/src/instantiate.rs)
applies entry arguments and budgets to produce `LinkedDocumentIr` and
`ExpansionTrace`. The backend in
[`../../crates/squish-backend/src/lib.rs`](../../crates/squish-backend/src/lib.rs)
then emits the product. This preserves the LLVM-inspired frontend/IR/backend
separation without claiming that the XML frontend restricts future output
media.

### Storage and public products

Immutable bytes are stored by digest in the content-addressed store. The action
index records validated transform results. Selected products are committed as
recoverable generations instead of sibling temporary files. The public build
vocabulary is:

| Product | Meaning |
|---|---|
| `*.xsir` | Canonical, reusable binary module IR exported when requested. |
| `*.prompt` | Final output produced by the squish backend. |
| `*.psdbg` | Self-contained debug and provenance bundle for the selected product. |
| build record/catalogue | Durable plan, action, generation, and terminal evidence used by inspection and recovery. |

The detailed schemas and evidence limits are recorded in
[`ir-model.md`](ir-model.md) and
[`project-storage-namespaces.md`](project-storage-namespaces.md).

## Historical pre-cutover sources and completed destinations

Every path in the first column is a **deleted pre-cutover path**, not a current
implementation locator. The right column names existing current owners.

| Deleted pre-cutover path (historical only) | Completed destination |
|---|---|
| `src/cli/mod.rs` | Typed parsing moved to [`../../crates/squish-cli/src/lib.rs`](../../crates/squish-cli/src/lib.rs); root composition and dispatch moved to [`../../src/main.rs`](../../src/main.rs). |
| `src/cli/console.rs` and `src/cli/diagnostics.rs` | Terminal capabilities, color/progress, stable rendering, and structured diagnostics moved to [`../../crates/squish-presentation/src/lib.rs`](../../crates/squish-presentation/src/lib.rs) and protocol types in [`../../crates/squish-protocol/src/lib.rs`](../../crates/squish-protocol/src/lib.rs). |
| `src/cli/pipeline.rs` | Planning, immutable action graphs, bounded execution, lifecycle events, and finalization moved to [`../../crates/squish-manager/src/orchestrator.rs`](../../crates/squish-manager/src/orchestrator.rs), [`../../crates/squish-build/src/plan.rs`](../../crates/squish-build/src/plan.rs), and [`../../crates/squish-build/src/scheduler.rs`](../../crates/squish-build/src/scheduler.rs). |
| `src/cli/files.rs` and `src/cli/paths.rs` | Source identity/snapshots, project discovery, storage, and publication moved to [`../../crates/squish-source/src/lib.rs`](../../crates/squish-source/src/lib.rs), [`../../crates/squish-repository/src/repository.rs`](../../crates/squish-repository/src/repository.rs), [`../../crates/squish-store/src/lib.rs`](../../crates/squish-store/src/lib.rs), and [`../../crates/squish-publish/src/lib.rs`](../../crates/squish-publish/src/lib.rs). |
| `src/compiler/parser.rs` | XML parsing and semantic lowering moved to [`../../crates/squish-xml-front/src/parser.rs`](../../crates/squish-xml-front/src/parser.rs) and [`../../crates/squish-xml-front/src/lower.rs`](../../crates/squish-xml-front/src/lower.rs). |
| `src/compiler/model.rs` | Typed relocatable, linked-document, trace, and persistent schemas moved to [`../../crates/squish-ir/src/model.rs`](../../crates/squish-ir/src/model.rs), [`../../crates/squish-ir/src/document_wire.rs`](../../crates/squish-ir/src/document_wire.rs), and [`../../crates/squish-ir/src/persistent.rs`](../../crates/squish-ir/src/persistent.rs). |
| `src/compiler/runtime.rs` | Static linking, entry instantiation, budgets, linked-document construction, and expansion provenance moved to [`../../crates/squish-link/src/linker.rs`](../../crates/squish-link/src/linker.rs) and [`../../crates/squish-link/src/instantiate.rs`](../../crates/squish-link/src/instantiate.rs). |
| `src/compiler/mod.rs` | The monolithic string-returning compiler contract was replaced by the frontend, IR, link, backend, and manager boundaries listed above. |
| `src/squish.rs` | Squish output behavior moved behind the backend contract in [`../../crates/squish-backend/src/lib.rs`](../../crates/squish-backend/src/lib.rs). |

The deleted loose-file route, sibling `*.i.xml`/`*.o.xml` convention, rendered
intermediate-string contract, and hidden fallback are not compatibility paths.
The root exposes only the manager route.

## Current external contract

```text
xmlsquish fmt [selection] [--check]
xmlsquish build [selection] [profile/options]
xmlsquish add <dependency> [source/options]
xmlsquish remove <dependency>
xmlsquish inspect <ir|link|source|cache|artifact> <selector>
```

The exact flags, output modes, exit behavior, and superseded proposal history
are maintained in
[`../product/cli-experience.md`](../product/cli-experience.md). The parser
contract tests are in
[`../../crates/squish-cli/tests/cli_contract.rs`](../../crates/squish-cli/tests/cli_contract.rs),
and composed process behavior is covered by
[`../../tests/process.rs`](../../tests/process.rs).

## Current scheduling and state model

All five commands enter `ManagerCapability` and use typed `PreparedPlan` data.
Domain modules construct their own work payloads; the shared orchestrator is the
only manager code that drives `squish_build::Scheduler` or translates worker
facts into protocol lifecycle events.

For executable work, the observable sequence is:

```text
Job
  -> PlanningAttempt and real planning steps
  -> immutable PreparedPlan + complete ActionDeclared set
  -> bounded Scheduler execution
  -> PlanClosed
  -> sequential typed finalization when required
  -> OperationCompleted
```

The current invariants are implemented in:

- [`../../crates/squish-manager/src/model.rs`](../../crates/squish-manager/src/model.rs):
  exact one-to-one correspondence between plan actions and typed work;
- [`../../crates/squish-manager/src/orchestrator.rs`](../../crates/squish-manager/src/orchestrator.rs):
  real planning observation, stable declarations, bounded worker admission,
  supersession, deterministic completion handling, and finalization;
- [`../../crates/squish-kernel/src/lib.rs`](../../crates/squish-kernel/src/lib.rs):
  lifecycle validation and final outcome reduction;
- [`../../crates/squish-manager/src/mutation.rs`](../../crates/squish-manager/src/mutation.rs):
  candidate planning followed by revision-checked manifest/lock mutation and
  bounded re-planning on a pre-decision race; and
- [`../../crates/squish-manager/src/build.rs`](../../crates/squish-manager/src/build.rs):
  build planning, cache restoration, typed generation publication, `BuildRecordV3`,
  catalogue recovery, and `PersistBuildCatalog` finalization.

`--dry-run` is represented as report-only planning rather than fake execution.
Pre-plan failure can terminate with zero actions. A worker never writes the
terminal stream, and plan completion is not inferred from output ordering.

## Current verification map

| Contract | Current executable evidence |
|---|---|
| CLI and all five composed commands | [`../../crates/squish-cli/tests/cli_contract.rs`](../../crates/squish-cli/tests/cli_contract.rs), [`../../tests/process.rs`](../../tests/process.rs), and [`../../.github/scripts/ci_smoke.py`](../../.github/scripts/ci_smoke.py) |
| Kernel lifecycle and finalization | Unit tests in [`../../crates/squish-kernel/src/lib.rs`](../../crates/squish-kernel/src/lib.rs) |
| Plan/scheduler invariants | [`../../crates/squish-build/src/tests.rs`](../../crates/squish-build/src/tests.rs) and [`../../crates/squish-manager/tests/core.rs`](../../crates/squish-manager/tests/core.rs) |
| Frontend and canonical IR | [`../../crates/squish-xml-front/src/tests.rs`](../../crates/squish-xml-front/src/tests.rs) and the tests embedded beside the IR codecs/validators under [`../../crates/squish-ir/src`](../../crates/squish-ir/src) |
| Link, instantiate, provenance, backend | [`../../crates/squish-link/src/tests.rs`](../../crates/squish-link/src/tests.rs), [`../../crates/squish-backend/src/tests.rs`](../../crates/squish-backend/src/tests.rs), and the multi-module process regression in [`../../tests/process.rs`](../../tests/process.rs) |
| Format semantics | [`../../crates/squish-format/src/tests.rs`](../../crates/squish-format/src/tests.rs), [`../../crates/squish-format/tests/adversarial_validation.rs`](../../crates/squish-format/tests/adversarial_validation.rs), and [`../../crates/squish-format/tests/semantic_oracle.rs`](../../crates/squish-format/tests/semantic_oracle.rs) |
| Build, mutation, and inspect behavior | [`../../crates/squish-manager/tests/build.rs`](../../crates/squish-manager/tests/build.rs), [`../../crates/squish-manager/tests/mutation.rs`](../../crates/squish-manager/tests/mutation.rs), and [`../../crates/squish-manager/tests/inspect.rs`](../../crates/squish-manager/tests/inspect.rs) |
| Real terminal interaction | [`../../tests/pty.rs`](../../tests/pty.rs) |
| Real process-death recovery | [`../../tests/recovery_process.rs`](../../tests/recovery_process.rs) and [`process-recovery-testing.md`](process-recovery-testing.md) |
| Supported desktop matrix | [`../../.github/workflows/ci.yml`](../../.github/workflows/ci.yml); accepted run and exact limits are recorded in [`project-manager-execution-status.md`](project-manager-execution-status.md) |

The accepted GitHub Actions matrix runs Rust 1.88 quality checks on Ubuntu,
full all-target/all-feature builds and workspace tests on Linux, macOS, and
Windows, composed CLI smoke on all three, source-install smoke on Linux, and
the site checks/build. It does not claim source-install coverage on macOS or
Windows.

## Preserved limits and future evidence

Completion of this cutover does not erase explicit evidence limits:

- authenticated registry behavior is accepted at component/composition scope;
  the CI workflow does not contact a live authenticated registry;
- source installation is exercised remotely on Linux only; and
- Publication layout is now exclusively owned by
  [`../../crates/squish-publish/src/lib.rs`](../../crates/squish-publish/src/lib.rs).
  Manager recovery exchanges `PublicationTargetId`, `GenerationRef`,
  `ArtifactDescriptor`, and `PublicationPath` through the typed
  `GenerationRepository` port; it never reconstructs publisher paths.

These are not hidden fallback implementations and did not invalidate the
accepted milestone. Any future migration must be opened from concrete product
requirements or failing evidence rather than by treating this historical map
as an unfinished checklist.
