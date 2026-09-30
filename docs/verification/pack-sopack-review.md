# Pack / SOPack v1.2 independent implementation review

Date: 2026-10-01. Status: evolving review; implementation and hosted validation pending.

## Scope and method

The review begins from `docs/research/pack-sopack-reproducibility.md` and
`docs/verification/pack-sopack-v1.2.md`. It traces production paths and source-level
invariants without compiling Rust locally; hosted CI owns executable validation.
The reviewer does not modify production code. Prioritized findings below require
an executable trigger and a violated contract, not style preferences.

## Initial observations

- The linker now consumes `RelocatableUnitIr` and shared definition/root accessors;
  no archive provider dependency has entered its public closure model.
- SOPack frontend parsing rejects includes even in parsed macro bodies, and IR
  validation independently rejects include operations inside SOPack objects.
- The optional JSONL recorder creates a new project metadata file per invocation,
  stores errors separately, and never turns persistence failures into build failures.
  Explicitly disabled recording returns before filesystem creation or payload work.
- Archive decoding, portable source relocation, provider loading and asset action
  inputs were not yet available at initial inspection; these are unreviewed, not
  verified absent defects.

## Findings

No substantive finding established at the initial checkpoint. This is not an
approval: core new distribution and dependency paths are still being implemented.

## Evidence still needed

1. Serialized source identities and imported bindings survive relocation without
   hidden producer checkout paths or caller-relative asset lookup.
2. Archive boundary validates duplicate/unsafe paths, IR versions, closure/binding
   completeness and content digests before storing dependency state.
3. Asset bytes participate in every action identity that can reuse product bytes;
   failed publication preserves the previous generation.
4. Unreferenced include entries do not leak bindings/symbols into another entry's
   output or create accidental global name conflicts.
5. End-to-end telemetry distinguishes frontend, optimization, linking and emission
   costs where applicable, rather than only a broad dispatch timer.
6. CI supplies relocated producer/consumer and immutable dependency drift evidence;
   local source inspection alone cannot establish cross-device or reproducible ZIP
   behavior.

## Checkpoint: archive/provider implementation appeared

These findings were sent to implementing owners before completion. Line numbers
are intentionally tracked by function names while concurrent edits are ongoing.

| Priority | Location | Trigger and impact | Confidence | Correction |
| --- | --- | --- | --- | --- |
| P1 | `squish-backend/src/archive.rs::read_sopack`, asset restoration loop | Many unique asset bindings reference a single large shared blob. ZIP unique-content bounds pass, but each binding clones the bytes; e.g. 1,000 bindings to 128 MiB requires approximately 128 GiB. Dependency addition can exhaust memory on an otherwise bounded archive. | High, direct allocation path | Preflight accumulated materialized bytes against limits, or share immutable asset storage and bound binding counts. |
| P2 | `archive.rs::write_reproducible_zip` | `A.txt` and `a.txt`, or `A` and `a/x`, pass the case-sensitive duplicate and prefix checks. Such archives cannot be safely used unchanged on common case-insensitive devices. | High, direct set comparison | Reject portable case-folded member and file/directory collisions on both write and read. |
| P2 | `archive.rs::logical_source_path` / `write_sopack` | Distinct imported packages both contain `src/lib.xml`. Package identity is discarded when producing the unique logical source-path map, so a legitimate closed library cannot be serialized. | High, direct path projection | Preserve a stable provider namespace for every imported package while relocating all references together. |
| P2 | `archive.rs::read_sopack` / `validate_manifest` | Manifest omits a decoded module's import slot or includes a nonexistent slot. Manifest only checks endpoints and duplicate slots, so dependency addition accepts a malformed closure that fails later when linked. Asset operations and source-record blobs are also not cross-validated at this boundary. | High for imports; source/asset correspondence requires final implementation check | Require exact import-slot agreement, module targets, asset completeness and source attachment digest/length agreement before returning a validated payload. |
| P2 | `squish-manager/src/build.rs::resolve_import` | Two aliases lock SOPacks with the same package name but distinct version/checksum. Export lookup filters only by package name and can return the first archive's export for both aliases. | High, direct provider selection | Index archives by exact locked package identity/checksum, never display package name alone. |

Additional semantic probe sent to compiler and validator: two independently valid
entries included in one pack may import distinct modules exposing the same symbol.
Following include edges into a single global symbol table can introduce a collision
between independent entry programs. Awaiting intended include-link boundary design.

Performance note, not a measured finding: `bind_imports` repeatedly flattens archive
import tables once per source and per import; identical shared payloads are scanned
multiple times. A single provider/binding index removes this avoidable long-tail
cost while enforcing exact provider identity.

## Resolution checkpoint

Source inspection after owner fixes confirms these earlier findings are resolved
in the implementation (hosted execution remains pending):

- ZIP duplicate and prefix checks use lowercased paths; ordinary case-insensitive
  member collisions no longer pass, and the strict reader reuses this validation.
- Foreign package source paths receive a stable `_providers/<digest>/` namespace.
- `validate_payload` compares exact import declarations to closure bindings,
  requires declared asset bytes, and cross-checks diagnostic SourceDigest/blob
  length against packaged sources.
- Decoder charges every materialized asset binding against the content limit,
  and binding counts are bounded.
- Manager indexes archive import bindings once and chooses public exports via
  the exact dependency lock ID -> package instance -> archive mapping, not name.
- Trace records and kernel events now share the same invocation identity.

New review checks communicated to manager:

1. `archive_build::freeze_inputs` reopens archives after the host's locked checksum
   validation but does not check the reopened bytes against that checksum. A
   concurrent replacement can cross the immutable snapshot boundary. Verify digest
   on the bytes actually frozen, not solely during an earlier acquisition step.
2. `install_archive_source` still obtains its source blob from a materialized cache
   directory whose `.complete` existence is trusted. Compare it to the authoritative
   archive source bytes or replace it with those bytes; changed cache attachments
   must not silently become diagnostic inputs for unchanged compiled IR.

Middle-end placement check: at this checkpoint runtime compilation returns frontend
IR directly and the linker builds the image before `LinkedProgram::reconstruct`
optimizes units. This does not yet realize the requested explicit pre-link middle
stage. Literal-scalar facts are now consumed by RenderText argument execution and
preserve per-operation output/provenance; the previously possible unused-fact
allocation concern no longer applies. Pre-link specialization and warmed reuse
remain under active compiler work.

## Archive emission review checkpoint

Two new direct correctness findings were sent to owners:

- **P1 stale included entry product:** include entry edges are correctly excluded
  from the pack root symbol image, but backend key inputs initially included only
  root image/map/document/trace/directives and asset bytes. Editing only included
  entry XML can leave those inputs identical and reuse stale pack bytes. The
  remedy is independent include result actions or explicit compiled object inputs
  covering included entries and their transitive modules. Independent validator
  added `included_entry_only_mutation_invalidates_warm_pack_product`.
- **P1 invalid SOPack projection:** `archive_build::sopack_bytes` initially converts
  a SopackObject to ModuleObject by cloning `.module`, leaving the root region in
  its arenas. Module validation only roots macro bodies, so an empty SOPack root
  is already an unowned region and `encode_unit_container` rejects emission.
  Preserve a supported portable SOPack object or implement a genuine arena/source
  provenance-preserving module projection; do not weaken ownership validation.

Resolved by subsequent source inspection: freeze inputs now SHA-256 verifies the
actual bounded archive bytes against the lock, and installation compares diagnostic
source bytes against authoritative archive sources. Pre-link middle specialization
now precedes symbol binding, moves owned IR, and feeds `reconstruct_optimized`;
link-time definition addresses use dense constant-time indexes. Include symbol
isolation is now explicit: entry target kind is checked but entry edges are not
followed into the parent symbol-aggregation closure.

Long-tail performance probe requested: E distinct included entries and U source
units currently result in E deep clones of the complete compiled map and E complete
snapshot payload validations. A repeated single include microbenchmark hides this
because `pack_bytes` memoizes by source key. Measure many distinct includes before
claiming full-chain long-tail optimization.

Provider namespace limit: project path/workspace revisions are manifest digests.
Two independent providers can have identical name/revision and different source
bytes; using only those fields is not a complete provider identity. Current CLI
source acquisition already rejects same-name/same-path SourceId collisions, so
that particular fixture fails before serialization. The standalone archive API
still needs a portable, content-derived exact provider namespace; root ownership
must not be inferred solely from package display name.

## Final source checkpoint before hosted CI

The manager has resolved both emission P1 findings in source:

- Archive backend actions explicitly include all compiled XSiR outputs and their
  dependencies, so independent included entry changes affect the backend key.
- SOPack emission preserves typed SopackObject rather than invalidly casting its
  arenas to ModuleObject; the archive codec accepts Module/Sopack library units.
- Include execution now constructs a restricted transitive unit closure before
  cloning/validating IR, removing the full-project deep clone per member.
- Persistent directives are validated against companion frame IDs and safe names
  before being restored to build state.

One remaining boundary issue was sent to the SOPack dependency owner:
`host::read_project_sopack` initially uses unrestricted `fs::read` before applying
ArchiveLimits. An arbitrarily large invalid dependency can allocate its entire
file during add/resolution, before the supposedly bounded decoder rejects it.
Apply metadata and bounded `Read::take(limit + 1)` checks in the host too.

No heavy local compilation or acceptance run was performed. The code review
findings above are direct source traces; fixed-source status must not be read as
passing hosted tests. CI result and exact tested commit remain required evidence.

## Frozen source verdict (2026-10-01, before first hosted CI)

**No unresolved release-blocking source finding remains in the inspected final
snapshot.** This is a source-review verdict, not a build/test/release approval.
Every demonstrated P1/P2 correction described above was re-inspected; no production
code was edited by the reviewer.

Final corrections confirmed:

- Host acquisition rejects nonregular or oversized archive files before full
  allocation and caps growing reads at the limit plus one byte.
- Explicit `SopackPayload.root_source` / manifest `root_path` distinguishes the
  packaging owner from nested imported typed SOPack roots. Manager sets the actual
  selected target root.
- Provider grouping uses exact PackageInstanceId internally. Portable namespace
  seeds include source/asset/normalized compiled-unit content and stable identity
  fields, with sorted frozen import graph refinement; physical canonical checkout
  paths are excluded. Distinct same-name/version provider content no longer
  collides. Content-equivalent providers with duplicate logical paths are explicitly
  rejected, not silently merged.
- Repository source-package IDs distinguish ambiguous same-name instances while
  preserving readable ordinary unique names; this removes the earlier acquisition
  collision before independent archive alias resolution.

Remaining review limits and nonblocking measurement work:

1. Rust type checking, executable archive interoperability, relocation/macro asset
   ownership, immutable dependency mutation, warm included-entry cache invalidation,
   telemetry persistence and platform behavior require the hosted CI suite. The
   reviewer did not run Cargo locally, honoring machine resource constraints.
2. Provider graph fingerprinting uses G rounds for G groups; long chain/cyclic
   provider closures should be included in future scaling measurements. This is an
   identified complexity cost, not a claimed measured regression.
3. Independent include closure extraction still scans frozen resolution tables per
   include. The serious full-IR project clone was removed; actual many-distinct-entry
   timings should distinguish residual coordinator costs from rendering/ZIP bytes.
4. Crash/fault injection, hostile concurrent filesystem mutation and Unicode
   filesystem-normalization equivalence are not established by this source pass.
5. Final hosted commit SHA/run URL and measured benchmark outputs must be recorded
   before stating that v1.2 passes or claiming performance improvements.

## Hosted integration follow-up: artifact classification and ABI boundaries

Trigger: `.temp/v1.2-ci/ninth-linux.log`, exact hosted source SHA
`1da3e6d2bfe38873aa5623ab81319c369b9d28c0`. The archive process target had 5
passing and 8 failing tests; the existing process (38), recovery (8), and skills
(11) suites passed. The earlier source-only verdict did not establish runtime
correctness, and this evidence reopens release approval pending fixes and rerun.

Confirmed CI integration defects under owner repair:

- Publisher `is_user_artifact` omitted `Other("pack")` / `Other("sopack")`.
  Successful emission could commit metadata while not materializing the product.
  The producer classification had changed but the consumer whitelist had not.
- Included-entry rendering traversed raw instantiate -> backend and received
  `xmlsquish-document-v1`, while the backend accepted `xmlsquish.document.v1`.
  Ordinary manager instantiation rewrote the ABI and hid this split authority.
  Compiler correction centralizes `squish_ir::DOCUMENT_ABI`; host regression covers
  raw frontend/link/instantiate/codec/render without the manager rewriting it.

Additional concrete reviewer finding:

**P2 — valid Pack/Sopack IR cannot be inspected.**
`crates/squish-manager/src/inspect.rs::validate_ir_bytes` initially accepts only
Module/Entry unit containers, then LinkedImage/LinkedDocument. `build --emit ir`
publishes Pack/Sopack unit containers with BinaryIr kind, but `inspect ir` rejects
those valid objects with XS3421. Self-describing IR provenance validation calls the
same helper and fails too. Add Pack/Sopack to the unit-container decoder branch and
cover both typed unit kinds in focused inspection tests. Sent to manager owner.

Systematic downstream source audit (no Cargo run locally):

| Boundary | Assessment |
| --- | --- |
| Protocol serde / persistent generation manifests | ArtifactKind derives serialization; Other carries its named value. No archive-specific whitelist. |
| Build action/output identity | `squish-build::model::hash_artifact_kind` hashes Other with its name and domain separation. |
| Store binary action-result codec | Tag 4 persists/restores nonempty Other names. Pack/Sopack names survive unchanged. |
| Store SQLite index kind codec | `("other", Some(name))` round trips arbitrary nonempty named kinds. |
| Manager action-result identity | Other names enter the result digest; classification remains truthful across reuse. |
| Presentation | Other names use the sanitized name, rather than pretending archives are prompt XML. |
| Ordinary artifact inspection / cache inspection | Validates exact digest and byte length; no Prompt-only restriction. |
| Product provenance | Archive Other kinds explicitly return UnsupportedKind; no XML byte map or psdbg is promised. This is coherent with current transport-evidence scope, not another rejection defect. |
| Skills | Tree/file installation pipeline has no ArtifactKind classification dependency; existing hosted skills suite passes. |
| Persistent IR decoder and inspect projections | Both support Pack/Sopack; only manager's validation dispatch missed the new kinds. |

No analogous additional ABI spelling mismatch was found in inspected production
literals after the centralized document ABI correction. Frontend uses its declared
FRONTEND_ABI; linker validates schema/language/regex compatibility; manager action
recipe namespaces include 1.2.0 to invalidate stale internal schemas. Hosted rerun
is still required to establish success of the fixes.
