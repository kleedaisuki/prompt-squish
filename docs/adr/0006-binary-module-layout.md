# ADR 0006: Binary-owned semantic modules / 二进制拥有的语义模块

- Status / 状态：Accepted
- Supersedes / 替代：ADR 0004 的 library/binary 双目标布局，保留单 package 原则。

## Decision / 决定

产品交付物是命令行程序，不为内部测试维护一个公开库门面。`src/main.rs` 拥有私有 `cli`、`compiler` 和 `squish` 模块。编译器入口及实现统一位于 `src/compiler/`，入口使用 `mod.rs`，不再并列保留 `src/compiler.rs`。

The product is a CLI executable. `src/main.rs` owns private CLI, compiler, and squish modules; tests do not justify a public library facade. The compiler entry and implementation live together under `src/compiler/mod.rs`.

测试按语义归属放在源码模块旁，通过 `#[cfg(test)]` 装配到二进制单元测试，不建立根 `tests/` 汇总目录。确实需要启动真实进程的测试放在 `src/cli/`，由 Cargo 的显式 `[[test]]` 注册，不引用或复制内部实现。

Semantic tests live next to their owning modules and join the binary's unit-test harness with `#[cfg(test)]`. Real process tests remain adjacent to the CLI and use explicit Cargo test targets, without importing or duplicating implementation modules.

```text
src/
├── main.rs
├── compiler/
│   ├── mod.rs
│   ├── model.rs
│   ├── parser.rs
│   ├── runtime.rs
│   └── *.test.rs
├── cli/
│   ├── mod.rs
│   └── *.test.rs
├── squish.rs
└── squish.test.rs
```

这一选择遵循 [Cargo targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html) 对二进制单元测试及真实可执行文件测试的支持。此次仅调整组织边界，不改变 DSL、来源信息、预算或最终产品空白压缩；研究层面的语言模型与 ADR 0005 保持不变。

Cargo supports unit tests inside binaries and process tests through `CARGO_BIN_EXE_*`. This is an ownership/layout change, not a language or algorithm change: DSL semantics, provenance, guards, and final whitespace compression stay unchanged.

## Verification / 验证

保留迁移前语义回归；检查 Cargo 元数据不再包含 lib target，运行所有测试、Clippy、格式检查、最低 Rust 版本检查及站点示例复现检查。

Preserve semantic coverage; verify that Cargo metadata has no library target, and run all tests, Clippy, formatting, MSRV checking, and the site's reproducible CLI examples.
