# xmlsquish

**XML 是数据，宏是计算。 / XML is data. Macros are computation.**

xmlsquish 是 Rust 编写的 XML 结构预处理器：冻结源码模块，以显式参数、XML 插槽和递归宏生成一个 XML 文档。语言规范以 [`docs/dsl.md`](docs/dsl.md) 为准；统一宏设计与迁移见 [ADR 0007](docs/adr/0007-unified-macro-expansion.md)。

最近发布版本 / Latest published version: **0.3.0** · [发布说明 / Release notes](docs/releases/0.3.0.md) · [更新日志 / Changelog](CHANGELOG.md) · [命名空间 / Namespace](https://xmlsquish.moesegfault.dev/ns)

0.3.0 将显式 `xs:entry` 构建入口与 `xs:module` 宏库分离，统一使用 `macro` / `expand`，不兼容 0.2.0 语法。以下示例使用 0.3.0；历史发布说明和版本快照保持不变。

0.3.0 separates explicit `xs:entry` build roots from `xs:module` libraries and uses unified `macro` / `expand` syntax, breaking compatibility with 0.2.0. The examples below target 0.3.0; historical release notes and snapshots remain unchanged.

## 安装与运行 / Install and run

直接下载对应系统和处理器的预编译二进制，无需安装 Rust。Linux 包在 Ubuntu 22.04 上构建，需要 glibc 2.35 或更新版本（不适用于 Alpine/musl）。

Download the prebuilt binary for your OS and CPU; Rust is not required. Linux packages are built on Ubuntu 22.04 and require glibc 2.35 or newer (not Alpine/musl).

| 系统 / OS | 处理器 / CPU | 下载 / Download |
| --- | --- | --- |
| Windows 10 / 11 | x86-64 | [x86_64-pc-windows-msvc.zip](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/xmlsquish-0.3.0-x86_64-pc-windows-msvc.zip) |
| Windows 10 / 11 | ARM64 | [aarch64-pc-windows-msvc.zip](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/xmlsquish-0.3.0-aarch64-pc-windows-msvc.zip) |
| Linux · glibc ≥ 2.35 | x86-64 | [x86_64-unknown-linux-gnu.tar.gz](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/xmlsquish-0.3.0-x86_64-unknown-linux-gnu.tar.gz) |
| Linux · glibc ≥ 2.35 | ARM64 | [aarch64-unknown-linux-gnu.tar.gz](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/xmlsquish-0.3.0-aarch64-unknown-linux-gnu.tar.gz) |
| macOS ≥ 11 | Intel | [x86_64-apple-darwin.tar.gz](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/xmlsquish-0.3.0-x86_64-apple-darwin.tar.gz) |
| macOS ≥ 11 | Apple Silicon | [aarch64-apple-darwin.tar.gz](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/xmlsquish-0.3.0-aarch64-apple-darwin.tar.gz) |

校验后解压，将 `xmlsquish`（Windows 为 `xmlsquish.exe`）所在目录加入 `PATH`，再运行 `xmlsquish --version` 和 `xmlsquish --help`；版本应为 `xmlsquish 0.3.0`。Windows 可用 `Expand-Archive` 解压 ZIP；Linux/macOS 可用 `tar -xzf <archive.tar.gz>`。未加入 `PATH` 时，可在解压目录运行 `./xmlsquish --version`，Windows PowerShell 使用 `.\xmlsquish.exe --version`。

Verify and extract the archive, add the directory containing `xmlsquish` (`xmlsquish.exe` on Windows) to `PATH`, then run `xmlsquish --version` and `xmlsquish --help`. Expect `xmlsquish 0.3.0`. Extract ZIPs with `Expand-Archive` on Windows or tarballs with `tar -xzf <archive.tar.gz>` on Linux/macOS. Before updating `PATH`, run `./xmlsquish --version` from the extracted directory, or `.\xmlsquish.exe --version` in Windows PowerShell.

下载同次发布的 [SHA256SUMS](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/SHA256SUMS)，在解压前比对所下载压缩包的 SHA-256：Linux 使用 `sha256sum <archive>`，macOS 使用 `shasum -a 256 <archive>`，PowerShell 使用 `Get-FileHash -Algorithm SHA256 <archive>`。仅与清单中完全相同文件名的一行比较。校验和证明文件完整性，不证明发布者身份；二进制未做代码签名（code signing）或 Apple 公证（notarization），系统可能显示安全提示，不要关闭全局安全保护。

Download [SHA256SUMS](https://github.com/kleedaisuki/prompt-squish/releases/download/v0.3.0/SHA256SUMS) from the same release and compare the archive's SHA-256 before extraction: `sha256sum <archive>` on Linux, `shasum -a 256 <archive>` on macOS, or `Get-FileHash -Algorithm SHA256 <archive>` in PowerShell. Compare only the row with the exact downloaded filename. Checksums establish integrity, not publisher identity. Binaries are unsigned and not Apple-notarized; OS security prompts may appear. Do not disable system-wide security protections.

六个平台由 GitHub Actions 在原生架构上构建并进行基本运行验证（smoke test）。0.3.0 的二进制从既有 `v0.3.0` 标签补充构建，标签和源码快照不变；未发布到 crates.io。

GitHub Actions builds and smoke-tests all six targets on native architectures. The 0.3.0 binaries are backfilled from the existing `v0.3.0` tag without moving the tag or changing its source snapshot. There is no crates.io publication.

### 可选：源码安装 / Optional: build from source

仅源码安装需要 Rust 1.88 或更高版本。使用固定标签和锁文件：

Only source installation requires Rust 1.88 or newer. Use the pinned tag and lockfile:

```bash
cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v0.3.0 --locked
```

从当前检出源码安装并运行 / Install and run from the current checkout:

```bash
cargo install --path . --locked
xmlsquish examples/semantic/prompt.xml
xmlsquish examples/site-demo/agent.xml
xmlsquish --debug --max-depth 128 --max-expansions 10000 --max-output-bytes 1048576 examples/semantic/prompt.xml
```

`--arg NAME=VALUE` 可重复，为 `xs:entry` 的声明参数提供字符串值。`-I` 生成带来源信息（provenance）的 `.i.xml`；默认 `-O` 生成干净的 `.o.xml`。**最终 `.o.xml` 移除所有属性与命名空间声明（namespace declaration），元素仅保留局部名（local name），并继续压缩空白。** 属性不属于提示词产品语义；空白是无意义的格式字符串。这些行为不能通过输出选项或 `xml:space` 改变。`--debug` 与 `--explain` 等价，保留中间诊断信息但不改变最终产品行为。源码中的 DSL 指令属性仍用于编译和展开，标量求值仍保留字符串内容；最终提示词输出不承诺通用 XML 数据语义等价。输入文件不覆盖，失败不得提交部分成功输出。

Repeat `--arg NAME=VALUE` for entry parameters. `-I` writes provenance-bearing `.i.xml`; default `-O` writes clean `.o.xml`. **Final `.o.xml` removes every attribute and namespace declaration, uses local element names, and squishes whitespace.** Attributes are not prompt product semantics; whitespace is formatting noise. Neither output options nor `xml:space` can override these rules. `--debug` and `--explain` are aliases that retain intermediate diagnostics without changing final product behavior. Source DSL directive attributes still drive compilation and expansion, and scalar evaluation still preserves string contents; final prompt output does not promise general XML data equivalence. Sources are never overwritten and failed expansion must not publish partial output.

路径可为文件、目录或引号括起的 glob。目录和 glob 发现的合法 `xs:module` 库文件会跳过；显式指定模块文件仍报错。无路径时显示帮助；生成的 `.i.xml` / `.o.xml` 不再作为输入。`--color auto|always|never` 控制终端颜色。独立文件可继续处理，但任一失败使退出码非零。具体选项以 `xmlsquish --help` 为准。

Paths accept files, directories, or quoted globs. Directory/glob discovery skips valid `xs:module` libraries; explicitly naming such a file remains an error. No paths prints help. Generated artifacts are excluded from discovery. Independent inputs may continue after an error, but any failure yields a nonzero exit status. Consult `xmlsquish --help` for options.

## 最小程序 / Minimal program

保存为 `hello.xml` / Save as `hello.xml`:

```xml
<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns">
  <xs:param name="name"/>
  <Greeting>Hello, <xs:insert get="arg.name"/>!</Greeting>
</xs:entry>
```

```bash
xmlsquish --arg name=Klee hello.xml
```

`insert` 产生转义后的文本，绝不把字符串重新解释为 XML。普通文本和属性没有 `$` 插值；普通属性不会进入 `.o.xml`。

`insert` emits escaped text, never reparsed markup. Ordinary text and attributes have no `$` interpolation; ordinary attributes never reach `.o.xml`.

## 语言地图 / Language map

| 结构 / Form | 契约 / Contract |
| --- | --- |
| `xs:module` | 只包含 import 与 macro 的库 / Library containing imports and macro definitions only |
| `xs:entry` | 导入、入口参数与产品构造正文，不是宏 / Imports, root parameters, and document construction; not a macro |
| `xs:import src="..."` | 仅装载定义，不执行 / Load definitions without execution |
| `xs:macro name="app:name"` | 按扩展名（Expanded Name）注册，不可重定义 / Immutable namespace-qualified definition |
| `xs:param name="x"` | 必需字符串参数，声明在主体之前 / Required string parameter before body |
| `xs:expand ref="app:name"` | 递归展开命名宏，返回节点序列 / Recursively expand a named macro into a node sequence |
| `xs:arg` | `value`、`get`、纯文本展开 body 三选一 / Exactly one scalar value form |
| `xs:fill` / `xs:slot` | 显式传递 XML 节点序列 / Explicit XML node-sequence passing |
| `xs:insert get="..."` | 读取 `file.*`、`arg.*`、词法 `match.*` / Read immutable scalar binding |
| `xs:ifr` | Unicode 正则条件与命名捕获（named capture） / Regex condition with named captures |

内建操作按命名空间 URI 识别，不按 `xs` 拼写识别。宏 `name` / `ref` 必须有绑定的前缀；`import` 不继承被导入文件的前缀绑定，引用方自行声明相同 URI 的别名。相对 `src` 与 `file.*` 绑定定义位置；不继承调用者参数。静态装载完整源码闭包，即使某个分支不会执行，也会验证其中的引用和模式。纯导入环合法；执行递归由可调预算约束。

Builtin identity uses the namespace URI, not prefix spelling. Macro names/references require bound prefixes. Imports do not inherit prefix bindings: the referencing file declares its own alias for the same URI. Relative sources and `file.*` bind at the definition site. Arguments are not inherited. Discovery validates the complete static source closure, including unselected branches. Import cycles are legal; execution recursion is guarded by configurable budgets.

模块没有隐式正文或 `main`。`xs:entry` 是独立源码根，只构造文档、不定义宏；它不是符号或隐式宏。`import` 是唯一装载操作，只接受模块，不能导入入口。`expand` 是唯一展开操作；每次展开拥有独立作用域，参数与 fill 在展开者环境中求值一次后按值传递，不捕获外部变量。宏返回有序节点序列；纯文本返回值可在另一个 `xs:arg` body 中组合传递，结构返回值可在 `xs:fill` 中组合。`fragment` 不是独立语言构造。

Modules have no implicit body or `main`. `xs:entry` is a separate source root for document construction, not a macro definition or symbol. `import` is the sole loading operation and accepts modules only, never entries. `expand` is the sole expansion operation: each expansion has an isolated scope with eagerly evaluated, immutable arguments and fills, without caller capture. Macros return ordered node sequences; text-only results compose in `xs:arg` bodies and structural results in `xs:fill`. There is no separate `fragment` construct.

正则表达式不允许普通位置捕获、反向引用（backreference）和环视（look-around）。用 `(?:...)` 分组、`(?<name>...)` 捕获；XML 属性中 `<` 写成 `&lt;`。空白敏感的标量 body 应写为紧凑内联形式。

Regexes reject positional captures, backreferences, and look-around. Use noncapturing groups and named captures; escape `<` as `&lt;` in XML attributes. Keep whitespace-sensitive scalar bodies inline.

核心不读取环境、时间或执行子进程。默认本地加载器不是文件访问沙箱；只编译信任的源码，并给自动化任务设置合适预算。

The core does not read environment variables or time or execute subprocesses. The local loader is not a filesystem sandbox: compile trusted sources and set suitable automation budgets.

## 示例与内部模块 / Examples and internal modules

- [组合与递归 / Composition and recursion](examples/semantic/README.md)
- [显式参数而非继承 / Explicit arguments, not inheritance](examples/inheritance/README.md)
- [网站演示源码 / Site demo sources](examples/site-demo/agent.xml)

编译器内部模块提供 `Compiler::default()`、`Compiler::with_options(CompileOptions)` 和可注入源码加载器；入口参数放在 `CompileOptions.args`。独立 `squish` 工具保留其词法空白转换用途；它**不是宏求值或 lowering**，而是在干净 XML 之后执行的最终产品压缩步骤，不保证 XML 文本语义。

The internal compiler module provides `Compiler::default()`, `Compiler::with_options(CompileOptions)`, and an injectable source loader; entry arguments belong in `CompileOptions.args`. `Compiler::prepare` freezes and links a reusable snapshot; `PreparedProgram::expand` executes it with fresh inputs and budgets, without reloading sources. Reprepare to observe edits. The standalone `squish` utility remains a lexical whitespace transformer, **not macro evaluation or lowering**; it runs after clean XML as the final product compression pass and does not preserve XML text semantics.

内部编译器支持 `prepare` 一次、`expand` 多次：复用冻结源码与静态 IR 载荷，不复用参数、执行帧或预算状态。源码修改后需重新准备快照。实现与性能取舍见 [性能报告](docs/performance/README.md)，包含原始样本、差分验证和复现命令。

See the [performance report](docs/performance/README.md) for implementation trade-offs, raw paired measurements, differential checks and reproduction commands. These are internal binary modules, not a new public library target.

## 开发与站点 / Development and site

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cd site
npm ci
npm run test
npm run demo:check
```

开发边界与验证要求见 [CONTRIBUTING.md](CONTRIBUTING.md)。网站由真实 Rust CLI 预生成示例，不在浏览器复制编译器。修改示例后运行 `npm run demo:generate`。GitHub Pages 使用 GitHub Actions；自定义域为 `xmlsquish.moesegfault.dev`，DNS 应指向仓库所有者实际的 GitHub Pages 域名。

See [CONTRIBUTING.md](CONTRIBUTING.md) for boundaries and validation. Site examples are generated by the actual Rust CLI, not a browser-side imitation; refresh with `npm run demo:generate`. GitHub Pages uses GitHub Actions and the custom domain `xmlsquish.moesegfault.dev`.

许可 / License: [`GPL-3.0-or-later`](LICENSE). 网站视觉来源 / Site visual foundation: [MoeSegfault Style](https://github.com/kleedaisuki/moesegfault-style).
