# 贡献指南 / Contributing

## 契约优先 / Contracts first

先阅读 [DSL 规范](docs/dsl.md) 和 [ADR 0005](docs/adr/0005-dsl-language.md)。ADR 0001–0003 是历史记录，其旧 DSL、元数据继承决策已被替代，最终产品空白压缩保持；ADR 0004 的单 package 原则保留，其 library/binary 布局由 [ADR 0006](docs/adr/0006-binary-module-layout.md) 替代。

Read the DSL specification and ADR 0005 first. ADRs 0001–0003 retain historical context, not current language authority. ADR 0004's single-package principle remains; ADR 0006 supersedes its library/binary layout.

- 源码身份使用词法规范 URI，不以文件内容或 symlink 真实路径折叠。 / Use logical canonical source URIs, not content or symlink identity.
- 源码装载闭包、不可变宏定义与运行时展开帧（Expansion Frame）分离。 / Separate discovery, immutable definitions, and runtime frames.
- 参数是字符串，slot 是 XML；禁止隐式转换和动态环境继承。 / Arguments are strings; slots are XML; no implicit conversion or dynamic inheritance.
- 宏求值与 lowering 保留用户文本语义；最终 `.o.xml` 产品阶段单独运行 `squish`。 / Preserve text during evaluation/lowering; run `squish` separately for the final `.o.xml` product.
- 诊断保留真实源码位置与完整调用链；失败不得发布部分结果。 / Retain real source positions and complete frame chains; never publish partial failures.

## 环境与验证 / Environment and validation

Rust 1.88+，工具链见 `rust-toolchain.toml`；站点使用 Node.js 22.12+ 与锁定的 npm 依赖。 / Rust 1.88+; see the toolchain file. The site uses Node.js 22.12+ and locked npm dependencies.

```bash
cargo build --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --locked

cd site
npm ci
npm run test
npm run demo:generate
npm run demo:check
npx playwright install chromium
npm run test:browser
```

测试应使用临时目录，不提交 `.i.xml` / `.o.xml`、`target`、`site/dist` 或浏览器缓存。不要在并行测试中修改进程环境；用子进程隔离。

Tests use temporary directories. Do not commit generated XML, build directories, or browser caches. Isolate environment changes in child processes.

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

保持一个 Cargo package 和一个 binary target，以 `src/main.rs` 为唯一入口，不保留库门面。编译器负责源码装载、静态验证、展开与来源记录；CLI 负责路径发现、预算/参数、诊断展示、统计和原子写入。词法转换器不参与宏求值，但负责最终产品空白压缩。

Keep one Cargo package with a single binary target rooted at `src/main.rs`, without a library facade. The compiler owns loading, validation, expansion, and provenance; the CLI owns discovery, options, presentation, metrics, and atomic persistence. The lexical utility is outside macro evaluation but performs final product whitespace compression.

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
| CLI | `-I` / `-O`、debug 等价、失败不覆盖、颜色重定向 / Stages, debug equivalence, failure safety, redirected color |

文档注释须中英双语，用 rustdoc 解释契约和不变量，不复述代码。新增公共 API 要有可运行示例。模块测试紧邻源码；只为真实共享需求引入抽象。

Use bilingual rustdoc for contracts and invariants, with runnable public API examples. Keep tests next to their modules and abstractions justified by real consumers.

站点不得重新实现编译器；`examples/site-demo` 经真实 CLI 生成 `site/src/data/build-demo.json`。中英文页面必须信息对等，验证键盘、窄屏、主题和无 JavaScript 降级。

The site must not reimplement compilation. Its checked-in demo data is generated from `examples/site-demo` by the real CLI. Keep both locales equivalent and test keyboard use, narrow screens, themes, and no-JavaScript fallback.

## 提交 / Contributions

保持提交主题单一，说明语义变化、测试与迁移影响；不提交无关格式化。贡献须有权按 [GPL-3.0-or-later](LICENSE) 发布；第三方内容保留来源与许可。

Keep commits focused and document behavior changes, tests, and migration impact. Avoid unrelated formatting. Contributions must be distributable under the project license; retain third-party attribution.
