# Site demo / 网站演示

Run `xmlsquish -O examples/site-demo/agent.xml` from the repository root.
在仓库根目录运行以上命令；入口不需要命令行参数。

- `agent.xml` imports named task definitions and explicitly supplies `audience` to the mounted persona.
  入口导入任务宏定义，并向挂载的 persona 显式传入 `audience`。
- `persona.xml` requires an `audience` parameter; `tasks.xml` exports `demo:task`.
  persona 要求必需参数，tasks 导出静态命名宏。
- `-I` records provenance; `-O` removes internal metadata and compacts XML whitespace.
  `-I` 保留来源信息，`-O` 清理内部信息并压紧 XML 空白。

`npm --prefix site run demo:generate` rebuilds the recorded website examples using the real CLI.
`npm --prefix site run demo:check` verifies reproducibility. Display-only source URIs are normalized;
IR size is measured in UTF-8 bytes, while source/final token counts come from the CLI.
生成器通过真实 CLI 重建网站示例，检查命令验证可复现性。仅展示用源码 URI 被归一化；
IR 展示 UTF-8 字节数，源码与最终产物的 token 数来自 CLI。
