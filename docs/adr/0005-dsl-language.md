# ADR 0005: Namespace-aware immutable DSL / 命名空间与不可变 DSL

- 状态 / Status: Accepted
- 规范 / Normative contract: [docs/dsl.md](../dsl.md)
- 替代 / Supersedes: ADR 0002 的旧宏语言、ADR 0003 的隐式元数据继承；ADR 0001 的最终产品空白压缩保持。ADR 0004 的单 package 布局继续适用。 / Supersedes the legacy macro language, implicit metadata inheritance, while retaining final product whitespace squashing and ADR 0004's package layout.

## 问题 / Context

旧实现混合物理文件、动态元数据与执行顺序，难以定义命名、重复装载和递归。XML 混合内容的空白压缩还会改变用户文本。新规范优先定义身份与值域，再选择实现。

The previous implementation conflated physical files, dynamic metadata, and execution order. Whitespace squashing also changed mixed-content meaning. The new language defines identity and value domains before implementation.

## 决定 / Decision

1. 按规范 URI 冻结源码单元（SourceUnit），装载完整静态闭包；不按内容或 symlink 真实路径合并。 / Freeze sources by logical canonical URI and discover the complete static closure, not physical/content identity.
2. 按 XML 扩展名（Expanded Name）注册不可变宏。前缀只是别名，重复定义报错。 / Register immutable macros by expanded name; prefixes are aliases and redefinitions fail.
3. 每次调用创建展开帧（Expansion Frame）；字符串参数和 XML slot 显式传递，在调用方恰好求值一次。 / Each call creates a frame; scalar arguments and XML slots are evaluated once in the caller and passed explicitly.
4. 定义位置绑定相对引用，标量仅有只读 `file.*`、`arg.*`、词法 `match.*`。 / Resolve references at definition sites with immutable lexical scalar bindings.
5. 区分有限源码装载环和实际执行递归；预算属于调用配置，不属于语言常数。 / Distinguish source-loading cycles from execution recursion; budgets are invocation options, not language constants.
6. `.i.xml` 携带来源信息（provenance），lowering 清除内部信息并保留用户 XML 文本与命名空间，然后最终 `.o.xml` 产品阶段执行 `squish`。 / Intermediate output carries provenance; lowering preserves user XML, then the final `.o.xml` product pass squishes whitespace.

## 外部证据与实现选择 / Evidence and implementation choices

[W3C Namespaces in XML](https://www.w3.org/TR/REC-xml-names/) 将扩展名定义为命名空间名和局部名的二元组。采用成熟的命名空间感知解析器，比按原始前缀识别操作更可靠；移除模块容器后，序列化器必须保持用户节点的命名空间绑定。具体内部树与来源序列化格式是实现选择，不是 W3C 或 DSL 额外要求。

W3C defines expanded names as namespace/local-name pairs. A namespace-aware parser avoids raw-prefix identity errors; serialization must preserve bindings after removing module containers. Internal tree and provenance encodings remain implementation choices.

生产选择是 Rust [`regex`](https://docs.rs/regex/latest/regex/) 的成熟非回溯匹配能力。单次搜索的最坏复杂度是 `O(m*n)`（模式规模与输入长度）；这不是整个递归程序的线性保证，仍需展开与输出预算。DSL 额外禁止位置捕获，要求静态检查；不能把库接受的全部语法自动视为语言规范。

The production choice is Rust regex's mature bounded-search behavior: a single search is `O(m*n)` in pattern and input size. This does not bound an entire recursive program; expansion/output guards remain necessary. DSL-specific capture restrictions require additional validation rather than accepting every library feature.

研究前沿表明，部分扩展能力并非必然要求灾难性回溯：[PLDI 2024, Linear Matching of JavaScript Regular Expressions](https://aurele-barriere.github.io/papers/linearjs.pdf) 探索了包括环视（look-around）在内的线性匹配。当前语言仍明确排除环视与反向引用（backreference），原因是范围与成熟实现契约，而非声称所有这些特性在理论上不可能高效实现。若未来改变，应先修改规范、明确捕获语义并提供对抗测试，而不是仅更换引擎。

Frontier research explores linear matching for richer JavaScript features, including look-around. The DSL intentionally excludes look-around and backreferences for scope and a mature implementation contract, not because every such feature is theoretically incompatible with efficient matching. Reconsideration needs a specification change, explicit capture semantics, and adversarial tests, not merely an engine swap.

## 迁移 / Migration

| 旧形式 / Legacy | 新形式 / Replacement |
| --- | --- |
| 固定 `xmlsquish:` 前缀 / Fixed prefix | 绑定 builtin URI 的任意前缀 / Any prefix bound to builtin URI |
| `mount path` / wrapper rename | `mount src` 与显式 XML 包装 / Explicit XML wrapper |
| `import` 插入子内容 / Inline child contents | `import` 仅定义；`mount` 调用 `main` / Definition import or main mount |
| `let`、`set`、`meta`、`openat`、`$...` | 必需 `param`、显式 `arg` / Required parameters and explicit arguments |
| 位置 regex capture / Positional captures | 命名 capture / Named captures |
| include cycle error | 源码驻留去重与执行预算 / Source interning and execution budgets |
| 压缩最终 XML / Squashed final XML | 保留：先语义 lowering，再产品压缩 / Retained: semantic lowering then product compression |

不提供旧语言兼容层。历史 ADR 保留用于追溯，不得反过来覆盖 `docs/dsl.md`。编译/lowering 的文本保留契约不延伸到最终产品压缩：`squish` 会改变混合内容空白，这是固定产品行为。

No legacy-language compatibility layer is provided. Historical ADRs remain for traceability and cannot override the normative DSL. The compilation/lowering text-preservation contract does not extend to final product compression: squishing mixed-content whitespace is fixed product behavior.

## 当前实现边界 / Current implementation boundaries

当前加载器仅支持可表示为本机路径的 `file:` URI，使用 `url` crate 解析相对引用并规范化 URI；不支持其他 scheme、query 或 fragment。路径规范化不解引用 symlink。DSL 的抽象源码模型不因此承诺 HTTP 等加载能力；未来增加 scheme 必须单独定义稳定身份与加载契约。

The current loader supports only `file:` URIs representable as native paths, resolving and canonicalizing references through the `url` crate. Other schemes, queries, and fragments are rejected; normalization does not dereference symlinks. The abstract source model does not imply HTTP support. Additional schemes need explicit stable-identity and loading contracts.

宏运行时使用显式任务栈（explicit task stack），不依赖本机调用栈表达宏递归。DSL 抽象语法树构建使用 [`stacker`](https://docs.rs/stacker/latest/stacker/) 按需增长的分段本机栈（segmented native stacks），而不是人为固定语法嵌套上限；抽象语法树（Abstract Syntax Tree, AST）销毁则用迭代方式拆解子节点所有权，避免深树递归析构。分段大小是实现参数，不是语言深度常数；可用内存和平台仍是实际限制。

Macro execution uses an explicit task stack rather than native recursion. DSL AST lowering uses `stacker` to grow segmented native stacks on demand instead of imposing a fixed syntax-depth limit. AST destruction iteratively detaches child ownership to avoid recursive teardown of deep trees. Stack segment sizes are implementation parameters, not language depth constants; available memory and platform support remain practical limits.


XML 解析依赖限定在 `roxmltree = "0.18"` 版本系列，锁文件采用其迭代式外部 `xmlparser` tokenizer。实现代理的本机复现实验中，0.20/0.21 的递归 tokenizer 在 6000 层嵌套 XML 上、进入 DSL 栈增长保护之前发生本机栈溢出；0.18.1 与 `xmlparser` 0.13.6 通过该场景，且所需 API 足够。这是针对已测版本的工程选择，不推断所有后续版本都存在同一缺陷。未来升级必须先通过深层 XML 回归；`stacker` 仅保护 DSL AST 构建，不能补救其之前的 tokenizer 栈溢出。

The XML parser dependency is restricted to the `roxmltree = "0.18"` series, using its external iterative `xmlparser` tokenizer. In the implementation agent's local reproductions, versions 0.20/0.21 overflowed the native stack while tokenizing XML nested 6000 levels deep, before DSL stack-growth hooks could run. Version 0.18.1 with `xmlparser` 0.13.6 passed that case and provides the required API. This decision concerns tested versions, not a claim that all future releases are defective. Upgrades must pass deep-XML regressions first: `stacker` protects DSL AST lowering, not preceding tokenizer recursion.

`CompileOptions.args` 提供入口参数；`max_depth`、`max_expansions`、`max_output_bytes` 提供运行预算。输出字节预算分别检查各个临时与最终语义 XML 缓冲区，不是所有活动缓冲区内存之和，也不包含来源中间表示的全部开销。因此它不是进程总内存限制；宿主面对不可信输入仍需额外的文件访问控制与进程资源隔离。

`CompileOptions.args` supplies entry arguments; `max_depth`, `max_expansions`, and `max_output_bytes` configure invocation budgets. The byte guard checks each temporary and final semantic XML buffer separately, not aggregate live-buffer memory or all provenance-IR overhead. It is not a process memory limit; hosts handling untrusted inputs still need filesystem access control and process resource isolation.

## 验证与风险 / Validation and risks

重点测试命名空间重绑定、重复定义双位置、静态死分支验证、导入环、参数/slot 契约、capture 作用域、递归预算、来源链以及混合内容保真。默认文件加载器不是沙箱；预算不是对任意恶意输入的完整 CPU/内存隔离。公共文档与站点必须展示同一种实际实现语言。

Test namespace rebinding, duplicate-definition locations, inactive-branch validation, import cycles, argument/slot contracts, capture scope, guards, provenance chains, and mixed content. The default loader is not a sandbox, and budgets are not complete isolation from arbitrary hostile input. Documentation and the site must demonstrate the language actually implemented.
