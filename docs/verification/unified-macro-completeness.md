# 统一宏核的计算完备性核查 / Unified macro completeness audit

## 结论及范围 / Result and scope

**推导结论：** 在字符串长度、展开次数和存储空间没有语言级固定上界的抽象语义下，当前 `docs/dsl.md` 的统一宏核能够模拟通用两计数器机。显式入口、声明式模块、静态 `xs:expand`、独立展开作用域与按值传参都不破坏这一能力。资源受限的实际编译器只能执行其中有限的运行前缀；这不是无界机器实现的证明。

**Derived result:** With no language-level fixed bound on strings, expansion count, or storage, the unified macro core simulates universal two-counter machines. Explicit entry construction, declaration-only modules, static expansion targets, isolated scopes, and value passing preserve that power. A resource-bounded compiler executes only finite prefixes; it is not an implementation of physically unbounded computation.

这是构造性模拟论证，不是 Lean/Coq 机器核验，也不是以有限测试冒充普适性证明。
This is a constructive simulation argument, not a Lean/Coq-checked proof or an inference of universality from finite tests.

## 机器模型 / Machine model

一个有限程序具有标签集合 Q、入口标签及如下三种指令；状态为 `(q, c0, c1)`，计数器属于自然数。
A finite program has labels Q, an entry label, and the following instructions; configurations are `(q, c0, c1)` with natural-number counters.

| 指令 / Instruction | 转移 / Transition |
|---|---|
| `INC(i, next)` | Increment counter i; jump to next. / 计数器 i 加一并跳转。 |
| `DECJZ(i, zero, positive)` | If zero, jump to zero; otherwise decrement and jump to positive. / 零则跳转，否则减一并跳转。 |
| `HALT` | Terminate and return the encoded counters. / 终止并返回编码后的计数器。 |

这一定义允许增量和两个条件分支显式指定目标，不能误换成受限指令集再沿用完备性结论。它包含 Dudenhefner 的 CM2 指令集：将增量后继及零分支目标设为下一标签即可。该论文给出了对应机器模型和形式化归约；特别提醒某些看似相近的两计数器指令集并不通用。[Dudenhefner, FSCD 2022](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.FSCD.2022.16)

Both branch targets and increment successors are explicit. This model contains the paper's CM2 instruction set by choosing the next label as the increment successor and zero branch. Universality must not be transferred silently to weaker instruction sets.

## 有效翻译 / Effective translation

为每个标签 q 建立 `m:q` 宏，显式声明 `c0`、`c1` 参数，以 `1^n` 编码 n（零为空串）。所有目标标签都在有限源码中，因此无需动态宏名、运行时 import、闭包或外部变量捕获。
Create one macro `m:q` per label with explicit `c0` and `c1` parameters. Encode n as `1^n`, including the empty string for zero. Every target is a static label in finite source; no dynamic names, runtime imports, closures, or caller captures are needed.

以下为指令体模板；在真正的标量程序中，应去掉模板为便于阅读增加的格式空白，避免把它们作为返回值输出。命名空间绑定为 `xs="https://xmlsquish.moesegfault.dev/ns"` 和 `m="urn:counter-machine"`。
The following instruction bodies are templates. Remove presentation whitespace in scalar programs so it does not become return data. Bind the namespaces as specified above.

`INC(0, next)`:

```xml
<xs:expand ref="m:next"><xs:arg name="c0">1<xs:insert get="arg.c0"/></xs:arg><xs:arg name="c1" get="arg.c1"/></xs:expand>
```

`DECJZ(0, zero, positive)`:

```xml
<xs:ifr get="arg.c0" pattern="^$"><xs:expand ref="m:zero"><xs:arg name="c0" value=""/><xs:arg name="c1" get="arg.c1"/></xs:expand></xs:ifr><xs:ifr get="arg.c0" pattern="^1(?&lt;rest&gt;1*)$"><xs:expand ref="m:positive"><xs:arg name="c0" get="match.rest"/><xs:arg name="c1" get="arg.c1"/></xs:expand></xs:ifr>
```

对计数器 1 交换参数名即可。`HALT` 的纯文本体可以为：
Swap the parameter names for counter 1. A text-only `HALT` body is:

```xml
<xs:insert get="arg.c0"/>#<xs:insert get="arg.c1"/>
```

入口 `xs:entry` 将初始值显式传入对应标签。`xs:module` 只组织指令宏定义，不选择入口；是否拆分到若干通过 `import` 连接的模块不影响翻译。入口是构造上下文，不是宏或符号，不对应机器指令标签。
An `xs:entry` source passes the initial counters explicitly. Modules only organize instruction macros; splitting definitions into statically imported modules does not affect the translation. The entry context is not a macro, symbol, or machine instruction label.

完整编译要求单个输出根元素，因此入口把机器的纯文本结果放进 `<Result>`。假设 `machine.xml` 是包含 `m:start` 及其余指令宏的模块：
Full compilation requires one output root, so the entry wraps the text result in `<Result>`. Assume `machine.xml` is a module defining `m:start` and the remaining instruction macros:

```xml
<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:counter-machine">
    <xs:import src="./machine.xml"/>
    <Result><xs:expand ref="m:start"><xs:arg name="c0" value="111"/><xs:arg name="c1" value=""/></xs:expand></Result>
</xs:entry>
```

入口不能被导入或递归展开；只有命名宏参与机器状态转移。这不限制通用性，因为任意两计数器机的有限标签都已被编码为命名宏。
Entries cannot be imported or recursively expanded. Only named macros encode machine transitions, which suffices because every finite program label is represented by such a macro.

## 模拟不变量 / Simulation invariant

1. 执行标签 q 对应宏时，其参数恰为 `c0=1^a, c1=1^b`。增量拼接保持该编码；非零分支的命名捕获恰为 `1^(a-1)`。这可对转移次数归纳证明。
   At label q, arguments encode exactly `(a,b)`. Concatenation preserves unary encoding, and the positive branch captures exactly the predecessor. Induct on machine steps.
2. 在一元字符串域内，`^$` 与 `^1(?<rest>1*)$` 互斥且穷尽。因此每条非停机指令恰好展开一个后继宏；空串和单个 `1` 的边界均包含在内。
   On unary strings, the two anchored patterns are disjoint and exhaustive. Every nonhalting instruction expands exactly one successor, including the zero/one boundary.
3. 非停机指令没有其他产出。若后继返回，待执行的兄弟分支仍检查原来的不可变参数，必然不产生内容；因此不会重复执行另一条路径。
   Nonhalting instructions emit nothing else. After a successor returns, a pending sibling guard still reads the original immutable arguments and emits nothing; it does not introduce a second path.
4. 若机器经过 k 步停机，宏经过有限次调度到达 `HALT` 并返回相同编码。若机器不停机，宏不断展开唯一后继，无法返回。因此翻译保持停机与非停机。
   A k-step halting run reaches `HALT` in finitely many scheduling steps with the same encoding. A nonhalting run keeps expanding its unique successor and cannot return. Halting and nonhalting are preserved.

最终产品的空白压缩、属性删除不修改所用的 `1` 与 `#`，因此不会破坏这个见证的观测结果。一般字符串计算则必须仍在产品压缩之前完成。
Final whitespace compression and attribute removal preserve the witness alphabet `1` and `#`. General scalar computation must still precede product compression.

## 返回值与实现证据 / Return values and implementation evidence

计算完备性见证只需要递归状态转移；它**不能单独证明非尾递归的返回值组合正确**。另需测试把一个宏的纯文本展开放入另一展开的 `xs:arg` body，再在递归结果之后拼接字符。例如递归反转 `abcd`，让递归结果先返回，再拼接首字符，最终得到 `dcba`。
The universality witness needs recursive state transitions; it **does not alone establish non-tail return composition**. Separately test a text-producing expansion inside another expansion's argument body, followed by a character appended after the recursive result. Recursive reversal of `abcd` should yield `dcba`.

当前实现证据位于 `crates/squish-link/src/instantiate.rs`：`Task::Arg` 先压入
`Task::ArgDone`，再压入参数 body 的 `Task::Region`，因此显式 LIFO 工作栈先完成 body；
`Task::ArgDone` 只连接文本 occurrence，并以 `RUN005` 拒绝结构节点。`enter` 把已求值的
参数/fill 移入新 `Env`，以空 `captures` 创建独立调用帧；`Task::Region` 对 op 反序压栈，
从而保持源码顺序。这些源码事实与上述模型一致，但不是整个实现的形式化验证。

The current evidence is `crates/squish-link/src/instantiate.rs`: `Task::Arg` pushes
`Task::ArgDone` before the argument body's `Task::Region`, so the explicit LIFO work stack
finishes the body first; `Task::ArgDone` concatenates text occurrences and rejects structural
nodes with `RUN005`. `enter` moves evaluated arguments/fills into a new `Env` with empty
`captures`, and `Task::Region` reverse-pushes operations to preserve source order. These source
facts support the model but are not a formal verification of the whole implementation.

具体回归覆盖应包含：两计数器交换/转移、零与非零分支、互递归、非尾返回组合、callee 不能读取 caller capture、可配置资源上限失败。任何有限测试集合都只核验这些实例；普适性来自上面的有效翻译与归纳不变量。
Regression coverage should include counter transfer, both branches, mutual recursion, non-tail result composition, inaccessible caller captures, and configured resource failures. Finite tests check instances; universality follows from the translation and invariant.

可执行回归不再用易过期的总测试数描述。`crates/squish-xml-front/src/tests.rs::entry_lowers_all_operation_families_and_round_trips`
验证入口、调用、参数/fill 与正则操作进入可重定位 IR；
`crates/squish-link/src/tests.rs::import_cycles_link_and_runtime_preserves_scope_slot_regex_and_file_bindings`
验证导入环、调用帧、slot、捕获与定义位置 `file.*`；同文件的
`recursion_uses_explicit_frames_and_reports_the_complete_budget_chain` 验证递归预算及完整帧链。
`crates/squish-manager/tests/build.rs::semantic_example_publishes_fully_traceable_debug_bundle`
再通过真实多模块递归 XML 工程验证管理器构建与来源链。它们是具体实例的实现证据；
两计数器机的一般性仍来自上面的有效翻译与不变量，而不是测试数量。
这些具名测试没有单独实现上文 `abcd -> dcba` 的非尾标量 body 见证；当前对此行为的
证据仍是 `Task::Arg`/`Task::ArgDone` 执行路径。本文明确保留这一测试覆盖缺口，而不把
相邻递归测试或总数冒充为直接回归。

Executable regressions are no longer summarized by a stale aggregate count.
`crates/squish-xml-front/src/tests.rs::entry_lowers_all_operation_families_and_round_trips`
covers entry/call/argument/fill/regex lowering into relocatable IR;
`crates/squish-link/src/tests.rs::import_cycles_link_and_runtime_preserves_scope_slot_regex_and_file_bindings`
covers import cycles, frames, slots, captures, and definition-site `file.*`; and
`recursion_uses_explicit_frames_and_reports_the_complete_budget_chain` covers recursive budgets
and the complete frame chain. The manager-level
`crates/squish-manager/tests/build.rs::semantic_example_publishes_fully_traceable_debug_bundle`
runs a real multi-module recursive XML project through build and provenance publication. These
are implementation witnesses for concrete instances; universality still follows from the
effective translation and invariant above, not from a test count.
The named tests do not separately implement the `abcd -> dcba` non-tail scalar-body witness above;
current evidence for that behavior remains the `Task::Arg`/`Task::ArgDone` execution path. This
document keeps that coverage gap explicit rather than treating a neighboring recursion test or
an aggregate count as a direct regression.
