---
name: prompt-squish
description: Author, configure, build, format, inspect, and clean xmlsquish prompt projects. Use when working with the XML macro DSL, xmlsquish.toml packages/workspaces/dependencies/targets, or the xmlsquish CLI.
---

# xmlsquish Agent Guide

Use the checked-in manifest and XML sources as truth. Do not infer entries from filenames or reach into dependency directories: targets and exports are explicit.

## Common workflow

```bash
xmlsquish new my-prompts [--name NAME] [--vcs git|none]
cd my-prompts
xmlsquish fmt --check                 # use `fmt` without --check to rewrite
xmlsquish build --offline             # local verified cache only
xmlsquish build -t chat --arg chat.name=Klee
xmlsquish inspect artifact target/xmlsquish/chat.prompt
xmlsquish clean                       # remove build products and provably stale dependency data
```

Project commands discover `xmlsquish.toml` upward; use `--manifest-path PATH` to select one. `clean` removes the current project/workspace products, private build directory, and dependency-cache entries proven stale or abandoned; it does not delete valid shared dependencies merely because this project does not reference them. Use `--locked` to forbid lockfile changes, `--offline` to forbid network access, or `--frozen` for both. In automation prefer exit codes and `--message-format=json` (NDJSON), not human text. Run `xmlsquish <command> --help` before using less common/version-specific flags.

Dependency edits are transactional and never rewrite XML imports:

```bash
xmlsquish add common --path ../common
xmlsquish add toolkit --git https://example.com/toolkit.git --tag v1.2.0
xmlsquish add prompt-common@^2 --registry community --rename common
xmlsquish remove common --dry-run
```

## `xmlsquish.toml` quick reference

```toml
manifest-version = 1

[workspace]                         # optional; may be a virtual root
members = ["packages/*"]
exclude = ["packages/old"]
target-dir = "target/xmlsquish"     # shared output root; this is the default

[workspace.dependencies]
common = { path = "packages/common" }

[package]
name = "agent-prompts"
version = "1.0.2"
dialect = "xmlsquish/1"             # optional default
source-root = "src"                 # optional default

[dependencies]                      # each dependency selects exactly one source
common = { workspace = true }
local = { path = "../local", package = "real-name" }
kit = { git = "https://example.com/kit.git", tag = "v2" } # tag|branch|rev
base = "^2"                         # default registry shorthand
corp = { version = "~1.4", registry = "corp", default-features = false, features = ["chat"], optional = true }

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

Unknown keys are errors. Names use ASCII letters, digits, `-`, or `_`. Paths are manifest-relative and may not escape their owning roots; outputs must end in `.prompt`. A dependency may additionally use `package`, `optional`, `features`, and `default-features`. Consumers import only declared exports of direct dependencies:

```xml
<xs:import src="pkg:common/macros"/>
```

Published locators are stable, readable paths below `workspace.target-dir` (for example `target/xmlsquish/chat.prompt`). Never depend on manager-private hashes, generations, journals, or storage paths.

Operational configuration is separate from the package manifest. Put it in `$XMLSQUISH_HOME/config.toml` or workspace `.xmlsquish/config.toml`; environment and repeated `--config KEY=TOML_VALUE` override files.

```toml
[source]
cache-root = "cache/sources"
[manager]
storage-root = "state"
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
