# Project-local cache and compilation-state map

- Status: repository investigation for the v1.0.4 storage redesign
- Scope: cache ownership, compilation metadata, `target/xmlsquish`, path resolution,
  `build`/`clean`/prospective `watch`, and affected tests
- Evidence date: 2026-09-20
- Constraint: this document describes the current repository and migration surface; it does not
  preserve the current global-cache design as a requirement.

## 1. Executive finding

The current executable has **three derived-state roots in two ownership domains**:

1. the configured project target directory (default `<project>/target/xmlsquish`) contains only
   stable user products;
2. `source.cache-root` contains fetched dependency data and defaults below the machine-level
   configuration home; and
3. `manager.storage-root` contains the build CAS, the SQLite action index, and a hash-partitioned
   per-project publication/build catalog. It also defaults below the machine-level configuration
   home.

The global behavior is not inherent to the compiler. It is chosen at the executable composition
root in `src/main.rs::compose_host` and in the default values of `squish-config`. The compiler and
manager already consume an explicit `StorageLayout`/`HostConfig`; therefore moving derived data
below the project target is principally a **composition and lifecycle migration**, not a rewrite of
action-key or IR semantics.

The difficult part is destructive behavior. Today `clean` deliberately removes the public target
and the project-private catalog while preserving the shared CAS and action index. If all caches and
metadata move below `target/xmlsquish`, recursively removing that directory also removes the cache,
SQLite WAL files, publication recovery state, and dependency materializations. The current clean
journal and lock are derived from the global catalog namespace specifically so they survive deletion
of both current clean participants. A local redesign must choose a coordination point outside the
deleted subtree or simplify the clean contract and prove crash recovery again.

## 2. Current physical ownership

### 2.1 Defaults and override chain

`crates/squish-config/src/model.rs::State::defaults` derives both storage defaults from
`ConfigHome`:

```text
<config-home>/
|-- config.toml
|-- cache/
|   `-- sources/                 source.cache-root
`-- state/                       manager.storage-root
```

`src/main.rs::config_home` selects `XMLSQUISH_HOME` when set, otherwise
`%APPDATA%/xmlsquish` on Windows, `$XDG_CONFIG_HOME/xmlsquish` or
`~/.config/xmlsquish` on Unix, and finally an absolute `.xmlsquish` fallback.

Both paths can currently be replaced by:

- user or workspace TOML: `[source].cache-root`, `[manager].storage-root`;
- environment: `XMLSQUISH_SOURCE_CACHE_ROOT`, `XMLSQUISH_STORAGE_ROOT`; or
- repeated CLI `--config` values.

Relative file-layer paths are resolved against the declaring config directory; relative CLI and
environment-derived overrides use the current working directory. `ConfigLoader` loads defaults,
user config, workspace config, then ordered CLI overrides. The executable translates environment
variables into that final override layer.

### 2.2 Layout composed for one project

After project discovery has produced a canonical root, `src/main.rs::compose_host` constructs:

```text
manager.storage-root/
|-- cas/                                      StorageLayout.cas_root
|   `-- blobs/v1/blake3/aa/<62-hex>          immutable build/intermediate blobs
|-- actions.sqlite3                           StorageLayout.action_index
|-- actions.sqlite3-wal                       SQLite runtime sidecar, when present
|-- actions.sqlite3-shm                       SQLite runtime sidecar, when present
`-- catalog/projects/<project-namespace>/     StorageLayout.catalog_root
    |-- target-publication-state/
    |   |-- generations/<target-key>/<generation-id>/
    |   |   |-- manifest.json
    |   |   `-- artifacts/<project-relative-locator>
    |   |-- targets/<target-key>/current.json
    |   `-- generation-journal.json           transient recovery record, when needed
    |-- build-catalog-state/
    |   |-- generations/<target-key>/<generation-id>/...
    |   |-- targets/<target-key>/current.json
    |   `-- generation-journal.json
    |-- build-catalog-artifacts/               artifact root (metadata is not projected here)
    `-- .squish-publish/                       legacy migration input only

source.cache-root/
`-- v1/
    |-- stage/                                 writer locks and staging
    |-- quarantine/
    |-- materialized/sha256/<2-hex>/<64-hex>/
    |-- registry-materialized/<mapping>
    |-- blobs/sha256/<2>/<2>/<digest>           exact registry archives
    |-- sparse/...                             sparse-index bodies/metadata
    |-- git/db/<repository-hash>/<object-format>/
    |-- git/observations/<repository-hash>/<selector-hash>.json
    `-- clean.lock

<project>/<workspace.target-dir>/              default: target/xmlsquish
|-- <declared>.prompt
|-- <declared>.psdbg                           only when requested
`-- <output-parent>/ir/<output-stem>/<package>/<source>.xsir
```

`project-namespace` is a domain-separated BLAKE3 hash of the canonical project-root path
(`src/main.rs::project_namespace`). On Unix it hashes raw path bytes; on Windows it hashes UTF-16LE
code units. This path opacity and cross-project partition exist only because the catalog is global.
A project-local catalog no longer needs path-derived namespacing.

### 2.3 What the build CAS and action index contain

`squish-store::Cas` uses `blobs/v1/blake3/aa/<remaining-hex>` and rehashes every read. During a
normal build, `crates/squish-manager/src/build.rs::BuildExecutor` stores compiler IR, linked image
and link evidence, instantiated document/trace, backend prompt/debug data, static link map, target
record, and the final BuildRecord in this CAS. User outputs and private metadata therefore share
one immutable byte store even though only selected output kinds are projected into the target.

`VerifiedActionIndex` binds `actions.sqlite3` to that CAS. It records successful transform actions:
compile, link, instantiate, and backend. Publish is deliberately not reused as a cache hit because
it materializes/reconciles the stable product projection. Each hit verifies every declared output
through the CAS; missing/corrupt output turns the action into a miss and removes the rebuildable
row. SQLite is opened in WAL mode with tables `actions`, `action_results`, `digest_results`, and
`run_events`.

## 3. Compilation metadata and publication data structures

The important split is between **logical metadata** owned by the manager and **physical generation
state** owned by the publisher.

| Structure | Owner | Semantics |
| --- | --- | --- |
| `StorageLayout` | `squish-manager::services` | Explicit absolute paths for CAS, action index, public artifact root, logical publication prefix, and catalog root. |
| `BuildRecordV3` | `squish-manager::build` | Schema, job, sealed plan, terminal action facts, all current target generations, and catalog target identity. |
| `RecordedGeneration` | `squish-manager::build` | Target ID, content-derived generation ID, and full typed artifact membership. |
| `CatalogArtifact` | `squish-manager::build` | Physical-layout-independent descriptor plus stable logical destination. |
| `TargetRecordV1` | private manager wire type | Manifest snapshot identity and every other artifact in one target generation. |
| `ActionRecord` | `squish-build`/`squish-store` | Action key plus named output kinds, digests, and sizes. |
| `CommittedGeneration` | `squish-build` | Typed target/generation identity plus verified members. |
| `GenerationManifest` | private `squish-publish` format | Physical immutable generation manifest and current-pointer payload. |

`FileArtifactPublisher::materialize_generation` exposes only `Prompt`, `BinaryIr`, and `DebugInfo`.
`Metadata` and `Other` (including target record and static link map) remain in the private generation
and CAS. This is why `.xsmap`, `.build.json`, generation hashes, and journals are absent from the
visible target even though their logical destinations occur in the generation catalog.

The BuildRecord is itself stored as `build-record-v3.json` in the private build-catalog generation.
Its bytes are also CAS-addressed. `read_current_build_catalog` validates the catalog generation,
every current target generation, every referenced CAS blob, schemas, sizes, digests, and target
record coverage before returning it to `inspect`.

## 4. Build and inspect call chains

### 4.1 Process composition and path resolution

```text
argv
  -> squish-cli: typed request, ProjectPath (default ".")
  -> src/main.rs::run
       ProjectRepository::discover
         implicit "."  -> canonicalize and search ancestors
         explicit path -> exact xmlsquish.toml or exact containing directory; no ancestor fallback
       set canonical project root back into request
       ConfigLoader(user home, workspace root, CLI/environment overrides)
       configured_target_dir(root manifest)
       compose_host
         StorageLayout(global CAS, global SQLite, project-hash catalog, project target)
         HostConfig(global source cache, StorageLayout, adapters)
       Kernel/Manager dispatch
```

`ProjectRepository::discover` recovers repository transactions and returns a canonical absolute
root. The manager intentionally discovers again from the canonical explicit request to keep the
domain operation independently valid. The repository snapshot resolves the shared target directory
from the root manifest (default `target/xmlsquish`) and validates output collisions.

The target path uses two coordinate systems:

- `publication_root`: absolute `<project>/<target-dir>` used for filesystem materialization;
- `publication_prefix`: project-relative `<target-dir>` retained in stable protocol locators.

`build.rs::publication` strips the canonical repository root from each resolved absolute output,
producing a locator such as `target/xmlsquish/chat.prompt`. The publisher then strips the logical
prefix exactly once before joining the artifact root. Preserving that distinction prevents the old
duplicated `target/xmlsquish/target/xmlsquish/...` bug.

### 4.2 Build execution

```text
manager build planning
  -> discover/recover repository
  -> Services::open_build_runtime
       ProductionBuildRuntime lazily opens CAS / VerifiedActionIndex / publishers
  -> recover BuildRecord and target generations
  -> snapshot manifest + lock; materialize locked dependencies
  -> resolve; freeze final snapshot and sources
  -> select targets; freeze toolchain/cache identities
  -> build closed action graph
  -> compile -> link -> instantiate -> backend
       lookup action in SQLite
       verify referenced CAS outputs
       hydrate in-memory stage on hit, otherwise execute + CAS write + index record
  -> publish target generation
       stage all members in private target-publication-state
       persist generation journal
       project user kinds to stable target paths
       atomically replace current.json
  -> finalization: encode BuildRecordV3, CAS-write, publish build-catalog generation
```

The runtime is invocation-scoped but cached inside one `ProductionHost`; individual persistence
capabilities initialize lazily. Target and catalog publishers share a project lock, project epoch,
CAS, and host lifetime, but have separate state roots.

### 4.3 Inspect consumers

`inspect cache` enumerates `VerifiedActionIndex::manifest_page`, verifies referenced CAS output,
and materializes an encoded action-result record into the same CAS. Artifact, link, source, and
provenance views read the validated current build catalog and then read CAS/generation members.
Consequently a path migration must update both build composition and query composition; moving only
the writer will make inspection report absence/corruption.

## 5. Clean call chain and destructive boundary

```text
CleanRequest
  -> manager clean planning (one non-cacheable write action)
  -> ProjectRepository::discover(exact canonical project)
  -> ProductionHost::clean_project
       acquire publisher's cross-process project lock, cancellation-aware
       recover any existing clean redo journal
       persist next clean epoch and redo journal
       rename publication root to same-volume tombstone; delete without following links
       rename catalog root to same-volume tombstone; delete without following links
       remove clean journal
       squish_fetch::clean_dependency_cache
         delete quarantine entries
         delete invalid/incomplete materialized trees
         delete stale registry mappings
         skip producer-locked entries
       return typed counts
```

Current clean intentionally preserves `StorageLayout.cas_root`, `action_index`, healthy dependency
data, sparse metadata, Git databases, and registry archives. The epoch fences already-open
publishers so an old runtime cannot recreate deleted state. The external redo journal and lock are
under the global catalog's `.locks` sibling (via `project_lock_path`), not inside the project catalog
being deleted. Tombstones are siblings of each removed root so rename stays on the same volume.

### Local-layout consequence

If the v1.0.4 contract is “all derived compilation state and caches are owned by the project and
live under `target/xmlsquish`,” the simplest coherent clean is deletion of that entire derived root.
Do not retain the present special case “delete products/catalog but preserve CAS/action/source
cache” merely because the old global layout required it. However:

- the clean lock/journal cannot be placed inside the directory it coordinates;
- Windows-open SQLite WAL handles make unlink-in-place unsafe; clean should still serialize against
  active runtimes and detach the whole root by same-volume rename before recursive deletion;
- a project-root or `target/` sibling coordination file is required if crash-recoverable clean is
  retained;
- `ProductionHost::open` currently creates the source-cache staging directory before clean recovery,
  so moving the source cache inside the clean root requires recovery/order changes to avoid
  recreating a partially cleaned tree;
- the current host rejects physical overlap between source cache and every storage responsibility.
  A nested unified root therefore needs either non-overlapping children (recommended) or revised
  validation that reasons about one owning aggregate rather than rejecting its children.

## 6. `watch` status

There is no production `watch` command, CLI grammar, scheduler loop, or filesystem watcher in this
repository. ADR 0008 explicitly says a snapshot does not watch for changes; each invocation reloads
a new snapshot. The only repository mention beyond that is research suggesting Salsa for a future
in-process project database.

This absence matters to storage design: no compatibility constraint requires a machine-global cache
for a resident watcher. A future watcher should hold one project-local runtime and rebuild from new
snapshots, coordinate with `clean` through the same project lock/epoch, and treat target/cache writes
as self-generated events to exclude from source invalidation.

## 7. Recommended local ownership seam (design input, not an implemented contract)

The existing public/private type boundary can support a single project-owned aggregate without
changing action keys or artifact locators. One low-special-case shape is:

```text
<project>/<target-dir>/                       default target/xmlsquish
|-- <declared>.prompt                         stable user product paths remain direct
|-- <declared>.psdbg
|-- ir/...
`-- .internal/                                tool-owned; one deletion root owns everything
    |-- cas/blobs/v1/blake3/...
    |-- actions.sqlite3                       plus transient -wal/-shm
    |-- sources/v1/...                        registry/Git/materialized dependency cache
    |-- target-publication-state/...
    `-- build-catalog-state/...

<project>/target/.xmlsquish-<id>.lock          example coordination sibling, outside deletion root
<project>/target/.xmlsquish-<id>.clean.json    only if redo recovery is retained
```

The exact private directory spelling is a product decision. The invariant is more important:
`target-dir` is one project-owned derived-state root, user locators stay stable and never point into
private state, and internal children do not need a canonical-project hash. If the product-page
redesign wants visible, non-hidden `cache/` and `metadata/` directories instead, publisher
materialization must still distinguish them from declared products and collision validation must
reserve those names.

Because backward compatibility is explicitly not required for this task, deleting
`SourceConfig.cache_root`, `ManagerConfig.storage_root`, both environment variables, and their TOML
keys is cleaner than accepting settings that no longer control ownership. Keeping inert aliases
would create two configurations for one physical layout and obscure diagnostics. User configuration
home can then return to configuration/credentials only.

## 8. Tests and documents that encode the old model

### 8.1 Must migrate: process-level contracts

| Test/evidence | Old assumption to replace |
| --- | --- |
| `tests/process.rs::project` fixture | Writes workspace `source.cache-root` and `manager.storage-root` overrides under `.xmlsquish/cache`. Remove this fixture indirection so ordinary tests exercise the production local layout. |
| `layered_config_drives_registry_storage_and_presentation_with_cli_precedence` | Asserts workspace overrides create `source-cache` and `manager-state`. Retain presentation/registry precedence coverage, remove obsolete storage settings, and add rejection tests if keys are removed. |
| `build_warms_cache_and_publishes_all_selected_artifact_kinds` | Only checks visible products and warm counts. Extend it to assert project-local CAS/SQLite/source state and absence of machine-home build state. |
| `clean_removes_project_state_prunes_invalid_dependencies_and_rebuilds_offline` | Seeds the old source root and asserts global CAS/index survive clean. Rewrite around the new whole-target clean contract; remote-dependency offline behavior must be decided explicitly rather than inferred from this local-only fixture. |
| `custom_workspace_target_dir_is_the_stable_product_root` | Should prove the internal cache/metadata also follow custom `target-dir`, not just the prompt. |
| `global_storage_isolates_project_catalogs_while_serving_both_projects` | Directly requires one shared CAS/index and two hashed catalog namespaces. Replace with isolation of two independent project targets and prove no cross-project hit/state leakage. |
| `absolute_manifest_path_builds_and_inspects_the_same_project_from_outside` | Preserve: local storage must derive from the discovered canonical project, never caller CWD. Add the local internal paths to its assertions. |
| `tests/recovery_process.rs::Fixture` | Uses a separate `XMLSQUISH_HOME` and discovers `state/catalog/projects/<hash>`. Point artifact/catalog journals at the deterministic project-local internal tree and retain crash-boundary coverage. |
| `tests/pty.rs` recovery-lock helpers | Reconstruct the global project hash/lock path. Update to the new external project-local clean/publication coordination path. |

### 8.2 Must migrate: unit/integration contracts

| Area | Tests/symbols |
| --- | --- |
| Configuration defaults/schema | `crates/squish-config/tests/loading.rs::precedence_is_defaults_user_workspace_then_ordered_cli`, `absent_and_empty_files_produce_complete_defaults`, and key/span tests for `source.cache-root`/`manager.storage-root`. |
| Root composition | `src/main.rs::configured_target_dir_preserves_default_and_workspace_override`, `project_namespace` tests, and environment override tests. The namespace function may become dead code. |
| Host composition | `crates/squish-host/tests/composition.rs::fixture`, `production_runtime_uses_one_cas_and_isolated_generation_spaces`. Preserve shared-within-project CAS and isolated target/catalog generation spaces, but use children of one target root. |
| Host overlap validation | `nonexistent_storage_descendant_cannot_alias_source_cache`, `verbatim_and_drive_spelling_are_the_same_storage_identity`, and related `ProductionHost::open` tests. Redesign around distinct children of a validated aggregate root. |
| Clean/recovery | `clean_removes_project_state_but_preserves_shared_build_storage`, cancellation/lock tests, symlink/reparse tests, corrupt-dependency pruning, interrupted journal/tombstone recovery, former-target layout recovery, and publisher-vs-clean locking tests in `crates/squish-host/src/lib.rs`. The safety properties remain valuable even though preservation assertions change. |
| Runtime layout | `crates/squish-host/src/runtime.rs` tests for lazy initialization, index retry, shared project lock, and epoch supersession. Paths change; invariants should remain. |
| Publisher | separated-layout, recovery, pending-clean, epoch, legacy-generation, whole-generation crash tests in `crates/squish-publish/src/tests.rs`. Most are layout-agnostic and should remain; legacy global-state migration can be deleted if backward compatibility is intentionally dropped. |
| Manager adapters | `StorageLayout::project_local_for_tests` and the many `Services::storage_layout` fakes in manager tests. This helper currently uses `.cache/xmlsquish` for CAS/catalog, unlike the desired production target tree, so it can hide composition drift. |
| Cache/store | `squish-store` CAS and SQLite tests remain semantically applicable. No key/schema migration is inherently required unless cache format/version is intentionally changed. |
| Fetch cache | `squish-fetch::clean` and materializer/registry/Git tests remain applicable after their root is injected from the project-local layout. |

### 8.3 Documents that contradict the requested ownership

- `docs/design/v1.0.2-target-layout-and-clean.md` explicitly chooses a global manager/source cache,
  public-only target tree, and clean preservation of CAS/action index.
- `docs/design/project-storage-namespaces.md` treats global cross-project CAS/action reuse as an
  invariant.
- `docs/releases/1.0.2.md`, `README.md`, and `CHANGELOG.md` promise that internal state is outside
  the target and global caches survive clean.
- `docs/performance/cache-and-presentation.md` records experiments using separately supplied global
  manager/source roots. Its semantic conclusion (presentation does not affect keys) remains valid,
  but its physical setup is historical.
- `docs/product/project-manager-scope.md` and `crates/squish-config/IMPLEMENTATION.md` document the
  old storage configuration keys and configuration-home role.

These should be superseded or clearly marked historical rather than partially edited into mutually
inconsistent decisions.

## 9. Destructive and compatibility analysis

| Risk | Why it is real | Required evidence after migration |
| --- | --- | --- |
| Cleaning the wrong directory | `workspace.target-dir` is manifest-controlled; a malformed or symlinked path must not turn clean into arbitrary recursive deletion. | Snapshot/manifest validation plus physical containment tests, symlink/reparse tests, custom target tests. |
| Lock/journal deleted before recovery | A journal inside `target/xmlsquish` disappears with the state it must finish deleting. | External coordination path; crash at every durability point; ordinary next invocation rolls forward. |
| SQLite still active during delete | WAL/SHM and open handles are process-sensitive, especially on Windows. | Same cross-process project lock covers runtime opening, build, inspect, watch, and clean; detach-by-rename tests. |
| Source cache recreated during recovery | `HostContext::new` eagerly creates `v1/stage`; current host opens it before/around clean recovery. | Open-order test showing recovery completes before any local cache child is initialized. |
| Internal/user collision | Direct user products coexist with a reserved cache/metadata subtree. | Manifest collision validation rejects the reserved private top-level name across case-folding filesystems. |
| CWD-dependent cache identity | CLI relative overrides currently use CWD, while project identity is canonical. | Build/inspect from outside with absolute manifest resolves the identical local root. |
| Cross-project contamination | Removing the hashed catalog boundary is safe only if every path is derived from the discovered project. | Two projects with same package/target names have distinct target trees and never inspect each other's records. |
| Lost remote offline rebuild after clean | Whole-target clean removes dependency objects as well as build results. | Decide and document whether `clean` intentionally makes remote offline rebuild impossible; test the chosen contract with a real cached remote fixture. |
| Stale configuration silently accepted | Old TOML/env keys would imply a storage location that is no longer honored. | Parse errors for removed TOML keys; remove environment mappings/help/docs; no inert compatibility aliases. |
| Self-triggering future watch | Writes under the project target could be observed as source changes. | Watch excludes the canonical target aggregate and handles target-dir changes by restarting/rebinding the runtime. |
| `.gitignore` mismatch | Root `.gitignore` ignores `target`, but projects may use a custom target directory. | `new` scaffold ignores the configured derived root, or documentation requires users to ignore it; custom-target fixture checks generated VCS rules if supported. |

## 10. Minimal implementation dependency order

1. Define one typed project-derived layout from canonical project root plus validated
   `workspace.target-dir`; reserve its internal namespace.
2. Remove global storage/source settings from the configuration model and executable environment
   mapping.
3. Compose CAS, SQLite, source cache, and both generation spaces as non-overlapping children of the
   project-derived aggregate.
4. Move clean coordination outside the deletion root, then change clean to retire the complete
   aggregate. Preserve no-follow deletion, cancellation semantics, epoch fencing, and recovery.
5. Update inspect/mutation/build host composition together; do not migrate writers before readers.
6. Rewrite process and crash-recovery tests to inspect deterministic local paths; remove tests whose
   sole purpose was global catalog namespacing.
7. Update user documentation and release notes only after the executable contract and tests agree.

This sequence removes the global-cache special case at the composition boundary while retaining the
well-tested content-addressing, action-key, generation, integrity, and crash-recovery mechanisms.
