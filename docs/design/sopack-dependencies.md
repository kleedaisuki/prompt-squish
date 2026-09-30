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

Instantiation persists an ordered `directives` output beside `document` and
`trace`; cached execution preserves asset/include ownership and import IDs.
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
