# Manager immutable stages and verified publication snapshots

Date: 2026-10-01. Scope: mechanism remediation after hosted v1.2 baseline.
Related: [archive dependencies](sopack-dependencies.md),
[archive mechanism evidence](../performance/v1.2-archive-mechanisms.md).
Status: implemented source, awaiting hosted compilation/acceptance/remeasurement.
No released tag or artifact is rewritten by this workstream.

## Ownership model

Each invocation owns one sealed local/source snapshot and immutable acquired archive
handles. A compiled unit retains its raw `Arc<RelocatableUnitIr>`, one sealed encoded
`VerifiedBlob`, exact object/semantic/debug identities, and a `PreparedUnit` capability.
IR-owned `ValidatedUnit` is the sole structural proof and identity authority: fresh
raw units use `encode_new` to validate, write output and mint internally derived
identities in one operation; cached bytes decode directly into the proof once. The
encoder's private section receipt replaces identity-before-output rehash passes and
decoding its own wire bytes. IR-only preparation can still use `new` without encoding.
PreparedUnit consumes
the proof without revalidation or caller-supplied digest labels. Regex/static facts are
lazy at whole-unit granularity: the first canonical reachable link prepares them once,
including every pattern inside that unit, and shares the result (or deterministic
error) across independent targets/includes. Unreachable local modules get no new
middle-end work. Every entry still binds its own symbol scope and receives fresh
argument/budget/trace state; consumers clone handles, not IR arenas or compiled facts.

Stage state retains `Arc<LinkOutput>`, `Arc<ResolutionSnapshot>` and
`Arc<InstantiateOutput>`. Document ABI normalization happens once in the instantiation
result before encoding; the former second document copy is gone. Backend rendering
borrows the document/trace through the additive runtime view. Returned transformed
trace and byte-map ownership moves into debug serialization instead of cloning again.
Backend state needs only sealed product/debug bytes and their artifact kind; it does
not retain a second full BackendOutput/provenance graph solely for publication.

Encoded link maps are sealed once and retained as shared handles for publication.
Publication receives explicit verified buffers for product, debug, requested IR and
metadata. It does not rely on a bounded host-session cache keeping large outputs;
retaining an acquired output is distinct from making all raw CAS data an unbounded
invocation cache. Converting a newly mutable Vec to Arc storage can allocate/move its
bytes; the contract is no further body copying/hashing once sealed, not zero allocations.

## Exact alias identities, not payload scans

- Image digest indexes the immutable linked result/map in this fixed invocation.
- Instantiation identity is **(document digest, trace digest, optional directive
  digest)**. Empty pack documents can be identical while assets/provenance differ;
  a document-only alias would publish the wrong archive for a single-flight follower.
- Backend identity is the complete product/debug digest pair. Matching output kind,
  name, size and order is still checked before publication alias recovery.
- Source-key-to-index lookup avoids rescanning all frozen sources per compile.
- Diagnostic source bytes have one indexed view including acquired archive attachments;
  pruning compile work never discards bytes needed by a reachable unit's source records.

All identities come from fresh sealed bytes or verified cache manifests. Alias
recovery no longer serializes/hashes every stored stage result to find a match.
Indexes are invocation-local, not cross-run trust caches or process-global pointers.

## Cached action boundary

`lookup_action_verified` returns the manifest plus caller-owned sealed output handles.
Before hydration the manager checks requested action key, exact handle count, and each
output's declared digest/length against its companion handle. Public/injected runtime
adapters cannot authorize a different action merely by returning valid unrelated bytes.
Hydration consumes those handles directly; it does not read CAS again after action
verification, including outputs larger than the host's bounded retained-byte budget.
Structural IR/backend evidence validation remains unchanged and occurs at decode.

`write_verified_blob` must return the sealed digest; the manager still rejects a wrong
runtime return value. ProducedOutput uses the same established digest, avoiding a
second hash just to construct action metadata. Product/debug snapshots passed to
publication survive cache eviction without suppressing generation schema/journal checks.
Action success is recorded only after every stage output was durably accepted.

## Archive reachability and safety boundaries

Acquisition shares one decoded library across resolver/materialization/freeze through
an opaque checksum-verified `AcquiredSopack`. Different locators must verify their own
bytes; equal exact SHA-256 snapshots can reuse one decode. Freeze separately performs
bounded streaming SHA-256 on the original locked locator, preserving mutation/deletion
rejection between acquisition and freeze without a second whole-archive Vec or decode.

All library IR structure/source/import closure and diagnostic attachments are checked
before pruning. Local namespace-aware static package-import declarations seed the typed
immutable import graph. **All local sources remain globally strict**; only unreferenced
precompiled archive units are removed from compile scheduling and encoded-CAS preparation.
Prescan syntax errors are deferred to the unchanged local frontend authority, never used
to make malformed XML succeed. Archived XML is neither reparsed nor recompiled.

A 64-module library whose consumer selects one standalone export should schedule one
local source plus that archive module, not 65 compile actions. Engine regex preparation
still checks all patterns inside every selected/prepared unit, including unused macros.
Wholly unreferenced archive units retain the prior structural-library validation boundary;
pruning does not invent an all-library engine-validation requirement.

SOPack emission selects `linked.image.units`; a global ResolutionSnapshot is not a
reachable payload list. Pack emission selects the union of independent include closures;
an asset-only pack selects no compiled child payloads. IncludeIndex preserves exact
revisions/bindings, legal cycles, canonical evidence ordering and independent entry scopes.
Borrowed ZIP staging and SOPack views retain defining-source asset ownership without
cloning bodies into temporary member/payload maps.

## Acceptance and measurement boundary

Manager private tests cover complete companion alias identities, shared IR/assets/facts
handles, true include closure selection, diagnostic attachment corruption and bounded
streaming freeze drift/deletion/oversize. Integration tests inject wrong cache manifest
keys/handle counts after proving healthy warm reuse. Invalid adapter results must emit
an MGB110 warning, never hydrate or count a cache hit, and safely execute fresh work;
lookup/index failures retain the established advisory-cache policy. Independent
process tests cover colliding empty pack documents, same-name archive providers, large
library/small export action counts and local malformed-unreachable source rejection.

No local Cargo/npm was run under the user's resource constraint. Rustfmt/source checks
are not a substitute for hosted compilation and actual workflows. Source ownership
repairs are not measured speedup claims; same-runner mechanism and end-to-end results
must separately quantify costs, preserve exact outputs/provenance and reveal regressions.

## Exact archive proof reuse during warm Compile hydration

Freeze has already encoded each selected immutable SOPack unit and retained the paired
`ValidatedUnit` capability and sealed `VerifiedBlob`. A verified cached Compile output
whose digest and size match that exact frozen encoding reuses the retained capability
and raw Arc arena. It is not decoded into a second typed unit, revalidated, or rehashed.
Private frozen-field construction and opaque VerifiedBlob identities are the authority;
no caller-supplied semantic labels are trusted. A different valid cached object with the
same source/ABI is rejected as an advisory cache miss, not substituted into the locked
library. Local-source cached objects continue to use the public checked decoder.

The shared archive regression uses Arc::ptr_eq to require arena identity and creates a
structurally valid same-source/ABI alternate encoding to prove exact identity rejection.
Malformed public decoder tests remain intact. This removes known own-encode/cache-decode
work; it is not a speculative persistent cache or a measured speedup claim.

## Patch cache migration

Manager recipe and option schema epochs advance to **1.2.1**, preventing previously
cached stage products from bypassing correctness repairs. Storage remains project-owned;
old CAS blobs are harmless, discardable content, not shared machine-wide authority.
Cleaning old private cache bytes is optional and does not replace any dependency archive.

SOPack's new SCC provider fingerprints change freshly emitted multi-provider archive
bytes. Its backend cache identity is `reproducible-stored-zip/sopack-scc-v3`; pack's
unchanged stored ZIP codec stays `reproducible-stored-zip/1`. Imported schema1 SOPack
bytes remain readable and their SHA-256 lock pins immutable. No migration silently
rewrites acquired libraries, existing lock identities or the released v1.2 tag.

## Committed publication versus disposable cache authority

Hosted native acceptance exposed a corruption-repair blocker: catalog recovery required
all historical action-output CAS blobs even though those are derived, reproducible cache.
A quarantined product prevented fresh work before verified cache lookup could miss safely.

The repaired boundary validates the canonical BuildRecord/generation membership,
actual published bytes against declared digest and size, artifact schemas, and exact
target-record coverage. A runtime claiming `ArtifactRead::Verified` is not sufficient:
the manager seals the returned bytes and compares their actual identity. Invalid committed
metadata, missing/changed published files, and true storage errors remain fatal.
Historical action outputs are inspected only by their own verified cache-reuse boundary,
not eagerly loaded by catalog recovery.

Inspection and normal build preparation perform **zero derived-CAS reads/writes**.
Actual published file bytes are already authoritative after digest/schema verification;
checking CAS availability adds no catalog correctness. In particular, products larger
than the 8 MiB bounded session cache were eagerly read/hashed and compared during
recovery, dropped, then read/hashed again by real backend cache acquisition. That
unnecessary work is removed rather than hidden in a bigger cache.

Private invocation-owned generation receipts retain verified published buffers one target
at a time. The base read verifies the canonical catalog record, then inspection/recovery
verify each target sequentially. An unchanged current generation consumes its receipt
without reread/revalidation; restoring a missing current pointer publishes those exact
sealed buffers. A genuinely newer committed generation is verified separately. Peak
retention is proportional to the largest target, not all targets combined, and receipts
are dropped with recovery rather than retained in a process cache.

Production verified publication serves explicit snapshot handles directly, so restoring a
missing current pointer does not require derived CAS acquisition either. The legacy runtime
bridge validates/persists supplied handles only when actual publication is invoked before
calling its existing publisher; it does not introduce cache work into preparation.
Missing/corrupt action outputs are verified at real cache acquisition, safely miss/quarantine,
and are regenerated by the actual stage. Untouched historical blobs need not be healed.
Invalid committed metadata/files and true authoritative-storage errors remain fatal.

Memory-port regression counters require exactly one generation-member read per base
recovery, zero CAS activity for both inspection and normal prepare, and CAS remaining empty
after prepare when all disposable blobs were cleared. The actual subsequent build must
recompute consumed content successfully and preserve exact target output/generation.
These are contract counters, not measured latency speedups. Independent fresh-process
CAS-corruption and previous-generation atomicity fixtures remain hosted acceptance.

## Failure source evidence through manager orchestration

Runtime errors can retain structured compiler diagnostics without changing public
WorkerFailure literals. ManagerError preserves manager code/phase/message and copies
upstream primary/related spans and help. Frontend, main/include link, instantiate and render
conversions retain that evidence. Build execution uses the existing ActionEvent::Diagnostic
carrier with a private `manager-source-failure-` identity convention. EventMapping associates
it with the terminal ActionFailed event and normalizes per-action diagnostic IDs; the
separate event is suppressed to avoid duplicate error rendering. Singleflight followers
receive the leader's source evidence through invocation-local leader/member associations.
Other worker diagnostic events retain their established forwarding behavior.

Focused tests require relocated source spans and call/definition-related evidence to survive
ManagerError conversion and leader/follower terminal mapping, while stable manager codes and
WorkerFailure shapes remain unchanged. Actual archived recursion/duplicate-symbol and local
parse process fixtures must still verify host source resolution and CLI output end to end.
