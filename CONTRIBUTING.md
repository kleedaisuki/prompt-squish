# 贡献指南 / Contributing

## 契约优先 / Contracts first

先阅读 [DSL 规范](docs/dsl.md)、[ADR 0005](docs/adr/0005-dsl-language.md) 和现行管理器架构 [ADR 0009](docs/adr/0009-microkernel-manager-and-reusable-ir.md)。ADR 0001–0004 是历史记录；ADR 0009 已取代 ADR 0006 的包边界决策。不要按旧的单包或松散文件编译模型扩展当前代码。

Read the DSL specification, ADR 0005, and current manager architecture in ADR 0009 first. ADRs 0001–0004 are historical; ADR 0009 supersedes ADR 0006's package-boundary decision. Do not extend the current code according to the old single-package or loose-file compiler model.

- 源码身份使用词法规范 URI，不以文件内容或 symlink 真实路径折叠。 / Use logical canonical source URIs, not content or symlink identity.
- 源码装载闭包、不可变宏定义与运行时展开帧（Expansion Frame）分离。 / Separate discovery, immutable definitions, and runtime frames.
- 参数是字符串，slot 是 XML；禁止隐式转换和动态环境继承。 / Arguments are strings; slots are XML; no implicit conversion or dynamic inheritance.
- 宏求值与 lowering 保留用户文本语义；后端只对最终 `.prompt` 产品执行空白压缩。 / Preserve text during evaluation/lowering; compress whitespace only in the final `.prompt` backend product.
- 诊断保留真实源码位置与完整调用链；失败不得发布部分结果。 / Retain real source positions and complete frame chains; never publish partial failures.

## 环境与验证 / Environment and validation

Rust 1.88+，工具链见 `rust-toolchain.toml`；站点使用 Node.js 22.12+ 与锁定的 npm 依赖。 / Rust 1.88+; see the toolchain file. The site uses Node.js 22.12+ and locked npm dependencies.

```bash
cargo build --workspace --all-targets --all-features --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo test --workspace --doc --all-features --locked
python -m unittest discover -s scripts/tests -p "test_*.py"
python scripts/check_architecture.py
python -m unittest discover -s .github/scripts -p "test_*.py"
python .github/scripts/release.py verify .
cargo build --release --locked

cd site
npm ci
npm test
npm run demo:check # Run from the checkout; uses the real workspace CLI.
npx playwright install chromium # Linux CI 以 --with-deps 安装系统依赖 / Linux CI adds --with-deps for OS packages.
npm run test:browser
```

测试应使用仓库 `.temp` 中的临时目录，不提交产品、`target`、`site/dist` 或浏览器缓存。只有在有意更新网站演示内容时才运行 `npm run demo:generate` 并评审数据差异。不要在并行测试中修改进程环境；用子进程隔离。

Keep test scratch space in the repository's `.temp` directory. Do not commit generated products, build directories, or browser caches. Run `npm run demo:generate` only when intentionally updating the site demo, and review its data diff. Isolate environment changes in child processes.

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

根包提供 `src/main.rs` 的唯一 CLI 二进制；领域逻辑分布在 `crates/` 下的工作区成员。遵守 `scripts/check_architecture.py` 检查的无环依赖边界：内核组合操作和端口，项目与解析器管理清单和依赖，前端/IR/链接/后端负责构建阶段，发布器负责产物提交。不要以根包门面绕过边界。

The root package supplies the sole CLI binary at `src/main.rs`; domain logic lives in workspace members under `crates/`. Respect the acyclic boundaries enforced by `scripts/check_architecture.py`: the kernel composes operations and ports, project and resolver own manifests and dependencies, frontend/IR/link/backend own build stages, and the publisher commits artifacts. Do not route around those boundaries through a root-package facade.

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
| 项目 / Project | 清单发现、工作区选择、锁文件模式、依赖解析、干净构建根 / Manifest discovery, workspace selection, lock modes, dependency resolution, clean build root |
| 发布 / Publication | `.prompt`、`.xsir`、`.psdbg` 定位符、generation 恢复、失败不覆盖 / Artifact locators, generation recovery, failure safety |
| CLI | 命令帮助、结构化消息、诊断与重定向颜色 / Command help, structured events, diagnostics, redirected color |

文档注释须中英双语，用 rustdoc 解释契约和不变量，不复述代码。新增公共 API 要有可运行示例。模块测试紧邻源码；只为真实共享需求引入抽象。

Use bilingual rustdoc for contracts and invariants, with runnable public API examples. Keep tests next to their modules and abstractions justified by real consumers.

站点不得重新实现编译器；`examples/site-demo` 经真实 CLI 生成 `site/src/data/build-demo.json`。中英文页面必须信息对等，验证键盘、窄屏、主题和无 JavaScript 降级。

The site must not reimplement compilation. Its checked-in demo data is generated from `examples/site-demo` by the real CLI. Keep both locales equivalent and test keyboard use, narrow screens, themes, and no-JavaScript fallback.

## 提交 / Contributions

保持提交主题单一，说明语义变化、测试与迁移影响；不提交无关格式化。贡献须有权按 [GPL-3.0-or-later](LICENSE) 发布；第三方内容保留来源与许可。

Keep commits focused and document behavior changes, tests, and migration impact. Avoid unrelated formatting. Contributions must be distributable under the project license; retain third-party attribution.
