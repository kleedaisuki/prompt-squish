# Project Manager Rework: Execution Status

**Last updated:** 2026-09-15  
**Decision purpose:** Keep the `xmlsquish` compiler-to-project-manager rework inspectable while implementation proceeds in parallel.  
**Normative sources:** The linked ADR and design documents remain authoritative; this file is the durable coordination index and cutover ledger.

## 1. Product Direction

`xmlsquish` is being rebuilt as a Cargo-like project manager with direct `fmt`, `build`, `add`, `remove`, and `inspect` commands. The architecture is intentionally top-down rather than an adapter around the legacy compiler:

```text
CLI bootstrap
    -> protocol request
    -> microkernel lifecycle and event stream
    -> manager capability
    -> repository / resolver / fetch / CAS / publisher ports
    -> XML frontend -> reusable XSIR -> link root -> .prompt + .psdbg
```

The cutover does not preserve the old internal architecture. It does preserve the current XML DSL and makes the external command and artifact contracts explicit before deleting legacy paths.

## 2. Durable Knowledge Map

| Question | Durable artifact | Status |
|---|---|---|
| Why a microkernel manager and reusable IR? | [`../adr/0009-microkernel-manager-and-reusable-ir.md`](../adr/0009-microkernel-manager-and-reusable-ir.md) | Accepted; planning lifecycle amendment in progress |
| What is the complete IR, linking, provenance, and debug model? | [`ir-model.md`](ir-model.md) | Implemented through canonical IR, link trace, and self-contained debug bundle |
| How does the old tree map to the new architecture? | [`refactor-map.md`](refactor-map.md) | Active cutover map |
| What should each CLI command feel like and how is cutover accepted? | [`../product/cli-experience.md`](../product/cli-experience.md) | Command contract and executable K3/K4 gates defined |
| What is in and out of the project-manager product? | [`../product/project-manager-scope.md`](../product/project-manager-scope.md) | Product scope defined |
| How do registry and Git dependencies work? | [`dependency-source-protocol.md`](dependency-source-protocol.md) | Protocol specified; fetch implementation in progress |
| Which industry and academic systems informed the architecture? | [`../research/compiler-manager-ir-prior-art.md`](../research/compiler-manager-ir-prior-art.md) | Compiler/manager/IR synthesis complete |
| Which mature project managers informed the product boundary? | [`../research/project-manager-prior-art.md`](../research/project-manager-prior-art.md) | Prior-art review complete |

The research is not an isolated literature dump. Each research artifact is connected to an ADR, a code boundary, or an executable acceptance gate.

## 3. Implemented and Committed Foundations

| Area | Repository evidence | Verified contract |
|---|---|---|
| Protocol and kernel | `crates/squish-protocol`, `crates/squish-kernel` | Typed operation results, honest unavailable results, request/result identity validation, unique completion lifecycle |
| CLI parser | `crates/squish-cli` | Direct command grammar, bare-help success, typed target arguments, feature parsing |
| Build scheduler | `crates/squish-build` | Dependency blocking, keep-going semantics, cooperative cancellation, stable event behavior |
| Canonical IR and debug bundle | `crates/squish-ir` | Canonical encoding, link/expansion trace separation, self-contained `.psdbg`, provenance cross-reference validation |
| XML frontend and backend pipeline | frontend/link/runtime/backend crates | XML to unit IR, link-root execution, prompt emission |
| Repository model | `crates/squish-repository` | Frozen workspace observations, source ownership, lock/package identity, recoverable transactions |
| Dependency resolver | `crates/squish-resolver` | Deterministic backtracking, single-version registry domains, lock-first Git behavior, workspace inheritance |
| Content and action storage | `crates/squish-store` | Verified CAS and persistent action index with migration/concurrency handling |
| Artifact publication | `crates/squish-publish` | Crash-recoverable whole-target generation publication with one logical commit point |
| Cross-platform CI | `.github/workflows` | Workspace-aware desktop matrix |

Relevant committed changes include `de3cbcf`, `b8f89fb`, `0ccfda8`, `c28a416`, `9f549e0`, `5260993`, `6d6853e`, `a8884f3`, `ad670dd`, `14ad2ff`, and `0b51912`.

## 4. Active Implementation Ownership

Parallel work uses single-writer ownership for each area to avoid shared-worktree collisions.

| Workstream | Owner | Required durable output | Completion condition |
|---|---|---|---|
| Manager capability and orchestration | `manager_core2` tree | Manager code plus architecture/code mapping in the design docs | All five operations execute real scheduled work; persistent cache restoration and generation publication pass tests and review |
| Planning lifecycle | `planning_lifecycle_arch` | Amendment to ADR 0009 | Planning I/O, cancellation, failure, immutable plan publication, and lock writes have one coherent observable state machine |
| Registry/Git fetch host | `fetch_host` | `squish-fetch` code plus protocol implementation mapping | Online/offline/frozen behavior, immutable identity, cache repair, and platform path rules pass tests and review |

Agent messages are not accepted as the only record of reusable conclusions. Each workstream must update a repository document and point it to code and tests before integration.

## 5. Known Open Technical Risks

These are confirmed review findings or unresolved architecture decisions, not speculative backlog items.

| Priority | Risk | Required resolution |
|---|---|---|
| P1 | Frontend ABI is not yet explicit in compile action identity | Include logical source identity and frontend ABI in the canonical compile key |
| P1 | Single-flight or persistent hits for link/instantiate/backend can skip target-local in-memory state | Restore typed named outputs from CAS for every hit/follower; never depend on an executor having run for each `ActionId` |
| P1 | Persistent action-result lookup currently covers compile more fully than later transforms | Implement and verify action-key-to-output manifests for every cacheable transform |
| P1 | Build planning performs meaningful I/O before the immutable execution plan exists | Adopt the planning lifecycle amendment; make planning observable and cancellable without fake replay actions |
| P1 | Root binary still has not cut over to the new CLI/kernel/manager composition | Build the production host and process-level contract tests before deleting the legacy path |
| P2 | Multi-origin provenance can be flattened if backend artifacts are attributed to the first source | Preserve the provenance DAG through backend emission and verify fused/concatenated origins |

Previously reported format precomputation, empty mutation stages, failure-path build-record panic, and per-file publication have implementation fixes, but they remain subject to focused re-review because the original reviewer supplied concrete failure evidence.

## 6. Cutover Gates

The work is not complete until all of the following hold:

1. `xmlsquish fmt`, `build`, `add`, `remove`, and `inspect` enter through the new typed CLI and microkernel; there is no legacy fallback.
2. Every expensive or mutating phase is real scheduled work with truthful events, cancellation, and failure attribution; no empty lifecycle playback remains.
3. A warm process invocation can restore all cacheable transform outputs from the persistent action index and CAS.
4. Each target publishes `.prompt`, `.xsir`, `.psdbg`, and its build record as one generation.
5. Registry, Git, path, and workspace dependencies obey the documented online/offline/locked/frozen source protocol.
6. Process-level fixtures exercise success, diagnostics, cancellation, cache hits, mutation contention, and inspect provenance on Windows, macOS, and Linux CI.
7. The old compiler entry path and obsolete tests are removed only after the replacement gates pass.
8. An independent reviewer rechecks the confirmed manager defects, followed by validator execution of the affected regressions and the full workspace suite.

## 7. Evidence Discipline

Status terms in this ledger have precise meanings:

- **Specified:** a durable contract exists, but production code may not yet implement it.
- **Implemented:** code exists and its local focused tests passed.
- **Reviewed:** an independent agent inspected the affected execution paths; any concrete defects were resolved.
- **Cut over:** the root binary and process tests use only the replacement path.
- **Complete:** cross-platform workspace validation passes and no required legacy fallback remains.

This distinction prevents a design document, a green unit test, or an agent completion message from being mistaken for a finished product.
