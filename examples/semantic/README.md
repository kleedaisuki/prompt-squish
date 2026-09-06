# 语义编译示例 / Semantic compilation example

从仓库根目录运行 / Run from the repository root:

```sh
cargo run -p xmlsquish-cli -- -I examples/semantic/prompt.xml
cargo run -p xmlsquish-cli -- -O examples/semantic/prompt.xml
```

`-I` 生成保留普通文本布局的 `prompt.i.xml`；`-O` 生成压缩后的 `prompt.o.xml` 并删除对应中间文件。源文件不修改。日志包含所属物理文件名及行号。

`-I` writes `prompt.i.xml` without whitespace compression; `-O` writes compact `prompt.o.xml` and removes the corresponding intermediate. Sources stay untouched. Logs identify the physical source file and line.

`mount` 保留 `greeting` 根，`import` 只保留 `fragment` 的内容。三个文件都可以定义 `msg`，但互不可见。普通标签、属性和文本中的 `$msg` 不会展开。

`mount` retains the `greeting` root; `import` retains only the contents of `fragment`. All three files can define `msg` independently. `$msg` in ordinary elements, attributes, and text is not expanded.

示例先用 `let` 声明 `result`，再用互补的 `if` / `ifn` 分支中的 `set` 赋值。`set` 不能创建变量，也不能修改其他文件的变量或内置命名空间。缺失的环境变量会报错，而不是自动当作空字符串。

The example declares `result` with `let`, then assigns it using `set` in complementary `if` / `ifn` branches. `set` cannot create variables or mutate other files' variables or built-in namespaces. Missing environment variables are errors, not empty strings.
