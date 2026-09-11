# 更新日志 / Changelog

记录面向用户的重要变化。版本遵循 [Semantic Versioning](https://semver.org/)；0.x 的次版本可以包含破坏性变化。

Notable user-facing changes are recorded here. Versions follow [Semantic Versioning](https://semver.org/); minor releases during 0.x may contain breaking changes.

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

[0.2.0]: https://github.com/kleedaisuki/prompt-squish/releases/tag/v0.2.0
