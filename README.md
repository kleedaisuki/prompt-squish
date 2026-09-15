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

每个示例目录都是可运行项目：

```bash
xmlsquish build --manifest-path examples/semantic/xmlsquish.toml
xmlsquish fmt --manifest-path examples/semantic/xmlsquish.toml --check
xmlsquish build --manifest-path examples/semantic/xmlsquish.toml --emit prompt --emit ir --emit debug
```

该示例的逻辑产品定位符是 `target/xmlsquish/prompt.prompt`。管理器把完整目标原子发布到项目内 `examples/semantic/target/xmlsquish/.squish-publish/generations/…`，并在构建事件中报告当前 generation 的实际路径；不要绕过 current manifest 修改 generation 内文件。重复 `--emit` 可物化：

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
| `xmlsquish build` | 解析依赖、冻结源码、编译 IR、链接、实例化并发布 | `-t/--target`, `-p/--package`, `--profile`, `-j/--jobs`, `--emit`, `--arg TARGET.NAME=VALUE` |
| `xmlsquish fmt` | 格式化项目自有 XML；保持 DSL 语义 | `--check`, `--diff`, `--path`, `--style-edition` |
| `xmlsquish add SPEC` | 新增或更新有类型依赖，并协调清单与锁文件 | `--path`, `--git`, `--rev/--tag/--branch`, `--registry`, `--rename`, `--dry-run` |
| `xmlsquish remove ALIAS` | 按别名移除直接依赖 | `-p/--package`, `--dev`, `--build`, `--dry-run` |
| `xmlsquish inspect …` | 只读检查 IR、链接、源码来源、缓存键或产物 | `ir`, `link`, `source`, `cache`, `artifact`; `--format human|json` |

所有项目命令从当前目录向上发现 `xmlsquish.toml`；`--manifest-path PATH` 显式选择清单。不存在松散文件编译语法：路径必须通过清单目标或 `fmt --path` 等有类型选项表达。

All project commands discover `xmlsquish.toml` upward from the current directory; `--manifest-path PATH` selects it explicitly. Loose-file compilation is no longer a command grammar.

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

相对路径按声明它的配置文件目录解析；CLI 覆盖中的相对路径按当前工作目录解析。支持的配置表是 `source`、`manager`、`build`、`term` 与 `registries.<alias>`。

操作消息支持 `--message-format human|short|json`。`json` 是换行分隔 JSON（Newline-Delimited JSON, NDJSON），每行一个版本化事件，写入 stdout；human/short 状态与诊断写入 stderr，stdout 留给查询数据。`inspect` 使用 `--format human|json` 返回一个文档，而不是操作事件流。`--plain` 禁用颜色和动态进度；`--quiet` 抑制成功状态。

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
cargo test --workspace --all-features --locked
```

许可 / License: [`GPL-3.0-or-later`](LICENSE).
