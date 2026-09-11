import specification from "../../../../docs/dsl.md?raw";

/** Serve the current working specification without duplicating its source.
 * 提供当前工作规范，避免复制源码；已发布版本快照保持不变。
 */
export function GET(): Response {
  return new Response(specification, {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
}
