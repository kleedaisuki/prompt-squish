# ADR 0004：单一 Cargo package / One Cargo package

- 状态 / Status：已接受 / Accepted
- 日期 / Date：2026-09-11
- 替代范围 / Supersedes：[ADR 0001](0001-lexical-canonicalization-and-layering.md) 的三包分层；词法、编译与文件输出契约不变。The three-package layering only; lexical, compilation, and output contracts remain unchanged.

## 观察 / Observations

仓库只有一个命令行产品，没有独立发布三个 package 的需求。当前 CLI 的两阶段流水线不使用 `xmlsquish-app::BatchProcessor`；app 自己维护另一套批处理、报告模型和测试。生产路径仅借用 app 的简单转换/计数 traits、重复的结果类型及输出路径函数。

There is one CLI product and no independent release requirement. The production two-stage pipeline bypasses the application's batch processor. The application package maintains a second, unused processing policy; its remaining consumers use only trivial adapter traits, duplicate result types, and an output-path helper.

## 决策 / Decision

采用 Cargo 的标准单 package 布局：`xmlsquish` 包含一个 library target 和一个 binary target。严格说这是两个编译 crate，而不是三个独立 package；目标是消除包级管理及重复实现，不是强迫所有代码进入一个文件。

Use one `xmlsquish` package with library and binary targets. These are technically two compilation crates, not three independently managed packages. The goal is to remove packaging overhead and duplicate implementations, not module boundaries.

| 边界 / Boundary | 职责 / Responsibility |
| --- | --- |
| `src/lib.rs` | 精简库入口 / Small library surface |
| `src/compiler.rs` | 宏语义与文件环境；保留注入加载函数 / Macro semantics and file frames; retain the injected loader |
| `src/squish.rs` | 纯词法压缩与空白账本 / Pure lexical normalization and whitespace accounting |
| `src/cli/` | 唯一两阶段流水线、统计、发现及终端呈现 / The only two-stage pipeline, metrics, discovery, and terminal presentation |
| `src/cli/files.rs` | 严格 UTF-8、BOM 与原子替换 / Strict UTF-8, BOM, and atomic replacement |
| `src/main.rs` | 进程入口与退出码 / Process entry and exit status |

删除 app package，而非将其改名为模块。CLI 直接使用压缩器的结果及 tokenizer，不再经过 `CoreSquasher`、`O200kTokens` 和三个单一用途 traits。保留真实的变化边界：编译器的加载函数支持确定性测试；文件细节和终端显示隐藏在 CLI 内部。输出阶段用枚举表达，避免在流水线内部传递含义不明的布尔值。

Delete the application package rather than rename it to a module. Call the squasher and tokenizer directly, without adapter objects or duplicate models. Preserve seams with demonstrated value: the compiler loader supports deterministic tests, and the CLI hides file persistence and presentation. Represent the output stage with an enum instead of passing a stage boolean through the pipeline.

测试紧贴所属模块，统一使用 `*.test.rs`，通过 `#[cfg(test)]` 和 `#[path]` 关联单元测试。真实进程测试同样放在 CLI 模块旁，通过 Cargo `[[test]]` 显式声明；不设根目录 `tests/`。发布清单只包含 Rust 源码、测试所需样例和项目文档，不打包网站资产。

Keep tests beside their owning modules as `*.test.rs`, using `#[cfg(test)]` and `#[path]` for unit tests. Register adjacent CLI process tests explicitly with Cargo `[[test]]`; do not introduce a root `tests/` directory. Restrict package contents to Rust sources, required example fixtures, and project documentation, excluding site assets.

## 迁移与不变量 / Migration and invariants

- 安装 / Install：`cargo install --path . --locked`。
- 运行 / Run：`cargo run -- <arguments>`；无需 / no `-p xmlsquish-cli`。
- 库 / Library：使用 `xmlsquish::Compiler` 和 `xmlsquish::squish`；不保留旧包名、旧 app API 或兼容包装。Use the unified library; old package names and the unused app API have no compatibility wrappers.
- 保持 CLI 参数、宏语言、日志次序、成功文件统计及退出状态。Preserve CLI arguments, language semantics, log ordering, successful-artifact metrics, and exit status.
- 输入永不覆盖；单文件失败不阻断其他输入。Never overwrite inputs; failures remain isolated per input.
- 保留 BOM，但不计入文本统计；临时文件仍创建在目标目录并原子替换。Preserve BOM outside text measurements and replace outputs atomically using a temporary file in the destination directory.
- 最终输出持久化成功后才删除对应中间文件；失败时保留诊断上下文和已有产物。Only remove the corresponding IR after successful final persistence; preserve diagnostic context and existing artifacts on failure.

这是结构简化，不宣称运行速度或 token 压缩率提升。保留编译器和词法算法，不顺带增加并行执行、缓存、插件框架或新的依赖。

This is structural simplification, not a runtime-speed or compression claim. Keep the compiler and lexical algorithms intact; do not introduce parallel execution, caching, plugin frameworks, or new dependencies.

## 依据与取舍 / Evidence and trade-offs

- [Cargo Targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html) 定义单 package 下的库、二进制与集成测试目标；本项目使用这一常规机制，而非自定义构建框架。Cargo directly supports the required library, binary, and integration-test targets.
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/future-proofing.html) 提醒公开表示会形成长期承诺；只有调用方需要的入口应公开，内部适配细节不是 API。Public representation is a commitment; internal adapter details should not become API.
- Parnas 的[信息隐藏论文（CACM, 1972）](https://doi.org/10.1145/361598.361623)支持按隐藏的设计决策划分模块，而非机械按步骤分层。它不是“单 package 更快”的实验证据；当前决策来自仓库实际依赖和使用路径。Information hiding motivates the module boundaries, not a universal package-count rule or performance claim.
- 现代 Rust 验证研究如 [RefinedRust（PLDI, 2024）](https://iris-project.org/pdfs/2024-pldi-refinedrust.pdf)研究更强的功能正确性证明，但不能替代本轮文件与进程级回归。若未来要证明扫描器不变量，可单独评估其工具链及证明成本；本轮不引入证明基础设施。Functional-verification research is relevant to future scanner proofs, not evidence that this refactor is verified or a reason to add proof tooling now.

单 package 使纯算法使用者也依赖当前 CLI 所用依赖集合。这是为单一命令行产品选择的明确取舍；若出现真实的轻量嵌入式消费者、浏览器平台隔离或独立发布节奏，再评估 feature 或 package 边界，不预先搭建框架。

A single package also gives library consumers the current CLI dependency set. This is an explicit trade-off for one CLI product. Revisit features or package boundaries when a real lightweight consumer, browser platform constraint, or independent release cadence exists.

## 验证要求 / Verification

运行格式化、Clippy、全部存活语义测试、库文档测试和最低 Rust 1.88 检查；从根目录安装到临时位置并运行实际二进制。若本地已有网页示例生成器，检查它调用新包名后的输出不漂移。删除的旧 app 测试不作为生产覆盖保留；不得删除活跃流水线测试来凑通过率。

Check formatting, Clippy, all live behavior tests, library doctests, and Rust 1.88. Install from the root into a temporary location and run the installed binary. If the local site demo generator exists, check its output under the new package name. Do not retain dead abstraction tests as production coverage, or remove live pipeline tests to obtain a green result.
