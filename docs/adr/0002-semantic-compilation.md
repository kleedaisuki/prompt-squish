# ADR 0002：两阶段语义编译 / Two-stage semantic compilation

- 状态 / Status：已接受 / Accepted
- 日期 / Date：2026-09-06
- 范围 / Scope：CLI 新流程；保留 ADR 0001 的底层 `squish` 契约。New CLI pipeline; the low-level `squish` contract in ADR 0001 remains unchanged.

> 后续修订 / Revision：处理指令字段已从 `file` 迁到 `meta`，并支持逐边 `openat` 继承、`ifr`、`insert` 与分阶段统计。以下相关段落记录初版决策，当前规则以 [ADR 0003](0003-metadata-inheritance-and-stage-metrics.md) 为准。PI fields have moved to `meta`; ADR 0003 supersedes the initial namespace and report rules below.

## 决策 / Decision

```text
source.xml -> semantic compiler -> source.i.xml -> lexical squasher -> source.o.xml
                                  -I stops here                      -O default
```

将结构与执行语义放在编译器中，避免在空白有限状态机（Finite-State Machine, FSM）中添加宏特判。编译器保留普通标签的原始字节及文本布局，不通过树序列化重新格式化整个文档。底层词法 API 与旧批处理 API 保持原行为；CLI 默认改为新流程。

Keep structural and execution semantics in a compiler, not special cases inside the whitespace FSM. Preserve ordinary markup bytes and text layout rather than reformatting the whole document through tree serialization. The low-level lexical API and legacy batch API retain their behavior; the CLI defaults to the new pipeline.

## 语言与环境 / Language and environments

| 语法 / Syntax | 契约 / Contract |
| --- | --- |
| `<?xmlsquish ...?>` | 属性按顺序定义为 `file` 元信息；空 PI 无操作。Attributes define file metadata in order; empty PI is a no-op. |
| `version="adaptive"` | 支持当前语言；其他版本值报错。Select the current language; other version values are errors. |
| `warnings="strict"` | 严格诊断，不容错恢复；其他值报错。Strict diagnostics without warning recovery; other values are errors. |
| `xmlsquish:let` | 属性按顺序定义局部变量。Attributes define local variables in order. |
| `xmlsquish:set` | 属性按顺序给当前文件已声明局部变量赋值；不创建变量，也不修改内置命名空间。Assign previously declared current-file locals in attribute order; never create variables or mutate built-in namespaces. |
| `xmlsquish:log msg="…"` | 日志定位到物理文件与宏行号。Log with physical filename and macro line. |
| `xmlsquish:if lhs="…" rhs="…"` | 精确字符串相等，执行所选分支。Exact string equality; evaluate the selected branch. |
| `xmlsquish:ifn lhs="…" rhs="…"` | 精确字符串不等。Exact string inequality. |
| `xmlsquish:mount path="…"` | 相对包含该宏的文件加载并编译目标，接入目标根。Load relative to the containing file, compile, and insert the target root. |
| `xmlsquish:import path="…"` | 同上，但只接入目标根的内容。As above, but insert only the root's contents. |

`xmlsquish:` 是固定内置前缀，不要求用户声明 `xmlns:xmlsquish`，也不按任意命名空间 URI 别名识别宏。这是 XML 形状的编译语言，不宣称所有源文档都是符合 XML Namespaces 标准的文档。

`xmlsquish:` is a fixed built-in prefix. It does not require an `xmlns:xmlsquish` declaration and is not matched through arbitrary namespace URI aliases. This XML-shaped source language does not claim all inputs conform to the XML Namespaces standard.

`import` 有意移除根的全部属性，包括命名空间声明及继承属性。需要这些属性的子树必须在自身声明，或改用 `mount`；编译器不进行命名空间修复（namespace fixup）。

`import` intentionally removes all root attributes, including namespace declarations and inherited attributes. Subtrees relying on them must declare them locally or use `mount`; the compiler does not perform namespace fixup.

每次文件接入创建独立文件环境（File Frame）：局部变量与文件元信息互不泄漏。`if` 不创建块作用域（block scope），因此执行分支内定义在随后可见，重复定义仍报错。再次接入相同文件时重新执行其宏；不使用“全局已访问集合”跳过合法的重复接入。

Each inclusion creates an independent file frame: locals and file metadata do not leak. Conditions do not introduce block scope, so selected definitions remain visible afterward and duplicate definitions still fail. Repeated inclusion evaluates macros again; a global visited set must not suppress valid repeated includes.

编译器实例固定系统和环境快照（snapshot），CLI 每次批量运行只创建一个实例。快照保证一次运行内一致，而非跨运行可复现：显式依赖时间、环境及被引用文件的内容仍会影响结果。

Each compiler instance fixes system and environment snapshots; the CLI uses one instance per batch. Snapshots guarantee within-run consistency, not cross-run reproducibility: time, environment, and included file contents remain inputs.

`sys:time` 为 Unix 时间戳秒数字符串。变量名使用 ASCII 字母或下划线起始，后续可含数字、连字符和点；引用按最长合法名字匹配，`$$` 表示字面美元符号，插入的值不会再次展开。文件内置键包括 `name`、`path`、`dir`；系统键包括 `platform`、`os`、`arch`、`time`。

`sys:time` is a Unix timestamp in seconds represented as a string. Names begin with an ASCII letter or underscore and may subsequently contain digits, hyphens, and dots. References consume the longest valid name; `$$` escapes a literal dollar, and inserted values are not recursively expanded. Built-in file keys are `name`, `path`, and `dir`; system keys are `platform`, `os`, `arch`, and `time`.

环境变量键在 Windows 上按 ASCII 大小写不敏感匹配，其他平台区分大小写。当前保护限制为每份输入最多 256 层 XML 嵌套，执行路径跨文件、元素和宏累计最多 128 层；超过限制返回诊断，不以扩大线程栈掩盖问题。

Environment keys use ASCII case-insensitive comparison on Windows and case-sensitive comparison elsewhere. Current safeguards allow at most 256 XML nesting levels per input and 128 cumulative rendering levels across files, elements, and macros; exceeding a limit returns a diagnostic instead of hiding the issue with a larger thread stack.

变量只在编译期参数和元信息中展开。普通文本、CDATA、普通元素属性不是模板。缺失环境变量不是空字符串。未选择条件分支不执行变量求值、日志或文件加载，但 XML 结构仍必须正确。

Variables expand only in compile-time parameters and metadata. Ordinary text, CDATA, and ordinary attributes are not templates. An absent environment variable is not an empty string. Unselected branches perform no variable evaluation, logging, or file loading, but XML structure must still be correct.

## 输出及兼容性 / Output and compatibility

- 移除注释、XML 声明和所有处理指令（Processing Instruction, PI），只解释精确的 `xmlsquish` PI；其他 PI 不作为宏执行。Remove comments, XML declarations, and all PIs; interpret only exact `xmlsquish` PIs.
- 普通标签和 CDATA 保留；DTD 不加载外部实体。Ordinary markup and CDATA remain; DTDs do not load external entities.
- 主输入允许原有的多根提示词片段；被接入文件必须有唯一根，以明确 `mount` / `import` 的含义。Top-level input may retain legacy multi-root prompt fragments; included files require a single root for unambiguous mount/import behavior.
- `-I` 写中间表示（Intermediate Representation, IR），不改写已有 `.o.xml`。`-I` writes IR without changing an existing optimized artifact.
- `-O` 写 IR 后压缩，最终输出持久化成功后只删除对应 IR。失败不删除其他文件，也不保证恢复失败前的 IR。`-O` writes IR, optimizes, then removes only its corresponding IR after final persistence succeeds. Failure does not delete unrelated files or promise to restore the previous IR.
- 原文件不覆盖；生成物原子替换（atomic replacement）；发现阶段忽略 `.i.xml` 和 `.o.xml`，包括大小写变体。Sources remain untouched; generated files are atomically replaced; discovery skips intermediate and optimized suffixes case-insensitively.

## 统计与安全 / Measurements and safety

字符与 Token 统计比较主输入和最终选定产物；空白账本只衡量 IR 到优化产物的变换。包含大量外部内容时，输出可能更大。不能把宏消除、注释移除或文件接入宣称为空白压缩收益。

Character and token totals compare the primary input with the selected final artifact. Whitespace counters measure only the IR-to-optimized transformation. Includes can make output larger; macro removal, comment removal, and includes are not whitespace savings.

仅编译可信源。文件接入和 `env` 访问是授权的本地功能，不是安全沙箱（sandbox）。不要用正则替换整个文档执行变量插值，也不要启用外部实体解析。递归引用应诊断为错误，而非栈溢出。日志可能包含环境变量的敏感值。

Compile trusted sources only. Local includes and environment access are intentional capabilities, not a sandbox. Do not interpolate the whole document using regexes or enable external entity resolution. Diagnose recursive includes instead of overflowing the stack. Logs may contain sensitive environment values.

## 外部依据及后续方向 / Evidence and future directions

[XML 1.0](https://www.w3.org/TR/xml/#sec-pi) 区分声明、PI 和普通内容；[quick-xml 的配置文档](https://docs.rs/quick-xml/latest/quick_xml/reader/struct.Config.html)说明结构检查需要明确的解析器策略。这支持将结构解析和保持字节的空白处理分开，但并不证明本实现是完整 XML 验证器。

[XML 1.0](https://www.w3.org/TR/xml/#sec-pi) distinguishes declarations, PIs, and content; [quick-xml configuration](https://docs.rs/quick-xml/latest/quick_xml/reader/struct.Config.html) documents explicit structural checking policies. These support separate structural and lexical stages, not a claim that this implementation is a full XML validator.

成熟配置语言 [Dhall 的导入完整性检查](https://docs.dhall-lang.org/tutorials/Language-Tour.html)提供可追踪依赖的参考；研究 [Reproducible Builds](https://arxiv.org/abs/2104.06020)强调生成物与输入之间可核验的联系。未来可按实际需要添加依赖清单、显式快照输入和内容哈希；当前不引入缓存、远程导入或复杂类型系统，也不声称已实现可复现构建。

[Dhall import integrity checks](https://docs.dhall-lang.org/tutorials/Language-Tour.html) provide a mature dependency-tracking reference; [Reproducible Builds](https://arxiv.org/abs/2104.06020) motivates verifiable links between artifacts and their inputs. Dependency manifests, explicit snapshot inputs, and content hashes remain possible future work. This change does not add caching, remote imports, or an elaborate type system, and does not claim reproducible builds.
