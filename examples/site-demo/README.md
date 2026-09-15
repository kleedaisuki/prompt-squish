# Site demo / 网站演示

Run `xmlsquish build --manifest-path examples/site-demo/xmlsquish.toml` from the repository root. Inspect the link with `xmlsquish inspect link agent --manifest-path examples/site-demo/xmlsquish.toml --format json`.
在仓库根目录运行以上命令；入口不需要命令行参数，逻辑产品定位符是 `target/xmlsquish/agent.prompt`；构建事件报告不可变 generation 中的实际路径。

- `agent.xml` is an `xs:entry` document: it imports persona and task modules, then constructs the prompt directly. All macros use `expand`, and persona receives an explicit `audience` value.
  `agent.xml` 使用 `xs:entry` 声明入口，导入 persona 与 task 模块后直接构造提示词；统一使用 `expand` 展开，并向 persona 显式传入 `audience`。
- `persona.xml` defines `demo:persona` with a required `audience` parameter; `tasks.xml` defines `demo:task`. Library modules contain only imports and macro definitions, never an entry.
  persona 宏要求必需参数，tasks 定义静态命名宏；库模块只包含导入与宏定义，不含入口。
- `--emit prompt|ir|debug` selects the `.prompt`, `.xsir`, and `.psdbg` artifacts; repeat the option instead of comma-separating values.
  `--emit prompt|ir|debug` 选择 `.prompt`、`.xsir` 与 `.psdbg` 产物；需多种产物时重复该选项，不使用逗号。

`npm --prefix site run demo:generate` rebuilds the recorded website examples using the real CLI.
`npm --prefix site run demo:check` verifies reproducibility. Display-only source URIs are normalized;
IR size is measured in UTF-8 bytes, while source/final token counts come from the CLI.
生成器通过真实 CLI 重建网站示例，检查命令验证可复现性。仅展示用源码 URI 被归一化；
IR 展示 UTF-8 字节数，源码与最终产物的 token 数来自 CLI。
