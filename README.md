# xmlsquish

**XML 是数据，宏是计算。 / XML is data. Macros are computation.**

xmlsquish 是一个以项目为中心的提示词构建器：它从版本化 `xmlsquish.toml` 清单发现包、工作区、依赖和命名目标，把 XML DSL 编译成可复用中间表示（Intermediate Representation, IR），以入口作为链接根（link root），再发布 `.prompt` 产品。当前 XML 语言原语与语义保持不变；规范见 [`docs/dsl.md`](docs/dsl.md)，管理器架构见 [ADR 0009](docs/adr/0009-microkernel-manager-and-reusable-ir.md)。

xmlsquish is a project-oriented prompt builder. It discovers packages, workspaces, dependencies, and named targets from a versioned `xmlsquish.toml`, compiles the XML DSL to reusable IR, treats each entry as a link root, and publishes `.prompt` products. The XML language primitives and semantics are unchanged.

## 安装 / Installation

### 预编译二进制 / Prebuilt binary

从 [GitHub Releases](https://github.com/kleedaisuki/prompt-squish/releases) 下载与操作系统和处理器匹配的归档，按同次发布的校验和验证后解压，并把 `xmlsquish`（Windows 为 `xmlsquish.exe`）加入 `PATH`。Linux 发布包需要其发布说明所列的 glibc 版本；它不是 Alpine/musl 二进制。

Download the archive for your OS and CPU from GitHub Releases, verify it against the checksums from the same release, extract it, and put `xmlsquish` (`xmlsquish.exe` on Windows) on `PATH`. Check the release notes for the Linux glibc requirement.

```bash
xmlsquish --version
xmlsquish --help
```

### 从 Git 源码安装 / Install from Git source

仅源码安装需要 Rust 1.88 或更高版本。仓库是完整 Cargo workspace，请使用锁文件；本项目**不发布到 crates.io**。

Source installation requires Rust 1.88 or newer. Install the complete Cargo workspace with its lockfile. This project is **not published to crates.io**.

```bash
cargo install --git https://github.com/kleedaisuki/prompt-squish --locked
```

从当前检出安装 / Install from the current checkout:

```bash
cargo install --path . --locked
```

## 五分钟上手 / Five-minute start

从一个不存在的目录创建项目，然后检查格式并离线构建：

Create a project in a destination that does not yet exist, then check formatting and build offline:

```bash
xmlsquish new hello-prompts
cd hello-prompts
xmlsquish fmt --check
xmlsquish build --offline
```

`new` 生成 `xmlsquish.toml` 与 `src/prompt.xml`，默认在不属于现有 Git 工作树时初始化 Git；`--vcs none` 可禁用。它拒绝覆盖任何已存在的文件、目录或符号链接。`init` 是未来面向已有目录的独立工作流，不是 `new` 的别名。

`new` creates `xmlsquish.toml` and `src/prompt.xml`, initializing Git by default when the destination is not already inside a Git worktree; use `--vcs none` to disable it. It never overwrites an existing file, directory, or symlink. `init` is a distinct future workflow for existing directories, not an alias for `new`.

每个示例目录都是可运行项目：

```bash
xmlsquish build --manifest-path examples/semantic/xmlsquish.toml
xmlsquish fmt --manifest-path examples/semantic/xmlsquish.toml --check
xmlsquish build --manifest-path examples/semantic/xmlsquish.toml --emit prompt --emit ir --emit debug
```

该示例的稳定逻辑产品定位符和真实文件路径都是 `target/xmlsquish/artifacts/prompt.prompt`，可原样传给 `xmlsquish inspect artifact`；加 `--format=raw` 可将摘要验证后的真实产物字节写到 stdout。发布器以完整目标为单位提交并验证 generation；其 journal、hash、current pointer、`.xsmap` 与 build record 位于同一项目构建根的 `metadata/` 与 `cache/` 内，不会混入 `artifacts/`、构建结果路径或普通终端输出。重复 `--emit` 可物化：

| 后缀 / Suffix | 含义 / Meaning |
| --- | --- |
| `.prompt` | 可交付提示词产品 / Deliverable prompt product |
| `.xsir` | 可复用、版本化二进制 IR / Reusable versioned binary IR |
| `.psdbg` | 自包含来源、链接与展开调试包 / Self-contained provenance, link, and expansion debug bundle |

`.xsir` 与 `.psdbg` 是伴随产物，不是 XML DSL 的新原语，也不是最终提示词。

## 项目清单 / Project manifest

最小 `xmlsquish.toml`：

```toml
manifest-version = 1

[package]
name = "agent-prompts"
version = "1.0.0"
source-root = "src"

[target.chat]
entry = "src/chat.xml"
output = "chat.prompt" # 可省略；默认 <target>.prompt / optional

[target.chat.args]
name = "Klee"
```

`target.entry` 是相对包清单的 `xs:entry` 源码，也是链接根；它不是宏、没有隐式 `main`。工作区可在根清单中声明 `[workspace]`、`members`、`exclude`、`target-dir` 和共享依赖；`-p/--package`、`--workspace` 与 `--exclude` 控制包选择。目标输出必须使用 `.prompt` 后缀且不得逃逸共享目标目录。

`target.entry` names an `xs:entry` source relative to its package manifest and is the link root. It is not a macro and has no implicit `main`. Root manifests may declare a workspace and shared dependencies. Outputs must use `.prompt` and remain within the shared target directory.

## 命令 / Commands

| 命令 | 作用 | 常用选项 |
| --- | --- | --- |
| `xmlsquish new PATH` | 在尚不存在的目标创建可立即构建的项目 | `--name`, `--vcs git\|none` |
| `xmlsquish build` | 解析依赖、冻结源码、编译 IR、链接、实例化并发布 | `-t/--target`, `-p/--package`, `--profile`, `-j/--jobs`, `--emit`, `--arg TARGET.NAME=VALUE` |
| `xmlsquish fmt` | 格式化项目自有 XML；保持 DSL 语义 | `--check`, `--diff`, `--path`, `--style-edition` |
| `xmlsquish add SPEC` | 新增或更新有类型依赖，并协调清单与锁文件 | `--path`, `--git`, `--rev/--tag/--branch`, `--registry`, `--rename`, `--dry-run` |
| `xmlsquish remove ALIAS` | 按别名移除直接依赖 | `-p/--package`, `--dev`, `--build`, `--dry-run` |
| `xmlsquish inspect …` | 只读检查 IR、链接、源码来源、缓存键或产物 | `ir`, `link`, `source`, `cache`, `artifact`; `--format human|json|raw` |
| `xmlsquish clean` | 原子分离并删除当前项目/工作区的完整本地构建根 | `--manifest-path` |

除 `new` 外，项目命令从当前目录向上发现 `xmlsquish.toml`；`--manifest-path PATH` 显式选择清单。`new` 接受待创建的目标路径，并在适用时把项目加入外围工作区。不存在松散文件编译语法：路径必须通过清单目标或 `fmt --path` 等有类型选项表达。

Except for `new`, project commands discover `xmlsquish.toml` upward from the current directory; `--manifest-path PATH` selects it explicitly. `new` accepts the destination to create and joins an enclosing workspace when applicable. Loose-file compilation is no longer a command grammar.

### 依赖来源与锁定模式 / Dependency sources and lock modes

```bash
xmlsquish add common --path ../common
xmlsquish add toolkit --git https://example.com/toolkit.git --tag v1.2.0
xmlsquish add prompt-common@^2 --registry community --rename common
xmlsquish remove common --dry-run
xmlsquish build --locked
xmlsquish build --offline
xmlsquish build --frozen
```

依赖恰好选择一种来源：本地路径（path）、Git、registry 版本或工作区继承（`{ workspace = true }`）。`add` 支持前三者；工作区来源写在清单中。解析器把可变要求记录为精确锁状态：

- `--locked`：禁止修改锁文件；
- `--offline`：禁止网络访问，只使用本地可验证缓存；
- `--frozen`：同时启用二者。

These modes apply to `build`, `add`, and `remove`. A dependency edit is planned and validated before commit; `--dry-run` writes neither manifest nor lock state. Adding or removing a dependency never rewrites `xs:import` automatically.

### 项目本地构建根 / Project-local build root

默认 `target/xmlsquish` 是项目或工作区拥有的唯一派生状态根，不是 Cargo 的机器级缓存：

```text
target/xmlsquish/
├── artifacts/                 # 稳定的 .prompt / .xsir / .psdbg 产品
├── cache/
│   ├── cas/                   # 内容寻址存储 / Content-Addressed Store (CAS)
│   ├── actions.sqlite3        # 可重建动作索引及其 WAL sidecars
│   └── sources/               # 锁定依赖源码
├── metadata/
│   ├── layout.json            # 布局格式判别，不含绝对路径
│   ├── publications/          # 发布 generations 与 current pointers
│   └── catalog/               # 构建记录和 inspect 证据
└── work/                      # 同文件系统暂存；永不作为权威状态
```

The default `target/xmlsquish` is the sole derived-state root owned by the project or workspace,
not a Cargo-style machine cache. Stable products live under `artifacts/`; disposable CAS, action,
and dependency-source caches live under `cache/`; rebuildable compilation and publication evidence
lives under `metadata/`; and non-authoritative same-filesystem staging lives under `work/`.

`clean` 不改写清单或锁文件。它取得独占维护锁，原子分离并删除整个构建根，所以产品、
项目缓存和编译元数据一起消失；下一次构建从项目输入重建。1.0.4 不读取或迁移旧的全局
缓存、`.xmlsquish/cache` 或分离的 manager storage。构建根同父目录可能短暂存在维护锁、
clean journal 与 trash，它们只用于并发和崩溃恢复，不是缓存。

`clean` rewrites neither manifests nor the lockfile. It takes the exclusive maintenance lock,
atomically detaches, and deletes the entire build root, so products, project caches, and compilation
metadata disappear together; the next build reconstructs them from project inputs. Version 1.0.4
neither reads nor migrates legacy global caches, `.xmlsquish/cache`, or separate manager storage.
The build-root parent may briefly contain a maintenance lock, clean journal, and trash entry used
only for concurrency and crash recovery.

## 配置与输出 / Configuration and output

配置按以下优先级分层合并（后者覆盖前者）：

```text
defaults
  < user config
  < workspace config
  < environment
  < CLI options / repeated --config KEY=VALUE
```

| 层 | 位置 / Location |
| --- | --- |
| 用户配置 | `$XMLSQUISH_HOME/config.toml`; Windows 默认 `%APPDATA%/xmlsquish/config.toml`; Unix 默认 `$XDG_CONFIG_HOME/xmlsquish/config.toml` 或 `~/.config/xmlsquish/config.toml` |
| 工作区配置 | 项目根 `.xmlsquish/config.toml` |
| CLI 覆盖 | `--config 'term.message-format="json"'`；值使用 TOML 语法，可重复 |

相对路径按声明它的配置文件目录解析；CLI 覆盖中的相对路径按当前工作目录解析。支持的配置表是 `build`、`new`、`term` 与 `registries.<alias>`。构建布局只由项目清单的 `workspace.target-dir` 决定；旧 `source.cache-root`、`manager.storage-root`、`XMLSQUISH_SOURCE_CACHE_ROOT` 与 `XMLSQUISH_STORAGE_ROOT` 已移除。

操作消息支持 `--message-format human|short|json`。`json` 是换行分隔 JSON（Newline-Delimited JSON, NDJSON），每行一个协议 3.1 事件，写入 stdout；human/short 状态与诊断写入 stderr，stdout 留给查询数据。`inspect` 使用 `--format human|json|raw` 返回一个查询结果；`raw` 仅适用于 `inspect artifact`，且只向 stdout 写入经摘要验证的产物字节。`--plain` 禁用颜色和动态进度；`--quiet` 抑制成功状态。

> **自动化迁移 / Automation migration:** xmlsquish 1.0.1 发送协议 `3.0`，而不是 1.0.0 的 `2.1`。这是一次机器协议主版本迁移，尽管产品版本只增加了补丁号。构建结果现在提供有类型的发布身份和稳定逻辑 `locator`，不再暴露发布器的物理 generation URI。解析 `--message-format=json` 的消费者必须在升级 CLI 时同步迁移到 v3 结构；不要将 v2 构建结果视为可加性更改。

> **Automation migration:** xmlsquish 1.0.1 emits protocol `3.0`, not the `2.1` emitted by 1.0.0. This is a machine-protocol major migration even though the product version advances only by a patch. Build results now carry typed publication identity and stable logical `locator` values instead of publisher-private physical generation URIs. Consumers parsing `--message-format=json` must migrate to the v3 shape when upgrading the CLI; do not treat the v2 build-result shape as an additive change.

xmlsquish 1.0.2 将协议可加性提升到 `3.1`，为 `clean` 增加有类型的请求、动作和统计结果；
现有 3.0 构建、格式化、依赖与查询结果结构保持不变。

xmlsquish 1.0.2 advances the additive protocol minor to `3.1` for typed `clean` requests, actions,
and statistics; existing 3.0 build, format, dependency, and inspection result shapes are unchanged.

xmlsquish 1.0.4 继续使用协议 `3.1`，但产品 locator 有意迁移到
`<target-dir>/artifacts/...`。磁盘布局不是机器协议；解析 locator 的消费者应把它当作完整的
不透明项目相对路径，而不是自行拼接 `target-dir`。

xmlsquish 1.0.4 retains protocol `3.1`, but intentionally moves product locators to
`<target-dir>/artifacts/...`. The disk layout is not the machine protocol; consumers should treat a
locator as one complete opaque project-relative path rather than prepend `target-dir` themselves.

## 退出与自动化 / Process exits and automation

| 退出码 | 含义 |
| ---: | --- |
| `0` | 成功；裸 `xmlsquish`、帮助和版本也成功 |
| `1` | 领域操作失败；包括 `fmt --check` 发现需格式化文件 |
| `2` | 命令行用法或参数解析错误 |
| `101` | 未映射的内部错误 |
| `130` | 收到中断并完成取消/清理 |

失败的构建不会把部分目标当作成功产品发布；默认继续运行独立工作，`--no-keep-going` 可停止接纳新工作。机器自动化应依赖退出码与 NDJSON 字段，不应抓取 human 文案。

## XML DSL 快速地图 / XML DSL quick map

| 结构 | 契约 |
| --- | --- |
| `xs:module` | 仅含 import 与 macro 的库 / Library of imports and macro definitions |
| `xs:entry` | 导入、入口参数与产品构造；链接根 / Imports, root parameters, construction; link root |
| `xs:import src="…"` | 静态装载定义，不执行 / Statically load definitions |
| `xs:macro name="p:name"` | 按 XML 扩展名注册的不可变宏 / Immutable macro keyed by expanded name |
| `xs:expand ref="p:name"` | 在独立帧中递归展开 / Recursive expansion in an isolated frame |
| `xs:param` / `xs:arg` | 必需字符串参数与按值实参 / Required scalar parameters and value arguments |
| `xs:fill` / `xs:slot` | 显式 XML 节点序列传递 / Explicit XML node-sequence passing |
| `xs:insert get="…"` | 把标量绑定写成转义文本 / Emit a scalar binding as escaped text |
| `xs:ifr` | Unicode 正则条件与命名捕获 / Regex condition with named captures |

内建操作按命名空间 URI 识别，不按 `xs` 前缀拼写识别。完整语义、作用域、正则限制与资源预算见 [DSL 规范](docs/dsl.md)。

## 示例与开发 / Examples and development

- [组合与递归](examples/semantic/README.md)
- [显式参数而非继承](examples/inheritance/README.md)
- [网站演示](examples/site-demo/README.md)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

许可 / License: [`GPL-3.0-or-later`](LICENSE).
