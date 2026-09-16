# `squish-host` production composition map

This document is the integration ledger for the concrete `squish_manager::Services`
composition root. It records contracts rather than copying another crate's private wire or
filesystem representation.

## Required configuration

The host is constructed from one immutable configuration value. The value must explicitly
contain the canonical project root, source-cache root, registry identities and endpoints, a
credential port, an HTTP client policy/client, the Git executable, and the workspace filesystem
port. No constructor is permitted to consult the current directory, environment variables,
home-directory conventions, a process-global client, or an implicit executable search path.

`ProductionHost::open` installs no-op publication observers. Recovery tests and executable
composition may replace them with
`ProductionHost::with_build_observers(target_observer, catalog_observer)`. The two arguments are
intentionally separate: target generations and build-catalog generations are distinct durability
domains and must never depend on a shared occurrence counter.

## Build runtime composition

`Services::open_build_runtime` lazily constructs and then retains one invocation-scoped
`ProductionBuildRuntime` after checking that the requested canonical root is the configured
project. Eager construction is deliberately avoided: storage-open failures remain inside the
manager's recorded planning step rather than becoming unrecorded host bootstrap failures.
Inspection calls reuse that same retained runtime. The runtime owns one shared `Arc<Cas>`, one
`VerifiedActionIndex` bound to that CAS, and two
`FileArtifactPublisher` values backed by the same CAS but rooted in the distinct target and catalog
publication directories. It also selects the production XML frontend, static linker, evaluator,
and squish backend.

The runtime exposes only `squish_manager::BuildRuntime` domain values. Concrete CAS paths,
SQLite handles, publisher journals, current pointers, and immutable-generation paths do not cross
back into the manager. `GenerationSpace::{TargetArtifacts, BuildCatalog}` selects one of the two
publisher instances without adding a third filesystem layout recipe. Runtime methods never emit
kernel events; scheduling and event ordering remain manager responsibilities.

The descriptor currently freezes these semantic identities before plan sealing:

| Stage | Identity |
| --- | --- |
| XML frontend | `xmlsquish.xml/1` |
| static linker | `xmlsquish.link/1` |
| evaluator | `xmlsquish.instantiate/1` |
| linked document | `xmlsquish.document.v1` |

Changing implementation semantics requires changing the corresponding identity so an older
action-cache entry cannot be reused by a different toolchain.

Runtime errors retain their causal category. CAS, SQLite, publisher-store, and publisher-I/O
availability failures are `Storage`; malformed journals, invalid or aliased destinations,
symlink violations, missing publication blobs, digest/size mismatches, and unsupported publisher
digests are `Corrupt`; frontend, linker, evaluator, and backend failures are `Tool`. Manager
recovery policy therefore never mistakes an operational I/O failure for evidence that persisted
state may safely be replaced.

## Port-to-owner map

| `squish_manager::Services` port | Authoritative owner | Contract used by this host |
| --- | --- | --- |
| `materialize_locked` | `squish-fetch` | Exact registry checksum or exact Git commit/tree/content identity; `Offline`, `Locked`, and `Frozen` never refresh remote state. |
| `resolve` | `squish-resolver` + `squish-fetch` ports | `ResolutionInput` and the resolver's lock-first mode semantics, followed by exact materialization of every remote lock node. |
| `cache_records` | manager-owned `squish-store` action index + CAS | Public validated catalog enumeration; corrupt/missing output blobs become recoverable misses and the rebuildable row is discarded. |
| `read_blob` | manager-owned `squish-store::Cas` | Protocol digest conversion followed by CAS integrity validation. The host never creates a parallel blob directory. |
| `artifact`, `artifact_at` | manager-owned publisher/catalog | Public catalog lookup over committed generations; only fully committed and digest-valid records are returned. |
| `link_map` | manager-owned persistent link-map catalog | Target identity lookup; the returned metadata blob remains subject to manager decoding and validation. |
| `provenance_evidence` | manager-owned artifact/provenance catalog | Complete committed companions (debug bundle/build evidence), retaining typed artifact identities. |
| `planned_actions` | manager-owned build-record catalog/codec | Public lossless `PlanInspection` projection containing job, plan, plan digest, mode, topological dependencies, and optional materialized action keys. |

The manager owns the production layout through its public `StorageLayout` contract. The host
returns the exact configured value and opens those same stores. A separately configured source
cache is only for registry/Git acquisition; it never becomes a second artifact truth.

## Resolution-mode truth table

| Mode | May query network | May create a new lock | Prior lock required | Locked remote content requirement |
| --- | ---: | ---: | ---: | --- |
| `Online` | yes | yes | no | acquire and validate as needed |
| `Offline` | no | yes, from local metadata | no | local acquisition cache only |
| `Locked` | yes | no | yes | exact identities only; no ref/version reselection |
| `Frozen` | no | no | yes | all exact content must already be local |

Source-host observer events are preserved so cache hits, local misses, remote access, integrity
recovery, and materialization remain observable. Credentials and response bodies never enter
events.

## Public contracts consumed

The composition deliberately does **not** walk private directories or decode private SQLite/JSON
representations:

1. `SparseRegistry::with_dependencies` receives a caller-owned `HttpTransport`; fetch retains
   redirect, origin/credential-scope, response-bound, and cache-integrity policy.
2. `GitHost::with_runner` receives a caller-owned `GitRunner`; the standard implementation can
   point directly at an explicit executable, while tests use an in-process fixture runner.
3. `VerifiedActionIndex::manifest_page` enumerates validated typed records and lazily
   materializes each canonical action-result record into the same bound CAS. The returned
   `result_digest` therefore satisfies manager blob inspection without inventing a second format.
4. `StorageLayout` is the single manager contract for CAS, action index, publication, and catalog
   locations. The host returns exactly its configured instance after canonical project-root
   matching.
5. `read_current_build_catalog` verifies the current publisher generation, BuildRecord v2 codec,
   every cataloged CAS blob, and its immutable publication before returning
   `BuildCatalogSnapshot`.
6. Artifact ID/path, target link map, provenance relation, and full `PlanInspection` are queried
   only through `BuildCatalogSnapshot`; the host never derives publisher hashes or paths.
7. Locked Git materialization first resolves the already-full revision (never a branch/tag),
   with `GitHost::materialize_locked_revision`, which derives the tree directly from the local
   object DB without requiring a selector observation. Its `LockedGitPackage` returns the verified
   exact candidate and materialized root together; the host rechecks commit/content identity and
   consumes that root directly. `Frozen` performs this sequence using local state only; absence
   is an observable cache miss rather than a remote fallback.

Registry aliases and stable registry identities share one explicitly validated routing namespace.
This is essential because a root manifest may use a friendly alias while registry metadata uses
stable IDs for transitive or cross-registry dependencies. Any alias/identity token owned by two
different endpoints is rejected during construction.

Exact lock materialization always attempts fully verified `LocalOnly` acquisition first. `Online`
and `Locked` retry with network access only after that attempt reports unavailable or invalid
local state; `Offline` and `Frozen` never retry. Both attempts flow through the same fetch
observer, preserving local-hit, corruption-recovery, miss, and network observability.

Before responsibility-overlap checks, every possibly nonexistent configured storage path is
normalized by canonicalizing its longest existing ancestor and reattaching the missing suffix.
This resolves junctions/symlinks and normalizes Windows verbatim (`\\?\`) versus drive spelling;
case folding is applied only for the Windows identity comparison.

## Corruption policy

CAS is authoritative and validates content on read. Catalog/index metadata is rebuildable: a row
whose result record, output blob, size, digest, typed identity, or relation is invalid is removed
or ignored through its owning crate's public recovery API, with an observer event where the owner
supports one. Durable publisher/build-record corruption is not silently interpreted as absence;
the owning codec reports an integrity error and preserves evidence for diagnosis.
