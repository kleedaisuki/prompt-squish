# 元数据继承与显式插入 / Metadata inheritance and explicit insertion

从仓库根目录运行 / Run from the repository root:

```sh
cargo run -p xmlsquish-cli -- -I examples/inheritance/prompt.xml
cargo run -p xmlsquish-cli -- -O examples/inheritance/prompt.xml
```

`file:name` 始终是实际源文件名；`meta:author` 则可继承。父文件通过 `openat="parent"` 加载 `section.xml` 时，`klee` 覆盖 `section-author`，并通过同样显式写 `openat="parent"` 的后续 `import` 继续覆盖 `leaf-author`。省略参数始终等于 `self`，不沿用上一次引用边的模式。

`file:name` always names the physical source; `meta:author` can be inherited. Loading `section.xml` with `openat="parent"` replaces `section-author` with `klee`, which also overrides `leaf-author` in the subsequent import explicitly using `openat="parent"`. Omission always means `self`, independent of the preceding edge's mode.

| 所在位置 / Location | `meta:author` | `file:name` |
| --- | --- | --- |
| 第一个 section / First section | `klee` | `section.xml` |
| 第一个 section 内 import / Import in first section | `klee` | `leaf.xml` |
| 第一个 section 内省略参数 / Omitted parameter in first section | `leaf-author` | `leaf.xml` |
| 第二个 section / Second section | `section-author` | `section.xml` |
| 第二个 section 内 parent import / Parent import in second section | `section-author` | `leaf.xml` |

`insert` 按名字查找变量，不写 `$`，输出是经过 XML 转义的文本，变量中的 `<researcher>` 不会变成元素。`ifr` 的 `str` 展开变量，`pattern` 是原样正则表达式（regular expression），因此 `$` 可以直接表示末尾锚点。

`insert` looks up a variable name without `$` and writes XML-escaped text: `<researcher>` inside the value does not become an element. `ifr` expands its `str` argument but treats `pattern` as a literal regular expression, so `$` can directly denote an end anchor.
