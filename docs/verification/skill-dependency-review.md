# Agent Skills dependency review (2026-09-29)

Scope: uncommitted implementation against `docs/design/skill-dependencies.md`; inspected project schema, resolver, manager, host/fetch, repository transaction/projection, and bundled installer. This is a focused code review, not a full hosted integration run. Findings below record discovered defects and their WIP resolution status.

**Re-review status (2026-09-29): all four P1 findings and the frontmatter concern below have been addressed in the current working tree.** This is source-level verification, not a claim that the hosted CI matrix has passed. The rows remain as a durable record of the failure modes.

## Necessary corrections

| Priority | Finding and executable path | Impact / correction |
| --- | --- | --- |
| P1 | **Package resolution can ratify stale skill pins.** `crates/squish-resolver/src/lib.rs:359-373` copies all `prior_lock.skills` into a newly resolved lock with the new manifest-set digest, without checking that root `[skills]` still has the same names and source locators. If a user edits/removes/adds a skill declaration by hand then runs ordinary package `add`/`remove`, that operation can publish a superficially current lock with stale or missing `[[skill]]` records. `sync-skills` later rejects the mismatch at `crates/squish-manager/src/skill.rs:751-761`. Validate skill declaration-to-pin coherence for every package-resolution path before lock publication; preserve valid pins, but fail truthfully on drift rather than blessing it with a fresh digest. **Confidence: high.** |
| P1 | **Absolute local skill source is accepted in planning but rejected at acquisition.** `crates/squish-manager/src/skill.rs:241-258` canonicalizes an absolute `--path` and produces a manifest-relative locator, but `prepare` passes the *original* request to `resolve_skill_source` (`:179-180`). The host forwards that path (`crates/squish-host/src/lib.rs:1734-1746`) to `read_local_skill_tree`, whose component loop rejects `RootDir`/`Prefix` (`crates/squish-fetch/src/skill.rs:21-43`). Any absolute local path therefore fails, including an explicit safe path outside the workspace promised by the contract. Acquire via the normalized, validated relative locator or support absolute input safely in the fetch adapter; add a process test. **Confidence: high.** |
| P1 | **Sync rechecks the lock but not the manifest before publishing.** `prepare_sync` establishes manifest/lock coherence (`crates/squish-manager/src/skill.rs:745-761`), but `sync_skill` records only `xmlsquish.lock` as an observation (`crates/squish-repository/src/transaction.rs:187-232`). If the manifest changes after the snapshot while the lock bytes remain unchanged, the commit can install a skill no longer declared (or with a different source). Include root manifest in the optimistic read-set and recheck declaration/lock coherence under the writer lock; treat a change as replan/contended. **Confidence: high.** |
| P1 (Windows) | **Bundled installer accepts junction/reparse ancestors.** `crates/squish-manager/src/skill.rs:521-530` rejects only `FileType::is_symlink`; on Windows a directory junction is a reparse point but may report as a directory rather than a symlink. `publish_bundle` (`:582-591`) then creates/writes through `.agents` or `skills` junctions outside the intended home/project. Apply the same reparse-point predicate used by the repository adapter (`crates/squish-repository/src/transaction.rs:1005-1015`) to all installer ancestors and destination, including post-lock rechecks; add a real Windows junction test. **Confidence: high.** |

## Lower-priority contract check

`validate_frontmatter` in `crates/squish-manager/src/skill.rs:317-359` is a line splitter, not a YAML parser. It accepts syntactically invalid or non-string values such as `description: [foo]`, while rejecting valid YAML forms such as an inline comment following a scalar (`name: demo # comment`). Since the command claims to validate Agent Skills frontmatter, use a bounded YAML mapping parser and require string `name`/`description`, or explicitly narrow the supported syntax and diagnostic. This is an interoperability/validation concern, not evidence of destructive behavior. **Confidence: medium.**

### Fix verification in the current WIP

| Original issue | Observed correction | Remaining evidence needed |
| --- | --- | --- |
| Package lock coherence | `validate_skill_lock` now checks root names and exact path/Git source identity against lock entries, and package mutation/build paths call it before and after resolution (`crates/squish-manager/src/skill.rs:1519-1558`, `mutation.rs:310,320`, `build.rs:1193,1202,1305,1322`). | Hosted end-to-end process test for a hand-edited `[skills]` declaration followed by package add/remove/build. |
| Absolute local path | `prepare` resolves the normalized manifest-relative `source_from_spec` instead of the original absolute request (`skill.rs:320-330`). | Hosted process test for an absolute sibling directory on each OS. |
| Sync manifest race | `sync_skill` now accepts the observed root manifest digest, checks exact manifest bytes under writer lock, and includes the manifest in its transaction observations (`crates/squish-repository/src/transaction.rs:169-269`). | Deterministic concurrent-change/fault-injection process test. |
| Windows junctions | Installer `reject_alias` now uses a Windows reparse-point attribute check (`skill.rs:953-985`) and is called before and during publish. | Real Windows junction test in hosted CI. |
| YAML frontmatter | Parser now bounds frontmatter to 64 KiB and deserializes required string fields with `yaml_serde` (`skill.rs:617-674`). | Invalid/non-string and valid inline-comment fixtures in CI. |

## Positive observations and limits

- Projection updates stage the marker with content and journal committed intent; update/removal checks an owned directory digest and refuses edited/unowned content (`crates/squish-repository/src/transaction.rs:349-449, 977-1129, 1163-1210`).
- Git skill acquisition reads Git objects rather than checkout files, pins exact commit IDs, and applies logical-tree file limits (`crates/squish-fetch/src/git.rs:134-222, 385-432`).
- The bundled installer embeds `SKILL.md` at build time and has a separate marker/backup path; this review did not establish crash-safety under every OS failure mode.
- No large local build or CI matrix was run here. Concurrent worker edits may resolve listed items; re-review each changed path before closing.
