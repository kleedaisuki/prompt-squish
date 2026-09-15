# `squish-fetch` implementation map

This crate is the concrete source-host implementation for
[`docs/design/dependency-source-protocol.md`](../../docs/design/dependency-source-protocol.md).
It is intentionally not yet a workspace member: the composition-root/lock-schema migration owns
that integration step.

## Code map

| Protocol responsibility | Implementation |
|---|---|
| Typed SHA-256 identities, algorithm-tagged Git OIDs, access policy, redaction-safe events | `src/types.rs` |
| Portable path validation, canonical package-tree digest, `.xspkg` validation, complete marker validation, writer locking, staging and atomic rename | `src/materialize.rs` |
| Sparse endpoint/config validation, Cargo shard paths, JSONL validation, ETag-preferred conditional caching, strict local-only path, exact archive CAS | `src/registry.rs` |
| Git protocol v2 fetch into a URL-keyed bare database, exact ref namespaces, type peeling, direct `ls-tree`/`cat-file` traversal, cached selector observations | `src/git.rs` |
| Workspace-confined path dependency loading | `src/filesystem.rs` |

## Verification and integration status

Unit tests cover shard vectors; canonical digest ordering and byte sensitivity; portable
ancestor/file-directory collisions; complete materialization byte revalidation; preservation of
last-known-good registry metadata; corrupt-body unconditional HTTP recovery; shorthand/detail
dependency normalization; concurrent Git ref acquisition; checkout-filter/raw-byte independence;
SHA-256 Git repositories when supported locally; HTTP/decompression allocation bounds; and
concurrent atomic sidecar publication. The crate is checked independently by temporarily adding it to the workspace;
the permanent workspace edit and resulting `Cargo.lock` update are deliberately left to the
composition owner. Local Git and HTTP acceptance fixtures remain integration work.

## Verified deviations and migration constraints

* `squish-resolver` still exposes its lock-v1 `RegistryCandidate { checksum, manifest }` and
  `GitCandidate { revision, checksum, manifest }` seams. The crate therefore exposes rich
  `SparseCandidate`/`ExactGitCandidate` APIs and also implements compatibility adapters for the
  old traits. The adapters necessarily project archive/content identities into the old generic
  fields; callers requiring protocol-v2 invariants must use the rich APIs until resolver migration.
* Resolver manifest-v1 has no Git `subdir`. `GitHost::resolve_exact` supports an explicit subdir,
  while the old `GitPort` adapter uses repository root. This is an API migration constraint, not
  implicit repository searching.
* The v1 cache layout uses JSON sidecars and filesystem writer locks rather than the eventual
  SQLite mapping/lease transaction. Immutable blob/tree publication and complete-marker
  validation are preserved; GC/read-lease accounting awaits store integration.
* Authenticated cross-origin redirect re-scoping requires a transport seam richer than the current
  compatibility ports. Redirect count and operational retries are bounded, retry facts are emitted,
  and reqwest strips sensitive headers on cross-host redirects; asking the credential port again for
  the new origin remains composition-layer integration work.
