# ADR 0007: Unified macro expansion / 统一宏展开

- Status / 状态：Accepted
- Supersedes / 替代：ADR 0005 中隐式模块正文、`main`、`call` 与 `mount` 的设计。
- Normative contract / 规范契约：[DSL](../dsl.md)。

## Problem / 问题

原模型将文件同时当作定义容器和隐式可执行宏，因而需要 `import`、`mount`、`call` 三套入口概念。片段又容易被误认为另一种可调用定义。用户编写提示词时不得依赖这些实现层特殊情况。

The previous model made a file both a declaration container and an implicitly executable macro. Separate import, mount, and call mechanisms obscured the distinction between definitions and expanded data.

## Decision / 决定

1. 模块（Module）仅包含静态 `import` 与零个或多个命名宏（Macro）。不接受模块正文、顶层参数或嵌套宏。
2. 独立源码根 `xs:entry` 包含 import、可选入口参数和输出构造正文，不得定义宏或根 slot。模块没有 entry 属性、正文或入口参数。入口是构造上下文，不是符号、宏或 MacroDefId；命令行参数仅绑定入口上下文。import 只接受模块，不能接受入口。
3. `xs:expand ref="prefix:name"` 是唯一递归展开操作。删除 `call`、`mount`，不设兼容别名。片段（Fragment）只是结果节点序列，不引入 `xs:fragment`。
4. 每次展开创建独立作用域。参数与 fill 在展开者环境中先求值一次，再按不可变值传入；不继承外部参数、捕获或 slot。定义位置的 `file.*` 固定不变。
5. 宏返回有序 XML 节点序列。纯文本结果可在 `xs:arg` body 中连接成字符串；结构结果可通过 `xs:fill` 传递。无返回寄存器、隐式写回、可变绑定或高阶宏值。
6. `import` 是唯一装载机制。按 canonical SourceId 去重，在展开前冻结完整闭包并解析静态宏引用。导入环合法，递归展开环不折叠。

Modules contain declarations only. A separate `xs:entry` source constructs the output using imports, root parameters, and a body. It is not a macro or symbol and cannot be imported. Modules have no entry attribute or executable body. `expand` is the only recursive evaluation form, and `import` is the only source-loading form. Every expansion has an isolated scope and immutable, eagerly evaluated inputs. The returned ordered node sequence composes directly, as text in argument bodies, or as structure in fills. No compatibility aliases or separate fragment construct remain.

## Mechanism and evidence / 机制与依据

入口构造与可复用宏定义明确分离。此前曾考虑模块 `entry` 属性选择命名宏，但最终不采用：它仍让模块承担构建产品的责任。现在以不同 XML 根元素区分库与产品入口，既不偷偷生成 `main`，也不把入口伪装为命名宏。

The earlier candidate selected a named macro through a module entry attribute. The final decision rejects that candidate: libraries and product construction use distinct source roots, without synthesizing a main macro.

完整静态宏图为跨模块分析提供基础，类似 [LLVM LTO](https://llvm.org/docs/LinkTimeOptimization.html) 在链接阶段结合跨模块信息进行优化的前提。但冻结图本身不是优化器；此决定不声称已经实现链接时优化（Link-time Optimization, LTO）、内联或性能提升。未来变换必须保留输出、静态错误、来源信息和预算契约；不可跳过未执行分支中的非法引用。

A frozen static graph enables whole-program analysis, but is not itself an optimizer. This decision makes no claim of implemented LTO, inlining, or measured speedup. Future transformations must preserve output, errors, provenance, and budget semantics.

递归、正则条件、命名捕获和字符串构造仍可逐条模拟两计数器机；规范给出增量、零测试/减量和停机翻译。独立入口构造上下文只负责把初始值显式传入机器宏，不改变这一构造。抽象语义不设有限常数上限；实际执行由可调预算保护。这里的计算完备性论证不是所有程序都会终止的保证。

The normative two-counter instruction translation preserves recursive computational completeness without implicit file macros or a macro identity for the entry. Abstract computation is unbounded; configurable runtime guards remain necessary and do not prove termination.

## Migration and invariants / 迁移与不变量

- 将产品正文及其参数放入独立 `xs:entry`；把可复用定义放入 `xs:module`，由入口导入。
- 目录/glob 批量发现跳过合法模块；显式指定模块作为编译根报错。
- 将 `call` 改为 `expand`；将 `mount src` 改为模块级 `import src` 加明确目标宏的 `expand ref`。
- 被导入库只提供定义，不能通过导入获得输出。
- 保持单二进制语义模块布局和相邻测试；不改历史版本快照。
- `.o.xml` 仍移除全部属性、命名空间声明和元素前缀，并执行固定空白压缩；这不是标量求值中的转换。

Move product bodies and root parameters into separate entry sources, keep reusable macros in modules, replace calls with expansion, and replace mounting with import plus named expansion. Preserve binary-owned modules, adjacent tests, immutable release snapshots, and attribute-free, whitespace-squished final output.

## Verification obligations / 验证要求

验证入口与模块根的区别、模块直接编译拒绝与批量跳过、导入入口拒绝、入口宏定义/根 slot 拒绝、宏 QName 别名、重复/循环导入、前向及跨模块递归、文本结果嵌套传参、结构结果填充、参数与匹配捕获不泄漏、非法旧语法拒绝、非文本标量结果拒绝及带来源链的预算失败。两计数器指令应由实际运行测试覆盖；一般计算完备性由规范中的参数化翻译论证，不能仅凭有限样例宣布证明。

Verify distinct entry/module roots, explicit module rejection and batch skipping, rejected entry imports and entry macro/slot declarations, alias resolution, duplicate/cyclic imports, forward and cross-module recursion, nested returned text, structural fills, scope isolation, rejected legacy forms, rejected non-text scalar results, and provenance-bearing budget errors. Executable tests cover instances; the parameterized translation supports the general expressiveness argument.
