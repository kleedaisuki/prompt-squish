# 显式传参，而非继承 / Explicit arguments, not inheritance

目录名保留以便找到迁移示例；语言已不支持 `openat` 或隐式 `meta` 继承。

The historical directory name is retained for migration discoverability. The language no longer supports `openat` or implicit metadata inheritance.

```bash
cargo run -- examples/inheritance/prompt.xml
cargo run -- --explain examples/inheritance/prompt.xml
```

从仓库根目录运行。输出的两个 `Section` 分别包含 `researchers` 与 `everyone`。每次 `mount` 创建独立展开帧（Expansion Frame），但 `section.xml` 和 `leaf.xml` 每次编译只装载一次。

Run from the repository root. Two sections contain `researchers` and `everyone` respectively. Each mount creates a fresh frame, while each source is loaded only once per compilation.

`section.xml` 显式把 `arg.audience` 传给 `leaf.xml`；删去该参数将报错，而不是自动继承。相对路径 `./leaf.xml` 始终相对 `section.xml` 解析。标题通过 `fill` 传递 XML 节点，不作为字符串；缺少必需标题也会报错。

The section explicitly forwards its argument to the leaf: removing it is an error, not inheritance. The leaf path resolves relative to the section definition. Titles pass as XML through fills, not strings; omitting the required title also fails.
