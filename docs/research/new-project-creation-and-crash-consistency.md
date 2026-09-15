# Evidence for `xmlsquish new` and recoverable project creation

Status: active design evidence; the normative product contract remains
[`cli-experience.md` Section 3.3](../product/cli-experience.md#33-new)
Date: 2026-09-15

## 1. Decision question and scope

`xmlsquish new` must create an immediately formattable and offline-buildable package at an absent
destination. When that destination belongs to an enclosing workspace, it may also have to change
the workspace manifest. This note asks two separate questions:

1. Which creation, workspace, and VCS behaviors have survived in production project managers?
2. What persistence protocol can honestly implement one **recoverable logical transaction** when
   no common cross-platform API atomically installs a directory and replaces another file?

The research covers Cargo, npm, Go, POSIX/Linux/macOS/Windows filesystem interfaces, Git and
SQLite, plus peer-reviewed crash-consistency work. It does not propose a registry/template
execution model and is not a security review.

### Evidence labels

| Label | Meaning |
| --- | --- |
| **Documented behavior** | A first-party user-visible contract. |
| **Upstream implementation** | Current implementation evidence; useful but not necessarily a stable public guarantee. |
| **Production pattern** | A mechanism used by a mature deployed system. Transfer still requires checking assumptions. |
| **Peer-reviewed evidence** | An evaluated research result under stated platforms/workloads. It is not automatically production-ready. |
| **Project inference** | A design judgment for `xmlsquish`, not a fact asserted by the cited source. |

## 2. Project-creation precedents

### 2.1 Comparison

| System | Observed creation semantics | Workspace behavior | What `xmlsquish` should adopt |
| --- | --- | --- | --- |
| Cargo | `new PATH` creates a manifest, sample source, and VCS policy at a new directory; `init` is the separate existing-directory operation; the package name defaults to the directory name. | Creating below a workspace automatically adds the package to `workspace.members`. | Strict absent-destination `new`, inferred name with explicit override, canonical buildable scaffold, automatic membership. |
| npm | Legacy `init` can be interactive and additive in an existing package; `init <initializer>` downloads/runs a `create-*` package. | `npm init -w DIR` creates child boilerplate and updates the root `workspaces` field. | Workspace coherence is precedent; interactive questions and ambient third-party initializer execution are not suitable for a deterministic core command. |
| Go | `go mod init` writes a new `go.mod` in the current existing directory. It does not create a complete source tree or VCS. | `go work use` is a separate membership-editing command. | Explicit identity and small manifests are useful; the split lifecycle does not meet the one-command buildable-project goal. |

### 2.2 Cargo is the closest lifecycle contract, not a transaction blueprint

**Documented behavior.** Cargo explicitly separates [`cargo new`](https://doc.rust-lang.org/cargo/commands/cargo-new.html),
which creates a package in a new directory, from
[`cargo init`](https://doc.rust-lang.org/cargo/commands/cargo-init.html), which initializes an
existing directory. `new` rejects an existing destination, defaults the package name from the
directory leaf, and supports a name override. Its VCS default is Git/configured VCS when outside a
repository and no new repository when already inside one. The Rust book also documents that
[`cargo new` automatically registers a package created inside a workspace](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html).

**Upstream implementation.** The current
[`cargo_new.rs`](https://docs.rs/cargo/latest/src/cargo/ops/cargo_new.rs.html) rejects every existing
destination and creates missing source parents. It edits TOML structurally, recognizes member
globs, avoids duplicate membership, and preserves an already-sorted member array. However, its
observable write order initializes the destination/VCS, atomically rewrites the root manifest, and
then writes the child manifest and sources. The root rewrite is atomic as a *single-file update*;
the complete root-plus-child operation is not one crash-recoverable transaction.

**Project inference.** `xmlsquish` should copy Cargo's lifecycle boundary but deliberately exceed
its persistence guarantee. A future operation that adopts populated directories should be named
`init`; adding `--force` to `new` would mix two ownership models and multiply destructive cases.

There is also one intentional VCS difference. Cargo selects `none` inside an existing VCS and does
not emit a child ignore file in that path. The `xmlsquish` contract instead treats `--vcs git` as
"Git is effective": reuse an enclosing Git worktree without a nested `.git`, but still create the
package-local `.gitignore`. This gives the package the same ignore behavior whether its Git
ownership is local or inherited.

### 2.3 npm and Go expose the alternative boundaries

**Documented behavior.** The npm documentation says
[`npm init -w DIR`](https://docs.npmjs.com/cli/v11/commands/npm-init/) creates the child directory
and package boilerplate while adding a reference to the root `workspaces` array. The same command
family can instead ask questions or fetch and execute a versioned `create-*` initializer.

**Project inference.** npm is strong evidence that users expect workspace creation and membership
to remain coherent. It is weak evidence for how to achieve crash consistency: the documentation
makes no atomicity or recovery guarantee. Remote initializers would also make identical
`xmlsquish new` arguments depend on registry state and third-party execution, so they should remain
outside the deterministic creation contract.

**Documented behavior.** Go defines
[`go mod init`](https://go.dev/ref/mod#go-mod-init) as creating only `go.mod` in the current module
directory. Workspace registration is independently performed by
[`go work use`](https://go.dev/ref/mod#go-work-use); missing directories are removed from the
workspace on reconciliation.

**Project inference.** Internally, scaffold generation and workspace editing should remain two
typed steps even though `xmlsquish new` composes them into one user operation. That separation
makes validation and recovery inspectable without forcing the user to repair intermediate state.

## 3. What filesystem primitives do—and do not—guarantee

| Primitive/evidence | Observed guarantee | Limitation relevant to `new` |
| --- | --- | --- |
| POSIX `rename` | Rename is atomic as a namespace operation. | It covers one rename, not a child-directory install plus an unrelated manifest replacement; atomic visibility is not durable persistence. |
| Linux `renameat2(RENAME_NOREPLACE)` | Installs without overwriting an existing destination, or returns `EEXIST`. | Linux-specific and dependent on filesystem support. |
| macOS `renamex_np(RENAME_EXCL)` | Exclusive rename is available on volumes that advertise support. | Non-portable and volume-dependent. |
| Windows `MoveFile`/`MoveFileEx` | Moves a directory on the same volume; replacement/write-through behavior is controlled separately. | A directory cannot be moved across volumes; `MOVEFILE_WRITE_THROUGH` is documented specifically for copy/delete moves and is not a general multi-object transaction. |
| `fsync(file)` on Linux | Flushes the file's data and associated metadata. | The directory entry is not thereby durable; the containing directory needs a separate sync. |

Sources: [POSIX `rename`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html),
[Linux `renameat2`](https://man7.org/linux/man-pages/man2/renameat2.2.html),
[Apple exclusive rename capability](https://developer.apple.com/documentation/foundation/urlresourcevalues/volumesupportsexclusiverenaming),
[Windows `MoveFile`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefile),
[Windows `MoveFileEx`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexa),
and [Linux `fsync`](https://man7.org/linux/man-pages/man2/fsync.2.html).

**Project inference.** The staged tree must be on the destination's filesystem (normally a sibling
under the nearest existing parent). Each platform should use its strongest no-replace installation
primitive where available. A portable fallback still needs exclusive writer coordination and a
final destination revalidation; a check-then-replacing-rename sequence is not equivalent and can
overwrite a concurrent winner.

The term **atomic** must therefore be qualified:

- *atomic namespace visibility* means a reader sees an old or new name around one rename;
- *durability* means completed data and namespace updates survive power loss;
- *logical transaction* means recovery converges several filesystem objects to one declared state.

Only the third phrase accurately describes child creation plus workspace registration.

## 4. Production-grade persistence patterns

### 4.1 Git: exclusive writer plus atomic single-file publication

Git's documented [lockfile API](https://git-scm.com/docs/api-lockfile) creates `filename.lock` with
`O_CREAT|O_EXCL`, writes the replacement there, and renames it to the final name at commit. Other
writers fail to acquire the lock; readers see either old or new content if rename is atomic. Git
also registers process-exit/signal cleanup for uncommitted locks.

**Adoption judgment: ready for the workspace-manifest sub-operation.** Use an exclusive lock and
compare the root manifest revision after locking, then structurally rewrite a staged file and
atomically replace the root manifest. Process cleanup is only convenience: abrupt termination and
power loss still require a durable journal and ordinary-invocation recovery.

### 4.2 SQLite: journal first, explicit decision, automatic recovery

SQLite's [atomic commit protocol](https://www.sqlite.org/atomiccommit.html) demonstrates the more
important multi-object pattern. It flushes rollback information before changing authoritative
data, detects a "hot journal" after interruption, serializes recovery with a lock, and restores a
coherent prior state automatically. For a multi-database transaction it creates and syncs a
super-journal naming every participant; deleting that durable marker is the commit point. SQLite
also syncs the containing directory so the marker itself survives a crash, and it tests simulated
incomplete, reordered, and garbage writes.

**Adoption judgment: use the protocol shape, not SQLite's page format.** `xmlsquish new` needs a
small versioned journal that names the destination, staged child, workspace manifest (if any),
expected old workspace digest, proposed new digest, created ancestors, and phase. A persisted
commit decision must make recovery unambiguous. The next ordinary manager invocation should
acquire the same writer coordination and roll the decided operation forward; it must not delegate
repair to the user.

## 5. Academic evidence and readiness

| Work | What was observed | Implication | Readiness for this project |
| --- | --- | --- | --- |
| Pillai et al., OSDI 2014, [*All File Systems Are Not Created Equal*](https://www.usenix.org/conference/osdi14/technical-sessions/presentation/pillai) | Application update protocols depended on subtle persistence properties that varied across six Linux filesystems. | Do not infer crash durability from POSIX namespace semantics or one development filesystem. | **Directly actionable:** keep assumptions explicit and test several filesystem/platform families. |
| Mohan et al., OSDI 2018, [bounded black-box crash testing](https://www.usenix.org/conference/osdi18/presentation/mohan) | Small generated workloads reproduced most studied historical bugs; the tools also found new bugs in mature filesystems. | Crash points around persistence operations deserve systematic enumeration, not only happy-path tests. | **Directly actionable:** inject termination/faults at every durable boundary and verify recovery invariants. |
| Hu et al., USENIX ATC 2018, [TxFS](https://www.usenix.org/conference/atc18/presentation/hu) | An ext4-journal extension exported ACID filesystem transactions and was evaluated with SQLite and Git. | Native filesystem transactions could eliminate user-space protocols. | **Not deployable here:** kernel/filesystem-specific and unavailable as a common Linux/macOS/Windows interface. |
| LeBlanc et al., OSDI 2024, [SquirrelFS](https://www.usenix.org/conference/osdi24/presentation/leblanc) | Rust typestate encoded required persistent-update order for a persistent-memory filesystem and checked it at compile time. | Phase-invalid calls, especially cancellation/rollback after a durable decision, can be made unrepresentable. | **Selective adoption:** use internal phase types where they simplify the state machine; do not import its PM-specific filesystem design. |
| Pan et al., OSDI 2025, [WOLVES/WOFS](https://www.usenix.org/conference/osdi25/presentation/pan) | A prototype writes checksum-protected metadata packages with one ordering point for synchronous PM crash consistency. | Research continues to move ordering and recovery into specialized filesystems. | **Watch, do not depend:** recent prototype for persistent memory, not a portable application API. |

The strongest competing explanation is that the generated tree is tiny, so ordinary sequential
writes are sufficient. That argument addresses probability and implementation cost, not the
accepted contract: one crash between workspace registration and child completion produces an
invalid workspace, and one concurrent destination race can overwrite another creator if the final
install is not exclusive. Cargo's own sequential implementation shows that this window is real at
the operation-model level even if failures are uncommon.

## 6. Recommended recoverable state machine

This is a project inference synthesized from the evidence, not a guarantee made by Cargo, SQLite,
or the operating-system specifications.

The central recovery invariant is one-way:

```text
workspace contains the new member  ==>  destination is a complete project from this transaction
```

A complete standalone child may exist briefly after the decision and before roll-forward finishes;
the inverse implication is intentionally not required. This makes the unavoidable intermediate
state non-destructive and prevents a workspace manifest from pointing at missing content.

```text
VALIDATE
  destination absent; name/scaffold valid; workspace relation unambiguous
  snapshot workspace bytes/revision; determine effective Git ownership
      |
      v
PREPARE
  create only tracked missing ancestors
  build complete child in same-filesystem staging directory
  initialize Git in staging only when no enclosing Git owns the destination
  prepare workspace-manifest candidate
  flush staged files/directories as supported
      |
      v
DECIDE
  hold exclusive destination/workspace writer coordination
  revalidate destination absence and workspace revision
  persist and sync journal with commit decision
  cancellation after here cannot convert the operation back to "not committed"
      |
      v
PUBLISH / RECOVER FORWARD
  install child with no-replace semantics
  atomically replace workspace manifest if required
  sync affected parent directories where supported
  mark complete, then remove staging/journal debris
```

### Why child-first after the decision

No portable single syscall covers the child directory and workspace manifest. If a crash occurs
between the two publications, child-first temporarily leaves a complete standalone child, whereas
manifest-first leaves a dangling workspace member pointing at missing/incomplete content. The
durable decision tells recovery to finish workspace registration. This ordering minimizes the
severity of the unavoidable physical intermediate state; it does **not** make the two updates
physically atomic.

### Concurrency rules

1. Destination installation is no-replace. A concurrent winner is never overwritten.
2. Workspace mutation is based on an expected old digest/revision while holding the writer lock.
3. A revision conflict before decision triggers a bounded re-plan and full semantic revalidation
   (membership glob, exclusion, duplicate package identity), not blind textual replay.
4. A conflict after the durable decision is a recovery concern. Writer coordination should make
   that state unreachable during correct operation; recovery must still validate before replacing.
5. Cleanup removes only staging objects and still-empty ancestors recorded as created by this
   invocation. Pre-existing parents are never recursively removed.

## 7. Evidence required before claiming the contract complete

| Claim | Minimum executable evidence |
| --- | --- |
| Strict ownership | Existing directory, file, symlink/junction, and a concurrent path winner are never merged or overwritten. |
| Complete scaffold | Exact tree/content golden tests; immediate `fmt --check`; immediate offline build on Linux, macOS, and Windows. |
| VCS coherence | Standalone default Git, enclosing Git reuse without nested `.git`, and `--vcs none`, including injected Git failure before decision. |
| Workspace coherence | Already-covered glob, explicit member, exclusion, duplicate identity, nested/ambiguous root, comment-preserving append, and concurrent root revision. |
| Pre-decision rollback | Fault/cancellation at each preparation boundary leaves no visible destination/root mutation and removes only recorded empty ancestors. |
| Post-decision recovery | Real process death after journal sync, child install, workspace replacement, and completion marking; next ordinary invocation converges automatically. |
| Durability assumptions | Platform adapters document their no-replace, file flush, and directory flush behavior and surface unsupported guarantees rather than silently claiming them. |
| Machine contract | Typed JSON events/result describe created files, VCS disposition, workspace disposition, and whether the durable decision committed. |

### What would materially revise this recommendation?

- A supported, production-quality, cross-platform API providing durable multi-object filesystem
  transactions would justify replacing the user-space journal.
- Evidence that required platforms cannot provide reliable no-replace directory installation or
  usable directory durability would require narrowing the power-loss guarantee explicitly rather
  than pretending equivalence.
- Fault-injection measurements showing the state machine adds unacceptable latency could justify
  group commit or fewer sync points, but only after preserving the same recovery invariant.
- Product evidence that automatic workspace membership surprises users more than it helps them
  could favor Go's explicit `work use` boundary. Cargo and npm currently provide the stronger
  production precedent for automatic membership.
