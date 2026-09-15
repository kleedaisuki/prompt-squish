# 组合与递归 / Composition and recursion

从仓库根目录运行 / Run from the repository root:

```bash
xmlsquish build --manifest-path examples/semantic/xmlsquish.toml
xmlsquish build --manifest-path examples/semantic/xmlsquish.toml --emit prompt --emit ir --emit debug
xmlsquish inspect link prompt --manifest-path examples/semantic/xmlsquish.toml --format json
```

逻辑产物 `target/xmlsquish/prompt.prompt` 在项目的不可变 generation 中包含 `Hello, Klee &amp; friends!`，随后按顺序包含 `XML`、`macros`、`recursion` 三个 `Item`。`.xsir` 是可复用中间表示（Intermediate Representation, IR），`.psdbg` 是自包含调试包；它们都不是最终提示词。

The `.prompt` product contains the escaped greeting and three ordered `Item` elements. `.xsir` is reusable intermediate representation; `.psdbg` is a self-contained debug bundle. Neither companion is the prompt product.

`xs:entry` 声明编译入口，`expand` 是唯一的递归展开操作。`import` 仅装载定义；调用方的 `s` 与定义方的 `str` 绑定同一 URI，因此引用同一符号。`items` 用命名捕获（named capture）拆分字符串，并以显式参数递归；捕获只在当前条件块内有效。`greet` 的紧凑宏体避免向标量结果意外加入排版空白。

The `xs:entry` document declares the compilation entry; `expand` is the sole recursive expansion operation. Imports load definitions without producing content. Caller prefix `s` and definition prefix `str` identify the same namespace. `items` decomposes a string with named captures and recursively passes explicit arguments; captures remain lexical. The compact greeting body avoids unintended formatting whitespace.

这些示例替代旧版 `let`、`set`、`meta` 和 `$` 插值演示；当前契约见 [DSL 规范](../../docs/dsl.md)。

These examples replace the legacy mutable-variable and metadata language. See the DSL specification for the current contract.
