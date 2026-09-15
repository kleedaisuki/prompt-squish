# 更新日志 / Changelog

记录面向用户的重要变化。版本遵循 [Semantic Versioning](https://semver.org/)；0.x 的次版本可以包含破坏性变化。

Notable user-facing changes are recorded here. Versions follow Semantic Versioning; minor releases during 0.x may contain breaking changes.

## [Unreleased]

### 项目管理器切换 / Project-manager cutover

- 公共命令改为直接的 `xmlsquish build|fmt|add|remove|inspect`；移除松散文件编译和过渡期命令命名空间。
  The public surface is now direct `build`, `fmt`, `add`, `remove`, and `inspect`; loose-file compilation and the transitional command namespace are removed.
- 加入版本化 `xmlsquish.toml`、向上项目发现、`--manifest-path`、命名目标、profile、工作区包选择和入口即链接根模型。
  Add versioned manifests, upward discovery, explicit manifest selection, named targets, profiles, workspace package selection, and entry-as-link-root semantics.
- 构建现在发布 `.prompt` 产品；可重复 `--emit prompt|ir|debug` 选择 `.prompt`、可复用 `.xsir` 和自包含 `.psdbg`。
  Builds publish `.prompt` products and optionally materialize reusable `.xsir` and self-contained `.psdbg` companions.
- 支持本地路径、Git、registry 与工作区继承依赖，机器管理精确锁状态，并提供 `--locked`、`--offline` 与 `--frozen`。
  Support path, Git, registry, and workspace-inherited dependencies with machine-managed exact lock state and locked/offline/frozen modes.
- `add` 与 `remove` 以可恢复事务协调清单和锁文件；保留无写入的 `--dry-run`，且不隐式修改 XML import。
  Coordinate manifest and lock edits through recoverable transactions; retain read-only dry runs and never edit XML imports implicitly.
- `fmt` 使用保留语义的 XML 具体语法树（Concrete Syntax Tree, CST）路径；`--check` 发现差异时不写入并退出 `1`，`--diff` 蕴含 check。
  Format through a semantics-preserving XML CST path; dirty checks write nothing and exit `1`, while `--diff` implies check.
- 统一配置优先级为 defaults < user < workspace < environment < CLI；`--config KEY=VALUE` 接受 TOML 值并可重复。
  Unify configuration precedence as defaults < user < workspace < environment < CLI, with repeatable typed `--config KEY=VALUE` overrides.
- 操作输出支持 `human`、`short` 和版本化 NDJSON `json`；`inspect` 以 `--format human|json` 返回单一查询文档。
  Add human, short, and versioned NDJSON operation output; inspect returns one query document with `--format human|json`.
- 进程退出码稳定为成功 `0`、领域失败 `1`、用法错误 `2`、取消 `130`；未映射内部错误为 `101`。脏格式检查属于领域失败。
  Stabilize exits at success 0, domain failure 1, usage error 2, cancellation 130, and unmapped internal error 101. A dirty format check is a domain failure.
- 完成 ADR 0009 的静态微内核（microkernel）、有类型事件、可复用 IR、内容寻址缓存与原子产物发布切换；XML DSL 原语与语义不变。
  Complete the ADR 0009 cutover to a statically composed microkernel, typed events, reusable IR, content-addressed caching, and atomic artifact publication without changing XML DSL primitives or semantics.

### 破坏性变化 / Breaking changes

- `build`、`fmt`、`add`、`remove` 与 `inspect` 成为保留命令；旧路径首参数不再解释为输入文件。
- 旧的 sibling XML 中间/最终产物不再属于产品契约；迁移到清单目标和 `.prompt` / `.xsir` / `.psdbg`。
- 当前仓库和安装包不发布到 crates.io；使用 Git 源码安装或 GitHub Releases 二进制。

## [0.3.0] — 2026-09-11

完整历史说明与安装方法 / Full historical notes and installation: [0.3.0](docs/releases/0.3.0.md).

- 将 `xs:entry` 构建入口与 `xs:module` 宏库分离，并以 `xs:expand` 统一递归展开。
  Separated explicit entry roots from module libraries and unified recursive expansion under `xs:expand`.
- 保留显式参数、XML slot、正则命名捕获、冻结源码闭包与受预算约束的递归。
  Preserved explicit parameters, XML slots, named regex captures, frozen source closures, and budgeted recursion.

## [0.2.0] — 2026-09-11

完整历史说明 / Full historical notes: [0.2.0](docs/releases/0.2.0.md).

- 引入命名空间感知模块、不可变命名宏、显式字符串参数与 XML slot。
  Introduced namespace-aware modules, immutable named macros, explicit scalar parameters, and XML slots.

[Unreleased]: https://github.com/kleedaisuki/prompt-squish/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/kleedaisuki/prompt-squish/releases/tag/v0.3.0
[0.2.0]: https://github.com/kleedaisuki/prompt-squish/releases/tag/v0.2.0
