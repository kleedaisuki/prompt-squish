import type { Locale } from "./messages";

/** Document routes are localized; the XML vocabulary identity is never localized.
 * 文档路径随语言变化，XML 词汇身份始终不变。 */
export const namespaceUri = "https://xmlsquish.moesegfault.dev/ns";
export type Page = "home" | "namespace" | "releases";
export const paths: Record<Locale, Record<Page, string>> = {
  "zh-CN": { home: "/", namespace: "/ns", releases: "/releases/" },
  en: { home: "/en/", namespace: "/en/ns/", releases: "/en/releases/" },
};
