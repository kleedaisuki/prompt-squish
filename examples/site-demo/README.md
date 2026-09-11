# Site demo / 网站演示

Run `xmlsquish -O examples/site-demo/agent.xml` from the repository root.
在仓库根目录运行以上命令；入口不需要命令行参数。

- `agent.xml` imports persona and task definitions; `entry="demo:main"` selects its explicit root macro. All macros use `expand`, and persona receives an explicit `audience` value.
  入口导入 persona 与 task 宏定义，以 `entry="demo:main"` 指定根宏；统一使用 `expand` 展开，并向 persona 显式传入 `audience`。
- `persona.xml` defines `demo:persona` with a required `audience` parameter; `tasks.xml` defines `demo:task`. Library modules need no entry.
  persona 宏要求必需参数，tasks 定义静态命名宏；库模块无需入口。
- `-I` records provenance; `-O` removes every attribute and namespace prefix, then compacts XML whitespace.
  `-I` 保留来源信息，`-O` 移除所有属性与命名空间前缀，再压紧 XML 空白。

`npm --prefix site run demo:generate` rebuilds the recorded website examples using the real CLI.
`npm --prefix site run demo:check` verifies reproducibility. Display-only source URIs are normalized;
IR size is measured in UTF-8 bytes, while source/final token counts come from the CLI.
生成器通过真实 CLI 重建网站示例，检查命令验证可复现性。仅展示用源码 URI 被归一化；
IR 展示 UTF-8 字节数，源码与最终产物的 token 数来自 CLI。
