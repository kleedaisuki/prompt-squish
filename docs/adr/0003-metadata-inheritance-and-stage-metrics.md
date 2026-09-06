# ADR 0003：元数据继承、显式插入与分阶段统计
# Metadata inheritance, explicit insertion, and stage metrics

- 状态 / Status：已接受 / Accepted
- 日期 / Date：2026-09-06
- 替代范围 / Supersedes：[ADR 0002](0002-semantic-compilation.md) 中处理指令字段属于 `file` 的规则及旧 CLI 统计展示；其余规则不变。Replaces the PI-to-`file` mapping and CLI report presentation in ADR 0002; other contracts remain.

## 1. 命名空间与引用边 / Namespaces and include edges

| 名称 / Name | 所有者与行为 / Ownership and behavior |
| --- | --- |
| locals | `let` 声明，`set` 赋值；仅当前文件可见。Declared by `let`, assigned by `set`, visible only in the current file. |
| `file` | 当前物理文件的 `name/path/dir`；不可继承或覆盖。Physical source `name/path/dir`; never inherited or overwritten. |
| `meta` | `<?xmlsquish ...?>` 声明的字段，可以从父文件覆盖合并。PI-defined fields, optionally overlaid by parent metadata. |
| `sys` / `env` | 一次编译实例的只读快照。Read-only snapshots for one compiler instance. |

每条 `mount` / `import` 引用边独立读取 `openat`：省略或 `self` 表示无父元数据；`parent` 表示获取调用时父文件的有效 `meta` 快照。模式不自动沿树传播，但合并后的值可以经过连续显式 `parent` 边传播。路径始终相对发出该宏的物理文件解析；`openat` 是本语言的元数据策略，不是操作系统 `openat` 系统调用或路径重定位。

Each `mount` / `import` edge independently interprets `openat`: omission or `self` means no inherited metadata; `parent` snapshots the caller's effective metadata at the include. The mode never propagates implicitly, but merged values can travel through consecutive explicit `parent` edges. Paths always resolve relative to the physical source containing the macro. This is a language-level metadata policy, not the OS `openat` syscall or path relocation.

令 `D` 为当前文件已执行的本地元数据声明，`P` 为该引用边传入的父元数据，`E` 为查询所见的有效元数据：

```text
E = D ⊕ P                       # 同名键右方优先 / right side wins
P_child = E_parent              # openat="parent"
P_child = {}                    # omitted or openat="self"
```

父方独有字段新增；子方独有字段保留；冲突父方优先。`D` 与 `P` 分开存储：子文件声明一个已继承的字段不是重复定义，但子文件自身声明同名字段两次仍报错。处理指令从左到右执行，表达式查询当前 `E`；即使声明值将被父值覆盖，其表达式仍会求值和检查未定义引用。`version` / `warnings` 的支持模式检查使用有效值。后来的父声明不追溯影响已完成的接入。

Parent-only fields are added, child-only fields retained, and collisions favor the parent. Keeping `D` and `P` separate permits a child declaration to coexist with an inherited field while still rejecting duplicate child declarations. PIs execute left-to-right and expressions read current `E`; even a shadowed declaration evaluates its expression and checks undefined references. Supported `version` / `warnings` modes are validated against the effective value. Later parent declarations do not retroactively alter completed includes.

这是明确的语言迁移：PI 中 `author` 现在读作 `$meta:author`，不再读作 `$file:author`。不增加模糊的兼容别名，避免把可继承业务字段再次混入物理身份。底层 `squish`、公开编译结果类型和旧应用批处理 API 不变。

This is an explicit language migration: PI `author` is now `$meta:author`, not `$file:author`. No ambiguous alias mixes inherited application fields back into physical identity. The low-level squasher, public compilation result type, and legacy application batch API remain unchanged.

### 根重命名 / Root renaming

`<xmlsquish:mount path="child.xml" rename="Persona"/>` 可重命名接入根；省略 `rename` 时不变。参数在调用方展开，故可与 `openat="parent"` 和 `rename="$meta:tag"` 组合。只替换已解析根节点起始、结束标签的名称区间，自闭合形式也保留；不对输出做全局文本替换，不改属性、子树、文件名或元数据。`import` 不保留根，因而拒绝此参数。

`mount` optionally renames the included root through `rename="Persona"`; omission is unchanged. Expansion occurs in the caller, so `rename="$meta:tag"` composes with `openat="parent"`. Replace only parsed root name spans in start/end tags, retaining self-closing form. Never globally replace output text or modify attributes, descendants, physical filenames, or metadata. `import` rejects the parameter because it discards the root.

新名称按 [XML 1.0 名称字符](https://www.w3.org/TR/xml/#NT-NameStartChar)与 [QName 语法](https://www.w3.org/TR/xml-names/#ns-qualnames)验证，支持 Unicode；空名、非法字符、多个冒号，以及 `xmlsquish:` / `xmlns:` 保留前缀报调用点错误。前缀声明沿用现有内容，不执行命名空间修复；此处保证名称语法，不保证完整的 XML 命名空间有效性。

Validate the expanded name against [XML 1.0 name characters](https://www.w3.org/TR/xml/#NT-NameStartChar) and [QName syntax](https://www.w3.org/TR/xml-names/#ns-qualnames), including Unicode. Empty names, illegal characters, multiple colons, and reserved `xmlsquish:` / `xmlns:` prefixes fail at the call site. Namespace declarations remain untouched: validation covers name syntax, not full namespace validity.

## 2. 正则条件与文本插入 / Regex conditions and text insertion

```xml
<?xmlsquish audience="researchers"?>
<prompt>
    <xmlsquish:ifr str="$file:name" pattern="\Aprompt\.xml\z">
        <audience><xmlsquish:insert get="meta:audience"/></audience>
    </xmlsquish:ifr>
</prompt>
```

`ifr` 的 `str` 是可展开字符串，`pattern` 是原样正则表达式（regular expression），两者都先执行 XML 属性实体解码。正则参数不执行 `$变量` 插值，避免破坏标准末尾锚点（anchor）。使用 Rust `regex` 的单次 `is_match` 查找，默认无需匹配整个字符串；`\A…\z` 要求全串匹配。子内容只在命中时执行；被外层条件跳过的非法模式不会编译。执行到的非法模式、资源限制或缺失参数都有源文件行号诊断。

`ifr` takes an expandable `str` and a literal regex `pattern`; both undergo XML attribute entity decoding first. Patterns do not interpolate variables, preserving standard dollar anchors. Rust `regex::is_match` performs a single search; `\A…\z` requests a whole-string match. Children execute only on a match; an invalid pattern under an unselected outer branch is not compiled. Reached invalid patterns, resource limits, and missing parameters produce source-located diagnostics.

选择有限自动机（finite automaton）路线而非回溯正则引擎；不支持回溯引用（backreference）与环视（look-around）。当前编译正则大小上限为 10 MiB，DFA 缓存上限为 2 MiB；这不是对进程总内存或总耗时的保证。具体复杂度与限制依据 [Rust regex 文档](https://docs.rs/regex/latest/regex/)；不宣称对任意输入都“零成本”。

Use an automata-based engine rather than backtracking; backreferences and look-around are unsupported. Compiled regex size is limited to 10 MiB and the DFA cache to 2 MiB, not a guarantee on total process memory or wall time. Complexity and limitations follow the [Rust regex documentation](https://docs.rs/regex/latest/regex/), not a claim of zero-cost matching.

`insert get="name"` 按一个不带 `$` 的名字查找变量，支持 `meta:author`、`file:name` 等限定名。值中的 `&<>` 被转义；XML 1.0 禁止的字符被拒绝；不重新解释成标签、宏或变量引用。普通内容仍不自动插值。`-I` 保留插入文本的空白，`-O` 继续遵循既有空白规范化规则。

`insert get="name"` looks up one bare variable name, including qualified names such as `meta:author` or `file:name`. Escape `&<>`, reject XML 1.0-forbidden characters, and never reinterpret values as markup, macros, or references. Ordinary content still does not interpolate automatically. `-I` retains inserted whitespace; `-O` applies the existing whitespace normalization contract.

安全文本边界依据 [XML 1.0 字符数据规则](https://www.w3.org/TR/xml/#syntax)。这防止插入值变成编译语法，但不是对提示词注入（prompt injection）的防御：插入的自然语言仍可能影响下游模型。

The text boundary follows [XML 1.0 character-data rules](https://www.w3.org/TR/xml/#syntax). It prevents inserted values from becoming compiler syntax, not prompt injection: inserted natural language can still influence a downstream model.

## 3. 有可解释基线的统计 / Measurements with an interpretable baseline

先问三个不同的问题：主文件有多大？引用组装后的提示词有多大？相同组装结果经过空白规范化实际少了多少 Token？不能拿很短的入口文件与很大的展开结果直接算“压缩率”。

Ask three distinct questions: how large is the primary source, how large is the assembled prompt, and how many tokens does normalization save on that same assembled content? Comparing a tiny entry file directly with its much larger assembled output is not a meaningful compression rate.

| 观测 / Observation | 定义 / Definition |
| --- | --- |
| Source / IR / Final | 各阶段文本 Token 数及 UTF-8 字节，排除 BOM。Stage token counts and UTF-8 bytes, excluding BOM. |
| Assembly ratio | `IR tokens / primary source tokens`；零分母 `N/A`。Zero denominator yields `N/A`. |
| Optimization savings | `IR - final`；增加时显示 `added`，不显示负节省率。Show `added` for growth, not negative savings. |
| Dependency loads / unique files | 成功主输入触发的加载次数与全批次去重的依赖物理路径。Loads for successful inputs and batch-wide distinct physical dependency paths. |
| Dependency bytes read | 加载调用读入的文本字节，重复计入；不是最终接入的字节。Bytes returned by dependency loads, counting repetitions; not bytes retained in output. |

所有大小与依赖汇总仅包括最终成功的主输入；发现/处理失败另计。批次比例基于汇总后的 Token 数，不平均每文件百分比。`-I` 显示优化未执行，无成功文件或零基线显示 `N/A`。不使用 `max(0, savings)` 掩盖真实增长。原有字符数与底层空白账本仍可用于回归验证。

Size and dependency totals include only successful final artifacts; discovery/processing failures are separate. Batch ratios use aggregated tokens, not average per-file percentages. `-I` reports optimization not run; no successes or zero baselines yield `N/A`. Do not hide growth with `max(0, savings)`. Existing character counts and low-level whitespace accounting remain available for regression checks.

固定 `o200k_base` 支持可比较的文本度量，不保证与每个目标模型实际计费相同。近期[跨语言提示词压缩审计](https://arxiv.org/abs/2608.26175)把预算匹配到目标模型分词器，并分别评估任务表现；它提供的研究启示是“Token 数”和“质量”必须分开观察，而非证明本工具改善质量。本轮不引入有损压缩、成本估算或速度承诺。

Fixed `o200k_base` supports comparable text measurements, not exact billing for every target model. A recent [cross-lingual prompt-compression audit](https://arxiv.org/abs/2608.26175) matches budgets to target tokenizers and separately evaluates task performance. The design inference is to measure size and quality separately, not evidence that this tool improves quality. This change adds no lossy compression, cost estimates, or speed promises.
