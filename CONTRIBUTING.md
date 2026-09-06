# 为 xmlsquish 贡献

感谢你来改善 xmlsquish！这里的首要目标是让编译语义和词法契约保持独立、简单、可审阅，而不是逐渐把空白扫描器变成一个不完整的 XML 解析器。

## 开发环境

- Rust 1.88 或更高版本；CI 会验证最低支持 Rust 版本（Minimum Supported Rust Version, MSRV），仓库 `rust-toolchain.toml` 声明日常开发使用的 stable 工具链、`rustfmt` 与 Clippy。
- Node.js 22.12 或更高版本；CI 使用 Node.js 24。
- npm；站点依赖由 `site/package-lock.json` 锁定。

```bash
git clone https://github.com/kleedaisuki/prompt-squish.git xmlsquish
cd xmlsquish
cargo build --workspace --locked

cd site
npm ci
```

## 先理解边界

提交编译器、FSM 或统计变更前，请阅读 [ADR 0001](docs/adr/0001-lexical-canonicalization-and-layering.md)、[ADR 0002](docs/adr/0002-semantic-compilation.md) 与 [ADR 0003](docs/adr/0003-metadata-inheritance-and-stage-metrics.md)。以下词法契约只约束 `squish` 阶段；编译阶段会先消除宏、注释和元信息：

- xmlsquish 是提示词词法规范化器，不承诺 XML Infoset 等价，也不尊重 `xml:space`。
- XML 空白严格是 U+0020、U+0009、U+000D、U+000A。
- Markup 内部原样保留；任意两个 atom 之间恰好一个 U+0020，文件首尾无空格。
- `recognized`、`removed`、`inserted` 是不同账目，并满足字符恒等式。
- 输入文件绝不覆盖；单个文件失败不阻止其他独立文件。

改变这些约束不是普通重构：请同时提交新的或替代的架构决策记录（Architecture Decision Record, ADR），更新中英文文档，并说明迁移影响。

## 分层与改动位置

```text
                 -> xmlsquish-app  (ports / use cases)
xmlsquish-cli --|
                 -> xmlsquish-core (pure FSM / domain)
```

| 需求 | 应修改的位置 |
| --- | --- |
| 编译语义、文件环境、扫描状态、atom、空白统计 | `crates/xmlsquish-core` |
| 批处理政策、端口、汇总模型 | `crates/xmlsquish-app` |
| 参数、路径发现、编码、文件 I/O、Tokenizer、终端输出 | `crates/xmlsquish-cli` |
| 文案、样式、国际化、Pages | `site`、`.github/workflows` |

CLI 是组合根（composition root）：它组合编译器、文件/Tokenizer 适配器与 app 报告模型。不要让 app 依赖具体适配器，也不要让 core 依赖 app、命令行或具体 Tokenizer。编译器通过注入的加载函数读取引用文件；默认快照读取系统环境，文件标识会使用规范路径以检测循环。不要在 TypeScript 中复制 FSM；未来的浏览器演示应复用 Rust core（例如 WebAssembly, WASM）。避免顺手重构与目标无关的模块。

## Rust 工作流

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --release --locked
```

开发时可直接运行：

```bash
cargo run -p xmlsquish-cli -- path/to/prompt.xml "prompts/**/*.xml"
```

测试只写入临时目录；不要提交本地生成的 `*.i.xml` / `*.o.xml`。

### 编译器测试要求 / Compiler test requirements

覆盖按顺序执行、独立文件环境、重复与未定义变量、惰性条件分支、只在编译语法展开变量、递归引用与循环诊断、物理文件行号，以及 `-I` / `-O` 的覆盖与失败清理边界。保留旧 `squish` 与批处理 API 回归测试。

Cover ordered execution, isolated file frames, duplicate and undefined variables, lazy conditions, compile-syntax-only expansion, recursive includes and cycles, physical source lines, and stage-specific overwrite/failure cleanup. Keep the existing lexical and batch API regression tests.

元数据测试须区分物理 `file` 与继承 `meta`，验证每条引用边省略 `openat` 均为 `self`、连续显式 `parent` 覆盖和兄弟隔离。`ifr` 要覆盖锚点与非法模式，`insert` 要覆盖 XML 转义及不递归执行。统计测试需用真实文件展开和分词增长验证 IR 基线，不以负数截断冒充节省。

Distinguish physical `file` from inheritable `meta`; test per-edge default `self`, consecutive explicit `parent` overlays, and sibling isolation. Cover regex anchors and errors, escaped nonrecursive insertion, and actual include/tokenizer growth when checking IR-based metrics. Never disguise growth by clamping negative savings.

诊断与颜色测试必须覆盖纯文本/彩色内容一致、输出重定向、环境偏好与显式覆盖、源码快照和控制字符转义。环境变量用子进程隔离，不在并发测试中修改进程级环境。原始源码与诊断展示分离，不为美化输出而修改输入，也不臆造核心未提供的列号。

Test plain/color equivalence, redirection, environment preferences and explicit overrides, source snapshots, and control-character escaping. Set environment variables on child processes, not globally in concurrent tests. Keep source data separate from presentation; do not alter inputs or fabricate source columns for prettier diagnostics.

### FSM 测试要求

扫描器变更至少应覆盖相关的：

- 标签单双引号中的 `>` 与空白；
- Comment、CDATA、PI 的精确结束符；
- DOCTYPE 引号、内部子集深度、嵌套 Comment/PI；
- UTF-8 多字节文本与字节偏移错误；
- 文件开头/结尾、空白游程、直接相邻 atom；
- 幂等性 `squish(squish(x)) == squish(x)`；
- `output_chars = input_chars - removed + inserted`；
- markup 内受保护空白计入 `recognized`，但不计入 `removed`。

优先添加小而明确的回归测试。只有属性测试（property-based testing）能表达的新不变量才值得引入相应依赖。

## 网站工作流

```bash
cd site
npm ci
npm run check
npm run build
# 或一次执行全部站点检查
npm run test
```

静态产物位于 `site/dist`，不要提交。简体中文 `/` 与英文 `/en/` 必须保持功能和信息对等；新增用户可见文案时同时更新两种语言。继续使用锁定版本且带 SRI 的 [MoeSegfault Style](https://github.com/kleedaisuki/moesegfault-style)，并检查键盘焦点、明暗主题、窄屏和减少动态效果（reduced motion）。

修改 `.github/workflows/pages.yml` 时遵循 [Astro GitHub Pages 指南](https://docs.astro.build/en/guides/deploy/github/)与 [GitHub 自定义工作流文档](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)，不要把密钥或账户相关 DNS 值写入仓库。

## 文档与提交

- 用户行为变化必须同步更新 `README.md` 的中文主体与 English 部分。
- 注释解释契约、不变量和非显然原因，不复述代码。
- 保持提交单一主题，例如 core、CLI、site、CI/docs 分开；使用祈使句描述做了什么。
- 不提交编辑器状态、构建产物、临时 XML 输出或无关格式化。
- Pull Request 应写明行为变化、边界情况、验证命令与潜在兼容影响。

## 许可

提交即表示你有权按项目的 [`GPL-3.0-or-later`](LICENSE) 许可提供该贡献。复制或派生第三方素材时，必须保留可核验的来源和许可说明；站点视觉系统来源见 [MoeSegfault Style](https://github.com/kleedaisuki/moesegfault-style)。
