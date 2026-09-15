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
   checks its locked content checksum, then calls `GitHost::materialize_locked` with the recovered
   typed commit and package-tree identities. `Frozen` performs this sequence using local state
   only; absence is an observable cache miss rather than a remote fallback.

## Corruption policy

CAS is authoritative and validates content on read. Catalog/index metadata is rebuildable: a row
whose result record, output blob, size, digest, typed identity, or relation is invalid is removed
or ignored through its owning crate's public recovery API, with an observer event where the owner
supports one. Durable publisher/build-record corruption is not silently interpreted as absence;
the owning codec reports an integrity error and preserves evidence for diagnosis.
