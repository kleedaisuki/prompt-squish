# Dependency Source Protocol: Sparse Registries and Immutable Git Trees

- Status: Proposed normative design for the first remote source host
- Scope: registry and Git source configuration, resolution metadata, fetching,
  verification, caching, materialization, offline operation, and source events
- Implements: the `RegistryPort` and `GitPort` seams in `squish-resolver`, and
  the remote-package side of `SourceProvider` described by ADR 0009
- Related: [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md),
  [Core IR and Linking Model](ir-model.md)

## 1. Problem and decision

The resolver already knows *when* it may consult a registry or Git source and
already produces an exact lock graph. It deliberately does not know how an
index is fetched, what a package archive contains, how a Git ref is peeled, or
how remote bytes become an immutable local package. The next implementation
must fill that seam without leaking URLs, checkouts, HTTP validators, or native
paths into source identity.

This design makes the following decisions.

1. The first registry transport is a Cargo-inspired **sparse HTTP index**. A
   configured `sparse+https://.../` endpoint serves `config.json`, one
   newline-delimited JSON metadata resource per package, and immutable package
   archives. A Git-backed registry index is not implemented in version 1.
2. A registry has a stable logical `registry-id`, separate from its current
   endpoint. The lockfile records the ID and immutable digests, not a cache path.
   A mirror can therefore replace an endpoint only when it declares the same ID
   and serves byte-identical locked archives.
3. Registry versions are pinned by both the SHA-256 digest of the downloaded
   archive bytes and a format-independent SHA-256 digest of the normalized file
   tree. The two values have different types and are never called a generic
   `checksum`.
4. A Git selector is resolved once to a full, algorithm-tagged commit object
   ID. The corresponding root tree ID and selected package-subtree ID are also
   recorded. A separate cross-source content digest identifies the materialized
   package bytes. Builds with a current lock never follow the ref again.
5. Registry and Git packages materialize through the same validated logical
   file-tree model and the same atomic directory publisher. This removes source-
   specific cases after acquisition.
6. `--offline` means no DNS, socket, or remote Git operation. `--frozen` remains
   `--locked` plus `--offline`; all exact remote objects and complete
   materializations must already be local. Missing cache state is reported with
   the exact fetch command or source identity needed to populate it.
7. Network and cache behavior is visible through typed events. Credentials,
   URL user information, query strings, and response bodies never enter events.

The core invariant is:

```text
mutable locator/ref/index
        |
        v
exact locked source identity
        |
        v
verified logical file tree --atomic publish--> immutable materialization
        |
        v
SourceEnvelope snapshot (logical SourceId + exact bytes)
```

Only the last line is visible to the XML frontend. Physical source-host state is
an adapter concern.

## 2. Evidence and what is deliberately reused

Cargo is useful production evidence, not a wire-compatibility requirement.
Cargo's index gives each package a lowercased, sharded path; stores one JSON
object per version per line; pins the exact `.crate` bytes with SHA-256; and
uses conditional sparse HTTP requests with `ETag` or `Last-Modified` (preferring
`ETag`). Its sparse protocol avoids cloning the full index and benefits from
HTTP/2 multiplexing. These behaviors are documented in the official
[Cargo registry index specification](https://doc.rust-lang.org/cargo/reference/registry-index.html).
Cargo also separates bare Git databases, revision-specific checkouts, registry
archives, and extracted sources, although its Cargo-home layout is explicitly
unstable; see the official [Cargo Home description](https://doc.rust-lang.org/cargo/guide/cargo-home.html).

Git itself is a content-addressed object database. A commit points to a root
tree, while trees name blobs/subtrees together with modes and names; see the
official [Git objects description](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects).
Git protocol v2 advertises an object format and separates `ls-refs` from
`fetch`; see the [protocol v2 specification](https://git-scm.com/docs/gitprotocol-v2).
Git's SHA-256 transition also means that a full Git object ID must carry its
algorithm, rather than being modeled as an unqualified 40-character string;
see [Git's hash transition design](https://git-scm.com/docs/hash-function-transition).

The design also follows the broader, peer-reviewed result that immutable,
functionally addressed package state makes side-by-side versions and
deterministic reconstruction tractable, while mutable in-place installation
does not: Dolstra, Löh, and Pierron's
[NixOS: A Purely Functional Linux Distribution](https://doi.org/10.1017/S0956796810000195).
Software Heritage independently distinguishes an intrinsic object identifier
from contextual origin information in a Merkle DAG; that distinction supports
keeping `ContentDigest` separate from registry/Git locators
([Di Cosmo, 2020](https://pmc.ncbi.nlm.nih.gov/articles/PMC7340894/)).

### 2.1 Cargo semantics reused

| Cargo behavior | xmlsquish decision | Reason |
| --- | --- | --- |
| Sparse endpoint is explicitly identified by a `sparse+` URL | Reuse | Configuration is unambiguous and no filesystem heuristic chooses a protocol. |
| `config.json` is fetched before package metadata | Reuse | The client learns registry identity, download template, and authentication requirement once. |
| One package metadata file, one JSON version per line | Reuse | Appending a version is cheap, conditional GET works well, and parsing is streamable. |
| Lowercase package-name sharding | Reuse exactly | It is proven and avoids directories containing unbounded numbers of entries. |
| `ETag`, otherwise `Last-Modified`, with HTTP 304 | Reuse exactly | This is ordinary HTTP caching and avoids an xmlsquish-specific freshness protocol. |
| SHA-256 of exact downloaded archive bytes in index/lock | Reuse | It binds the resolver result to transport bytes and makes mirrors/cache verification simple. |
| A yanked bit may change, other version metadata is immutable | Reuse | Existing locks remain buildable while new resolution avoids withdrawn versions. |
| Git branch/tag/default HEAD becomes an exact locked commit | Reuse | Human intent may be movable; execution identity may not be. |
| Bare Git database shared by revision-specific materializations | Reuse | Network objects are deduplicated without exposing a mutable checkout to compilation. |
| `--offline`; `--frozen = --locked + --offline` | Reuse | The meanings are familiar and already adopted by ADR 0009 and `ResolutionMode`. |

### 2.2 Cargo semantics not copied

| Cargo behavior | xmlsquish decision | Why it is not copied |
| --- | --- | --- |
| Both Git and sparse registry-index protocols | Sparse only in v1 | A second index transport doubles cache and identity cases without helping the dominant small-metadata fetch. |
| Registry URL itself is a lockfile source identity | Stable `registry-id` in lock; endpoint in config | Endpoint migration and intentional mirroring should not rewrite the resolved graph. Cargo documents protocol/URL transition difficulties. |
| Cargo index's Rust-specific `deps`, target, link, and feature schema | Small xmlsquish resolver projection | XML packages have no Rust target triples, build-dependency kind, native `links`, or Cargo feature language. |
| `.crate` packaging rewrites Cargo manifests and includes VCS/build-specific files | `.xspkg` carries exact `xmlsquish.toml` and ordinary source files | Rewriting human intent would make manifest verification and source provenance needlessly indirect. Cargo's packaging transformations are documented by [`cargo package`](https://doc.rust-lang.org/cargo/commands/cargo-package.html). |
| Archive checksum is the only published package-content identity | Archive digest **and** normalized tree digest | Archive recompression and cross-source equality are distinct from source-tree equality. |
| Automatically search an entire Git repository for matching manifests | Manifest at explicit `subdir` (root by default) | Explicit ownership removes ambiguous monorepo traversal and makes the subtree identity cheap to verify. |
| Recursively initialize Git submodules | Reject gitlinks in v1 | Recursive repositories introduce another resolver, credential, lock, and offline graph. A normal xmlsquish dependency expresses that graph directly. Cargo does recurse into submodules, but that is not required language behavior. |
| Worktree checkout semantics, including filters and platform line-ending conversion | Read blobs directly from the Git object database | Exact source bytes must not depend on `core.autocrlf`, smudge filters, hooks, or a native checkout. |
| Cargo-home physical directory names | Hash-derived private cache keys | Cargo explicitly says its layout is unstable; it is evidence for separation, not an API to clone. |

## 3. Configuration and canonical source locators

Registry endpoints are machine/user configuration, not package intent. A
manifest names a registry alias; resolved metadata names a stable registry ID.

```toml
# .xmlsquish/config.toml (workspace) or the user configuration file
[registries.corp]
id = "https://packages.example.com/xmlsquish"
index = "sparse+https://mirror.example.net/xmlsquish-index/"

[registries.default]
id = "https://registry.xmlsquish.example/v1"
index = "sparse+https://registry.xmlsquish.example/index/"
```

The initial precedence, highest first, is repeated CLI `--config`, workspace
`.xmlsquish/config.toml`, user configuration, then compiled defaults. There is
no implicit endpoint inferred from a dependency name. An endpoint value must:

- use `sparse+https` for remote registries (`sparse+file` is allowed only by
  the test/local-registry adapter);
- contain no user information, query, or fragment;
- end in `/` so URL joining cannot discard a path component; and
- canonicalize the embedded HTTPS URL by lowercasing scheme/ASCII host,
  removing its default port, and removing RFC 3986 dot segments while
  preserving path case.

`id` is a canonical absolute HTTPS URI with no user information, query, or
fragment. It is a logical authority, not necessarily a reachable endpoint. The
server's `config.json` must return the same `registry-id`; otherwise the
endpoint is misconfigured for the selected alias. Two aliases with the same ID
are the same resolver domain. Two IDs at the same endpoint are not.

Dependency intent remains concise:

```toml
[dependencies]
common = { package = "common-prompts", version = "^1.4", registry = "corp" }
ui = { git = "https://git.example.com/prompts/ui.git", tag = "v2.1.0" }
mono = { git = "https://git.example.com/prompts/all.git", branch = "stable", subdir = "packages/mono" }
```

`registry` is an alias resolved only in the effective configuration. `git` is
a canonical absolute `https:`, `ssh:`, or `file:` URL. SCP-like syntax is not
accepted because it has no single URI normalization. URL user information and
fragments are rejected; credentials are supplied through a separate credential
port. Scheme and ASCII host are lowercased and a default port is removed;
repository path case and a trailing `.git` are preserved because servers do not
universally treat alternative spellings as the same repository.

Exactly zero or one of `branch`, `tag`, and `rev` may occur. Zero means the
remote symbolic `HEAD`. `branch` and `tag` are names, not revision expressions.
`rev` in v1 is a full 40- or 64-hex object ID only; abbreviated IDs and Git
revision-expression syntax are rejected. `subdir` is a portable logical path
and defaults to the repository root. These rules require adding `subdir` to
`DependencyDetail` in the next manifest schema while retaining manifest-v1
reading as described in section 12.

## 4. Sparse registry wire protocol

### 4.1 `config.json`

The client first performs `GET <index-base>config.json` with an accept header
for `application/vnd.xmlsquish.registry-config+json; version=1`. Version 1 is:

```json
{
  "v": 1,
  "registry-id": "https://packages.example.com/xmlsquish",
  "dl": "https://cdn.example.com/xspkg/{package}/{version}/{archive-sha256}.xspkg",
  "auth-required": false
}
```

- `v` is required. Unknown values make this endpoint unusable; they are not
  interpreted as v1.
- `registry-id` must match effective configuration.
- `dl` is an absolute HTTPS URL template. Supported markers are `{package}`,
  `{version}`, `{prefix}`, `{lowerprefix}`, and `{archive-sha256}`. Each textual
  value is percent-encoded as URL-path data before substitution. A template
  cannot place a marker in scheme, authority, user information, query, or
  fragment. If no marker occurs, `/{package}/{version}/download` is appended,
  matching Cargo's useful default.
- `auth-required` defaults to false. It controls whether the credential port is
  consulted for index and archive requests. Secret values never enter cache
  keys, lockfiles, diagnostics, or events.
- Unknown keys are ignored and retained in the raw cached blob; they do not
  become semantic inputs until a later understood config version defines them.

The server may use ordinary redirects. The transport strips credentials before
following a redirect to a different origin and asks the credential port again
for that origin. Redirect chains are bounded and observable.

When no validated config is cached, the client first tries `config.json`
without a credential. A 401 causes one credential-port lookup and one
authenticated retry, following Cargo's sparse-registry bootstrap behavior. A
cached `auth-required = true` allows the lookup before the first request. The
credential port returns an authorization value scoped to the registry ID and
request origin; the source host neither persists nor prints that value.

### 4.2 Package index path

Names are restricted to 1--64 ASCII characters, begin with an ASCII letter,
and subsequently contain only ASCII alphanumeric characters, `-`, or `_`.
Names compare case-insensitively and with `-`/`_` folded for publication
uniqueness, while the declared spelling remains part of package metadata.

After ASCII lowercasing, the Cargo shard algorithm is used:

```text
len 1: 1/<name>
len 2: 2/<name>
len 3: 3/<first>/<name>
else:  <first-two>/<second-two>/<name>
```

Thus `common-prompts` is fetched as
`co/mm/common-prompts`. The metadata media type is
`application/vnd.xmlsquish.package-index+jsonl; version=1`.

### 4.3 Version metadata

Each non-empty line is one UTF-8 JSON object. A `(folded-name, SemVer without
build metadata)` version occurs at most once. Lines have a configured maximum
length and the whole response has a configured maximum size. The schema is:

```json
{
  "v": 1,
  "name": "common-prompts",
  "vers": "1.4.2",
  "package": {
    "dialect": "xmlsquish/1",
    "source-root": "src"
  },
  "deps": [
    {
      "alias": "base",
      "package": "prompt-base",
      "req": "^2.0",
      "registry-id": "https://packages.example.com/xmlsquish",
      "optional": false,
      "default-features": true,
      "features": []
    }
  ],
  "archive": {
    "format": "xspkg-tar-gzip/1",
    "size": 18342,
    "sha256": "c4f0...64 lowercase hex...",
    "content-sha256": "7e21...64 lowercase hex..."
  },
  "manifest-sha256": "91ab...64 lowercase hex...",
  "yanked": false,
  "published-at": "2026-09-15T03:00:00Z"
}
```

Normative rules:

- `v`, `name`, `vers`, `package`, `deps`, `archive`, `manifest-sha256`, and
  `yanked` are required. An unknown `v` is skipped as an unsupported candidate,
  not partially interpreted. Unknown fields within a recognized v1 row are
  ignored after all required fields validate, allowing additive presentation
  metadata without changing resolution semantics.
- `vers` is SemVer 2.0.0. Build metadata does not make a second publishable
  version. The package `name` and `version` reconstructed from the row must
  match the selected archive manifest.
- `deps` contains only direct normal xmlsquish dependency edges. Version-1
  registry packages may depend only on registry packages. Path, workspace, and
  Git locators in a published archive are rejected. Entries sort by UTF-8 alias
  bytes when published; clients accept any order and canonicalize it.
- `registry-id` defaults to the current registry ID. It names an identity, not
  a local alias or endpoint. Resolution fails with a precise configuration
  diagnostic if no effective alias maps the referenced ID.
- `archive.sha256` is SHA-256 over the exact downloaded bytes.
  `archive.content-sha256` is the logical tree digest from section 6.
  `manifest-sha256` is SHA-256 over the exact root `xmlsquish.toml` file bytes.
  All are lowercase, exactly 64 hex digits.
- `archive.size` is the exact compressed byte length and is checked before the
  blob becomes visible in the CAS.
- `yanked` is the sole mutable field. A fresh resolution excludes yanked rows;
  a current lock may continue to use its exact version and digests. An explicit
  exact-version update may select a yanked version only with an explicit policy
  flag.
- `published-at` is optional presentation metadata. It never affects ordering,
  resolution, a lock identity, or an action key.

The index projection contains everything required by version solving; the
resolver does not download every candidate archive merely to discover its
dependencies. Once a candidate is selected, materialization parses its complete
manifest and verifies that `(name, version, dialect, source-root, dependencies)`
exactly matches the canonical projection. Targets, profiles, and exports live
only in the archive manifest and become available after selection.

The same closed-tree rule applies to Git packages: a manifest loaded from a
remote Git `subdir` may depend on registry or other Git packages, but v1 rejects
path/workspace dependencies in that remote manifest. Such a path would otherwise
refer outside the locked package tree or create an implicit second monorepo
resolver. Authors represent another in-repository package as its own explicit
Git dependency and `subdir`, so it receives its own exact lock node.

### 4.4 HTTP freshness and errors

For `config.json` and each package resource, the cache stores response bytes,
the canonical request identity, `ETag` if present, otherwise `Last-Modified`,
and the time it was observed. Online refresh sends `If-None-Match`, otherwise
`If-Modified-Since`. A 304 reuses the already validated body. A 200 is parsed
and validated completely in staging before atomically replacing the cached
mapping. Malformed new metadata does not destroy the last valid body.

Status 404, 410, and 451 mean no usable package resource and may be negatively
cached for the response freshness lifetime. A 401/403 is an authentication
failure, not "package absent". HTTP 408, 429, and 5xx responses use bounded,
jittered retry and `Retry-After` when valid; every retry is an event. Other 4xx
responses are terminal for that request. Ordinary offline cache misses never
attempt a request.

## 5. Registry archive format

The filename suffix is `.xspkg`; media type
`application/vnd.xmlsquish.package+gzip; version=1`; format tag
`xspkg-tar-gzip/1`. The bytes are a gzip stream containing a POSIX ustar/PAX
tar archive. Every member is below one top-level directory exactly named
`<name>-<version>/`. The root contains `xmlsquish.toml`.

Version-1 semantic members are directories and regular files only. Symlinks,
hard links, devices, FIFOs, sparse entries, sockets, and additional top-level
roots are rejected. Empty directories and tar metadata are ignored after
validation. Regular-file executable bits, owner/group, timestamps, xattrs, and
PAX keys are not semantic and are not reproduced in a materialization. This
deliberately makes all package inputs ordinary read-only files.

A publisher should produce members in UTF-8 byte order, use uid/gid/mtime zero,
empty owner/group names, modes `0755` for directories and `0644` for files, and
gzip mtime zero. These canonical producer rules make republishing deterministic,
but a client does not substitute a locally recompressed archive for the exact
locked archive digest.

Extraction is streaming. Before writing a member, the adapter validates its
raw archive name (section 7), rejects duplicates, reserves its destination with
create-new semantics, and charges compressed bytes, expanded bytes, member
count, path length, and per-file length against configured acquisition limits.
Digest and declared compressed size are checked while downloading into the CAS.
The complete logical content digest and manifest projection are checked before
the staged tree is committed.

## 6. Digests and exact lock identity

These identities are intentionally not interchangeable:

```text
ArchiveDigest  = sha256(exact .xspkg bytes)
GitCommitOid   = <git-object-format>:<full commit object id>
GitTreeOid     = <git-object-format>:<full tree object id>
ContentDigest  = sha256(canonical logical regular-file tree)
ManifestDigest = sha256(exact xmlsquish.toml bytes)
```

The content preimage is independent of tar encoding, Git object format, file
mode, and physical filesystem. Let files be sorted by their exact UTF-8 logical
path bytes. The encoding is:

```text
UTF8("xmlsquish") || 0x00 || UTF8("package-tree/v1") || 0x00 ||
u64be(file_count) ||
for each file:
    u32be(path_byte_length) || path_utf8 ||
    u64be(content_byte_length) || exact_content_bytes
```

`ContentDigest` is SHA-256 of that preimage and is serialized
`sha256:<64-lowercase-hex>`. Directories are implied by file paths. Empty
directories, modes, mtimes, archive headers, Git commit messages, and origin
URLs do not participate. This is appropriate because xmlsquish consumes
ordinary source bytes and does not execute dependency files. If a future
backend makes an executable bit semantic, it requires `package-tree/v2`, not a
silent reinterpretation.

The next lock schema removes ambiguous `checksum` fields:

```toml
lock-version = 2

[[package]]
id = "common-prompts@1.4.2#..."
name = "common-prompts"
version = "1.4.2"
manifest-digest = "sha256:91ab..."

[package.source]
kind = "registry"
registry-id = "https://packages.example.com/xmlsquish"
archive-format = "xspkg-tar-gzip/1"
archive-digest = "sha256:c4f0..."
content-digest = "sha256:7e21..."
archive-size = 18342

[[package]]
id = "ui@2.1.0#..."
name = "ui"
version = "2.1.0"
manifest-digest = "sha256:..."

[package.source]
kind = "git"
repository = "https://git.example.com/prompts/ui.git"
selector = "tag:v2.1.0" # provenance; execution never follows it
object-format = "sha1"
commit = "0c0990399270277832fbb5b91a1fa118e6f63dba"
root-tree = "8f94139338f9704f26296befa88755fc2598c289"
subdir = "."
package-tree = "8f94139338f9704f26296befa88755fc2598c289"
content-digest = "sha256:..."
```

The package version remains a package-node field for every source kind.
`selector` is diagnostic provenance and is checked against manifest intent when
validating whether a lock is current, but is not dereferenced during a locked
build. The object format applies to commit and both tree IDs. `subdir = "."`
means the root tree and is the sole dot-path exception.

The lock maps directly to the `PackageInstanceId` required by the IR design:

```text
registry canonical_source = "registry:" + registry-id
registry exact_revision   = version + "@" + archive-digest

Git canonical_source      = "git:" + canonical-repository-URL + "#" + subdir
Git exact_revision        = object-format + ":" + commit + "/" + package-tree
```

These strings use the canonical, length-delimited `PackageInstanceId` encoding
when hashed; the displayed separators are not an invitation to parse an
unescaped concatenation. `ContentDigest` remains a revision-verification fact,
not a substitute for package instance identity. Two Git commits with identical
trees therefore retain distinct provenance identities while safely sharing one
physical materialization.

## 7. Portable logical file trees

Registry archive names and Git tree names first enter a source-neutral
`LogicalTreeBuilder`. Validation happens on raw names before the existing
`LogicalPath` constructor can translate a backslash into a separator.

Every member path must:

- be valid UTF-8, relative, non-empty, and slash-separated;
- contain no empty, `.` or `..` segment, NUL, backslash, or control character;
- have bounded segment and total UTF-8 byte lengths;
- contain neither `.git` nor the reserved `.xmlsquish-source.json` marker as a
  segment; and
- avoid Windows device basenames, a colon, and trailing space or dot in every
  segment.

Logical identity remains exact UTF-8 and case-sensitive with no Unicode
normalization, matching `squish-source`. In addition, a package is rejected if
two exact logical paths collide under the portable materialization key:
Unicode NFC, Unicode default case folding, and Windows trailing-dot/space
folding applied segment by segment. The portable key is only a collision test;
it never replaces the original logical path or enters `SourceId`. This slightly
over-rejects some Unix-only trees in exchange for one lock graph that can be
materialized consistently on Windows, macOS, and Linux.

Git modes `100644` and `100755` become ordinary files and contribute identical
content semantics. Mode `040000` is a directory. Symlink mode `120000`, gitlink
mode `160000`, unknown modes, duplicate names, and case/normalization collisions
are rejected in v1. No checkout filter, line-ending conversion, or symlink
resolution occurs.

The materialized package root is a physical locator only. A file such as
`src/main.xml` becomes a `SourceIdentity` using the resolved package instance
and logical path; the CAS directory name is never observable through
`file.uri`, diagnostics' stable identity, IR, or product bytes.

## 8. Git acquisition and ref-to-tree protocol

### 8.1 Shared object database

Each canonical repository URL maps to a private bare object database keyed by
`sha256(canonical URL)`. It has no worktree and is never used as a user
repository. Concurrent fetches serialize ref updates for that database but
readers of already locked objects do not wait for an unrelated network refresh.
Fetched objects are anchored below private `refs/xmlsquish/...` until the lock
and cache index transaction records them; ordinary remote-tracking branch names
are not semantic state.

The implementation may use a Rust Git library or system Git behind the same
adapter. A system-Git implementation invokes commands directly, never through
a shell; supplies `--end-of-options` where supported; disables interactive
prompts; and captures porcelain/plumbing output rather than inheriting terminal
streams. This is an execution contract, not a general Git security policy.

### 8.2 Selector resolution

Online resolution performs one of:

| Intent | Remote name requested | Result |
| --- | --- | --- |
| no selector | symbolic `HEAD`, then its advertised target | peeled commit |
| `branch = B` | exact `refs/heads/B` | commit at branch tip |
| `tag = T` | exact `refs/tags/T`, recursively peeled | commit targeted by lightweight or annotated tag |
| `rev = OID` | exact full object ID | that object, if cached or fetchable |

Names are never passed through Git's ambiguous short-ref disambiguation. The
fetch writes a unique temporary ref, verifies it as a commit, reads the storage
object format, and obtains the full commit and tree IDs. Git documents the
`^{commit}`/`^{tree}` type-peeling operators and full-object verification in
[`git rev-parse`](https://git-scm.com/docs/git-rev-parse.html). A full OID that
the server will not provide as an unadvertised object produces a specific
"revision not fetchable" result; the client does not silently fetch every head
and tag. Supplying a branch/tag that reaches it or pre-populating the cache are
the bounded remedies.

Before remote access, branch/tag names are validated as exactly one legal Git
ref suffix and then prefixed with `refs/heads/` or `refs/tags/`; control bytes,
empty/path-dot components, `..`, `@{`, backslash, a leading/trailing slash, and
other names rejected by Git ref-format rules are invalid manifest intent. This
validation plus direct process arguments prevents a selector from becoming an
option or a revision expression.

The protocol sequence is conceptually:

```text
canonicalize URL and selector
  -> read cached exact candidate if policy permits
  -> ls advertised exact ref / symbolic HEAD when needed
  -> fetch selected objects into a temporary private ref (atomic ref update)
  -> verify <candidate>^{commit}
  -> record object-format + full commit OID
  -> read <commit>^{tree}
  -> resolve explicit subdir to a tree object
  -> walk that tree and blobs into LogicalTreeBuilder
  -> verify manifest + calculate ContentDigest
  -> publish materialization
  -> return GitCandidate
```

Git `fetch --atomic` provides all-or-nothing local ref updates when multiple
refs are involved, while fetched object files themselves are safe to leave
unreferenced for later housekeeping; see [`git fetch`](https://git-scm.com/docs/git-fetch).
The adapter verifies the required commit/tree/blob connectivity before commit.
System Git can use `git fsck --connectivity-only <commit>` for this purpose;
the official [`git fsck` documentation](https://git-scm.com/docs/git-fsck)
states that this checks that reachable referenced objects are present.

### 8.3 Locked and offline Git behavior

If a current lock exists, normal and `--locked` builds request the exact commit
and package tree from cache first. They do not contact the remote merely to see
whether a branch or tag moved. If objects are missing, normal/locked online mode
fetches the exact locked commit without changing the lock. Only an explicit
update or resolution caused by changed intent follows the selector again.

`--offline` may resolve a selector only if the cache has a previously observed
selector-to-commit record and the complete selected tree. Its resulting lock
records that exact observation. `--frozen` requires the current lock, exact Git
objects, verified content digest, and complete materialization. A bare database
containing a commit but missing promised/lazy blobs is not a frozen hit.

## 9. Cache layout and atomic materialization

The cache root is obtained from the platform cache-directory API, with
`XMLSQUISH_CACHE_HOME` as an explicit override. Typical locations are
`$XDG_CACHE_HOME/xmlsquish`, `~/Library/Caches/xmlsquish`, and
`%LOCALAPPDATA%\xmlsquish\cache`; none is a semantic value.

The following is a versioned implementation layout, not a public API:

```text
<cache>/v1/
  state.sqlite3                 # validators, mappings, leases, generations
  blobs/sha256/ab/cd/<full>     # exact archives and other CAS blobs
  sparse/<registry-key>/
    bodies/sha256/ab/<full>     # validated config/index response bodies
  git/db/<repo-key>/            # shared bare object database
  materialized/sha256/ab/<content-digest>/
    .xmlsquish-source.json      # completion marker, written in staging
    root/...
  stage/<process-id>-<nonce>/   # same-volume temporary trees
  quarantine/...               # bounded evidence for diagnosed corruption
```

`registry-key = sha256(registry-id)` and `repo-key = sha256(canonical
repository URL)`. User-controlled names and URLs never become native directory
components. The SQLite mapping connects registry versions and Git
commit/subtree identities to immutable blob/content digests. CAS files and
complete materializations are never modified in place.

Materialization uses this transaction:

1. Look up a complete materialization by `ContentDigest`; validate its marker
   and acquire a read lease. A valid hit returns immediately.
2. On a missing/incomplete entry, acquire a bounded per-content writer lease.
   Routine contention waits with progress events, rechecks the winner's result,
   and does not fail merely because another process is doing the same work.
3. Create a random stage directory under the cache's `stage` directory, so the
   final rename stays on one filesystem.
4. Stream validated ordinary files into `stage/.../root` with create-new
   semantics. Flush files, calculate the tree digest from exact bytes, and
   write a versioned `.xmlsquish-source.json` marker containing the content and
   manifest digests, canonical file path/length/digest records, file count, and
   total bytes. It does **not** contain a registry or Git source identity because
   several exact sources may share the same content-addressed tree.
5. Durably close the marker and best-effort synchronize the stage and parent
   directories where the platform exposes that operation.
6. Atomically rename the stage directory to the digest destination. If a
   concurrent winner already created it, verify the winner and discard this
   stage. Never merge directory contents.
7. In one SQLite transaction, record the source-to-content mapping and lease,
   then emit `MaterializationCommitted`.

The source host does not rely on read-only permission bits as its immutability
boundary. It never opens files in a complete tree for writing. It may mark them
read-only as a diagnostic aid, but GC can still remove an unleased tree. It
does not hard-link cache files into user-writable destinations; reflink/copy is
allowed only when later publication semantics cannot mutate the shared object.

Reuse validates the marker, exact file set, and file bytes against those records
and `ContentDigest`; it does not trust read-only permission bits or timestamps.
The validation can share the byte reads performed by source snapshotting, but a
stale stat cache alone cannot turn modified bytes into a hit. An invalid marker
or digest is a cache fault, not a successful hit. Online mode moves bounded
diagnostic evidence to quarantine and reconstructs the entry. Offline mode
searches other exact local representations (for example the verified archive or
Git objects) and rematerializes without network. Only when no complete local
representation exists does it return `OfflineMiss`.

## 10. Resolution-mode matrix

| Mode | Lock required/current | May update lock | May refresh metadata/ref | May fetch exact locked bytes | Cache-miss result |
| --- | --- | --- | --- | --- | --- |
| normal | no; current lock preferred | yes when intent requires | yes only during resolution/update | yes | network fetch or typed failure |
| `--locked` | yes | no | no ref/version re-resolution | yes | network fetch of exact lock or typed failure |
| `--offline` | no | yes if command normally may | cached observations only | local representations only | `OfflineMiss` with missing identities |
| `--frozen` | yes | no | no | local representations only | `FrozenContentMissing` with all missing identities |

This preserves Cargo's useful offline vocabulary but tightens a common source
of confusion: an existing current lock always wins in offline mode. If no lock
exists, resolving from cached metadata is allowed but emits
`OfflineResolutionUsed { registry_snapshot_digests, git_observations }`; the
new lock makes that exact result explicit. Cargo similarly warns that offline
selection can differ from online selection in its
[`--offline` documentation](https://doc.rust-lang.org/cargo/commands/cargo-fetch.html).

`xmlsquish fetch --locked` is the supported preparation operation. It ensures
all registry archives, Git objects, validated manifests, and complete
materializations referenced by the lock are local. A succeeding invocation is
therefore a positive assertion that `build --frozen` will not miss remote
package content unless the cache is later removed or diagnosed corrupt.

## 11. Observable events

Source adapters emit events to the same `EventSink` as other manager actions.
Workers do not render them. Every event has the NDJSON envelope version,
invocation/plan/action ID, monotonic sequence within its action, source kind,
stable source key, operation, and outcome-specific fields.

| Event | Required fields | Meaning |
| --- | --- | --- |
| `SourceCacheLookupFinished` | `key`, `layer`, `status=hit|miss|invalid`, `elapsed-ms` | A registry body, archive, Git object set, or materialization was checked. |
| `SourceNetworkRequestStarted` | `request-id`, redacted `origin`, `method`, `purpose`, `attempt` | A real HTTP/Git remote operation began. |
| `SourceNetworkRequestFinished` | request ID, status class/Git outcome, received bytes, elapsed, validator-used | Operation ended; HTTP response bodies are not events. |
| `SourceRetryScheduled` | request ID, reason class, attempt, delay-ms | A bounded operational retry will occur. |
| `SparseMetadataRevalidated` | registry ID, package/config, `modified` | A 304 or validated replacement completed. |
| `SourceIntegrityVerified` | source identity, archive/content/commit/tree typed digests, byte/file counts | Required immutable identities matched. |
| `SourceMaterializationStarted` | content digest, source identity | Staging began. |
| `SourceMaterializationWaited` | content digest, elapsed-ms | This action waited for a concurrent writer. |
| `SourceMaterializationCommitted` | content digest, file/byte counts, cache status | A complete immutable tree is available. |
| `SourceCacheFaultRecovered` | layer, expected digest, recovery=`archive|git|network` | Invalid disposable state was replaced automatically. |
| `OfflineResolutionUsed` | snapshot/observation digests | A lock was produced solely from cached mutable metadata. |
| `SourceUnavailable` | stable identity, mode, reason code, attempts | No permitted path can provide the required source. |

Redacted origins contain scheme, host, optional non-default port, and a bounded
path template; they omit user information, query, fragment, credentials, local
home paths, response headers that may contain secrets, and arbitrary Git stderr.
Human diagnostics may attach a sanitized causal summary. Verbose output does
not disable source progress: quiet controls rendering, not event collection.

Metrics aggregate cache hits/misses/invalidations by layer, bytes transferred,
request and materialization latency, retry count, concurrent-wait time, and
offline misses. Package names and full repository URLs are high-cardinality
attributes and are included in trace events, not default metric labels.

## 12. Implementation ports and migration

The resolver remains deterministic and synchronous at its domain boundary; the
scheduler may run independent source actions concurrently. The concrete host
is split into metadata/ref observation and exact package materialization:

```rust
/// Stable logical registry identity; endpoint aliases are configuration only.
pub struct RegistryId(pub CanonicalUri);

/// An exact digest of downloaded archive bytes.
pub struct ArchiveDigest(pub Sha256Digest);

/// A format-independent digest of a validated logical package tree.
pub struct ContentDigest(pub Sha256Digest);

/// A full Git object identifier with its repository object format.
pub struct GitOid {
    pub format: GitObjectFormat, // Sha1 | Sha256
    pub hex: Box<str>,
}

/// Resolver-only projection of a published manifest.
pub struct ResolutionManifest {
    pub package: PackageResolution,
    pub dependencies: BTreeMap<DependencyAlias, RegistryRequirement>,
}

pub struct RegistryCandidate {
    pub version: Version,
    pub yanked: bool,
    pub resolution: ResolutionManifest,
    pub archive: RegistryArchive,
    pub manifest_digest: Sha256Digest,
}

pub struct GitCandidate {
    pub commit: GitOid,
    pub root_tree: GitOid,
    pub package_tree: GitOid,
    pub subdir: LogicalPackagePath,
    pub content_digest: ContentDigest,
    pub resolution: ResolutionManifest,
    pub manifest_digest: Sha256Digest,
}

/// Mutable observations needed by resolution, obeying an explicit access policy.
pub trait RemoteResolutionPort {
    fn registry_candidates(
        &self,
        registry: &RegistryId,
        package: &PackageName,
        access: Access,
    ) -> Result<Vec<RegistryCandidate>, SourceUnavailable>;

    fn resolve_git(
        &self,
        repository: &CanonicalGitUrl,
        selector: &GitSelector,
        subdir: &LogicalPackagePath,
        access: Access,
    ) -> Result<GitCandidate, SourceUnavailable>;
}

/// Produces or reuses a complete immutable tree for an exact locked source.
pub trait PackageMaterializer {
    fn contains_complete(&self, source: &LockedRemoteSource) -> bool;

    fn materialize(
        &self,
        source: &LockedRemoteSource,
        access: Access,
    ) -> Result<MaterializedPackage, SourceUnavailable>;
}

pub struct MaterializedPackage {
    pub content_digest: ContentDigest,
    pub manifest_digest: Sha256Digest,
    pub manifest: Manifest,
    pub root: PhysicalPackageRoot, // adapter-only, never serialized
    pub files: Arc<[LogicalFileRecord]>,
}
```

`squish-resolver` should replace the current `RegistryCandidate { checksum,
manifest }` with the explicit registry fields above, and replace
`GitCandidate { revision, checksum, manifest }` with algorithm-tagged commit and
tree fields. `RegistryPort::contains`/`GitPort::contains` then delegate to
`PackageMaterializer::contains_complete`; a mere archive or commit-object hit is
not enough for frozen readiness. `squish-source` receives only the verified
`MaterializedPackage` and creates `SourceEnvelope`s under the package's logical
instance ID.

`LockedSource::{Registry, Git}` likewise gains typed fields and lock schema 2.
The reader retains lock-v1 support:

- registry v1 `checksum = sha256:...` is interpreted as `archive-digest`; if a
  complete old cache record supplies the content digest it can be upgraded;
- Git v1 `revision` length determines `sha1` versus `sha256`, and its old
  `checksum` is interpreted as the content digest because that is the existing
  resolver contract;
- normal mode may enrich and rewrite an otherwise current v1 lock after exact
  local/remote verification;
- `--locked` never rewrites it and may use it only when the old fields plus
  verified cache state determine every required v2 identity;
- `--frozen` reports a targeted `LegacyLockNeedsFetch` when the missing identity
  cannot be reconstructed locally. It does not silently weaken verification.

This migration preserves readable historical locks without keeping the generic
`checksum` ambiguity in newly written state.

## 13. Acceptance tests

All tests use in-process fake HTTP/Git adapters or repositories under the
repository's `.temp`/`.cache` directories. No test depends on public network
availability.

### 13.1 Sparse registry and resolution

1. **Shard vectors:** names of lengths 1, 2, 3, 4, mixed case, `-`, and `_`
   produce the Cargo shard paths exactly; publication-fold collisions are
   rejected.
2. **Config identity:** an endpoint whose `registry-id` differs from configured
   ID is rejected before package metadata is used.
3. **Conditional refresh:** first 200 stores a validated body and ETag; second
   request sends `If-None-Match`; 304 returns byte-identical candidates.
4. **Validator preference:** when both validators are present, only ETag is used.
5. **Bad replacement survival:** malformed/truncated 200 metadata produces a
   diagnostic but preserves the preceding valid cached body.
6. **Unknown row version:** an unknown `v` row is skipped while v1 rows remain
   usable; an all-unknown package reports unsupported metadata, not not-found.
7. **Yank semantics:** fresh resolution excludes yanked versions; an unchanged
   exact lock continues to materialize one.
8. **Projection verification:** name, version, dependency, dialect, source-root,
   or manifest-digest disagreement between row and archive rejects the selected
   candidate.
9. **Cross-registry edge:** a dependency's registry ID resolves through a
   differently named local alias; endpoint names do not enter the lock graph.

### 13.2 Archives and content identity

10. **Archive byte digest:** one flipped compressed byte cannot enter the CAS as
    the expected archive.
11. **Size and expansion bounds:** short/long transport length, excessive
    expanded bytes, too many files, and excessive per-file size fail while
    leaving no complete marker.
12. **Same tree, different archive:** two valid gzip/tar encodings have different
    archive digests but the same content digest.
13. **Non-files:** symlink, hard link, device, FIFO, sparse member, duplicate
    member, second root, absolute path, and escaping path are rejected.
14. **Exact bytes:** CRLF, LF, UTF-8 BOM, and non-ASCII source bytes materialize
    byte-for-byte; no newline conversion occurs.

### 13.3 Git

15. **Branch movement:** resolve branch to commit A and lock it; move branch to
    B; normal build with current lock still uses A; explicit update selects B.
16. **Annotated/lightweight tags:** both peel to commits and record full commit,
    root-tree, and package-tree OIDs.
17. **Object formats:** SHA-1 and, when supported by the test Git, SHA-256 repos
    round-trip algorithm-tagged full OIDs without assuming 40 hex digits.
18. **Wrong object type:** a `rev` naming a blob/tree instead of a commit is
    rejected before materialization.
19. **Explicit subdir:** root and monorepo subdir resolve to the expected distinct
    package-tree OIDs; missing manifest or subdir is precise.
20. **No checkout transforms:** configured autocrlf and a smudge filter do not
    change materialized blob bytes and the filter is never invoked.
21. **Git special modes:** executable blobs become ordinary semantic files;
    symlinks and gitlinks are rejected with the exact logical path.
22. **Unavailable full rev:** a remote refusing an unadvertised exact OID does
    not trigger a fetch of all refs and reports `RevisionNotFetchable`.

### 13.4 Offline, atomicity, paths, and events

23. **Zero-network offline:** a transport that panics on any DNS/socket/fetch
    call still completes an offline cached build and produces no network event.
24. **Frozen completeness:** cached metadata or archive alone is insufficient;
    a complete locally rematerializable archive/Git tree succeeds, while a
    missing blob is listed in one `FrozenContentMissing` diagnostic.
25. **Fetch guarantee:** after `fetch --locked` succeeds, `build --frozen` with a
    disabled network succeeds for the same lock.
26. **Interrupted stage:** termination before marker/rename leaves the previous
    complete tree visible; the next invocation cleans the stale stage and
    reconstructs without user repair.
27. **Concurrent writers:** N processes materializing one content digest converge
    on one verified directory; losers verify the winner and report wait/reuse.
28. **Cache fault recovery:** modify a cached file/marker; online and locally
    reconstructible offline modes recover automatically and emit exactly one
    fault-recovery fact.
29. **Portable collisions:** case-only, NFC/NFD-equivalent, Windows device,
    trailing-dot, colon, backslash, and reserved-marker paths fail identically
    on Windows and Linux test runners.
30. **Relocation invariance:** moving the cache root changes only physical
    locators; `PackageInstanceId`, `SourceId`, action keys, IR, and final product
    bytes remain identical.
31. **Event redaction:** a URL containing test credentials/query secrets and a
    credential-bearing HTTP exchange yields useful request/retry metrics with
    none of those secret strings in NDJSON or human diagnostics.
32. **Deterministic event facts:** varied response timing/concurrent completion
    changes timestamps and progress ordering only; final source facts and
    report counts remain stably ordered by action/source identity.
33. **Lock-v1 compatibility:** registry and Git v1 fixtures remain readable;
    normal verified migration writes v2, `--locked` does not mutate, and frozen
    reports the precise missing enrichment rather than a generic parse error.

## 14. Failure taxonomy and operational policy

Failures are typed at the source boundary:

```text
ConfigInvalid
RegistryIdentityMismatch
MetadataUnavailable | MetadataInvalid | MetadataUnsupported
AuthenticationUnavailable | AuthenticationRejected
PackageNotFound | VersionUnavailable | YankedByPolicy
ArchiveUnavailable | ArchiveSizeMismatch | ArchiveDigestMismatch
ManifestProjectionMismatch | ContentDigestMismatch
GitRefNotFound | GitRevisionNotFetchable | GitObjectMissing | GitObjectInvalid
LogicalPathInvalid | PortablePathCollision | UnsupportedTreeEntry
OfflineMiss | FrozenContentMissing | LegacyLockNeedsFetch
CacheFaultRecovered | CacheFaultUnrecoverable
AcquisitionLimitExceeded | IoUnavailable
```

Routine cache corruption, stale staging, and concurrent insertion are recovered
inside the adapter when a permitted exact source representation exists. Errors
do not ask users to delete arbitrary cache directories. A diagnostic states the
stable source identity, failed stage, permitted mode, attempted recovery, and
the smallest useful remediation (`xmlsquish fetch --locked`, configure registry
ID X, or make commit Y fetchable). Network unavailability is distinct from a
proved unsatisfiable version graph.

## 15. Deferred choices

The following are explicitly outside version 1 and must not appear as partially
working special cases:

- Git-backed registry indexes, publish APIs, and registry search;
- dependency package signatures or a transparency-log policy;
- Git submodules, symlink materialization, LFS/smudge filters, and arbitrary
  Git revision expressions;
- partial-clone optimization (the cache model allows it later, but frozen
  completeness must remain exact);
- a wire protocol for transferring already materialized trees;
- cross-user/system-wide caches and remote build-result caches; and
- treating the physical cache layout as stable user-facing API.

The extension seams are versioned registry config/row/archive formats,
algorithm-tagged digests/OIDs, `LockedSource` schema versions, and adapter ports.
None requires teaching the XML frontend about HTTP, Git, or native paths.

## 16. Implementation order

1. Introduce typed digest/OID/registry-ID values and lock-v2 parsing plus v1
   compatibility before implementing I/O.
2. Implement and property-test `LogicalTreeBuilder` and content-digest vectors.
   Both archive and Git adapters must use this one path.
3. Implement atomic materialization and cache events with an in-memory mapping,
   then connect the existing SQLite/CAS adapter.
4. Implement sparse `config.json`/JSONL fetching, conditional cache semantics,
   archive verification, and resolver projections.
5. Implement bare Git object acquisition, exact ref peeling, direct blob-tree
   walking, and offline completeness.
6. Wire the concrete adapters at the `xmlsquish` composition root, add
   `fetch --locked`, then execute the acceptance matrix on Windows and Linux.

This order establishes one verified immutable-tree invariant before adding two
transport mechanisms. The special cases disappear at the
`LogicalTreeBuilder -> MaterializedPackage` boundary instead of leaking into
resolution, source loading, or compilation.
