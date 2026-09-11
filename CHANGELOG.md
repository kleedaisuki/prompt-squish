# 更新日志 / Changelog

记录面向用户的重要变化。版本遵循 [Semantic Versioning](https://semver.org/)；0.x 的次版本可以包含破坏性变化。

Notable user-facing changes are recorded here. Versions follow [Semantic Versioning](https://semver.org/); minor releases during 0.x may contain breaking changes.

## [0.3.0] — 2026-09-11

完整说明与安装方法 / Full notes and installation: [0.3.0](docs/releases/0.3.0.md).

### 破坏性变化 / Breaking changes

- 独立 `xs:entry` 构建入口与 `xs:module` 宏库；模块只包含导入和宏定义，没有隐式正文、`main` 或模块入口属性。
  Separate `xs:entry` build roots from `xs:module` libraries. Modules contain only imports and macro definitions, without implicit bodies, `main`, or module entry attributes.
- `xs:expand` 取代 `call` 与 `mount`，成为唯一递归展开操作。`import` 只导入模块，按源码身份去重，不执行入口或继承前缀绑定。引用方声明自己的命名空间别名。
  Replace `call` and `mount` with `xs:expand` as the sole recursive expansion operation. Imports load modules only, deduplicate source identities, and neither execute entries nor inherit prefix bindings. Referencing sources declare their own namespace aliases.
- **最终 `.o.xml` 移除全部属性、命名空间声明和元素前缀，继续压缩空白。** 不提供保留属性或绕过压缩的选项；源码 DSL 属性仍参与编译。
  **Final `.o.xml` strips every attribute, namespace declaration and element prefix, and still squishes whitespace.** No preservation or compression-bypass option exists; source DSL attributes still participate in compilation.

### 展开与复用 / Expansion and reuse

- 保留独立作用域、按值参数、纯文本与结构返回组合，以及预算约束下的递归。目录与 glob 自动跳过合法模块库，显式编译库仍报错。
  Preserve isolated scopes, value-passed inputs, text and structural return composition, and budgeted recursion. Directory/glob discovery skips valid module libraries; explicitly compiling one remains an error.
- 内部支持准备一次、多次展开的冻结快照，复用静态 IR 载荷、惰性转义及来源序列化；不缓存完整宏结果，不跳过执行帧与预算。
  Internally prepare a frozen snapshot once and expand repeatedly, reusing static IR payloads, lazy escaping and provenance serialization without whole-macro result caching or skipped frames and budgets.
- 七组内存中完整编译微基准耗时下降 17.4%–81.1%，不包含 CLI 启动、I/O、token 计数与最终压缩；不宣称 GSP 端到端显著加速或总内存下降。原始数据与阶段回归见 [性能报告](docs/performance/README.md)。
  Seven in-memory full-compilation microbenchmarks reduce elapsed time by 17.4%–81.1%, excluding CLI startup, I/O, token counting and final squishing. No material GSP end-to-end speedup or total-memory reduction is claimed. See the [performance report](docs/performance/README.md), including raw data and phase regressions.

## [0.2.0] — 2026-09-11

完整说明与安装方法 / Full notes and installation: [0.2.0](docs/releases/0.2.0.md).

### 破坏性变化 / Breaking changes

- 以命名空间感知的模块与不可变命名宏取代旧语言；内建 URI 为 `https://xmlsquish.moesegfault.dev/ns`。不提供旧语法兼容层。
  Replace the legacy language with namespace-aware modules and immutable named macros. No legacy-syntax compatibility layer is provided.
- 字符串参数与 XML 插槽（slot）显式传递；不再隐式继承调用者元数据，也不在普通文本或属性中进行 `$` 插值。
  Pass scalar arguments and XML slots explicitly; remove implicit caller-metadata inheritance and `$` interpolation in ordinary text or attributes.
- 统一为单二进制架构；编译器属于内部模块，不再提供 Rust 库入口。测试放在语义所属模块附近。
  Consolidate into one binary with an internal compiler, not a public Rust library. Keep tests beside their semantic owners.

### 新增 / Added

- 静态源码闭包检查、命名空间宏身份、导入、模块挂载、显式参数/填充、正则条件与命名捕获、预算约束的递归。
  Static source-closure validation, namespace-based macro identity, imports, module mounts, explicit arguments/fills, regex conditions and named captures, and budgeted recursion.
- 带来源信息的 `.i.xml`、展开调用链诊断，以及深度、展开次数和输出字节预算。
  Provenance-bearing `.i.xml`, expansion-call diagnostics, and configurable depth, expansion-count, and output-byte guards.

### 保持与限制 / Retained behavior and limits

- **最终 `.o.xml` 继续压缩空白；这是产品语义，并非需要恢复的 XML 文本保真行为。**
  **Final `.o.xml` still squishes whitespace: this is product semantics, not an XML text-fidelity regression to undo.**
- 最低 Rust 版本为 1.88。默认加载器仅支持本地 `file:` 源码，不是文件访问沙箱；输出字节预算不是进程总内存上限。
  Minimum Rust version is 1.88. The default loader supports local `file:` sources only and is not a filesystem sandbox; output-byte guards are not total-process memory limits.

[0.3.0]: https://github.com/kleedaisuki/prompt-squish/releases/tag/v0.3.0
[0.2.0]: https://github.com/kleedaisuki/prompt-squish/releases/tag/v0.2.0
