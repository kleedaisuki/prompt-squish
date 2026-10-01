# Immutable SOPack dependencies (v1.2.0)

## Contract

`[dependencies] common = { sopack = "vendor/common.sopack" }` selects a compiled,
self-contained library, not a mutable source directory. `DependencySpec::sopack`
feeds the existing comment-preserving add/remove manifest editor. The file path
is relative to the declaring manifest; the lock stores the canonical
workspace-relative locator plus SHA-256 of the entire archive bytes.

The resolver filesystem port gains `load_sopack(LocalRequest) -> SopackPackage`
with portable path, embedded manifest, and checksum. Default adapters fail
explicitly rather than accidentally treating an archive as a source package.
Resolver identity version is `squish-backtracking/2`. Locked source identity is
`LockedSource::Sopack { path, checksum }`; unlike Path, no mutable flag exists.
Frozen validation checks that exact archive content is present. Host
materialization always checks the locked digest, including online/locked builds.

## Compilation boundary

`read_sopack` relocates compiled source identities. The manager must use those
identities and archived import/export bindings directly and must not recompile
diagnostic source attachments. Linker inputs remain ordinary relocatable IR,
independent of whether origin was local XML or compiled archive.

SOPack producer metadata must contain `metadata["manifest"]`: validated TOML
whose package name/version agrees with the archive header. The consumer clears
external dependency intent, targets and workspace declarations because archived
imports are already self-contained. Public exports remain available.

Repository package source kind is 5 (registry 1, git 2, path 3, workspace 4).
Repository identity uses the exact locked checksum; archived units carry their
archive decoder's relocated identity and override the initial source identity
when manager consumes compiled units.

## Materialization and portability

Host copies exact diagnostic sources into project-owned cache
`v2/sopack/sha256-<digest>/`, writes the normalized manifest and completion marker,
and publishes by atomic directory rename from a same-parent temporary directory.
No device-absolute path is serialized into manifest or lock. Attachments must
be normal relative components and cannot overwrite manifest/completion authority.
Assets are consumed from the immutable archive payload by the manager, not
re-read from caller-relative filesystem paths.

## Verification

Focused tests added: manifest add/remove and comment retention; resolver archive
identity and frozen digest rejection. Rustfmt parsing succeeds. Full Rust tests
are intentionally delegated to GitHub Actions under the requested resource
policy; no heavy local Cargo build was run.

## External evidence

Cargo's registry index pins exact archive bytes with SHA-256, rather than trusting
mutable location or version text: https://doc.rust-lang.org/cargo/reference/registry-index.html

Reproducible Builds archive guidance identifies timestamp, member order and
ownership metadata as nondeterminism sources:
https://reproducible-builds.org/docs/archives/
Archive generation remains owned by the backend; dependency identity validates
its output without introducing platform metadata into serialized identities.


## Manager compilation and cache contract (2026-10-01)

The authoritative source freeze re-reads each SOPack with a 256 MiB hard bound
and rechecks its locked SHA-256, preventing acquisition/freeze races from changing
an immutable dependency. Exact source attachments are checked against archive
source bytes. Archived units bypass the frontend and retain content-relocated
`SourceKey` identities; imports are taken from the archive's immutable indexed
bindings. Package exports are selected by exact lock-node identity, never by
package name alone. The linker receives the same `UnitClosure` for local and
archive-backed objects.

Asset syntax is pre-scanned namespace-aware from frozen XML; assets resolve
relative to the source defining the operation, not the caller's working directory.
Lexical and canonical checks reject package escape (including symlinks), while
`../` within the package is allowed. Only declared assets are read, avoiding whole
project resource walks. Worker stages consume frozen bytes and perform no hidden
filesystem reads. Backend action keys declare every frozen resource digest.

Archive instantiation persists an ordered `directives` output beside `document`
and `trace`; cached execution preserves asset/include ownership and import IDs.
Ordinary prompt instantiation has only `document` and `trace`. Nonempty archive
directives are rejected with MGB150/Emit before a prompt result is serialized,
stored, marked successful or cached. Prompt hydration therefore reconstructs an
empty sidechannel, while archive hydration must load and validate it. Action
keys hash their exact output schema, isolating these contracts and older
three-output prompt records without generic optional-output infrastructure.
Archive backend keys additionally include all compiled unit objects. This is
essential: a pack's included entry may change while its own empty document,
link image and directives remain identical, and a reusable library may change an
uncalled macro body. Neither change may reuse an obsolete ZIP.

A pack independently links each included entry with a fresh empty parameter
scope and emits it through the prompt backend. Repeated includes of the same
entry are memoized per target. Independent includes pass a compact transitive
unit closure, avoiding deep-copying every project unit once per include. Packaging
roots may emit only asset/include directives; finished text is not silently lost.
SOPack preserves its own typed root IR and module closure, exact sources, all
syntactic assets (including unused macro bodies), immutable bindings, metadata,
and exports. The default `main` export names the SOPack root; explicit exports
must refer to units in the packaged closure. No finished prompt/pack is embedded.

Archives have transport-byte cache evidence rather than a fabricated XML byte
map. Optional `.psdbg` remains an XML prompt companion only. Published archives
retain normal static-link-map and build-catalog metadata outside the ZIP; trace
files never enter distributable archives. Cache action recipes carry the v1.2.0
manager schema so previous instantiate results without directives cannot be used.

Validation must cover fresh byte equality, resource-only mutation, included-entry
mutation, uncalled macro-body mutation, two providers sharing a package name,
macro-defined resources invoked from a different directory, immutable archive
drift, and cross-root archive reuse. Heavy Rust verification is delegated to
GitHub Actions because the authoring machine is disk constrained.

## Same-name package source identities

Repository-owned source snapshots retain the plain package name when it refers
to a unique exact PackageInstanceId. Multiple distinct instances sharing a name
use `<name>.<full domain-separated BLAKE3 instance digest>` in SourceId.PackageId.
The digest length-frames canonical source, package name and exact revision and
includes source kind; lock aliases and physical checkout roots never enter it.
Compiler PackageInstanceId.package_name and logical source paths remain unchanged.
Pure bounded tests cover distinct same-name instances, order independence,
unchanged paths, and readable identity for unique/repeated identical instances.
No Cargo commands were run for this follow-up due to the disk-space restriction.

Repeated lock nodes pointing at the same exact source provider are deduplicated
only when ProjectSource equality agrees (identity, instance and locator). Same
SourceId with a conflicting locator or instance still fails without replacing
the existing source. A focused pure test covers all three cases.

Host archive reads now precheck regular-file metadata against ArchiveLimits and
use a `take(limit + 1)` stream read, so a growing archive cannot bypass the
allocation boundary. Pure Cursor tests cover exact-limit acceptance and stopping
one byte beyond the limit; the decoder receives only bounded input.


### Complete attachment discovery

SOPack diagnostic materialization is not an authoring project source-root. Every
regular declared attachment, including `_providers/<content-group>/...` and
non-XML original filenames, must be frozen. The repository uses one deterministic
non-symlink walker with an archive-attachment policy, excluding only generated
`xmlsquish.toml` and `.complete` transport metadata. In particular, the surrounding
project cache directory must not be applied as an exclusion to the immutable
package it contains. Local XML source-root discovery retains its previous suffix
and derived-directory rules. Exact archive/source-byte verification remains the
authority; adding an undeclared file to the materialized tree is rejected rather
than silently compiled.


### Frontend context binding for disambiguated providers

A diagnostic `PackageId` is not the public package name when distinct locked
providers share that name. Manager compilation and format candidates bind
`FrontendSourceContext::new_with_source_package` using the exact `ProjectSource`
or `FrozenSource` diagnostic package ID alongside its authoritative
`PackageInstanceId`. The frontend checks exact equality against that typed source
package ID. No package-name prefix heuristic or hashed-name exemption is used;
the repository's resolved mapping remains the authority for source ownership.


### Public generation projection

`ArtifactKind::Other("pack")` and `Other("sopack")` are user-facing backend
products and must be materialized onto stable artifact paths, just like prompts.
The publisher deliberately recognizes those two extension kinds rather than
publishing arbitrary `Other` evidence. Static-link maps and build records remain
private generation metadata. The same classification is used for old-generation
cleanup, so replacing a generation also removes its obsolete archive products.
A successful private generation commit alone does not establish end-to-end user
publication; separated-layout regression checks verify the public files exist.


## Manager immutable ownership repair (2026-10-01)

The mechanism audit in `../performance/v1.2-archive-mechanisms.md` distinguished
transport deduplication from expanded in-memory ownership. Manager staging now
uses immutable `Arc<RelocatableUnitIr>` and `Arc<[u8]>` handles instead of cloning
compiled arenas and resource bodies between phases. These are ownership changes,
not a relaxation of source identity, validation, or archive limits.

* Acquisition supplies `AcquiredSopack` handles keyed by exact lock-node ID. The
  manager still bounded-streams and SHA-256 hashes each locked file at the freeze
  barrier, rejecting acquisition-to-freeze content drift. A missing acquisition
  handle is an explicit error rather than an implicit second decode. The freeze
  recheck uses a fixed 64 KiB buffer, not another owned archive body. Exact
  `PackageInstanceId` keys include source kind, canonical source, package name,
  and exact revision; a same-name local package cannot inherit archived objects.
* Each exact archive is indexed once by diagnostic logical path and asset owner.
  Per-source installation performs indexed lookup and clones only handles for
  its own bindings. Duplicate attachment paths are rejected. Materialized
  diagnostic bytes must match the immutable archive attachment exactly before
  any compiled object is installed. This removes repeated archive-unit and
  entire-asset-map scans without treating attachment files as compilable inputs.
* Archive emission does not mistake the project-wide frozen resolution for a
  target closure. SOPack selects `linked.image.units`; pack selects the union of
  independently projected included-entry closures. Only these compiled handles
  are taken from the
  execution state. Pack member staging shares frozen asset bodies and memoized
  include-output bodies; the reproducible writer receives borrowed member views.
  Repeated includes therefore share bytes but retain separate archive names.
* Each include projects its own exact revision/import evidence, builds a shared
  compiled-unit closure, and invokes the ordinary validated shared linker with
  fresh arguments and the same independent budget contract. Sharing payload
  handles does not merge executable symbol scopes or skip child validation.
* SOPack output constructs a borrowed payload view over reachable unit handles,
  frozen diagnostic source bytes, and resources. Export resolution uses an exact
  owner/path index rather than an export-by-unit nested scan. Final archive
  ownership and canonical relocation remain the backend's responsibility.

Private tests use actual frontend-generated module IR to verify pointer identity
of installed objects and shared assets, exact diagnostic drift rejection, and
ambiguous attachment-path rejection. Existing include tests retain independent
cyclic closures, exact revisions, dangling-edge errors, and missing-payload
errors. Local `rustfmt` and diff whitespace validation were run; Rust execution
is delegated to hosted CI. No CLI speedup or allocation reduction is claimed
until repaired production paths are measured on the same hosted workloads.

## Invocation-scoped verified acquisition (repair)

`host::sopack::SopackAcquisition` is created for each dependency-resolution or
standalone materialization operation. Resolver metadata loading, locked/frozen
availability checks and diagnostic materialization use the same captured archive
handle. A canonical workspace-relative locator is read at most once in that
operation. Different locator aliases are each read (their content is not assumed
immutable) but identical SHA-256 content shares one decoded payload.

Manager `AcquiredSopack::acquire` computes the exact archive SHA-256 once and
returns opaque checksum-bearing `Arc` ownership; a previously verified digest is
reused before decoding. Shared source/asset buffers and compiled unit objects
remain shared into manager freeze. `ResolvedDependencies.sopacks` maps exact lock
node IDs to these handles, not public package names. The host never keeps the
locator memo in process-global or persistent state.

The authoritative acquisition/freeze drift contract is retained: manager freeze
boundedly rehashes the original archive locator against the expected checksum,
but consumes the shared decoded payload rather than decoding again. This second
byte-read is an explicit safety boundary, not a payload clone/decode optimization
omission. Materialized source attachments are independently matched to archived
exact bytes. A later invocation always reopens and verifies each locator.

Regression counters cover repeated path acquisition (one read of B bytes, one
decode), a byte-identical second locator (two reads totaling 2B, still one decode),
exact lock-node handle reuse, wrong locked checksum rejection, fresh-invocation
reacquisition and malformed later-run archive rejection. `Arc::ptr_eq` checks
shared ownership directly. Fixtures live under the project `.temp` namespace.
Only rustfmt is run locally; test execution and measurements remain hosted CI.

Archive products and evidence now survive emission as sealed `VerifiedBlob`
handles. Product evidence references the established digest rather than hashing
again; cache hydration checks that digest and size without recreating a backend
output or copying the expansion trace. The backend state keeps publication bytes
and product kind, not a redundant full `BackendOutput`.

After immutable reachability pruning the relocated unit is encoded and sealed once
as `FrozenSource::precompiled_blob`. Compile action inputs use this handle's
stored digest, and execution reuses the same encoded bytes instead of performing
another serialization or byte hash. The semantic unit remains a shared IR handle;
cache ownership is still project-local.


Immutable compile scheduling now has a separate `archive_reachability` boundary.
All local sources remain in the strict global frontend plan. A namespace-aware
prescan of their static `xs:import src="pkg:ALIAS/EXPORT"` declarations resolves
exact lock-node dependencies to archived exports, then follows validated typed
archive bindings across provider identities and cycles. Only reachable archive
sources survive into compile scheduling and encoded-blob sealing. All library
units and materialized diagnostic attachments are validated *before* pruning;
unreachable malformed library content is not hidden. Archived XML is never
prescanned, and local prescan syntax errors still fail in the normal frontend.
Pure tests verify namespace/attribute/entity handling and cyclic cross-provider
selection; the 64-module tiny-export compile-count acceptance test belongs to
hosted end-to-end validation, not an algorithm-imitation benchmark.
