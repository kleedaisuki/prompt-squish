# Agent Skills as project dependencies

**Status:** target architecture and implementation contract, 2026-09-29.
**Scope:** `add-skill`, `remove-skill`, project-local `.agents/skills`, and the bundled `install-skill`.
**Related:** [source protocol](dependency-source-protocol.md), [manager ADR](../adr/0009-microkernel-manager-and-reusable-ir.md), [skill prior art](../research/skill-package-manager-prior-art.md).

## Decision in one picture

```text
workspace-root xmlsquish.toml [skills]   human intent; may use a moving Git ref or live path
                | resolve and validate
                v
workspace-root xmlsquish.lock [[skill]]  exact selected source and complete tree digest
                | reconcile verified snapshot
                v
workspace-root .agents/skills/<name>/   agent-visible, owned copy, never a symlink
```

A skill is **not** an XML package. It has no `pkg:` imports, target, semver requirement, registry package, or transitive XML edges. A skill is a bundle whose required `SKILL.md` provides agent instructions and whose optional files may be scripts, references, or assets. The [Agent Skills specification](https://agentskills.io/specification) defines that shape and requires frontmatter `name` to match the containing directory. GitHub Copilot [recognizes `.agents/skills`](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills) as a repository scope. Dependency management here means declared acquisition, exact resolution, auditable provenance, deterministic projection, and safe removal—not treating Markdown as source code for the XML compiler.

The three state planes have different owners. The workspace-root manifest is user-authored intent. The workspace-root lock is machine-managed resolution evidence. `.agents/skills` is a user/agent-visible derived projection, **not** the lock and not a place to store manager journals. The verified cache and journal remain private. This is the same separation that makes Cargo's path/Git dependencies and lockfile useful ([Cargo dependency specification](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)), without pretending that a skill registry exists.

## Data and state model

### Manifest

`[skills]` is owned by the **workspace root** manifest, including a virtual workspace root; a single-package project is its own root. A workspace member cannot declare another independently projected `[skills]` table. This reflects the single agent-discovery namespace at `<workspace-root>/.agents/skills`, avoids collisions between member packages, and makes `-p` inapplicable for skill commands. Project discovery from a member still edits the workspace root. An explicit `--manifest-path` may point to any member, but resolves to that same root.

```toml
[skills]
review-checks = { path = "tools/skills/review-checks" }
release-notes = { git = "https://github.com/example/agent-skills.git", tag = "v2", subdir = "release-notes" }
```

`SkillSpec` is exactly one of:

| Source | Fields | Meaning |
| --- | --- | --- |
| Local directory | `path` | Manifest-relative path to the **exact skill directory**, containing `SKILL.md`; no recursive discovery. |
| Git directory | `git`, optional one of `rev`/`tag`/`branch`, optional `subdir` | Repository and one explicitly selected skill directory; omitted selector means symbolic remote HEAD and omitted `subdir` means repository root. |

No bare-name/default-registry syntax is accepted until a real, skill-specific publication/index protocol exists. The existing `.xspkg` registry expects `xmlsquish.toml` and is not a skill registry. No built-in source variant belongs in this project dependency schema: `install-skill` is a separate bundled self-install operation. The key must satisfy the Agent Skills name grammar (1–64 ASCII lowercase letters, digits, single interior hyphens; no leading/trailing hyphen) and equal the parsed `SKILL.md` frontmatter `name`; it is also the destination basename. No rename alias is offered because directory and frontmatter names are part of the interoperable skill format. `description` must be present, nonempty, at most 1024 characters. Optional standard frontmatter fields remain intact; unsupported optional fields are not silently reinterpreted.

Path intent is relative to the **declaring manifest**, never the process current directory. Validate normalized containment and actual filesystem traversal, including symlinks/reparse points, before reading a local source. A source may be outside the workspace only when an explicit local path names it and traversal is safe; the lock records the relative locator, not an absolute machine-specific path. Local paths are mutable inputs, not immutable packages. Therefore the lock's digest records the last installed snapshot and `--locked` detects drift rather than claiming the path itself is reproducible from a fresh checkout.

### Lock

The existing `xmlsquish.lock` gains sorted `[[skill]]` records, independent of the `[[package]]` graph:

```toml
[[skill]]
name = "review-checks"
digest = "blake3:<normalized-complete-tree-digest>"
source = { kind = "path", path = "tools/skills/review-checks", mutable = true }

[[skill]]
name = "release-notes"
digest = "blake3:<normalized-complete-tree-digest>"
source = { kind = "git", repository = "https://github.com/example/agent-skills.git", tag = "v2", revision = "<full-commit-oid>", subdir = "release-notes" }
```

The exact serialization follows the typed `LockedSkill`/`LockedSkillSource` model. Each record has one unique name, one exact source identity, and an algorithm-tagged digest over the **entire** logical skill tree (paths and bytes, with domain separation), not only `SKILL.md`. A Git record preserves the original HEAD/branch/tag/rev selector for intent matching and pins a full commit object ID; its selected subtree and normalized content are reverified when materialized. Reuse the existing Git object acquisition and logical-tree validation infrastructure, but expose a raw skill-tree operation: the current `GitHost::resolve_exact` package API requires `xmlsquish.toml` and cannot directly acquire a skill. Do not route skills through the XML package resolver or `LockedPackage`.

Previously valid manifest-v1/lock-v1 files with no skills remain readable and behaviorally unchanged. If additive v1 fields are chosen, both new fields must default empty and old lock serialization must remain stable; older binaries may reject the new fields with an explicit unsupported-input diagnostic, not crash. If a v2 schema is chosen instead, the new reader must continue reading v1 and mutation must upgrade deliberately. Either way, **all** package add/remove/build paths that regenerate the lock must preserve and validate skill records. It is a correctness bug if adding an XML dependency silently drops skill pins. The complete manifest-set digest can change after a skill edit; XML dependency versions should nevertheless retain previously valid pins rather than opportunistically update.

### Projection ownership

`<workspace-root>/.agents/skills/<name>` contains an ordinary copied skill tree. It is not a symlink to a mutable source or to an opaque cache. Each owned tree includes a reserved `.xmlsquish-managed` marker carrying product/schema identity, skill name, and installed locked digest. The marker is generated by the repository adapter, is excluded from the source/content digest, and is rejected if supplied by an upstream skill. Keeping marker and content in the same staged directory makes ownership proof co-move with publication and avoids a second ledger journal. Agent discovery still sees a valid `SKILL.md` directory; the marker is private metadata, not an instruction or trust credential.

The manager **never overwrites or deletes an unowned directory**, even if its bytes happen to match, and refuses to remove a managed directory whose current tree differs from its recorded installed digest. Explicit `--force` may be offered later, but cannot silently convert an unowned tree into owned data. Remove leaves unrelated manually installed skills alone. The repository does not silently add `.agents/skills` to `.gitignore`: users may review and commit a projection, while the lock remains the source of exact dependency intent.

## Command contract

| Command | Primary behavior | Important flags / outcomes |
| --- | --- | --- |
| `xmlsquish add-skill NAME --path DIR` | Validate the exact local directory, edit `[skills]`, resolve complete tree, lock digest, and install owned copy. | `--manifest-path`, `--dry-run`, `--locked`, `--offline`, `--frozen`; repeat on same name is an explicit update, never silent adoption of unowned destination. |
| `xmlsquish add-skill NAME --git URL [--rev OID\|--tag TAG\|--branch BRANCH] [--subdir DIR]` | Resolve ref to full commit, read exactly one subtree, validate and materialize. | Existing unchanged locked refs remain pinned. `--offline` may use already observed/cached exact objects; no DNS or remote Git. |
| `xmlsquish remove-skill NAME` | Remove the declared root skill, its lock record, and only its unchanged owned projection. | Same project, dry-run, and resolution flags; missing declaration is a typed error, not a glob deletion. |
| `xmlsquish install-skill` | Install **the executable-bundled xmlsquish guide**, independent of any project manifest/lock. | Default user scope `$HOME/.agents/skills/<bundled-name>` (`%USERPROFILE%` on Windows); `--project` selects workspace `.agents/skills/<bundled-name>`; `--dry-run`; `--force` only for differing **owned** self-install. |
| `xmlsquish sync-skills` | Reconcile every declaration/lock with the project projection after checkout, interrupted publication, or cache loss. | `--locked --offline`/`--frozen` for CI; reports exact missing source when it cannot reproduce. This command or an equivalent explicit reconciliation entry point is needed for a complete dependency lifecycle. |

The checked-in root `SKILL.md` currently declares `name: prompt-squish`. Thus `install-skill` must install to `.../prompt-squish/SKILL.md`, not `.../xmlsquish/SKILL.md`, unless the author intentionally changes that public skill name. Embed the authoring file at build time (`include_str!` or equivalent), so source-installed and released binaries work without a checkout or working-directory assumption. A same-digest installation is idempotent. An existing manual `prompt-squish` directory, or an owned but user-edited one, is a conflict; `--force` replaces only a verified self-owned prior installation. The installer does not run bundled or third-party scripts.

`--dry-run` must perform parsing, source resolution as allowed by offline policy, validation, and collision preflight, then report intended manifest/lock/destination changes without authoritative writes. `--locked` forbids any lock change; `--offline` forbids network; `--frozen` means both. These are the existing manager meanings, not new per-command interpretations. Mutations that would change a lock under `--locked` fail truthfully. A lock-current sync may repair missing derived directories without changing the lock. The JSON/NDJSON event stream needs typed operation kinds and named destination/source/digest fields; human prose is not the machine contract.

## Lifecycle, failure, and concurrency

1. **Discover/observe:** find workspace root and recover earlier repository transactions. Snapshot root manifest, lock, marker, and the exact destination directory state. Explicitly reject a destination tree or ancestor that escapes via symlink/reparse point.
2. **Resolve into immutable candidate:** acquire Git objects or snapshot local files into a validated logical tree with bounded file count, path length, file bytes, and expanded bytes. Parse frontmatter and enforce name identity. Reject nonregular files, symlinks/gitlinks, traversal, portable case-fold collisions, Windows device names, and ambiguous Unicode/normalization paths. Never execute `scripts/` while acquiring or installing.
3. **Preflight:** compare observed authoritative state and projection ownership. Preserve unrelated skills. A local source changed while copying is retried or rejected; bytes published must hash to the candidate digest, not a mixture of two reads.
4. **Commit intent:** use the existing narrow workspace writer lock, optimistic read-set, staged candidates, durable commit decision, and roll-forward journal for manifest plus lock. Extend the transaction or add a sibling recoverable projection journal for directory publication and deletion. Do not hold the writer lock during network fetch or expensive hashing.
5. **Publish/reconcile view:** stage a fully verified directory **with its marker** on the same filesystem, then replace only the named owned destination. If publication fails after intent commit, the command reports incomplete reconciliation and next `sync-skills`/skill command recovers. Never report success while lock and projection disagree. A reader may observe old or new **per-skill** directory, not partial files. Cross-skill atomic view switching is not promised because ordinary `.agents/skills/<name>` directories are the interoperability contract, and portable whole-directory symlinks/junctions would complicate Windows.
6. **Remove:** commit declaration/lock removal, then detach only a matching marked skill directory whose current content digest still equals the marker. Retain the verified source cache opportunistically; `clean` may reclaim cache under its own policy but must not delete skill declarations or manually installed skills.

The lock is authoritative after a durable commit decision; a crash between that decision and projection publication is a **recoverable out-of-sync view**, not permission to roll back a committed manifest. Reconciliation is idempotent and checks the current lock under the workspace lock before each publish. Concurrent add/remove/build processes must not overwrite each other's candidate: stale read-set leads to bounded replan, never last-writer-wins. Existing build/format operations do not execute skill scripts or interpret skill instructions.

## Security and trust boundary

Integrity proves *which bytes* were installed, not whether a third-party instruction bundle is safe. Agent Skills can influence tool use and include runnable scripts. [OpenAI's Skills security guidance](https://developers.openai.com/api/docs/guides/tools-skills#risks-and-safety) recommends inspecting untrusted skills; a [USENIX Security 2026 study](https://www.usenix.org/conference/usenixsecurity26/presentation/liu-yi) reports confirmed malicious skills in a large observed corpus. Accordingly, the CLI exposes source URL/path, exact Git commit, full-tree digest, and destination in dry-run and machine events; installation never grants tools, executes hooks, or claims a hash is a safety certificate. `allowed-tools` frontmatter, if present, is data for the consuming agent and must not become xmlsquish authorization. Local paths are especially mutable: lock verification detects drift, but an intentional update requires review of changed content.

Fetching obeys the existing credential and redirect boundaries from [Dependency Source Protocol](dependency-source-protocol.md). Git acquisition should read objects, not invoke checkout filters or hooks. No URL credentials, query strings, environment secrets, or file content enter diagnostics/events. Shared cache entries are verified before use. On Windows, exercise real reparse-point and case-insensitive collisions rather than assuming Linux test results generalize.

## Module responsibilities and implementation order

| Owner | Responsibility |
| --- | --- |
| `squish-project` | Typed `SkillSpec`, `LockedSkill`, validation, comment-preserving `[skills]` edits, candidate/lock coherence; no filesystem/network. |
| `squish-protocol` | `SkillName`, typed `SkillSource`, `AddSkillRequest`, `RemoveSkillRequest`, `InstallSkillRequest`, operation/result/event schema. |
| `squish-fetch` | Skill-directory logical-tree acquisition from local/Git, limits, source verification, digest, immutable cache; **not** XML-package manifest parsing. |
| `squish-repository` | Workspace-root ownership marker, safe path resolution, staged projection, journal/recovery, narrow writer lock. |
| `squish-manager` | One planning lifecycle for add/remove/sync; source and destination preflight, resolution-mode semantics, coordinator of committed intent and recovered projection. |
| `squish-host` / `squish-cli` | Wire source adapters and command grammar; embed and install bundled self skill; present typed outcomes. |

Implement in dependency order: (1) schema/protocol with round-trip and old-file compatibility; (2) common validated skill tree and raw Git/local acquisition; (3) projection ownership plus crash recovery; (4) manager add/remove/sync lifecycle and preservation of skill locks through existing package resolution; (5) standalone bundled installer; (6) CLI help/docs and process tests; (7) hosted CI matrix. Each slice should have an honest integration test, but the public behavior is designed as one final model rather than temporary user-visible partial modes.

The existing [GitHub Actions CI](../../.github/workflows/ci.yml) already tests Rust on Linux, Windows, and macOS, uses read-only `contents` permissions for PRs, and pins checkout/setup actions by full SHA, consistent with [GitHub's secure-use guidance](https://docs.github.com/en/actions/reference/security/secure-use). Extend it rather than requiring large local build artifacts. Keep test fixtures and generated experiments under repository `.temp`/`.cache`. Hosted process tests must cover local directory with assets, Git tag/branch ref movement with pinned lock, offline/frozen cache misses, old-file compatibility, package add/remove preserving skill locks, unmanaged/user-edited destination refusal, invalid frontmatter and path attacks, no install-time script execution, interrupted publication recovery, same-content idempotence, and true Windows path behavior.

## Explicitly rejected alternatives and remaining checks

* **Reuse `[dependencies]`/`LockedPackage`:** false package identity and version requirements; would let skills leak into XML import resolution.
* **Reuse `.xspkg` registry by accepting bare skill names:** there is no skill registry wire format or publication authority, so this would promise a nonexistent source.
* **Symlink local skill paths:** convenient live updates, but source drift bypasses lock and Windows portability/safe traversal become harder.
* **Copy only `SKILL.md`:** silently drops scripts/assets/references and creates an unusable bundle.
* **Overwrite by name alone:** destructive to manually installed or edited skills; ownership and digest checks are required.
* **Force a whole-set atomic view using links/junctions:** high platform complexity for little benefit; per-skill complete publication and recoverable reconciliation are the useful contract.

The material open check is whether the current repository transaction adapter can journal directory detachment/publish directly. If not, keep manifest/lock as the sole authority and add a small, idempotent projection journal in existing private repository state; the co-located marker removes any need for a second ownership ledger. Validate recovery with failure injection immediately before and after the durable commit decision on all CI operating systems. Do not weaken collision or recovery semantics merely to reuse a file-only transaction API.
