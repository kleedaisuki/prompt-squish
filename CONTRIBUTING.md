# 贡献指南 / Contributing

## 契约优先 / Contracts first

先阅读 [DSL 规范](docs/dsl.md)、[ADR 0007](docs/adr/0007-unified-macro-expansion.md) 和 [ADR 0009](docs/adr/0009-microkernel-manager-and-reusable-ir.md)。ADR 0001–0006 保留历史理由，不是当前 package 边界或命令契约；[ADR 0010](docs/adr/0010-transactional-new-project-creation.md) 补充 `new` 的事务边界。当前职责与源码路径见[重构映射](docs/design/refactor-map.md)，已完成的迁移过程见[执行状态](docs/design/project-manager-execution-status.md)。

Read the DSL specification, ADR 0007, and ADR 0009 first. ADRs 0001–0006 preserve historical rationale, not today's package boundaries or command contract. ADR 0010 extends the transactional `new` boundary. The refactor map locates current owners; the execution status is a historical cutover ledger.

- 源码身份使用词法规范 URI，不以文件内容或 symlink 真实路径折叠。 / Use logical canonical source URIs, not content or symlink identity.
- 源码装载闭包、不可变宏定义与运行时展开帧（Expansion Frame）分离。 / Separate discovery, immutable definitions, and runtime frames.
- 参数是字符串，slot 是 XML；禁止隐式转换和动态环境继承。 / Arguments are strings; slots are XML; no implicit conversion or dynamic inheritance.
- 宏求值与 lowering 保留用户文本语义；后端独立压缩并发布 `.prompt` 产品。 / Preserve text during evaluation/lowering; let the backend compress and publish the final `.prompt` product.
- 诊断保留真实源码位置与完整调用链；失败不得发布部分结果。 / Retain real source positions and complete frame chains; never publish partial failures.

## 环境与验证 / Environment and validation

Rust 1.88+，工具链见 `rust-toolchain.toml`；CI 显式使用最低支持版本 1.88.0。站点使用 Node.js 22.12+ 与 `site/package-lock.json` 锁定的 npm 依赖；CI 使用 Node.js 24。 / Rust 1.88+; CI explicitly tests MSRV 1.88.0. The site requires Node.js 22.12+ and npm dependencies locked in `site/package-lock.json`; CI uses Node.js 24.

```bash
cargo +1.88.0 check --workspace --all-targets --locked
python -m unittest discover -s scripts/tests -p "test_*.py"
python scripts/check_architecture.py
python -m unittest discover -s .github/scripts -p "test_*.py"
python .github/scripts/release.py verify .
python -m unittest discover -s docs/performance/tests -p "test_*.py"
cargo +1.88.0 fmt --all -- --check
cargo +1.88.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.88.0 test --workspace --doc --all-features --locked
cargo +1.88.0 build --workspace --all-targets --all-features --locked
cargo +1.88.0 test --workspace --all-targets --all-features --locked

cd site
npm ci
npm run check
npm run test
npm run demo:generate
npm run demo:check
npx playwright install chromium
npm run test:browser
```

GitHub Actions 的 `ci.yml` 在 Linux、Windows、macOS 上构建和测试 Rust workspace，并验证组合后的二进制；站点 CI 执行 `npm ci`、类型检查和构建。上面的 demo 与浏览器命令属于额外本地回归，不要误认为现有站点 CI 已覆盖它们。依赖边界由 `scripts/architecture_edges.txt` 和 `scripts/check_architecture.py` 检查；修改 crate 依赖时应一同评审该契约，而不是为了通过检查机械地添加边。

GitHub Actions `ci.yml` builds and tests the Rust workspace on Linux, Windows, and macOS and smoke-tests the composed binary. Site CI runs `npm ci`, type checks, and builds; demo and browser commands above are additional local regressions, not current site-CI coverage. `scripts/architecture_edges.txt` and its checker enforce reviewed crate edges. Review that contract when changing dependencies rather than adding edges merely to appease the check.

临时实验与手工测试文件放在仓库根目录 `.temp/` 或 `.cache/`，不要提交生成的 `.prompt`、`.xsir`、`.psdbg`、`target/`、`site/dist/` 或浏览器缓存。自动化测试使用隔离的临时目录；不要在并行测试中修改进程环境，必要时用子进程隔离。

Place ad-hoc experiments and manual test files under the repository's `.temp/` or `.cache/`. Do not commit generated artifacts, build directories, or browser caches. Automated tests use isolated temporary directories; isolate environment changes in child processes rather than mutating a parallel test process.

### IntelliJ IDEA / RustRover indexing diagnostics

Treat Cargo as the source of truth when an editor reports missing trait members. In the
2026-09-15 `Renderer::cancellation_requested` incident,
`cargo +1.88.0 check -p xmlsquish --all-targets --locked` succeeded and Cargo resolved the
single local `squish-presentation` crate. The ignored `.idea/prompt-squish.iml` still listed
deleted `xmlsquish-app`, `xmlsquish-cli`, and `xmlsquish-core` modules and based source roots
on `$MODULE_DIR$`; the errors were therefore attributed to a stale IDE project model, not to
the Rust trait contract. Reload the project from the root `Cargo.toml` using JetBrains'
[Cargo project workflow](https://www.jetbrains.com/help/rust/loading-cargo-projects.html),
then use [Repair IDE](https://www.jetbrains.com/help/idea/repair-ide.html) if reloading does
not clear the index. Do not rewrite a compiling public API or commit personal `.idea` state
to suppress an editor-only diagnostic. The editor-side recovery for this incident has not
yet been independently verified.

## 结构与测试 / Structure and tests

根 package 保持单一 `xmlsquish` binary 入口 `src/main.rs`，但仓库是多 crate Cargo workspace。CLI 解析、配置、协议、内核生命周期、管理器编排、构建运行时、源码/依赖、IR/链接、后端、发布与展示各由有边界的 crate 负责；不要在根入口或 CLI 中重建业务流水线。具体所有权见[当前重构映射](docs/design/refactor-map.md)。

Keep the root package as one `xmlsquish` binary entry in `src/main.rs`, but treat the repository as a multi-crate Cargo workspace. Bounded crates separately own CLI parsing, configuration, protocol, kernel lifecycle, manager orchestration, build runtime, sources and dependencies, IR and linking, backend, publication, and presentation. Do not rebuild the domain pipeline in the root entry or CLI. See the current refactor map for exact ownership.

最低回归矩阵 / Minimum regression matrix:

| 边界 / Boundary | 必测 / Coverage |
| --- | --- |
| 源码 / Sources | 路径别名、冻结、导入环、未执行分支引用 / Logical aliases, freezing, import cycles, inactive references |
| 符号 / Symbols | 前缀别名、前向引用、重复定义两处位置 / Prefix aliases, forward calls, both duplicate-definition sites |
| 调用 / Calls | 缺失/重复/未知 arg 与 fill、定义位置路径 / Missing/duplicate/unknown arguments and fills, definition-site paths |
| 标量 / Scalars | XML 转义、非文本参数错误、空白不裁剪 / Escaping, non-text errors, exact whitespace |
| 正则 / Regex | 非法/位置/重复捕获、可选捕获、嵌套遮蔽与恢复 / Invalid and positional captures, optional groups, lexical restoration |
| 递归 / Recursion | 停机、相互递归、三类预算与完整帧链 / Termination, mutual recursion, all guards and complete chains |
| 输出 / Output | 单根文档、命名空间、混合内容、PI、来源清理 / Single document root, namespaces, mixed content, PIs, provenance cleanup |
| 项目与 CLI / Project and CLI | 清单/锁文件、workspace 选择、依赖来源、`--locked`/`--offline`、事件协议、取消与失败后不发布 / Manifests and locks, workspace selection, dependency sources, locked/offline modes, event protocol, cancellation, and failure-safe publication |
| 产物与恢复 / Artifacts and recovery | 稳定 locator、`.xsir`/`.psdbg`、原子发布、`clean` 完整构建根与进程恢复 / Stable locators, companion artifacts, atomic publication, complete-build-root clean, and process recovery |

文档注释须中英双语，用 rustdoc 解释契约和不变量，不复述代码。新增公共 API 要有可运行示例。模块测试紧邻源码；只为真实共享需求引入抽象。

Use bilingual rustdoc for contracts and invariants, with runnable public API examples. Keep tests next to their modules and abstractions justified by real consumers.

站点不得重新实现编译器；`examples/site-demo` 经真实 CLI 生成 `site/src/data/build-demo.json`。中英文页面必须信息对等，验证键盘、窄屏、主题和无 JavaScript 降级。

The site must not reimplement compilation. Its checked-in demo data is generated from `examples/site-demo` by the real CLI. Keep both locales equivalent and test keyboard use, narrow screens, themes, and no-JavaScript fallback.

## 提交 / Contributions

保持提交主题单一，说明语义变化、测试与迁移影响；不提交无关格式化。贡献须有权按 [GPL-3.0-or-later](LICENSE) 发布；第三方内容保留来源与许可。

Keep commits focused and document behavior changes, tests, and migration impact. Avoid unrelated formatting. Contributions must be distributable under the project license; retain third-party attribution.
