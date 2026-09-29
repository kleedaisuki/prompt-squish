---
name: prompt-squish
description: Author, configure, build, format, inspect, and clean xmlsquish prompt projects; manage project Agent Skills. Use when working with the XML macro DSL, xmlsquish.toml packages/workspaces/dependencies/skills/targets, .agents/skills, or the xmlsquish CLI.
---

# xmlsquish Agent Guide

Use the checked-in manifest and XML sources as truth. Do not infer entries from filenames or reach into dependency directories: targets and exports are explicit.

## Common workflow

```bash
xmlsquish new my-prompts --vcs none
cd my-prompts
xmlsquish fmt --check                 # 检查；去掉 --check 即改写 / Check; omit --check to rewrite
xmlsquish build --offline -t prompt   # 脚手架目标，仅使用本地已验证缓存 / Scaffold target, verified local cache only
xmlsquish inspect artifact target/xmlsquish/artifacts/prompt.prompt
xmlsquish clean                       # remove all project-local products, cache, and build metadata
```

Project commands discover `xmlsquish.toml` upward; use `--manifest-path PATH` to select one. `clean` atomically detaches and deletes the complete project/workspace build root, including products, project-local caches, and compilation metadata; a later build reconstructs it from project inputs. Use `--locked` to forbid lockfile changes, `--offline` to forbid network access, or `--frozen` for both. In automation prefer exit codes and `--message-format=json` (NDJSON), not human text. Run `xmlsquish <command> --help` before using less common/version-specific flags.

Dependency edits are transactional and never rewrite XML imports:

```bash
xmlsquish add common --path ../common
xmlsquish add toolkit --git https://example.com/toolkit.git --tag v1.2.0
xmlsquish add prompt-common@^2 --registry community --rename common
xmlsquish remove common --dry-run
```

Agent Skills are a separate workspace-root dependency domain (available since
xmlsquish v1.1.0). Add one exact local skill directory or Git repository/subtree;
do not use XML package registry syntax or `-p` for skills. A path is resolved
relative to the root manifest, not the shell's current directory. Restoring a
checkout requires reconciling the agent-visible copies from the shared lock:

```bash
xmlsquish add-skill review-checks --path tools/skills/review-checks
xmlsquish add-skill release-notes --git https://example.com/skills.git --tag v2 --subdir release-notes
xmlsquish sync-skills --frozen          # equivalent to --locked --offline
xmlsquish remove-skill review-checks --dry-run
xmlsquish install-skill                  # user-global .agents/skills/prompt-squish
xmlsquish install-skill --project        # current workspace .agents/skills/prompt-squish
```

`add-skill` and `remove-skill` edit the root manifest and lock together, then
publish/remove only the corresponding owned `.agents/skills/<name>` copy.
Repeating `add-skill` for the same name explicitly updates it. Git sources may
select one `--rev`, `--tag`, or `--branch`; without one, they follow remote HEAD
when deliberately updated. The lock pins the exact commit and whole-tree digest,
while local sources remain mutable: `sync-skills --locked` rejects source drift
rather than silently changing the lock. `sync-skills` repairs missing copies and
prunes stale **owned** copies without changing the manifest or lock. Use
`--dry-run` to inspect planned changes, `--offline` to prohibit network access,
and `--locked` to prohibit lockfile changes.

`install-skill` is distinct: the executable embeds this guide and installs it
under its public name `prompt-squish`, without editing any project manifest or
lock. `--project` selects the current workspace; `--manifest-path PATH` is
available with `--project`. An identical bundled installation is idempotent;
`--force` may replace only an unmodified prior bundled installation, never an
unmanaged or user-edited directory. Skill names must match the `SKILL.md`
frontmatter and destination directory. xmlsquish copies the complete tree, does
not execute scripts, and refuses to overwrite unmanaged or locally modified
skill directories. Verify third-party content before enabling it: a lock digest
is integrity evidence, not a trust or safety certificate.

## `xmlsquish.toml` quick reference

```toml
manifest-version = 1

[workspace]                         # optional; may be a virtual root
members = ["packages/*"]
exclude = ["packages/old"]
target-dir = "target/xmlsquish"     # shared build-state root; this is the default

[workspace.dependencies]
common = { path = "packages/common" }

[package]
name = "agent-prompts"
version = "1.1.0"
dialect = "xmlsquish/1"             # optional default
source-root = "src"                 # optional default

[dependencies]                      # each dependency selects exactly one source
common = { workspace = true }
local = { path = "../local", package = "real-name" }
kit = { git = "https://example.com/kit.git", tag = "v2" } # tag|branch|rev
base = "^2"                         # default registry shorthand
corp = { version = "~1.4", registry = "corp", default-features = false, features = ["chat"], optional = true }

[skills]                            # workspace root only; separate from XML dependencies
review-checks = { path = "tools/skills/review-checks" }
release-notes = { git = "https://example.com/skills.git", tag = "v2", subdir = "release-notes" }

[exports]                            # public modules for pkg: imports
macros = "src/macros.xml"

[target.chat]
entry = "src/chat.xml"              # xs:entry link root, relative to manifest
backend = "squish"                  # optional default
output = "chat.prompt"              # optional; default is chat.prompt
features = ["trace"]

[target.chat.args]
name = "Klee"

[target.chat.limits]
max-depth = 128
max-expansions = 10000
max-output-bytes = 1048576

[profile.base]
debug-info = false

[profile.release]
inherits = "base"                  # optional, acyclic
debug-info = true
optimization = "size"
args = { locale = "zh-CN" }
features = ["release"]
```

Unknown keys are errors. XML package names use ASCII letters, digits, `-`, or `_`; Agent Skill names instead use lowercase ASCII letters, digits, and interior hyphens, with a maximum of 64 characters. Paths are manifest-relative and may not escape their owning roots; outputs must end in `.prompt`. A dependency may additionally use `package`, `optional`, `features`, and `default-features`. Consumers import only declared exports of direct dependencies:

```xml
<xs:import src="pkg:common/macros"/>
```

Published locators are stable, readable paths below `workspace.target-dir/artifacts` (for example `target/xmlsquish/artifacts/chat.prompt`). Treat the returned locator as one complete project-relative path. Never depend on manager-private hashes, generations, journals, or storage paths.

The complete project-local derived layout is `<target-dir>/{artifacts,cache,metadata,work}`. `cache` contains CAS, the action index, and dependency sources; `metadata` contains rebuildable publication/catalog evidence; `work` is non-authoritative staging. There is no machine-global build cache. Since v1.0.4, xmlsquish does not import the old layout.

Operational configuration is separate from the package manifest. Put it in `$XMLSQUISH_HOME/config.toml` or workspace `.xmlsquish/config.toml`; environment and repeated `--config KEY=TOML_VALUE` override files.

```toml
[build]
jobs = 0
keep-going = true
[new]
vcs = "git"                         # git|none
[term]
color = "auto"                      # auto|always|never
progress = "auto"                   # auto|always|never
message-format = "human"            # human|short|json
verbosity = "normal"
[registries.corp]
id = "https://packages.example.com/xmlsquish"
index = "sparse+https://index.example.com/xmlsquish/"
auth-scope = "corp-read"             # non-secret credential lookup name
```

Never place registry tokens in TOML.

## XML DSL quick reference

Built-ins are identified by namespace URI, not by the spelling of `xs`:

```xml
xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
```

**Design model:** XML nodes are data; macros are pure computation. `xs:module` contains only imports and macro definitions. `xs:entry` is the product/link root: imports and entry parameters must precede its construction body. Imports load definitions; only `xs:expand` executes a macro.

```xml
<!-- Macro library / 宏库：src/macros.xml -->
<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
           xmlns:m="urn:example:macros">
  <xs:macro name="m:panel">
    <xs:param name="title"/>
    <Panel title="fixed">
      <Title><xs:insert get="arg.title"/></Title>
      <Body><xs:slot name="body" required="true"/></Body>
    </Panel>
  </xs:macro>
</xs:module>
```

```xml
<!-- Product entry / 产品入口：src/chat.xml -->
<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
          xmlns:m="urn:example:macros">
  <xs:import src="./macros.xml"/>
  <xs:param name="name"/>
  <Prompt>
    <xs:expand ref="m:panel">
      <xs:arg name="title">Hello, <xs:insert get="arg.name"/></xs:arg>
      <xs:fill name="body"><Message>Welcome!</Message></xs:fill>
    </xs:expand>
  </Prompt>
</xs:entry>
```

Essential forms:

| Form | Contract |
|---|---|
| `xs:import src="..."` | Static module load; relative to the defining source. |
| `xs:macro name="p:name"` | Immutable definition keyed by expanded QName; no overload/override. |
| `xs:expand ref="p:name"` | Recursive call in a fresh frame. |
| `xs:param name="x"` | Required Unicode string parameter; declare before body content. |
| `xs:arg name="x" value="..."` | Literal argument; alternatively use `get="arg.y"` or a text-only body. |
| `xs:slot` / `xs:fill` | Explicit immutable XML-node-sequence parameter; not a string. |
| `xs:insert get="arg.x"` | Emit a scalar as escaped text, never reparsed markup. |
| `xs:ifr get="arg.x" pattern="..."` | Emit body only on regex match; `str="literal"` is the alternative input. |

Scalar bindings are read-only: `file.uri|dir|name`, explicit `arg.*`, and lexically scoped `match.*`. Regexes expose named captures only (`(?&lt;tail&gt;...)` in XML); no positional captures, look-around, or backreferences. Calls must provide exactly the declared args/fills, values are evaluated once in the caller, and callee frames inherit nothing implicitly. Recursion is valid and bounded by configured execution limits. The final entry expansion must be one well-formed XML document.

For full semantics and invariants, read [`docs/dsl.md`](docs/dsl.md). For runnable examples, inspect [`examples/`](examples/).
