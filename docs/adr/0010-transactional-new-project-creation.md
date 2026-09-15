# ADR 0010: Transactional New-Project Creation

- Status: Accepted
- Date: 2026-09-15
- Extends: [ADR 0009](0009-microkernel-manager-and-reusable-ir.md)
- Product contract: [CLI experience section 3.3](../product/cli-experience.md#33-new)

## Context

The five-command manager accepted in ADR 0009 can operate on an existing
project, but it cannot create one. Treating that omission as an acceptable
boundary was a product error: a project manager that requires a user to
hand-author its initial manifest and source tree does not own the project
lifecycle.

`new` is not a template-copying shortcut and must not become a root-binary
special case. It creates authoritative project state, may edit an enclosing
workspace, applies VCS policy, participates in cancellation and recovery, and
must report through the same protocol, kernel, action, and presentation
contracts as every other manager operation. The complete user-visible syntax,
canonical bytes, workspace rules, VCS policy, stream behavior, and acceptance
matrix are normative in the linked product contract. This ADR fixes the
internal boundaries and transaction semantics needed to implement that
contract.

## Decision

### 1. `new` is a sixth manager operation

The protocol gains a typed `New` request and result, and the action vocabulary
gains a truthful `CreateProject` kind. The manager plans and executes this
operation through the microkernel. It must not disguise project creation as a
manifest-edit transaction, perform filesystem work while parsing arguments, or
bypass lifecycle events in `main`.

The plan has one externally mutating action. Location, package-name inference,
enclosing workspace/VCS discovery, scaffold construction, collision checking,
workspace candidate construction, and revision validation are observable
planning work. The `CreateProject` action owns the complete staged publication
and returns the typed `NewResult` only after all mandatory convergence work has
finished.

The pure scaffold builder owns canonical file contents and order. It consumes a
validated package specification and produces a typed list of relative files;
it does not inspect the filesystem, run Git, edit a workspace, or print. The
repository layer owns observations and recovery. The host supplies filesystem
and Git ports. The presentation layer owns all human, short, quiet, and NDJSON
rendering.

### 2. Existing and prospective operation locations

The root bootstrap must decode the operation before requiring an existing
project. It then constructs one of two typed locations:

```text
OperationLocation
  = ExistingProject { discovery input }
  | ProspectiveProject { destination, nearest existing parent }
```

`build`, `fmt`, `add`, `remove`, and `inspect` use the existing-project route.
`new` uses the prospective route: it can load user, environment, CLI, and any
unambiguous enclosing-workspace configuration, but it must not attempt to open
the absent destination manifest, derive a destination project storage
namespace, or eagerly open its CAS. Workspace and VCS discovery remain manager
services so the root does not acquire a second policy implementation.

Both routes reconverge before dispatch into the same composed host, kernel,
cancellation scope, renderer, and process-exit reduction. This split removes
the former unconditional `ProjectRepository::discover -> load_config ->
Cas::open` assumption without adding a root-side creation bypass.

### 3. Authoritative observations and validation

Planning records, at minimum:

- the lossless native destination and its nearest existing parent;
- the user's lexical spelling for reporting and the ordinarily resolved ancestor
  identity used for workspace/VCS ownership checks;
- the inferred or explicit validated package name;
- the identity and revision of an enclosing workspace manifest, if any;
- whether membership is already effective, needs one append-only edit, or is
  forbidden by exclusion, duplicate identity, or ambiguous nesting;
- the effective VCS disposition: create Git, reuse enclosing Git, or disabled;
- absence of every final destination component that must not be overwritten;
- the exact canonical scaffold and workspace-manifest candidate bytes.

An explicit invalid `--name` is a CLI usage error. Failure to infer a valid name
from a native path is a domain error with `--name` guidance. Native paths remain
lossless protocol values; lossy display strings are never used for identity or
filesystem access.

The final destination must not exist as a directory, empty directory, file,
symbolic link, junction, or other filesystem object. No `--force`, merge, or
adoption path is part of `new`. A later `init` operation, if accepted, requires
a separate contract.

### 4. Staging and missing parents

The repository transaction creates the missing ancestor chain needed to reach
the destination parent and records exactly which directories it created. It
then creates a uniquely named staging directory beside the final destination.
All public scaffold files, directories, and optional Git metadata are completed
inside staging. File bytes are flushed before the staging directory is eligible
for publication.

Before the commit boundary, any ordinary error or cooperative cancellation:

1. removes the manager-owned staging tree;
2. leaves the workspace manifest unchanged;
3. removes recorded ancestor directories from deepest to shallowest, but only
   while each is still empty and still denotes the object created by this
   invocation.

The cleanup rule deliberately tolerates concurrent useful content: a created
ancestor that another actor has populated is retained rather than recursively
deleted. The final destination is never used as staging, so users cannot observe
a half-written manifest/source pair.

Publication uses a repository port with **no-replace directory rename**
semantics. A check followed by an overwriting rename is forbidden. The atomic
winner of the destination name establishes the transaction's linearization
point; losing creators fail without modifying the winner.

### 5. Workspace membership is one recoverable transaction

Standalone creation linearizes at the successful no-replace publication of the
complete staged directory. Creation that needs a workspace edit uses a durable
journal located in manager-owned state associated with the enclosing workspace,
not inside the not-yet-published child.

The state machine is:

```text
Observed
  -> Prepared(journal + staged child + staged workspace candidate)
  -> ChildPublished(commit decision / linearization point)
  -> WorkspacePublished
  -> Complete(journal retired)
```

Before `Prepared`, the repository validates the observed workspace revision and
membership decision. Manager writers serialize the short publication window
with the existing workspace mutation lock. If the revision changed before the
linearization point, the manager discards the candidate and performs a bounded
re-observation/re-plan. It never replaces a manifest derived from a stale
revision.

The durable prepared record contains enough identity and digest information to
distinguish four states after a crash: neither resource published, only the
expected child published, both expected resources published, or a conflicting
external object/revision. A successful no-replace child publication is the
no-return commit decision. From that point the transaction rolls forward: it
publishes the validated workspace candidate, synchronizes the containing
directories, and retires the journal. Recovery is idempotent and verifies
identity/digests before each step; it never overwrites an unrelated destination
or workspace revision.

Every ordinary manager invocation runs new-project journal recovery before it
uses the affected workspace. A prepared journal with staging still present and
no published child is old state and may be rolled back. A journal whose exact
child has been published is committed state and must roll forward to effective
workspace membership. Thus process death may leave recoverable intermediate
filesystem evidence, but no successful or cooperatively cancelled process
returns with a member pointing at a missing child or a committed child missing
its required membership.

Already-effective membership needs no workspace mutation and follows the
standalone publication path. The manager preserves unrelated TOML text and
comments when it appends a member. Exclusion, duplicate package identity,
outside-root placement, and ambiguous/nested workspace relationships fail
before publication.

### 6. VCS policy is an injected effect

Git discovery and initialization are host ports, not shell calls embedded in
the scaffold builder or manager policy. The effective choice is the explicit
`--vcs` value, then typed `new.vcs` configuration, then `git`. With effective
Git:

- an enclosing Git work tree is reused, `.gitignore` is generated, and no
  nested repository is created;
- otherwise Git is initialized inside the staged project, before publication;
- no files are staged and no commit is created;
- Git failure aborts the pre-commit transaction and reports `--vcs=none` as the
  explicit recovery option.

`--vcs=none` creates neither `.git` nor `.gitignore`. The public result lists
only scaffold files and the typed VCS disposition; implementation-specific
`.git` contents are not public project files.

### 7. Cancellation and truthful terminal state

Checking a shared token after an effect has committed is insufficient: it can
misreport a successfully created project as if no effect occurred. Dispatch and
work completion therefore carry a typed disposition, conceptually:

```text
DispatchOutcome<T>
  = Completed(T)
  | Cancelled { committed: None | Some(T) }
  | Failed(...)
```

Before the no-replace child publication, first-interrupt cancellation rolls
back and exits `130` with no committed result. Once child publication succeeds,
cooperative cancellation is deferred while mandatory workspace convergence and
journal finalization run. The terminal lifecycle then reports cancellation and
the committed `NewResult`; it must not manufacture an action failure or claim
that the destination was absent. A second interrupt may terminate bounded
cleanup, but the durable journal remains sufficient for automatic recovery on
the next ordinary invocation.

The renderer observes terminal disposition from the kernel/manager lifecycle;
it does not poll domain state or infer commitment from path existence. Human
status remains on stderr, native v2 NDJSON remains on stdout, and a normal
successful JSON stream ends in the existing operation/job terminal sequence.

### 8. Acceptance and release boundary

This ADR is implemented only when evidence covers the normative product matrix,
not merely when a template unit test passes. Required evidence classes are:

| Boundary | Minimum evidence |
|---|---|
| CLI/protocol | parser classifications, native path round trips including Unix non-UTF-8, request/result/event round trips, help and exit classes |
| Scaffold | golden canonical bytes, deterministic ordering, explicit/inferred-name behavior, `fmt --check`, offline `.prompt` build |
| Filesystem transaction | existing-object collisions, no-replace concurrent creators, deep missing parents, injected failures at every write/sync/rename boundary, ownership-safe cleanup |
| Workspace transaction | effective/no-op member, append preserving unrelated TOML, exclusion/duplicate/ambiguity failures, bounded revision re-plan, recovery at every journal phase |
| VCS | create, reuse, disabled, and initialization failure through the composed host |
| Cancellation/process death | first interruption on both sides of the linearization point, emergency interruption, real child death, automatic ordinary-invocation recovery, truthful committed disposition |
| Presentation | human/short/quiet/native NDJSON stream ownership, no ANSI in JSON, complete typed result |
| Portability | the same composed-binary workflow on the supported Linux, macOS, and Windows jobs at the exact release snapshot |

Until those rows have executable and cross-platform evidence, `new` is
**specified**, not accepted. The previous five-command acceptance record remains
valid for the snapshot it tested but cannot be cited as evidence for this sixth
operation.

## Consequences

- Project creation becomes a normal manager operation with one policy owner and
  one observable lifecycle.
- Prospective locations remove an invalid bootstrap assumption without weakening
  existing-project validation.
- A multi-resource filesystem update needs a small durable state machine because
  no portable atomic primitive can publish both a directory and a separate
  manifest in one step.
- Missing-parent support adds cleanup bookkeeping, but the ownership/emptiness
  rule avoids destructive rollback special cases.
- No-replace publication, revision validation, and recovery are repository
  invariants rather than best-effort command conventions.
- Post-commit cancellation is more explicit: the process may exit `130` while
  also truthfully reporting that creation committed.

## Rejected alternatives

### Implement `new` directly in `main`

Rejected. It would bypass scheduling, lifecycle events, cancellation,
presentation, host injection, and recovery, recreating the compiler-era
monolith at the first lifecycle command.

### Create the final directory and fill it incrementally

Rejected. Failure and cancellation would expose partial userspace state, and
each file would require compensating special cases.

### Overwrite or merge an existing empty directory

Rejected. Existence is the collision boundary. Adoption has different safety
and compatibility semantics and belongs to a possible future `init` command.

### Edit the workspace before publishing the child

Rejected. It can expose a member that points at a missing project. Publishing
the child establishes the recoverable roll-forward decision.

### Treat child and workspace writes as unrelated successful operations

Rejected. It can return a package that the enclosing workspace cannot select,
violating the product purpose of automatic placement.

### Re-check the cancellation token after commit and discard the result

Rejected. It confuses request cancellation with effect rollback and makes the
event stream lie about userspace.
