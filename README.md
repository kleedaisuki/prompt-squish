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
xmlsquish [-I | -O] [PATH]...
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

完整可运行示例见 [examples/semantic](examples/semantic/README.md)，语义及兼容边界见 [ADR 0002](docs/adr/0002-semantic-compilation.md)。

| 编译期语法 | 行为 |
| --- | --- |
| `<?xmlsquish author="klee"?>` | 将属性定义到当前文件的 `file` 命名空间；空指令不做事 |
| `<xmlsquish:let msg="Hello" dir="parts"/>` | 按声明顺序定义当前文件变量；重复定义报错 |
| `<xmlsquish:set msg="Updated"/>` | 按属性顺序给当前文件已声明变量赋值；未声明报错，不修改内置命名空间 |
| `<xmlsquish:log msg="$msg"/>` | 输出 `文件名:行号: 信息` |
| `<xmlsquish:if lhs="$sys:platform" rhs="win32">…</xmlsquish:if>` | 字符串相等时执行内部内容；移除条件包装标签 |
| `<xmlsquish:ifn lhs="…" rhs="…">…</xmlsquish:ifn>` | 字符串不等时执行内部内容 |
| `<xmlsquish:mount path="$dir/hello.xml"/>` | 相对当前物理文件解析路径，递归编译并接入目标根 |
| `<xmlsquish:import path="$dir/another.xml"/>` | 递归编译，接入目标根的内容，不保留根包装 |

每个文件拥有独立文件环境（File Frame）：局部变量、`file` 元信息、只读 `sys` 和只读 `env`。条件不新建环境，引用文件不继承或导出局部变量。`$file:name` 是当前文件名；`$sys:platform` 在 Windows 上为 `win32`；`$sys:time` 在一次 CLI 编译运行内固定。`$env:NAME` 引用环境变量，不存在时同样报未定义。

**变量只在编译期语法中展开**，普通 XML 文本、CDATA 和普通属性里的 `$msg` 保留原样。未选择分支不执行宏，不读取其引用文件。循环引用、执行路径上的未定义变量、重复定义和未知宏均报错。XML 声明不作为 `xmlsquish` 指令处理。

源文件与引用文件应受信任：宏可以读取本地文件和环境变量，日志也可能输出敏感值；这不是不可信模板的安全沙箱。

### 退出码与错误继续

| 退出码 | 含义 |
| ---: | --- |
| `0` | 帮助/版本请求，或所有发现的文件均成功 |
| `1` | 至少一个发现或文件处理错误；其他独立文件仍会继续 |
| `2` | 命令行语法错误 |

发现、读取、FSM 扫描、Token 计数和写入错误会写入标准错误，并带路径、阶段与原因。汇总仍写入标准输出；失败文件不计入成功文件的字符/Token/空白总数。

## 统计口径

默认 Tokenizer 固定为 [`o200k_base`](https://github.com/openai/tiktoken)，以便运行间可复现。它统计文件逻辑文本，不包含聊天 API 的角色或消息封装。

| 输出字段 | 定义 |
| --- | --- |
| `Encoding` | Tokenizer 编码；当前固定为 `o200k_base` |
| `Processed files` | 输入发现阶段得到的唯一 XML 文件数 |
| `Succeeded` / `Failed` | 成功写出数 / 发现与处理失败总数 |
| `Discovery errors` | 无效/空匹配 glob、不可访问路径或遍历错误数 |
| `Input/Output tokens` | 成功文件转换前/后的 `o200k_base` Token 总数 |
| `Input/Output characters` | 去 BOM 后的 Unicode 标量值总数，不是 UTF-8 字节或字素簇数 |
| `Recognized whitespace` | 空白压缩阶段输入（IR）中的 XML `S` 数；`-I` 不执行该阶段 |
| `Removed whitespace` | 从字符账本中消去的 XML `S` 槽位数；atom 间游程会复用一个槽位作为规范化空格 |
| `Inserted whitespace` | 原本直接相邻的两个 atom 之间新增的 U+0020 数 |
| `Token compression rate` | `1 - output_tokens / input_tokens`；输入 Token 为零时显示 `N/A`，输出更大时可以为负 |

空白账本仅对空白压缩阶段满足（不把宏消除或文件接入算作空白压缩）：

```text
optimized_characters = intermediate_characters - removed_whitespace + inserted_whitespace
```

注意，`recognized` 不等于 `removed`：标签、属性、注释、CDATA、PI 和 DOCTYPE 内的 XML 空白会被识别但受到保护；两个 atom 之间已有的一个分隔空格也会保留。

例如，对内容为 ` <a>\n  x\n</a><b/> ` 的 `sample.xml` 运行后，实际报告为：

```text
Encoding: o200k_base
Measurements: original source -> selected output; whitespace: IR optimization only
Processed files: 1
Succeeded: 1
Failed: 0
Discovery errors: 0
Input tokens: 12
Output tokens: 9
Input characters: 18
Output characters: 15
Recognized whitespace: 6
Removed whitespace: 4
Inserted whitespace: 1
Token compression rate: 25.00%
```

对应输出是 `<a> x </a> <b/>`。

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

### Compile-time language

`<?xmlsquish ...?>` defines `file` metadata; XML declarations are ignored as compiler instructions. `let` declares file-local variables; `set` assigns already declared locals in attribute order (undefined targets and built-in namespace writes are errors). `log` prints `filename:line: message`, `if` / `ifn` compare strings for equality / inequality, `mount` includes a compiled root, and `import` includes its contents without the wrapper. All macros use the literal `xmlsquish:` prefix. Include paths are relative to the physical file containing the macro.

Each file has an independent frame with locals, `file` metadata, read-only `sys`, and read-only `env`. Conditions share their file's frame; includes neither inherit nor export locals. `$file:name` is the source basename, `$sys:platform` is `win32` on Windows, and `$sys:time` is fixed for one CLI run. Undefined references (including absent environment variables), duplicate definitions, unknown macros, and include cycles are errors. Unselected branches do not execute macros or load includes.

Expansion is restricted to macro parameters and xmlsquish metadata. Ordinary XML attributes, text, and CDATA keep `$variables` literally. Compilation removes macros, comments, and metadata before squashing. See the [runnable example](examples/semantic/README.md) and [bilingual semantic contract](docs/adr/0002-semantic-compilation.md). Compile trusted sources only: local includes and environment access are not a sandbox, and logs can expose secrets.

Only the four XML `S` characters—space, tab, carriage return, and line feed—are canonicalized. Markup interiors are preserved. Input must be strict UTF-8; an optional UTF-8 BOM is excluded from all measurements and preserved on output.

The report uses the fixed `o200k_base` tokenizer. Character counts are Unicode scalar values. Input/output character and token totals compare the original source against the selected final artifact. Whitespace measurements apply only to the IR-to-optimized stage, not macro removal or included files; `-I` does not run that stage. Token compression is `1 - output_tokens / input_tokens`, or `N/A` for zero input tokens. Includes can make output larger. The existing `xmlsquish_core::squish` API retains the pure lexical contract in [ADR 0001](docs/adr/0001-lexical-canonicalization-and-layering.md).

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
