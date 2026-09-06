<p align="center">
  <img src="site/public/favicon.svg" width="88" height="88" alt="xmlsquish 图标" />
</p>

# xmlsquish

用 Rust 2024 编译 XML 提示词中的宏，再以手写有限状态机（Finite-State Machine, FSM）压紧空白：人类继续维护可读源码，Agent 读取旁路生成的紧凑版本。

**文档：简体中文（当前） · [English](#english)**

> [!WARNING]
> xmlsquish 是面向提示词的**词法规范化器（prompt-oriented lexical canonicalizer）**，不是保持 XML 信息集（XML Information Set, XML Infoset）的通用压缩器。它会改写普通字符数据中的空白，可能改变混合内容（mixed content）的含义，并且**不遵守 `xml:space="preserve"`**。只应处理已确认这些空白是布局噪声的提示词。

## 它做什么

xmlsquish 先进行语义编译（semantic compilation），消除宏、注释及元信息，生成中间表示（Intermediate Representation, IR）；再将普通字符数据中的 XML 空白规范化，保证任意两个相邻词法单元之间有且只有一个 ASCII 空格：

```xml
<!-- input -->
<role>
  You are a careful agent.
</role><task>Summarize this.</task>

<!-- output -->
<role> You are a careful agent. </role> <task> Summarize this. </task>
```

根据 [XML 1.0 `S` 产生式](https://www.w3.org/TR/xml/#sec-common-syn)，只有空格、`\t`、`\r`、`\n` 被识别为空白。压缩阶段保留 markup 内部字节；编译阶段已经消除注释、宏及元信息。底层 `xmlsquish_core::squish` 仍保留原有纯词法契约。

转换不覆盖输入。`prompt.xml` 的结果写到同目录的 `prompt.o.xml`。

## 安装与构建

项目使用 [Rust 2024 Edition](https://doc.rust-lang.org/edition-guide/editions/creating-a-new-project.html)，经过持续验证的最低 Rust 版本（Minimum Supported Rust Version, MSRV）为 1.88。仓库的 `rust-toolchain.toml` 声明使用 stable 工具链以及 `rustfmt`、Clippy 组件。

```bash
# 从当前 checkout 安装命令行程序
cargo install --path crates/xmlsquish-cli --locked

# 或只在仓库中构建
cargo build --workspace --release --locked
./target/release/xmlsquish --help
```

Windows PowerShell 下可运行 `./target/release/xmlsquish.exe --help`。

## 命令行

```text
xmlsquish [-I | -O] [--color auto|always|never] [PATH]...
```

一次可传入任意数量的 XML 文件、目录和 glob 表达式：

```bash
xmlsquish prompts/system.xml prompts/shared
xmlsquish "prompts/**/*.xml" "agents/[ab]-*.xml"
```

- 文件：处理以 `.xml` 结尾的文件。
- 目录：递归查找其下所有 `*.xml`；不跟随符号链接。
- glob：支持 `*`、`?` 和 `[...]`。建议加引号，让 xmlsquish 而非 shell 展开表达式；匹配到目录时也会递归。
- 路径经稳定排序与去重；大小写不敏感地排除 `*.i.xml` 和 `*.o.xml`，避免再次处理编译产物。
- `-I`：只生成 `*.i.xml`，不压缩空白。
- `-O`（默认）：生成 `*.i.xml`，压缩为 `*.o.xml`，输出成功后移除对应中间文件；不会清扫未选中源文件的其他中间产物。
- 多个参数发现同一文件时只处理一次。
- 无参数时向标准输出打印帮助并返回成功。

每个成功输入都会在旁边生成对应名称：

| 输入 | 输出 |
| --- | --- |
| `prompt.xml` | `prompt.o.xml` |
| `nested/agent.v2.XML` | `nested/agent.v2.o.xml` |

既有输出会被原子替换（atomic replacement）；源文件从不覆盖。

## 宏与文件环境

```text
source.xml ──语义编译──▶ source.i.xml ──空白压缩──▶ source.o.xml
                            -I 停止                    -O 默认
```

可运行示例见 [基础宏](examples/semantic/README.md)与[元数据继承、正则匹配和文本插入](examples/inheritance/README.md)。基础语义见 [ADR 0002](docs/adr/0002-semantic-compilation.md)，本轮命名空间迁移和统计口径见 [ADR 0003](docs/adr/0003-metadata-inheritance-and-stage-metrics.md)。

| 编译期语法 | 行为 |
| --- | --- |
| `<?xmlsquish author="klee"?>` | 将属性定义到当前文件的 `meta` 命名空间；空指令不做事 |
| `<xmlsquish:let msg="Hello" dir="parts"/>` | 按声明顺序定义当前文件变量；重复定义报错 |
| `<xmlsquish:set msg="Updated"/>` | 按属性顺序给当前文件已声明变量赋值；未声明报错，不修改内置命名空间 |
| `<xmlsquish:log msg="$msg"/>` | 输出 `文件名:行号: 信息` |
| `<xmlsquish:if lhs="$sys:platform" rhs="win32">…</xmlsquish:if>` | 字符串相等时执行内部内容；移除条件包装标签 |
| `<xmlsquish:ifn lhs="…" rhs="…">…</xmlsquish:ifn>` | 字符串不等时执行内部内容 |
| `<xmlsquish:ifr str="$file:name" pattern="^agent\.xml$">…</xmlsquish:ifr>` | 正则表达式匹配成功时执行内部内容；`str` 展开变量，`pattern` 是原样正则，不展开变量 |
| `<xmlsquish:insert get="meta:author"/>` | 显式插入变量值，`get` 写不带 `$` 的变量名；作为 XML 文本转义，不解析成标签或宏 |
| `<xmlsquish:mount path="$dir/hello.xml"/>` | 相对当前物理文件解析路径，递归编译并接入目标根 |
| `<xmlsquish:mount path="child.xml" rename="Persona"/>` | 可选 `rename` 仅改接入根的名称，保留属性及子树；支持在调用文件中展开变量 |
| `<xmlsquish:import path="$dir/another.xml"/>` | 递归编译，接入目标根的内容，不保留根包装 |

每个文件拥有独立文件环境（File Frame）：局部变量、只读 `file` 物理信息、可继承 `meta`、只读 `sys` 和只读 `env`。条件不新建环境，引用文件不继承或导出局部变量。`$file:name`、`$file:path`、`$file:dir` 始终属于当前物理文件；`$sys:platform` 在 Windows 上为 `win32`；`$sys:time` 在一次 CLI 编译运行内固定。`$env:NAME` 引用环境变量，不存在时同样报未定义。

`mount` 与 `import` 都接受可选的 `openat="self|parent"`。**每条引用边省略参数时始终为 `self`**，使用子文件自身元数据；`parent` 把当前文件的有效 `meta` 叠加到子文件，父方同名字段覆盖子方，父方独有字段新增，子方独有字段保留。连续显式 `parent` 边可逐层传递；没有自动沿用模式。物理路径、局部变量和兄弟文件互不受影响。

迁移提示：原先的 `$file:author`、`$file:version` 等处理指令字段现在使用 `$meta:author`、`$meta:version`；`$file:name` 等物理字段不变。`meta:name` 可以自定义，与 `file:name` 不冲突。

`mount` 的 `rename` 可与 `openat` 一起使用，例如 `rename="$meta:tag" openat="parent"`。省略时保留原根名；指定时同步改起始和结束标签，自闭合根同样支持。空值及非法 XML 限定名（qualified name, QName）报错，不能改成 `xmlsquish:*` 宏。`import` 不接受 `rename`，因为它不保留根。命名空间声明不会自动添加或修复，使用前缀时须自行确保声明存在。

**变量只在编译期语法中展开**，普通 XML 文本、CDATA 和普通属性里的 `$msg` 保留原样。未选择分支不执行宏，不读取其引用文件。循环引用、执行路径上的未定义变量、重复定义和未知宏均报错。XML 声明不作为 `xmlsquish` 指令处理。

`ifr` 使用 Rust `regex` 语法，默认查找任意位置的匹配；全串匹配可用 `\A…\z`，常见单行匹配可用 `^…$`。不支持回溯引用（backreference）和环视（look-around），非法或过大的模式报源文件行号错误。`insert` 将 `&`、`<`、`>` 转义，拒绝 XML 1.0 不允许的字符；插入值不会再次求值，`-O` 仍会压缩其中普通文本的空白。

源文件与引用文件应受信任：宏可以读取本地文件和环境变量，日志也可能输出敏感值；这不是不可信模板的安全沙箱。

### 退出码与错误继续

| 退出码 | 含义 |
| ---: | --- |
| `0` | 帮助/版本请求，或所有发现的文件均成功 |
| `1` | 至少一个发现或文件处理错误；其他独立文件仍会继续 |
| `2` | 命令行语法错误 |

发现、读取、FSM 扫描、Token 计数和写入错误会写入标准错误，并带路径、阶段与原因。汇总仍写入标准输出；失败文件不计入成功文件的字符/Token/空白总数。

### 诊断与颜色

错误以独立的摘要、位置和源码行展示，引用文件失败时附上主输入：

```text
error[compile]: undefined variable '$result'
 --> prompts/child.xml:17
    |
 17 | <xmlsquish:log msg="$result"/>
 note: while compiling prompts/main.xml
```

源码行来自本次编译实际读取的快照，不在报错后重新读取；没有精确列号时不会伪造列号或插入符（caret）。过长源码行截断显示，控制字符和双向文本控制符以转义形式呈现，防止输出变成终端控制指令。工作目录内的路径尽量显示为相对路径，Windows 的内部路径前缀不直接展示。

| 颜色模式 | 行为 |
| --- | --- |
| `--color auto`（默认） | stdout / stderr 分别检测终端能力；普通管道、文件重定向不着色 |
| `--color always` | 强制颜色，适合支持 ANSI 的日志查看器；覆盖环境变量偏好 |
| `--color never` | 完全禁用样式，适合脚本与纯文本日志；覆盖环境变量偏好 |

自动模式遵循 `NO_COLOR`、`CLICOLOR`、`CLICOLOR_FORCE` 等环境约定：`NO_COLOR` 优先禁用，随后 `CLICOLOR_FORCE` 可强制输出（包括管道），`CLICOLOR=0` 禁用。环境没有强制设置时再结合终端能力判断。Windows 终端适配由 [`anstream`](https://docs.rs/anstream/latest/anstream/) 处理，包括旧控制台回退。帮助、参数错误、宏日志、诊断及统计分组使用同一颜色策略；颜色不改变文字含义和退出码。

```sh
xmlsquish --color never prompts
xmlsquish --color always -I prompts/main.xml
```

## 统计口径

默认分词器（Tokenizer）固定为 [`o200k_base`](https://github.com/openai/tiktoken)。只统计文本，不包括 BOM、聊天消息封装、缓存折扣或模型推理开销。所有大小与依赖统计只包含成功写出的主输入。

```text
主输入 Source ──宏/引用展开──▶ Compiled IR ──空白规范化──▶ Final prompt
                  组装倍率                      Token 节省或增加
```

| 统计 | 它回答的问题 |
| --- | --- |
| Source / Compiled IR / Final prompt 的 Token 和 UTF-8 文本字节 | 写了多少源文本，组装出多少内容，最终要发送多大的提示词？ |
| Assembly token ratio | 编译后 Token 数 / 主源文件 Token 数；这是组装倍率，不叫压缩率；源为零则 `N/A` |
| Optimization (IR -> final) | 相同已组装内容经空白规范化后，实际节省、增加或保持多少 Token？只在确有节省或持平时报告非负节省比例 |
| Dependency loads / Unique dependency files | 发生多少次引用加载，涉及多少个不同物理文件？重复引用计多次加载但只计一个唯一文件 |
| Dependency UTF-8 text bytes | 引用加载读入多少文本字节？重复加载累计，不包含主文件初次读取及 BOM |
| Processed / Succeeded / Failed / Discovery errors | 哪些主输入完成，哪些发现或编译失败？ |
| Input / Output characters | 主源文件与最终产物的 Unicode 标量值数；它不同于字节数和 Token 数 |

`-I` 显示空白优化未运行。空批次及零 Token 基线显示 `N/A`，不除以零。批次比例由总数计算，不平均各文件百分比。若空白变化恰好导致分词结果变长，会诚实显示 **tokens added**，而不是截断为零或输出负的“节省率”。

例如，主文件只有 100 Token，却引用组装了 1,000 Token 的内容，压缩后为 900 Token：有意义的结论是“组装为 **10×**，空白优化节省 **100 Token（10%）**”，而不是“压缩率 **−800%**”。这是说明口径的假设例子，不是性能基准。

旧的空白槽位账本仍保留在底层 `squish` API，用于验证 `optimized_characters = intermediate_characters - removed + inserted`，不再充当 CLI 的主要成效指标。Token 减少不自动证明提示词质量、延迟或账单同比改善。

## FSM 语义

扫描器把普通字符数据的连续非空白片段作为 `Word` atom，把完整 markup 结构作为 `Markup` atom，再由统一发射器连接 atom。概念状态如下：

| 状态 | 关键规则 |
| --- | --- |
| Data / Text | 形成 `Word`，暂存 XML `S` 游程；`<` 开始 markup |
| Tag | 只在属性引号外由 `>` 结束 |
| Comment / CDATA / PI | 分别只由 `-->`、`]]>`、`?>` 结束 |
| DOCTYPE | 跟踪引号、内部子集方括号深度、注释与 PI，避免错误地停在内部 `>` |

底层 `squish` 中的 markup 内字节序列原样复制。这个空白扫描器不构造 DOM，不验证元素名或标签配对，也不展开 DTD/实体；未闭合结构以类型和原输入字节偏移报告。CLI 的前置编译器另行检查标签结构，并消除编译语法。完整词法设计见 [ADR 0001](docs/adr/0001-lexical-canonicalization-and-layering.md)。

### UTF-8 与 BOM

- 只支持严格 UTF-8，不做有损解码，也不解码 UTF-16LE/BE 等其他编码；字节序列不是合法 UTF-8（包括常见的带 BOM UTF-16 文件）时会按文件报错并继续其他文件。
- 可选 UTF-8 BOM 会在处理前移除、写出时保留。
- BOM 不进入字符、Token 或空白统计。

## 仓库结构

```text
crates/
  xmlsquish-core/  # 领域算法：语义编译、FSM、错误、空白账本
  xmlsquish-app/   # 应用用例：批处理、报告、端口 traits
  xmlsquish-cli/   # 适配器：CLI、发现、I/O、tiktoken、展示
site/              # Astro + TypeScript + React GitHub Pages
docs/adr/          # 架构决策记录
.github/workflows/ # Rust/Site CI 与 Pages 发布
```

`xmlsquish-cli` 是组合根，同时依赖 `xmlsquish-app` 的用例/端口与 `xmlsquish-core` 的编译器和纯 FSM；app 和 core 彼此不依赖。网站不复制 Rust FSM。

## 网站与本地开发

发布目标是 <https://xmlsquish.moesegfault.dev>。简体中文位于 `/`，English 位于 `/en/`；两条路由共享强类型消息和页面组件，React 只用于主题控件。

```bash
cd site
npm ci
npm run dev       # 本地开发服务器
npm run check     # Astro / TypeScript 检查
npm run build     # 静态产物：site/dist
npm run test      # check + build
```

网站采用固定版本 [MoeSegfault Style](https://github.com/kleedaisuki/moesegfault-style) CSS（含子资源完整性校验，即 Subresource Integrity, SRI），并使用继承该视觉语言的项目 SVG 图标。

### GitHub Pages 首次启用

仓库已经提供 `site/public/CNAME` 与基于 [Astro 官方 GitHub Pages 方案](https://docs.astro.build/en/guides/deploy/github/)的自定义工作流（custom workflow）。仓库管理员仍需：

1. 在 **Repository Settings → Pages → Build and deployment → Source** 选择 **GitHub Actions**。
2. 在 DNS 服务商处将 `xmlsquish` 配置为 CNAME，目标为仓库所有者实际的 `<GitHub 用户名>.github.io`；不要猜测或复用示例值。
3. 在 Pages 设置确认自定义域 `xmlsquish.moesegfault.dev`，等待 DNS 校验通过后启用 **Enforce HTTPS**。

流程细节参见 [GitHub Pages 自定义工作流文档](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)。

## 贡献与许可

开发检查与提交约定见 [CONTRIBUTING.md](CONTRIBUTING.md)。项目按 [`GPL-3.0-or-later`](LICENSE) 发布。

视觉样式源自 [MoeSegfault Style](https://github.com/kleedaisuki/moesegfault-style)；`site/public/favicon.svg` 是本项目沿用其珊瑚渐变、奶油字形与金色星芒视觉母题的原创变体。相关源码分别受其上游许可和本项目许可约束。

---

## English

xmlsquish is a Rust 2024 semantic compiler and hand-written finite-state lexical canonicalizer for XML-shaped LLM prompts. It resolves compile-time macros into `prompt.i.xml`, then writes a compact `prompt.o.xml` without overwriting the source.

> [!WARNING]
> This is **not** a semantics-preserving XML minifier. It rewrites whitespace in ordinary character data, may change mixed-content meaning, and does not honor `xml:space="preserve"`. Use it only where that whitespace is known layout noise.

### Install and run

```bash
cargo install --path crates/xmlsquish-cli --locked
xmlsquish prompts/system.xml prompts/shared "templates/**/*.xml"
```

With no paths, the program prints help and exits successfully. Directories are recursive; glob syntax supports `*`, `?`, and `[...]`; existing `*.i.xml`, `*.o.xml` files and symlinks are skipped. `-I` stops at the uncompressed intermediate; `-O` (default) writes the compact output and removes only its corresponding intermediate after success. Existing outputs are atomically replaced. One bad path or file does not stop independent files; the final exit code is `1` if any such error occurred.

### Diagnostics and color

Errors show a stage label, readable source path and line, and an excerpt of the original loaded source; included-file errors name the primary input separately. No column or caret is fabricated when only a line is known. Long source lines are clipped. Terminal and bidirectional control characters are escaped in diagnostics, logs, and displayed argument values; original input paths and XML payloads are not modified.

`--color auto` (default) detects stdout and stderr independently. Redirected output is plain unless forced. `--color always` and `--color never` override environment preferences. Auto honors `NO_COLOR` first, then `CLICOLOR_FORCE`, then `CLICOLOR=0`, otherwise checking terminal capabilities. [`anstream`](https://docs.rs/anstream/latest/anstream/) provides Windows console adaptation. Help, argument errors, macro logs, diagnostics, and report headings share the policy. Colors are supplementary: wording and exit codes do not depend on them. The injectable `run` API treats unknown writers as non-terminals; the executable uses the terminal-aware `run_stdio` entry point.

### Compile-time language

`<?xmlsquish ...?>` defines `meta` metadata; XML declarations are ignored as compiler instructions. `let` declares file-local variables; `set` assigns already declared locals in attribute order (undefined targets and built-in namespace writes are errors). `log` prints `filename:line: message`, `if` / `ifn` compare strings, `ifr` regex-matches an expanded `str` against a literal `pattern`, and `insert get="name"` writes XML-escaped variable text without recursive evaluation. `get` accepts a bare local or qualified variable name, not a `$` expression. Patterns follow Rust `regex` syntax; anchors work literally, matching searches anywhere by default, and invalid/oversized patterns are errors. All macros use the literal `xmlsquish:` prefix.

Each file has independent locals, physical `file` information, inheritable `meta`, read-only `sys`, and read-only `env`. Conditions share their file's frame; includes neither inherit nor export locals. `$file:name` is always the physical source basename, `$sys:platform` is `win32` on Windows, and `$sys:time` is fixed for one CLI run. Undefined references (including absent environment variables), duplicate definitions, unknown macros, and include cycles are errors. Unselected branches do not execute macros or load includes.

`mount` includes a compiled root; `import` includes its contents without the wrapper. Both accept `openat="self|parent"`: omission always means `self` **on each edge**, while `parent` overlays the current effective metadata on the child's metadata (parent wins collisions). Consecutive explicit `parent` edges propagate metadata; the mode itself is never inherited. Paths remain relative to the physical source, and siblings stay isolated. Migrate PI fields from `$file:author` to `$meta:author`; physical `file:name/path/dir` remain unchanged. See the [inheritance example](examples/inheritance/README.md) and [ADR 0003](docs/adr/0003-metadata-inheritance-and-stage-metrics.md).

`mount` additionally accepts `rename="Persona"` (or a caller-expanded variable such as `$meta:tag`) to rename only the included root. Both paired and self-closing roots preserve their attributes and descendants; omission leaves the name unchanged. Empty/invalid XML QNames and the reserved `xmlsquish:` macro prefix are rejected. `import` does not accept `rename`. No namespace declarations are synthesized; callers using prefixes must provide appropriate declarations. Renaming never changes source files, physical file information, or metadata inheritance.

Expansion is restricted to macro parameters and xmlsquish metadata. Ordinary XML attributes, text, and CDATA keep `$variables` literally. Compilation removes macros, comments, and metadata before squashing. See the [runnable example](examples/semantic/README.md) and [bilingual semantic contract](docs/adr/0002-semantic-compilation.md). Compile trusted sources only: local includes and environment access are not a sandbox, and logs can expose secrets.

Only the four XML `S` characters—space, tab, carriage return, and line feed—are canonicalized. Markup interiors are preserved. Input must be strict UTF-8; an optional UTF-8 BOM is excluded from all measurements and preserved on output.

The report uses fixed `o200k_base` tokenization and BOM-free UTF-8 text bytes. It separates primary sources, compiled IR, and final prompts, showing assembly expansion separately from actual IR-to-final token savings or increases. `-I` explicitly skips optimization; empty baselines have no percentage. Dependency loads, unique dependency files, and bytes read reveal the cost of repeated includes. Only successful outputs contribute to size and dependency totals. These are text-size measurements, not estimates of API billing, model quality, or inference speed. The existing `xmlsquish_core::squish` API retains the pure lexical contract in [ADR 0001](docs/adr/0001-lexical-canonicalization-and-layering.md).

### Develop

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked

cd site
npm ci
npm run test
```

The bilingual Astro/TypeScript/React site targets <https://xmlsquish.moesegfault.dev>. GitHub Pages must use **GitHub Actions** as its source; DNS must point the `xmlsquish` CNAME at the repository owner's actual `<username>.github.io`, then the custom domain and HTTPS should be confirmed in Pages settings.

Licensed under [`GPL-3.0-or-later`](LICENSE). The site uses [MoeSegfault Style](https://github.com/kleedaisuki/moesegfault-style), and its project SVG adapts that established visual motif.
