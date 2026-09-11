# 组合与递归 / Composition and recursion

从仓库根目录运行 / Run from the repository root:

```bash
cargo run -- examples/semantic/prompt.xml
cargo run -- -I examples/semantic/prompt.xml
```

`prompt.o.xml` 包含 `Hello, Klee &amp; friends!`，随后按顺序包含 `XML`、`macros`、`recursion` 三个 `Item`。最终产品阶段会压缩空白，编译与中间表示不做此压缩。`prompt.i.xml` 是来源可追踪的中间表示，而不是最终提示词。

The final document contains the escaped greeting and three ordered `Item` elements. The final product pass compresses whitespace; compilation and the intermediate representation do not. The intermediate file includes provenance and is not the final prompt.

`entry="s:main"` 选择根宏，`expand` 是唯一的递归展开操作。`import` 仅装载定义；调用方的 `s` 与定义方的 `str` 绑定同一 URI，因此引用同一符号。`items` 用命名捕获（named capture）拆分字符串，并以显式参数递归；捕获只在当前条件块内有效。`greet` 的紧凑宏体避免向标量结果意外加入排版空白。

The entry selects `s:main`; `expand` is the sole recursive expansion operation. Imports load definitions without producing content. Caller prefix `s` and definition prefix `str` identify the same namespace. `items` decomposes a string with named captures and recursively passes explicit arguments; captures remain lexical. The compact greeting body avoids unintended formatting whitespace.

这些示例替代旧版 `let`、`set`、`meta` 和 `$` 插值演示；当前契约见 [DSL 规范](../../docs/dsl.md)。

These examples replace the legacy mutable-variable and metadata language. See the DSL specification for the current contract.
