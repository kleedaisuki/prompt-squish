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
raw units mint it once, cached bytes decode directly into it once, and encoded output
uses the retained proof rather than decoding its own wire bytes. PreparedUnit consumes
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
keys/handle counts and require MGB110 before successful compile hydration. Independent
process tests cover colliding empty pack documents, same-name archive providers, large
library/small export action counts and local malformed-unreachable source rejection.

No local Cargo/npm was run under the user's resource constraint. Rustfmt/source checks
are not a substitute for hosted compilation and actual workflows. Source ownership
repairs are not measured speedup claims; same-runner mechanism and end-to-end results
must separately quantify costs, preserve exact outputs/provenance and reveal regressions.
