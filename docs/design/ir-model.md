# Core IR and Linking Model

- Status: Implemented normative design
- Implementation authority: `crates/squish-ir`, `crates/squish-xml-front`,
  `crates/squish-link`, `crates/squish-backend`, and the manager build pipeline
- Scope: XML front end, relocatable IR, linker, evaluator, provenance, cache identity, and backend contract
- Related language specification: [`docs/dsl.md`](../dsl.md)
- Product artifact suffix: `*.prompt`

## 1. Decision summary

xmlsquish uses a small family of explicit representations rather than treating one XML-shaped tree as source syntax, executable program, debug record, and product at the same time.

```text
exact XML bytes
  |-- LosslessXmlTape -------------------------------> fmt
  |
  `-- XML front end
        -> RelocatableUnitIR (module or entry object)
        -> link(entry, resolved module closure)
        -> non-executable LinkedImage metadata + LinkTrace
        -> reconstruct session-only LinkedProgram from semantic unit blobs
        -> evaluate(arguments, budgets)
        -> LinkedDocumentIR + ExpansionTrace
        -> Backend
        -> <target>.prompt
```

The architecture has four representation boundaries:

| Representation | Purpose | Preserves | Must not do |
| --- | --- | --- | --- |
| `LosslessXmlTape` | Formatting and source edits | Exact bytes, BOM, token spelling, quotes, entity spelling, comments, processing instructions, CDATA boundaries, and trivia | Execute macros or serve as semantic IR |
| `RelocatableUnitIR` | Independently cacheable machine-readable compilation object | DSL operations, user document data, imports, external symbol references, signatures, logical source identity, and source attachments | Resolve project dependencies or bind calls to process-local pointers |
| `LinkTrace` + `ExpansionTrace` | Explain resolution and dynamic execution | Import resolutions, symbol bindings, definition/call sites, every invocation frame, parentage, output-origin mappings, and diagnostics | Affect successful product bytes |
| `LinkedDocumentIR` | Backend-independent result of linking and expansion | Ordered document structure and data after every DSL control operation has executed, with references into the trace | Contain unresolved symbols, macro operations, or XML serialization policy |

`LinkedImage` is the persistable, non-executable metadata result of linking. It is an internal companion to the link trace, not a fifth source representation: it contains direct portable definition references and the entry region, but no unit bodies or process-local indexes. A session reconstructs the executable `LinkedProgram` from the image and its referenced semantic unit blobs. `LinkedProgram` is never serialized, and `LinkedDocumentIR` is the only normal backend input.

This separation is mandatory. In particular:

1. `fmt` never round-trips through semantic IR.
2. Module compilation never depends on which entry happens to use the module.
3. Linking never serializes machine addresses or Rust enum layouts.
4. Debug data is complete but is not mixed into product semantics.
5. A backend never observes XML source control elements such as `xs:expand`.
6. The initial `squish` backend is one backend implementation, not the definition of the core IR.

The final product is a plain `*.prompt` artifact. Canonical IR, source blobs, traces, and build records live in the project artifact store and may be materialized as debug bundles; they are not injected into the prompt.

## 2. Design constraints

### 2.1 Language constraints

The IR must preserve the current DSL primitives and their semantics:

- source units are explicit `xs:module` or `xs:entry` documents;
- macro symbols are XML expanded names `(namespace URI, local name)`;
- `xs:import` is static source discovery, not execution;
- `xs:expand` is a statically named macro call;
- scalar parameters are immutable Unicode strings;
- slots and fills carry immutable ordered document-node sequences;
- `file.*`, `arg.*`, and lexical `match.*` are the only scalar namespaces;
- regex matching, scalar-body evaluation, slot substitution, and recursive calls retain their defined evaluation order;
- source-import cycles are legal and terminate through source interning;
- call-graph cycles are legal recursion and are limited only by invocation budgets;
- an entry is not a macro and never receives a synthetic macro symbol;
- all definitions, including unreachable ones, are validated;
- the abstract language remains unbounded while the implementation applies explicit resource budgets.

### 2.2 System constraints

The system must additionally provide:

- portable and deterministic identities;
- independent compilation of source units;
- canonical serialization suitable for a content-addressed store;
- exact invalidation when semantic inputs change;
- complete source and dynamic provenance;
- lazy machine reading without parsing diagnostic XML;
- an explicit frontend/core/backend boundary;
- schema evolution without interpreting unknown semantic operations as no-ops;
- a storage model usable both as immutable files and as records in a local database;
- no dependence on timestamps, directory enumeration order, thread scheduling, physical checkout location, or hash-map iteration order.

### 2.3 Non-goals

The core IR is not:

- a serialized Rust object graph;
- a generic XML DOM;
- a lossless formatter tree;
- a universal document model that promises every XML tree has a meaningful JSON, Markdown, or text interpretation;
- a place for project manifests, dependency solving, terminal styling, or job scheduling;
- a promise to keep byte-for-byte encodings stable across incompatible schema majors.

The core provides enough structure for multiple backends. Each backend still defines and validates its own mapping from the document model to output bytes.

## 3. Formal domains

Let `Bytes` be finite byte strings and `UString` be Unicode scalar-value strings encoded as valid UTF-8. The IR contains no floating-point numbers, platform-sized integers, native paths, or locale-dependent strings.

### 3.1 Logical source identity

A source has a logical identity independent of its content:

```text
SourceKey ::=
    Project { package: PackageInstanceId, path: LogicalPath }
  | AdHoc { uri: CanonicalAbsoluteUri }

LogicalPath ::= nonempty vector<PathSegment>
```

`PackageInstanceId` is the structured tuple `{ source_kind: u16, canonical_source: UString, package_name: UString, exact_revision: UString }` supplied by the manager's resolved lock graph and encoded with the canonical primitives in §10. Its digest is `D_package(canonical PackageInstanceId)`. `exact_revision` is resolver identity (for example a registry version or Git commit); an editable path package uses its declared relocation-invariant logical package identity, not an absolute checkout path or changing source-tree hash. Source byte digests remain revisions, not identities. Path segments are valid UTF-8, contain neither slash nor NUL, and are stored after RFC 3986 dot-segment removal. `..` cannot escape the owning package root. Segments compare bytewise, case-sensitively, with no Unicode normalization.

The project display URI uses the fixed lowercase scheme/authority `xmlsquish://package/`, followed by the percent-encoded canonical package instance and separately percent-encoded path segments:

```text
xmlsquish://package/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/src/prompt.xml
```

The first path segment is the full 64-character lowercase hexadecimal package-instance digest, never an abbreviated identity. Source-path percent encoding leaves only RFC 3986 unreserved bytes literal and uses uppercase hexadecimal; decoding must recover valid UTF-8. `file.uri` is this URI, `file.dir` removes the last source-path segment and retains a trailing slash, and `file.name` is the decoded last segment.

An ad-hoc root uses a canonical absolute `file:` URI. Canonicalization lowercases the scheme and ASCII host, removes dot segments, uses `/`, and rejects control characters, query, and fragment. A Windows drive designator is the explicit syntax exception: it is emitted as uppercase `file:///C:/...` with its colon literal; every ordinary path segment uses the percent rule above. Canonicalization does not case-fold path segments or resolve symlinks. Relative imports from an ad-hoc source are resolved by RFC 3986 against that URI and canonicalized again.

The manager must diagnose two logical paths that cannot coexist distinctly on the current physical filesystem (for example, a case-only collision on a case-insensitive Windows volume); it must not silently fold their language identities. A symlink target, inode, physical cache path, and content digest do not change `SourceKey`.

`SourceKey` is a semantic input because `file.uri`, `file.dir`, and `file.name` are observable DSL bindings. A checkout path used only to read bytes is debug/manager metadata and is not part of the semantic digest.

### 3.2 Content and object identities

All persistent digests use an algorithm-tagged value:

```text
Digest {
    algorithm: DigestAlgorithm,
    bytes: byte[algorithm.output_size]
}
```

The first format uses SHA-256. Algorithm tags make later migration explicit; two digests with different tags are never equal. Hash inputs are domain separated:

\[
D_k(x) = SHA256(UTF8(\text{"xmlsquish"}) \Vert \texttt{0x00} \Vert UTF8(k) \Vert \texttt{0x00} \Vert x)
\]

The following identities are different and must never be substituted for one another:

| Identity | Definition | Meaning |
| --- | --- | --- |
| `SourceDigest` | `D_source(exact source bytes)` | Invalidates parsing when bytes change |
| `SemanticUnitDigest` | `D_unit(canonical semantic section)` | Identifies executable unit meaning under a declared IR ABI |
| `DebugDigest` | `D_debug(canonical source/debug sections)` | Identifies exact source attachments and mappings |
| `ObjectDigest` | `D_object(complete canonical container)` | Content-addressed storage key for a complete object |
| `LinkedImageDigest` | `D_linked(canonical linked-image section)` | Identifies the metadata needed to reconstruct the resolved executable program |
| `DocumentDigest` | `D_document(canonical LinkedDocumentIR)` | Identifies backend-independent expanded content |
| `ArtifactDigest` | `D_artifact(product bytes)` | Identifies a final `*.prompt` byte sequence |

Hash equality is a practical collision-resistant identity, not a proof of mathematical equality. Cache implementations must retain object length and verify the digest when reading untrusted or corrupted storage.

### 3.3 Symbol identity

A macro symbol is exactly its XML expanded name:

\[
SymbolKey=(NamespaceURI, LocalName).
\]

Both fields are length-delimited UTF-8 strings. A prefix is lexical spelling and belongs in source attachments, never in `SymbolKey`. Symbol comparison is bytewise comparison of normalized UTF-8 code-unit sequences; the compiler performs no Unicode normalization.

The global symbol table of a linked program is a partial function:

\[
Symbols : SymbolKey \rightharpoonup DefAddr.
\]

Linking succeeds only if it is a function: two distinct reachable definitions of the same `SymbolKey` are a hard error. There is no overload set, weak definition, shadowing, or link-order winner.

### 3.4 Local references

Persistent local IDs are deterministic dense unsigned integers, not hashes and not memory addresses:

```text
LocalDefId  ::= macro declaration ordinal in source order
RegionId    ::= canonical preorder of semantic regions
OpId        ::= canonical preorder within all regions
StringId    ::= index in a bytewise lexicographically sorted unique string table
SourceRef   ::= index in a SourceRecord table sorted by canonical SourceKey
```

For diagnostics and linker records, the durable definition key is:

\[
DefKey=(SourceKey,SymbolKey,DeclarationOrdinal).
\]

The ordinal distinguishes two illegal duplicate declarations long enough to report both. A successful global symbol table still has at most one `DefKey` per `SymbolKey`. `DefIndex`/`DefAddr` are compact linked-image addresses derived by sorting units by `SourceKey` and symbols by `SymbolKey`; neither discovery order nor worker completion order participates.

Whitespace between XML tags is a text node when the language treats it as character data; it therefore participates in operation order. Markup-only lexical trivia represented only by `LosslessXmlTape` does not.

These IDs satisfy the useful determinism property:

\[
Compile(s, c)=Compile(s, c)
\]

byte for byte for equal source bytes and equal declared compilation context `c`. They are not promised to survive a semantic source edit. Durable cross-build identity is the relevant content digest plus the local ID.

## 4. Exact source and frontend representation

### 4.1 `SourceRecord`

Every compiled object has an exact source attachment:

```text
SourceRecord {
    key: SourceKey,
    digest: SourceDigest,
    encoding: Utf8,
    bom_len: u8,
    exact_bytes: BlobRef,
    line_start_offsets: vector<u64>
}
```

`BlobRef` is a digest plus byte length and is the only encoding inside a unit object. The exact source is a separate immutable CAS blob. A standalone debug bundle includes that blob as a separate bundle member; it never rewrites the unit object into an embedded alternative. Source spans are half-open byte ranges into the UTF-8 payload after the optional UTF-8 BOM:

\[
Span=[start,end), \qquad 0\le start\le end\le |payload|.
\]

`bom_len` is either `0` or `3`. Every span and line-start offset is relative to the payload after those bytes, never to `exact_bytes`. `line_start_offsets` begins with `0`; scanning XML source adds the byte after each LF, after each bare CR, and after the LF of a CRLF pair (which is one line break). The table is derived debug data and is validated against the exact payload. Machine diagnostics use one-based line and one-based UTF-8 byte column; a human renderer may additionally compute a Unicode/display column without changing the record.

### 4.2 `LosslessXmlTape`

The formatter owns a separate token tape:

```text
LosslessXmlTape {
    source: SourceRecord,
    tokens: vector<LexToken>,
    matching_delimiters: vector<(TokenId, TokenId)>,
    namespace_scope_index: ...
}
```

It preserves exact quote style, entity spelling, attribute order, prefix spelling, empty-element spelling, newline convention, comments, processing instructions, CDATA delimiters, BOM, and all trivia. Formatter edits are ranges over this tape. The semantic compiler may share the immutable source blob and lexer, but must not use `RelocatableUnitIR` to regenerate source.

This is a hard boundary because semantic parsing intentionally decodes entities and resolves names; that transformation is many-to-one and cannot support a lossless source round trip.

### 4.3 Source origin records

Each semantic definition, region, operation, import, parameter, slot, and external symbol reference has an origin attachment:

```text
Origin {
    source: SourceRef,
    span: Span,
    lexical_qname: optional DebugStringId,
    syntax_kind: SyntaxKind
}
```

Origins are indexed by stable `(entity-kind, local-id)` pairs in a separate debug section. The semantic section does not repeat source spans. Moving an element without changing its semantics may change the debug digest without forcing an unrelated backend artifact to change.

`OriginRef` is local to one object. A cross-object record uses `QualifiedOriginRef { object: ObjectDigest, local: OriginRef }`; a bare integer is never interpreted relative to whichever object happened to be decoded last.

Decoded textual values also retain a segment map:

```text
DecodedValueMap {
    owner: { entity_kind, local_id, field },
    segments: vector<{
        value_utf8_range: [u64, u64),
        source_span: Span,
        syntax: LiteralText | CharacterReference | EntityReference | CData
    }>
}
```

Ranges are ordered, non-overlapping, and cover the complete decoded value. This is what lets an output `&` point back to the exact `&amp;` source bytes rather than only to its containing element.

`DecodedValueMapRef` is local to its object; cross-object traces use `QualifiedDecodedValueMapRef { object: ObjectDigest, local: DecodedValueMapRef }`.

## 5. Relocatable unit IR

### 5.1 Container variants

Compiling one `SourceUnit` produces exactly one relocatable object:

```text
RelocatableUnitIR = ModuleObject | EntryObject
```

Both variants share:

```text
UnitHeader {
    ir_schema: { major, minor },
    language_abi: LanguageAbiId,
    frontend_abi: FrontendAbiId,
    regex_abi: RegexAbiId,
    source: SourceKey,
    imports: vector<ImportDecl>,
    semantic_strings: sorted unique vector<UString>,
    feature_bits: BitSet
}

UnitSourceAttachment {
    source: SourceKey,
    source_digest: SourceDigest,
    source_record: SourceRef
}
```

`language_abi` defines DSL meaning. `frontend_abi` changes whenever frontend lowering could produce observably different semantic IR. `regex_abi` fixes regex syntax, Unicode tables, and matching/capture semantics. A package language version is not a substitute for these stage ABIs.

The complete container also records a nonsemantic producer description (tool version and build fingerprint) for diagnosis. During development, before cross-version conformance proves an ABI reusable, the conservative action descriptor includes that build fingerprint. A stage may omit it only after its versioned ABI, golden corpus, and upgrade policy are enforced; forgetting to bump an ABI must never be the optimistic cache policy.

`UnitSourceAttachment` lives in the debug/source section. It binds the complete object and its origins to the exact input revision, but is excluded from the narrower `SemanticUnitDigest` projection when the lowered semantic section is unchanged. This distinction permits semantic reuse after a source-only lexical change without ever reusing stale source mappings.

Semantic and debug/source sections have separate sorted string tables. No semantic record may address a `DebugStringId`; otherwise inserting one debug-only spelling could renumber semantic strings and defeat the promised projection boundary.

### 5.2 Imports are relocations

The frontend does not load imported objects. It records import requests:

```text
ImportDecl {
    local_id: ImportId,
    spec: RelativeUri(UString)
        | AbsoluteFileUri(CanonicalAbsoluteUri)
        | PackageExport { dependency_alias: UString, export: UString },
    expected_kind: Module
}
```

The origin table attaches the `xs:import` span by `ImportId`; spans are never embedded in the semantic `ImportDecl`. `src` is a nonempty static URI without control characters, query, or fragment. A relative URI is resolved at the defining `SourceKey` using RFC 3986 and the normalization rules in §3.1. An absolute URI must use the supported `file` scheme. The closed `pkg:<alias>/<export>` grammar is decoded as `PackageExport`; alias/export validation and mapping come from the immutable project snapshot. Percent decoding happens exactly once, invalid UTF-8 is rejected, dot segments are removed before root-boundary checks, and no arbitrary network scheme reaches the compiler.

The manager's immutable resolution snapshot later maps `(importing SourceKey, ImportDecl)` to a resolved `SourceKey` and object digest. Keeping the spelling and the resolved identity separate permits independent frontend caching while retaining definition-site resolution and diagnostics.

Imports do not inject definitions, execute a body, create a frame, or impose a topological execution order.

### 5.3 `ModuleObject`

```text
ModuleObject {
    header: UnitHeader,
    definitions: vector<MacroDef>,
    external_symbols: sorted unique vector<SymbolKey>,
    interface: InterfaceSummary,
    origins: OriginTable,
    sources: SourceArchive
}

MacroDef {
    id: LocalDefId,
    symbol: SymbolKey,
    signature: Signature,
    body: RegionId
}

Signature {
    params: vector<LocalName>,
    slots: vector<{ name: LocalName, required: bool }>
}
```

Parameter and slot vectors retain declaration order for exact diagnostics. Validation also constructs a name-indexed view and rejects duplicates. `InterfaceSummary` duplicates only information needed for closure-wide symbol and signature validation; it is checked against bodies when the object is decoded and is never independently authoritative.

Module objects contain no executable entry and no implicit `main`.

### 5.4 `EntryObject`: an explicit link root

An `xs:entry` compiles to an `EntryObject`:

```text
EntryObject {
    header: UnitHeader,
    required_params: vector<LocalName>,
    root_region: RegionId,
    external_symbols: sorted unique vector<SymbolKey>,
    origins: OriginTable,
    sources: SourceArchive
}
```

It has no `SymbolKey`, `LocalDefId`, macro signature, or export. It is a **link description** in the following precise sense:

- its import declarations are roots of the source-object closure;
- its root region is the sole execution root;
- its external symbols are requirements that the closure must satisfy;
- its parameter list is the runtime input contract;
- selecting an entry selects one link operation, not a macro call.

Backend selection, output path, color, scheduler policy, and CLI options do not belong in `EntryObject`. The manager supplies them in a `BuildAction` so that the same entry can be linked once and emitted by more than one backend.

### 5.5 Core value sorts

The core uses only the two semantic sorts already present in the DSL:

```text
Scalar      = immutable UString
DocumentSeq = immutable ordered sequence of document nodes
```

They are deliberately not interchangeable. `RenderText` is an explicit checked conversion from a region result to `Scalar`; it succeeds only when every emitted node is character data. `Insert` is the explicit conversion from `Scalar` to one Unicode text node. Escaping and target-character validity belong to a backend serializer.

### 5.6 Region and operation algebra

A region is an ordered list of operations and returns a `DocumentSeq`:

\[
\llbracket Region(o_1,\dots,o_n)\rrbracket_ρ
=
\llbracket o_1\rrbracket_ρ \cdot \dots \cdot \llbracket o_n\rrbracket_ρ.
\]

The relocatable operation set is closed and versioned:

```text
DocOp ::=
    EmitText { value: StringId }
  | EmitComment { value: StringId }
  | EmitPi { target: StringId, data: StringId }
  | EmitElement {
        name: ExpandedName,
        attributes: vector<Attribute>,
        children: RegionId
    }
  | InsertScalar { value: BindingRef }
  | MatchRegex {
        input: MatchInput,
        pattern: StringId,
        captures: vector<LocalName>,
        matched: RegionId
    }
  | ReadSlot { name: LocalName }
  | Call {
        target: SymbolKey,
        args: vector<Argument>,
        fills: vector<Fill>
    }

MatchInput ::= Literal(StringId) | ReadBinding(BindingRef)

Argument {
    name: LocalName,
    value: ScalarExpr
}

ScalarExpr ::=
    Literal(StringId)
  | ReadBinding(BindingRef)
  | RenderText(RegionId)

Fill {
    name: LocalName,
    body: RegionId
}

BindingRef ::=
    File(Uri | Dir | Name)
  | Arg(LocalName)
  | Match(LocalName)
```

`Match(name)` performs a nearest-present binding lookup from the innermost active regex scope outward, but never crosses a call frame. This dynamic presence rule is necessary because an optional named capture that did not participate creates no binding; in that case an outer binding of the same name becomes visible again. Calls remain symbolic in relocatable IR.

`MatchRegex` evaluates its input once and applies the pattern under the declared `RegexAbiId`. Failure yields the empty document sequence and does not push a scope. Success pushes one lexical scope containing only named captures that actually participated, evaluates `matched`, and pops the scope before the next sibling. An inner present name shadows an outer name only for that region. Positional/unnamed captures and duplicate named captures are frontend errors; noncapturing groups remain legal. Captured values are decoded Unicode substrings and their offsets are UTF-8 byte ranges in the decoded input scalar.

Element names in semantic IR are `ExpandedName` values. Attributes preserve expanded name, decoded value, and author order. Lexical QNames, prefix choices, namespace-declaration spelling/order, entity spelling, and CDATA boundaries remain complete in the parallel source/debug attachment, not in the semantic value. Debuggers, formatters, and source viewers can consume that attachment; product backends see only expanded names and Unicode text.

The operation set intentionally contains no generic `eval`, mutable store, native path read, environment read, clock read, or backend-specific serialization operation.

### 5.7 Evaluation-order invariants

For `Call(target, args, fills)`:

1. resolve the already linked target;
2. evaluate arguments exactly once in their lexical source order in the caller frame;
3. evaluate fills exactly once in their lexical source order in the caller frame;
4. require each `RenderText` result to contain character data only and enforce runtime budgets;
5. allocate a fresh callee frame;
6. evaluate the body under definition-site `file.*`, the new `arg.*`, supplied slots, and an empty match stack.

Names and required/optional fill presence were already validated for every reachable and unreachable call by the linker. Rechecking these static facts during execution must not create a different error precedence. The serializer preserves call argument and fill order even though signature equality is name based; a canonical encoder must not sort these vectors.

A macro signature contains each slot exactly once; duplicate slot declarations are a frontend error. Every slot declaration is also one `ReadSlot` insertion site. The linker rejects unknown or duplicate fills and a missing required fill. At runtime a supplied fill is evaluated exactly once in the caller and reused immutably; an unsupplied optional slot yields the empty sequence. `EntryObject` cannot contain `ReadSlot`.

## 6. Linking

### 6.1 Inputs and outputs

```text
LinkRequest {
    entry_object: ObjectDigest,
    resolution_snapshot: ResolutionSnapshotDigest,
    linker_abi: LinkerAbiId,
    pass_pipeline: PassPipelineId
}

link(LinkRequest, ObjectStore)
    -> (LinkedImage, LinkTrace)
```

Both results are portable metadata. Linking does not serialize an executable
session object; evaluation first reconstructs a `LinkedProgram` from the image
and the referenced semantic unit blobs.

`ResolutionSnapshot` is supplied by the manager and contains both a canonical `SourceKey -> UnitRevision` map and import-edge bindings:

```text
UnitRevision { kind: Entry | Module, semantic: SemanticUnitDigest, object: ObjectDigest }
ImportBinding { importer: SourceKey, import: ImportId, target: SourceKey }
```

Before traversal, the linker verifies that each `SourceKey` has exactly one revision and kind, every binding target exists in that map, every binding's object agrees with its semantic section, and the requested entry object is the unique entry revision for its `SourceKey`. Conflicting revisions are a snapshot error; “first discovered wins” is forbidden. The linker performs no directory search, manifest mutation, registry lookup, network request, or terminal output.

### 6.2 Closure construction

Starting with the entry, the linker performs:

```text
pending := [entry]
intern entry SourceKey before traversing its imports

while pending is not empty:
    decode and verify one object
    for each import in declaration order:
        resolve through the immutable snapshot
        require target kind == Module
        if target SourceKey is new:
            intern it before traversal
            append it to pending
```

Intern-before-traverse makes every finite source graph terminate, including import cycles. The prevalidated revision function makes link correctness independent of queue discipline. The final unit table (entry plus modules) is sorted by canonical `SourceKey`, not discovery order.

### 6.3 Global validation and relocation

After closure construction, the linker:

1. verifies every object schema, checksum, declared ABI, source identity, and interface/body agreement;
2. constructs the global `SymbolKey -> DefAddr` table and rejects every redefinition;
3. validates every call in the entry and every module body, including unreachable definitions;
4. validates argument and slot names against the target signature;
5. replaces each symbolic target with a portable direct address;
6. emits a complete resolution trace.

```text
DefAddr {
    unit_slot: u32,   // index in SourceKey-sorted unit table
    local_def: LocalDefId
}

LinkedRegionRef { unit_slot: u32, region: RegionId }
```

`DefAddr` is stable for equal link inputs. It is never a process pointer and never leaks into a relocatable unit.

### 6.4 `LinkedImage`

```text
LinkedImage {
    schema: Version,
    language_abi: LanguageAbiId,
    entry: {
        source: SourceKey,
        semantic_digest: SemanticUnitDigest,
        required_params: vector<LocalName>,
        root_region: LinkedRegionRef
    },
    units: vector<{ kind: Entry | Module, source: SourceKey, semantic_digest: SemanticUnitDigest }>,
    definitions: vector<LinkedMacroDef>,
    symbol_table: sorted vector<(SymbolKey, DefAddr)>,
    relocations: sorted vector<(LinkedOpRef, DefAddr)>,
    feature_bits: BitSet
}
```

The relocation table is the portable equivalent of patched calls. An evaluator resolves a call operation through this verified table or an equivalent decoded index; it never repeats QName lookup during execution.

`LinkedImage` is deliberately non-executable metadata. Its addresses and region references become usable only after `LinkedProgram` reconstruction has loaded and validated every referenced semantic unit blob and built any session-local indexes. `StaticLinkMap` and `LinkedImage` may be persisted; `LinkedProgram` may be memoized only within a process session and has no wire encoding or CAS identity.

Each complete unit object publishes its canonical semantic section as a separately addressable `SemanticUnitBlob` under `SemanticUnitDigest`. `LinkedImage.units` includes the entry and all modules, so every `LinkedRegionRef` closes over an explicit semantic blob without choosing among multiple full objects that share semantics but carry different debug attachments. A portable export bundle includes the image and its transitive semantic blobs; a database build record additionally points to the exact full objects used for provenance. Missing referenced semantic blobs are storage corruption, not an unresolved language symbol.

No dead-definition elimination is permitted unless a future pass can prove that it preserves the language's requirement to validate unreachable definitions and preserves requested explain/debug behavior. Recursive strongly connected components need no special execution representation; calls are direct `DefAddr` edges and the evaluator uses explicit frames and budgets.

### 6.5 Link trace

```text
LinkTrace {
    entry_object: ObjectDigest,
    resolution_snapshot: ResolutionSnapshotDigest,
    imports: vector<{
        importer: SourceKey,
        import_id: ImportId,
        spec: ImportSpec,
        resolved_source: SourceKey,
        resolved_object: ObjectDigest,
        origin: QualifiedOriginRef
    }>,
    symbols: vector<{
        reference_origin: QualifiedOriginRef,
        symbol: SymbolKey,
        definition: DefAddr,
        definition_origin: QualifiedOriginRef
    }>,
    diagnostics: vector<DiagnosticRecord>
}
```

Trace records are emitted in canonical source/entity order, never in worker completion order. They are queryable independently of human terminal rendering.

## 7. Expansion and provenance

### 7.1 Expansion action

```text
ExpansionAction {
    linked_image: LinkedImageDigest,
    evaluator_abi: EvaluatorAbiId,
    arguments: sorted unique map<LocalName, UString>,
    budgets: {
        max_depth: u64,
        max_expansions: u64,
        max_document_items: u64,
        max_scalar_utf8_bytes: u64,
        max_temporary_items: u64
    }
}

evaluate(ExpansionAction)
    -> (LinkedDocumentIR, RuntimeTranscript, vector<DiagnosticSeed>)
```

Entry argument map order is not semantically observable, so it is canonically sorted. Duplicate spellings are rejected before construction of the action. Before creating the entry frame or charging any counter, the evaluator requires the supplied argument-name set to equal `LinkedImage.entry.required_params`; missing and unknown names are signature errors. Budgets are included because changing a budget can change success into a deterministic failure. `ExpansionTrace` is created only by the full-provenance join of this runtime transcript with current `LinkTrace` and unit debug objects.

Expansion budgets measure backend-neutral values only. Serialized payload bytes are not an evaluator metric: XML escaping, JSON encoding, or a binary backend can assign different byte lengths to the same document. Each backend instead receives an `EmissionBudget { max_payload_bytes }`, included in its own action options. The existing mixed `max-output-bytes` setting must be split at this boundary; otherwise the supposedly generic evaluator still embeds the squish/XML serializer.

Counters have one executable definition:

| Counter | Initial value | Charge point | Limit meaning |
| --- | ---: | --- | --- |
| `depth` | 1 for the entry frame | Before pushing a macro frame; decrement on return | `max_depth` is the maximum simultaneous active frames, including entry |
| `expansions` | 1 for the entry frame | Before allocating every macro frame; never decrement | `max_expansions` is the total frames created, including entry |
| `document_items` | 0 | Before every item occurrence appended to any final or temporary region | `max_document_items` is cumulative work, including items later consumed by `RenderText` or fills |
| `scalar_value_bytes` | size of each provided entry argument when admitted | Before constructing each literal/read/render/capture scalar result | `max_scalar_utf8_bytes` bounds every individual scalar UTF-8 value; equality is allowed |
| `live_temporary_items` | 0 | Before retaining an item in an argument/fill temporary; subtract when that temporary is released | `max_temporary_items` bounds the invocation-wide simultaneous total across all temporary sequences |

All counters use checked `u64` arithmetic and reject before committing the operation that would exceed the limit. Argument values larger than the scalar limit fail before entry evaluation. These rules are part of `EvaluatorAbiId`, so two implementations cannot legitimately disagree about the same action.

### 7.2 Frame model

Each runtime invocation allocates a fresh frame:

```text
FrameRecord {
    id: FrameId,
    parent: optional FrameId,
    identity: Entry | Macro(DefAddr),
    call_origin: optional QualifiedOriginRef,
    definition_origin: QualifiedOriginRef,
    depth: u64,
    args: vector<NamedValueRef>,
    fills: vector<NamedSequenceRef>
}
```

`FrameId` is the monotonically increasing creation ordinal under the specified deterministic evaluation order. Equal expansion inputs therefore produce equal frame IDs and traces. It is not a definition identity, and two calls with equal parameters never share a frame.

Captured strings and evaluated scalar arguments use `ScalarValueId`, an index into one bytewise-sorted unique UTF-8 value table in the trace. Fill sequences use `SequenceValueId`, an index into a sequence table ordered by deterministic evaluation creation; each sequence is a vector of document-occurrence IDs. This is the only persistent representation—writers cannot choose ad hoc between inline, digest, and table forms. Implementations may redact selected values in a user-exported diagnostic view, but the project artifact store's complete trace retains the canonical values needed to explain frame behavior. Redaction policy changes presentation, not the executable or document digest.

### 7.3 Output origin sets

Every emitted document-item occurrence has a parallel `TraceRef` in the full `DocumentBundle`; the semantic `LinkedDocumentIR` itself does not embed it:

```text
TraceRef {
    producer_op: LinkedOpRef,
    frame: FrameId,
    definition_origin: QualifiedOriginRef,
    call_origin: optional QualifiedOriginRef,
    substitution_chain: vector<SubstitutionStep>
}
```

The chain records slot/fill substitution, nested scalar-body evaluation, and insert origin. This avoids the false choice between attributing output only to a macro definition or only to its caller. Repeated output may share immutable source data while retaining distinct frame references.

The persistent form is an origin DAG rather than only a single source span:

```text
OriginNode ::=
    SourceSpan { origin: QualifiedOriginRef }
  | DecodedSegment { map: QualifiedDecodedValueMapRef, segment_index: u32 }
  | ExternalArgument { name: LocalName, value: ScalarValueId }
  | Expansion { frame: FrameId, producer: LinkedOpRef }
  | Import { edge: LinkImportRef, child: OriginNodeId }
  | RegexCapture { input: OriginNodeId, capture: LocalName, matched_range: [u64, u64) }
  | Concat { ordered_inputs: vector<OriginNodeId> }
  | BackendTransform { backend_step: DebugStringId, inputs: vector<OriginEdge> }
  | Fused { role: DebugStringId, inputs: vector<OriginEdge> }
  | Synthetic { reason: DebugStringId, nearest: optional OriginNodeId }
  | Unknown { reason: DebugStringId }

OriginEdge { role: OriginRole, parent: OriginNodeId }
```

Origin IDs use construction order fixed by semantics: static nodes use canonical object/entity order, runtime nodes use evaluator event order, and backend nodes use emitted output-range order; when one event creates several nodes, numeric node-kind order breaks the tie. Nodes are not structurally interned. Every edge points to an earlier ID, making the encoding unique, acyclic, and stream-verifiable. All reason/role text comes from the trace's sorted debug string table. Regex and decoded-value ranges are half-open UTF-8 byte ranges in the decoded scalar value, never Unicode-scalar indexes or source-byte offsets. `Unknown` is allowed only when a declared transformation cannot preserve a more precise origin; it is never a shortcut for an implementation that discarded available provenance. The origin graph captures many-to-one and one-to-many transformations such as entity decoding, scalar concatenation, repeated macro expansion, local-name lowering, and whitespace compression.

Backend byte mappings are interval records:

```text
ArtifactMapEntry {
    output_range: [u64, u64),
    origin: OriginNodeId
}
```

Entries are sorted, non-overlapping, and cover every product byte exactly once. An entry can point to a `Fused` or `BackendTransform` node with arbitrarily many typed parent edges, so one compressed whitespace byte can retain all contributing origins. Adjacent ranges with the same origin may be coalesced. Document occurrences that a backend drops retain trace records but have no output interval. Mapping is in output bytes, not characters or terminal columns.

### 7.4 Diagnostics

A structured diagnostic contains:

```text
DiagnosticRecord {
    code: DiagnosticCode,
    severity: Error | Warning | Note,
    message_args: canonical map<StringId, DiagnosticValue>,
    primary_origin: QualifiedOriginRef,
    related_origins: vector<{ role, origin: QualifiedOriginRef }>,
    frame_chain: vector<FrameId>
}
```

A semantic stage that lacks current source attachments returns a `DiagnosticSeed` with a stable code, arguments, and an entity/document-item token. The corresponding full-provenance action joins that token to `DiagnosticRecord`. It is invalid to fabricate an `OriginRef` merely to satisfy presentation.

Human messages are rendered after execution, allowing color and localization without changing cache identity. Stable diagnostic codes and structured arguments, not English prose, are the machine interface.

## 8. Linked document IR

### 8.1 Purpose and invariants

`LinkedDocumentIR` is the backend-independent, fully expanded document. It contains no imports, unresolved symbols, calls, parameters, slots, bindings, regex operations, or frames.

```text
LinkedDocumentIR {
    schema: Version,
    document_abi: DocumentAbiId,
    root: DocumentRegion,
    strings: sorted unique vector<UString>,
    feature_bits: BitSet
}
```

The source image, runtime transcript, and trace references are carried in a parallel `DocumentBundle`, not in the semantic document digest:

```text
DocumentBundle {
    document: DocumentDigest,
    linked_image: LinkedImageDigest,
    runtime_transcript: Digest,
    expansion_trace: DebugDigest
}
```

A `DocumentDigest` alone authorizes reuse of document content; it does **not** authorize reuse of an old trace. A consumer requiring complete current provenance must also request the matching full-debug action result.

The semantic document nodes are:

```text
DocumentItem ::=
    Text { value }
  | Comment { value }
  | ProcessingInstruction { target, data }
  | Element {
        expanded_name,
        attributes,
        children
    }
```

The persistent encoding uses a flat balanced event tape with explicit child ranges, not recursively owned host-language objects. This prevents deeply authored or deeply expanded content from overflowing the native stack and permits streaming backend reads. The verifier checks balanced elements, valid ranges, document order, XML-name constraints inherited from the frontend, string-table bounds, and trace-reference bounds.

The root is an ordered document region and may contain a forest or pure text. Structural event balance is a core invariant, but “exactly one XML document element” is not. The squish backend validates the current product rule that its input can become one well-formed XML document. Keeping that capability check in the backend is what allows a future text backend to accept a document sequence without contaminating link or evaluation semantics.

### 8.2 Why it is not “XML output with another extension”

The document IR preserves expanded namespace identity, attributes, text values, comments, processing instructions, and ordering as typed records. Lexical namespace declarations remain in source/debug attachments. The document IR contains no angle-bracket serialization. A backend may therefore:

- serialize and squish XML;
- emit plain text from a backend-defined projection;
- translate a documented element vocabulary to JSON;
- produce a binary prompt envelope;
- reject a document using features it cannot represent.

Backends must never silently invent a universal mapping. Supporting JSON, for example, requires a declared JSON mapping contract and validation, not a heuristic `Element -> object` conversion.

## 9. Backend contract

### 9.1 Interface

Conceptually:

```text
Backend {
    identity() -> { backend_id, backend_abi }
    capabilities() -> CapabilitySet
    validate(document: LinkedDocumentIR, options: CanonicalOptions)
        -> Result<(), BackendDiagnosticSeed>
    emit_payload(document, options, sink)
        -> Result<{ artifact_digest, emission_transcript, metrics }, BackendDiagnosticSeed>
    map_origins(emission_transcript, expansion_trace)
        -> Result<ArtifactByteMap, BackendDiagnosticSeed>
}
```

A backend is pure with respect to its declared inputs. It receives no project root, current directory, environment map, clock, random source, terminal, or mutable global configuration. If a future backend needs an external resource, that resource must be an explicit digest-addressed action input or the action must be marked non-cacheable.

`CanonicalOptions` includes the backend's `EmissionBudget`. A successful backend result includes an emission cost certificate (payload bytes and any backend-specific bounded resource counters). Core expansion and backend emission therefore have independent policies and independently checkable cache entries. Splitting payload emission from origin joining permits product-byte cache reuse while still requiring current provenance before returning a byte map.

`BackendDiagnosticSeed` identifies a backend step and, when applicable, a `DocumentToken = Item(DocumentItemId) | Region(DocumentRegionId) | EndOfDocument`; the full-provenance join resolves it through the `DocumentBundle` into a `DiagnosticRecord`. This permits semantic payload caching without forcing the backend to invent a source origin it was not given.

### 9.2 Squish backend

The first backend is `xmlsquish.squish`. Its declared lowering policy reproduces current product semantics:

1. remove DSL control operations (already guaranteed by `LinkedDocumentIR`);
2. drop all element attributes and never reintroduce lexical namespace declarations from debug attachments;
3. lower each element expanded name to its local name;
4. preserve user data-node order and character values through structural serialization;
5. validate the resulting XML document;
6. apply the existing final whitespace-squish algorithm;
7. return/stream the payload to the manager-provided sink; the manager stages and materializes it as `<target>.prompt`.

The precise treatment of comments, processing instructions, CDATA serialization, escaping, and whitespace is part of `SquishBackendAbi`, not an incidental serializer choice. A semantic change increments that ABI and invalidates only the backend cache layer.

Debug data remains in the build record:

```text
BuildRecord {
    product: ArtifactDigest,
    product_path: *.prompt,
    backend: { id: BackendId, abi: BackendAbiId, media_type: UString },
    unit_objects: vector<ObjectDigest>,
    linked_image: LinkedImageDigest,
    link_trace: DebugDigest,
    document: DocumentDigest,
    expansion_trace: DebugDigest,
    backend_map: DebugDigest
}
```

Thus the prompt stays clean while `explain`, diagnostics, IDE integration, and machine tooling can recover the full source-to-product chain.

## 10. Canonical binary serialization

### 10.1 Three contracts that must not be conflated

The implementation distinguishes:

1. **Live semantic IR:** ergonomic, strongly typed Rust values used by passes. It may contain arenas, compiled regex accelerators, and memoized indexes, none of which are persistent identity.
2. **Portable wire IR:** the versioned binary protocol specified below. It contains fixed semantic values and references, never host object layout.
3. **Canonical hash projection:** the explicitly classified semantic or full-object sections used to derive fingerprints and digests.

The CAS stores portable wire bytes. An action cache indexes canonical action descriptions. Equal live objects, equal wire bytes, equal semantic fingerprints, equal blob digests, and equal action keys are related but distinct propositions.

### 10.2 Storage-independent schema

The normative representation is a sectioned binary object. A database stores the same canonical byte strings as blobs keyed by digest; it does not redefine entities as mutable SQL rows. This makes immutable files, SQLite, a future remote CAS, and test fixtures interchangeable transports.

```text
XsIrContainer {
    magic: byte[8] = "XSQIR\r\n\x1a",
    container_major: u16-le,
    container_minor: u16-le,
    kind: u16-le,
    flags: u16-le,
    section_count: u32-le,
    sections: vector<SectionDirectoryEntry>,
    section_payloads: byte[]
}

SectionDirectoryEntry {
    tag: u32-le,
    flags: u32-le,          // semantic, debug, critical
    offset: u64-le,
    byte_len: u64-le,
    digest_algorithm: u16-le,
    digest_bytes: byte[32]
}
```

In version 1, digest algorithm tag `1` means SHA-256 and no other tag is accepted. The 20-byte header is followed by exactly `section_count` 58-byte directory entries. Tags are unique and strictly increasing. Container flags are zero. Section flag bits are `0=semantic`, `1=debug/source`, and `2=critical`; exactly one of bits 0 and 1 is set, every semantic section is critical, and every other bit is zero. The first payload offset equals `20 + 58 * section_count`; each subsequent offset equals the previous `offset + byte_len`. Offsets are absolute from the first magic byte. There is no alignment, padding, overlap, gap, or trailing data.

Each section digest is `D_section(tag || byte_len || payload)` under the fixed-width encodings above. `SemanticUnitDigest` hashes `(container major, object kind, ordered semantic section tag/length/payload triples)`; `DebugDigest` does the same for debug/source sections; `ObjectDigest` hashes every byte of the complete container. Canonical objects are uncompressed. A filesystem, database, or network transport may compress the complete object transparently, but decompression must reproduce and verify the exact canonical bytes before decoding. Compressor versions therefore cannot create multiple “canonical” encodings.

### 10.3 Primitive encoding

Canonical section payloads use one specified grammar:

- unsigned integers use minimal unsigned LEB128 unless a fixed width is stated;
- signed integers, if ever introduced, use a specified zig-zag mapping and minimal LEB128;
- booleans are exactly `0x00` or `0x01`;
- enums use registered numeric discriminants;
- strings are length-delimited valid UTF-8 and are never Unicode-normalized;
- byte strings are length-delimited raw bytes;
- vectors encode length followed by elements in semantic order;
- sets encode sorted unique elements;
- maps encode sorted unique keys under each key type's normative bytewise order;
- optional fields encode a one-byte presence tag followed by the value;
- floats are forbidden in current schemas;
- duplicate keys, invalid UTF-8, non-minimal integers, unknown critical sections, and trailing bytes are errors.

Source-order vectors such as document children, arguments, fills, parameters, slots, imports, and attributes are never sorted merely for serialization. Canonicality chooses one encoding of the same IR value; it does not erase observable or provenance-relevant order.

### 10.4 Semantic and debug sections

Every persistent object separates sections by effect:

| Class | Included in semantic digest | Examples |
| --- | --- | --- |
| Semantic | Yes | ABIs, logical `SourceKey`, operations, strings used by operations, imports, signatures, linked addresses, document content |
| Debug/source | No | exact source blobs, spans, lexical spellings that cannot affect evaluation, physical display locator, frame trace, formatted message text |
| Integrity | Derived | section digests, lengths, complete object digest |

If a field can affect `file.*`, evaluation order, error/success under budgets, or backend-visible document content, it is semantic even if it looks like metadata. Classification is by observability, not by the field's name.

### 10.5 Do not hash arbitrary serializer output

The canonical codec is part of the IR specification and has golden test vectors. We must not use default `serde`, `bincode`, host struct memory, or ordinary Protocol Buffers serialization as the content identity. Protocol Buffers explicitly warns that deterministic serialization is not canonical across schema, build, and library changes. RFC 8949 similarly distinguishes a format's many possible encodings from an application profile's deterministic requirements. A library may implement the codec, but the xmlsquish profile above remains normative.

### 10.6 Schema evolution

Versioning rules are:

- a major version changes semantics or the encoding of an existing critical construct;
- a minor version may add ignorable noncritical sections or fields with a fully specified default;
- unknown operation/type discriminants are always errors;
- unknown critical sections are errors;
- an old reader may skip an unknown noncritical debug section after verifying its bounds and digest;
- a new reader may decode an old version and run an explicit, tested upgrader;
- upgrading creates a new object and normally a new digest; the cache may retain an alias record from old to new;
- writers emit exactly one canonical current version, not a mixture selected by host data layout.

This follows the useful part of MLIR's versioned bytecode approach while avoiding the assumption that an evolving dialect remains magically compatible.

## 11. Cache model

### 11.1 Stage actions

The cache stores immutable values under action keys, distinct from the content digest of each result.

#### Frontend action

\[
K_F = H(
  FrontendABI,
  LanguageABI,
  RegexABI,
  SourceKey,
  SourceDigest,
  CanonicalFrontendOptions
).
\]

Result: `ObjectDigest` for a `ModuleObject` or `EntryObject`.

`SourceKey` is required because the observable `file.*` bindings can differ for identical bytes at two logical locations.

#### Semantic link action

\[
K_{L,sem} = H(
  LinkerABI,
  PassPipelineID,
  EntryUnitSemanticDigest,
  ResolutionSemanticDigest,
  SortedClosure[(SourceKey,SemanticUnitDigest)]
).
\]

Result: `LinkedImageDigest`.

`EntryUnitSemanticDigest` is the `SemanticUnitDigest` of the entry-kind object. `ResolutionSemanticDigest = D_resolution(canonical sorted [(importer SourceKey, ImportId, resolved SourceKey, resolved SemanticUnitDigest, target kind)])`. The closure is sorted to eliminate discovery-order nondeterminism. It is a flat canonical graph description, not a recursively defined Merkle hash of imports: naive recursive hashing has no base case on an import cycle. Physical locators and display metadata are excluded from this semantic projection.

#### Full-provenance link action

\[
K_{L,full} = H(
  K_{L,sem},
  ResolutionSnapshotDigest,
  EntryObjectDigest,
  SortedClosure[(SourceKey,ObjectDigest)]
).
\]

Result: `LinkTrace` digest and the exact source/debug bundle.

`ResolutionSnapshotDigest = D_resolution_full(canonical sorted [(importer SourceKey, ImportId, resolved SourceKey, resolved ObjectDigest, target kind)])`. The semantic key deliberately excludes source-only span changes; the full key cannot. A `K_{L,sem}` hit must never return an older `LinkTrace`. If current provenance is requested, the manager looks up `K_{L,full}` or rematerializes the trace from current objects.

#### Semantic expansion action

\[
K_{E,sem} = H(
  EvaluatorABI,
  LinkedImageDigest,
  CanonicalArguments,
  CanonicalExpansionBudgets
).
\]

Result: `DocumentDigest` and a runtime occurrence transcript whose IDs refer only to linked definitions/operations.

#### Full-provenance expansion action

\[
K_{E,full} = H(
  K_{E,sem},
  K_{L,full},
  RuntimeTranscriptDigest
).
\]

Result: `ExpansionTrace` and the document-item origin map. This action joins deterministic runtime occurrences to the current static origins; it prevents a semantic cache hit after a source-only edit from returning stale spans.

#### Backend action

\[
K_B = H(
  BackendID,
  BackendABI,
  DocumentDigest,
  CanonicalBackendOptions,
  CanonicalEmissionBudget
).
\]

Result: `ArtifactDigest` and backend metrics. A separate full-provenance backend-map action additionally includes `K_{E,full}` and maps current origin nodes to product byte ranges. Product reuse therefore does not imply debug-map reuse.

These semantic/full pairs are not optional bookkeeping. A semantic digest authorizes reuse only of the semantic value it covers. It can never be used to retrieve a complete debug object whose source digest or spans were excluded.

### 11.2 Cache soundness criterion

Let `A_S(x)` be the complete canonical action descriptor for deterministic stage `S`. The mathematical design obligation is:

\[
A_S(x)=A_S(y) \Longrightarrow Obs(S(x))=Obs(S(y)).
\]

`Obs` includes success versus error, semantic result bytes, structured diagnostics promised by that cache, and budget accounting. The stored lookup key is the finite digest `K_S(x)=D_action(A_S(x))`; treating digest equality as descriptor equality is an engineering inference under collision resistance, not a mathematical implication. An action-cache record should retain the canonical descriptor length and may retain/verify the descriptor bytes on a hit where adversarial or corruption concerns justify it.

This criterion gives a direct review question for every new input: can changing it change an observation? If yes, it belongs in the action or disables caching. Build time, terminal color, thread count, physical checkout directory, and scheduler completion order fail the opposite test and must not enter semantic keys.

### 11.3 Cache policy

- Content-addressed objects are immutable; writes use temporary data, flush, digest verification, and atomic publication.
- Action-cache records point to immutable object digests and may be evicted independently.
- A cache hit is decoded and fully verified before use; corruption becomes a cache miss plus a diagnostic event, not a successful build.
- Concurrent writers for the same digest may race safely because equal digest objects must have equal canonical bytes.
- Positive successful results are cacheable. Deterministic failure caching is permitted only when the failure schema and every observation are included in the stage contract.
- The database may maintain mutable last-access and garbage-collection metadata, but those fields are not part of any object or action digest.
- Garbage collection traces reachability from build records and pinned artifacts; it never interprets native row IDs as durable references.

This mirrors the production distinction in remote build systems between content-addressed blobs and an action cache: an action captures all execution inputs, while its result refers to immutable content.

## 12. Database and file layout

The object model does not require a database, but a production local store benefits from transactional metadata and deduplicated blobs:

```text
.xmlsquish/
  store.db                    # action index, build records, pins, GC metadata
  objects/sha256/ab/cdef...   # canonical immutable object/source/product blobs
  tmp/                        # same-filesystem publication staging
target/
  <target>.prompt             # user product
```

Recommended database relations are logical, not a required SQL schema:

```text
objects(digest, kind, byte_len, verified_at)
actions(action_key, result_digest_set, created_at)
builds(build_id, target_id, linked_image, document, product, traces...)
refs(owner_digest, child_digest, role)
pins(name, digest)
```

`created_at`, `verified_at`, access counters, and absolute paths are operational data. They never enter canonical objects. The manager owns transactions, locking, garbage collection, and materialization. The compiler owns only codecs and immutable value validation.

Portable debug export is a deterministic bundle containing the build record and its transitive objects. It must not depend on database row numbers.

## 13. Verification obligations

### 13.1 Structural verifier

Every decoded object is rejected unless all of the following hold:

- section bounds, lengths, digests, ordering, and feature bits are valid;
- every integer and collection has canonical encoding;
- string and source references are in bounds;
- local IDs are dense, unique, and in canonical order;
- every region and operation is owned exactly once where required;
- element event ranges are balanced and non-overlapping;
- signatures contain unique valid local names;
- import and call origins exist;
- regex patterns validate under the declared regex ABI and capture table;
- semantic/interface summaries agree;
- debug spans are inside the referenced exact source blob;
- trace frame parents precede children and form one rooted tree;
- document trace references identify existing operations and frames.

The verifier is iterative and budgeted. A malformed deeply nested object must not exhaust the native stack or allocate based on unchecked length fields.

### 13.2 Determinism tests

Required tests include:

| Property | Test method |
| --- | --- |
| Same input, same object | Compile in fresh processes and compare canonical bytes |
| Hash-map and discovery independence | Randomize internal insertion/traversal orders; output must stay equal |
| Cross-platform identity | Golden objects and digests on Windows, Linux, and macOS GitHub Actions |
| Relocation invariance | Build identical project snapshots under different absolute roots |
| Debug/semantic separation | Change only physical display path; semantic digest and product remain equal |
| Source sensitivity | Change logical `SourceKey` with identical bytes; `file.*` behavior invalidates correctly |
| Import-cycle termination | Generate cyclic source graphs and verify one interned unit per `SourceKey` |
| Recursive-call distinction | Equal calls receive distinct deterministic frame IDs |
| Canonical decoder strictness | Reject duplicate keys, nonminimal integers, trailing bytes, bad sections, and unknown critical ops |
| Round trip | `decode(encode(x)) = x` for generated valid IR values |
| Canonical re-encode | `encode(decode(canonical_bytes)) = canonical_bytes` |
| Cache differential | Cached and uncached builds have equal product, structured diagnostics, metrics, and traces |
| Backend isolation | Backend-only ABI change invalidates backend action, not frontend or link objects |
| Formatter isolation | `fmt` edits via `LosslessXmlTape`; semantic compilation of unchanged meanings remains equal |

Property-based generators must cover extreme nesting, empty strings and sequences, non-BMP Unicode, namespace aliases, duplicate symbol attempts, import strongly connected components, argument/fill permutations, missing optional captures, and limits around every length/budget boundary.

### 13.3 Semantic preservation tests

The manager route is now the authoritative executable semantics; the deleted monolithic compiler
is not retained as a differential oracle:

```text
XML source
  -> squish-xml-front RelocatableUnitIr
  -> squish-link StaticLinker / LinkedProgram / Instantiator
  -> LinkedDocumentIr + ExpansionTrace
  -> squish-backend
  -> .prompt + optional self-contained .psdbg
```

Semantic preservation is checked at the durable boundaries instead:

- `crates/squish-xml-front/src/tests.rs` covers module/entry distinction, all operation families,
  static imports, decoded scalar provenance, and canonical IR round trips;
- `crates/squish-link/src/tests.rs` covers complete-closure validation, import cycles, symbolic
  relocation, frame/capture/slot/file scope, stage-key boundaries, recursion, and budgets;
- `crates/squish-backend/src/tests.rs` covers document lowering, exact emission budgets, byte maps,
  and iterative deep documents;
- `crates/squish-manager/tests/build.rs::semantic_example_publishes_fully_traceable_debug_bundle`
  exercises the shipped multi-module XML example through publication and verifies every published
  output origin reaches a source archive; and
- `crates/squish-format/tests/semantic_oracle.rs` proves formatter rewrites preserve the current
  frontend semantics without routing formatting through semantic IR.

Expected failures compare typed codes and origins where those are protocol data, not unstable human
prose. These tests are implementation evidence; they do not replace the language argument in
`docs/verification/unified-macro-completeness.md` or make unsupported performance claims.

## 14. Implemented architecture

The production component graph follows the domain boundaries defined above:

| Boundary | Implementation | Owned values/operations |
| --- | --- | --- |
| Lossless formatting syntax | `crates/squish-format/src/syntax.rs` | `LosslessXml`, token spans, semantic-preserving rewrite input |
| XML frontend | `crates/squish-xml-front/src/{parser,lower}.rs` | DSL validation and lowering to `RelocatableUnitIr` |
| Portable schemas and codecs | `crates/squish-ir/src/{model,codec,wire,persistent,link_wire,document_wire,trace_wire,debug_bundle,validate}.rs` | canonical unit containers, link/document/trace payloads, `.psdbg`, validation and digests |
| Static linking and live reconstruction | `crates/squish-link/src/{linker,program,key}.rs` | complete closure, `StaticLinkMap`, `LinkedImage`, session-only `LinkedProgram`, stage keys |
| Instantiation | `crates/squish-link/src/instantiate.rs` | explicit task stack, immutable call inputs, frames, budgets, `LinkedDocumentIr`, `ExpansionTrace` |
| Squish backend | `crates/squish-backend/src/lib.rs` | backend validation, prompt bytes, backend origin nodes and artifact byte map |
| Store/publication | `crates/squish-store`, `crates/squish-publish` | verified CAS/action records and recoverable target generations |
| Planning/composition | `crates/squish-manager/src/build.rs`, `crates/squish-manager/src/orchestrator.rs` | action graph, cache restore, link/instantiate/backend execution, build catalog finalization |

Dependencies point inward toward immutable IR contracts. The manager composes stages, but IR and
compiler stages do not import CLI or terminal types. `LinkedProgram` is reconstructed from the
persisted image and semantic unit blobs and is never serialized. Formatting uses its lossless tape,
not `RelocatableUnitIr`. Backend code consumes `LinkedDocumentIr`, not XML parser nodes.

This table replaces the former implementation-order plan. Schema, codec, verifier, frontend,
linker, evaluator, backend, store, and manager integration are production code now. Cross-platform
workflow execution and ongoing golden/corruption coverage remain release evidence tracked in
`project-manager-execution-status.md`; they are not reasons to describe this normative model as a
proposal.

## 15. Implementation reconciliation

The old monolithic types named in earlier revisions no longer exist. Their architectural risks are
resolved at explicit current boundaries:

| Former risk | Current implementation and evidence |
| --- | --- |
| Compilation-global traversal-order IDs | `LocalDefId`, `RegionId`, and `OpId` are unit-local; the linker produces portable `DefAddr` values and canonical ordered link data (`squish-ir::model`, `squish-link::linker`). |
| Native `PathBuf` in portable provenance | `SourceKey` and protocol source IDs are logical values; physical project paths stay in repository/host adapters. Codec and relocation tests use canonical logical identities. |
| Process-local indexes serialized as targets | Unit IR keeps symbolic `SymbolKey` references; `StaticLinker` emits relocations and `LinkedImage`; only live `LinkedProgram` owns session indexes. |
| Compiled regex engine state in wire objects | IR stores patterns, named captures, and `RegexAbiId`; `Instantiator` compiles/uses live accelerators outside the canonical encoding. |
| Pointer sharing or lazy-cell state leaking into persistence | Wire types are owned deterministic values. `Rc` is confined to the live evaluator environment and is absent from the persistent schema. |
| Frontend/evaluator silently applying squish loss | `LinkedDocumentIr` retains document data; namespace/local-name and whitespace loss occurs in `squish-backend` with provenance edges and an artifact byte map. |
| Diagnostic intermediate containing only surviving tokens | `.xsir` retains complete unit semantic/source attachments and `.psdbg` packages link trace, expansion trace, source archives, document IR, backend mapping, prompt digest, and build metadata. `debug_bundle` validation rejects incomplete or inconsistent relations. |
| Output budgeting coupled to XML serialization | `Budgets` constrain backend-neutral instantiation and are part of `InstantiateKeyProjection`; backend emission has its own exact byte budget. Link/backend boundary tests cover both. |

No adapter from the deleted compiler is authoritative. Conformance now means that the current
schema/types, canonical encoders, validators, linker/evaluator, backend, and published artifacts
satisfy the invariants in Section 18. Evidence may reveal a defect in one of those implementations,
but it must be fixed against this model rather than reviving the old architecture.

## 16. Rejected alternatives

| Alternative | Rejection reason |
| --- | --- |
| Keep `*.i.xml` as executable IR | Expensive to parse, mixes diagnostic presentation with semantics, weak schema typing, and unsuitable as canonical cache identity |
| Serialize the current Rust AST directly | Host layout, crate upgrades, pointer ownership, regex objects, and map order become accidental file-format contracts |
| One IR for parser, formatter, linker, evaluator, and backend | Forces lossless trivia into execution or loses source fidelity; makes every change invalidate every consumer |
| Content hash as `SourceKey` | Breaks observable `file.*` identity and merges distinct logical resources |
| Absolute path as project source identity | Prevents relocation-invariant caches and reproducible artifacts |
| Assign global random UUIDs | Nondeterministic, not derivable, and unnecessary for immutable build values |
| Sort all vectors before hashing | Destroys source/evaluation/document order; canonical encoding must preserve semantic sequence |
| Drop attributes/namespaces in module IR because squish drops them | Bakes one backend's lossy policy into the frontend and prevents future backends or complete debugging |
| Store only definition provenance | Loses caller and substitution origin for repeated macro expansions |
| Put traces inside `*.prompt` | Pollutes the product and couples consumers to diagnostics |
| Use database row IDs as references | Makes exports, recovery, compaction, and cross-machine cache sharing unstable |
| Treat unknown semantic opcodes as no-ops | Silently changes programs; unknown semantics must be rejected |
| Include the entire tool version in one monolithic cache key | Correct but needlessly invalidates unrelated stages; explicit stage ABIs localize change while remaining safe |

## 17. Evidence and design lineage

This design borrows principles, not surface syntax:

- LLVM separates modules, symbolic references, metadata, and debug mappings; its debug philosophy explicitly maps source-language objects to generated objects without making debug information executable. See the [LLVM Language Reference](https://llvm.org/docs/LangRef.html) and [Source Level Debugging with LLVM](https://llvm.org/docs/SourceLevelDebugging.html).
- LLVM bitcode separates a generic bitstream/container from the IR encoding and uses magic numbers, blocks, records, and string tables. See the [LLVM Bitcode File Format](https://llvm.org/docs/BitCodeFormat.html).
- MLIR demonstrates the value of explicit abstraction levels and dialect boundaries rather than forcing all compiler phases through one representation. The research rationale is described in [MLIR: A Compiler Infrastructure for the End of Moore's Law](https://arxiv.org/abs/2002.11054); the production [MLIR bytecode format](https://mlir.llvm.org/docs/BytecodeFormat/) provides concrete lessons in sections, versioning, lazy reading, and dialect upgrades.
- StableHLO's production compatibility model adds the versioned, add-only VHLO wire dialect rather than assuming ordinary MLIR bytecode makes a changing dialect stable. Its [bytecode documentation](https://openxla.org/stablehlo/bytecode), [VHLO specification](https://openxla.org/stablehlo/vhlo), and compatibility corpus motivate a distinct portable schema, explicit upgrade/downgrade rules, and historical fixtures in CI.
- Bazel's Remote Execution API distinguishes content-addressed inputs/results from action-cache keys and defines an action as the complete input needed to reproduce an execution. See the primary [Remote Execution API repository](https://github.com/bazelbuild/remote-apis).
- The peer-reviewed [IRHash study (USENIX ATC 2025)](https://www.usenix.org/conference/atc25/presentation/landsberg) demonstrates why an IR-level semantic projection can improve reuse, but also deliberately excludes debug information from its default hash. That trade-off directly motivates separate semantic and full-provenance actions here; its LLVM/C results are not evidence for the magnitude of gains on prompt projects.
- [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949.html#section-4.2) shows why a binary data model still needs explicit deterministic-encoding rules. The [Protocol Buffers documentation](https://protobuf.dev/programming-guides/serialization-not-canonical/) is an important counterexample: deterministic protobuf output is not a stable canonical hash format.
- Reproducible Builds documents absolute build paths and timestamps as common nondeterminism sources; see [Build path](https://reproducible-builds.org/docs/build-path/) and [Timestamps](https://reproducible-builds.org/docs/timestamps/).

These sources support the architecture, but they do not prove xmlsquish's cache keys sound. Soundness remains an obligation of the explicit stage equations, ABI discipline, canonical codec, verifier, and differential tests defined above.

## 18. Final invariants

The implementation is conforming only if all of these remain true:

1. Exact source bytes and formatter syntax are recoverable independently of semantic IR.
2. Equal declared frontend inputs produce byte-identical relocatable objects across supported platforms.
3. Every module object is independently cacheable and contains symbolic, not process-local, call references.
4. An entry is an explicit link root, never a synthetic macro.
5. Source cycles terminate by interning; call cycles remain runtime recursion.
6. Linking validates the complete closure and produces order-independent symbol resolution.
7. `StaticLinkMap` and non-executable `LinkedImage` metadata are persistable; the executable `LinkedProgram` is reconstructed for a session and never serialized.
8. Every dynamic call creates a distinct frame with a complete parent/call/definition chain.
9. The document IR contains all user document information and no DSL control operation.
10. Backend loss is explicit in a versioned backend contract, never hidden in frontend lowering.
11. The squish backend emits `*.prompt`; provenance remains available through the build record.
12. Cache identity is computed from canonical specified bytes, not an incidental serializer.
13. Semantic and debug digests are separate, but every field is classified by observability rather than convenience.
14. Unknown semantic constructs are rejected, not ignored.
15. Physical paths, timestamps, scheduler order, terminal color, and database row IDs cannot affect semantic identity.
16. Cached and uncached execution are observationally equivalent under the same declared inputs.
