import type { Locale } from "./messages";

/** Document routes are localized; the XML vocabulary identity is never localized.
 * 文档路径随语言变化，XML 词汇身份始终不变。 */
export const namespaceUri = "https://xmlsquish.moesegfault.dev/ns";
export type Page = "home" | "namespace" | "releases";
/** One route represented in every supported locale. / 同一路由在所有支持语言中的路径。 */
export type LocalizedPaths = Readonly<Record<Locale, string>>;

export const paths: Readonly<Record<Locale, Readonly<Record<Page, string>>>> = {
  "zh-CN": { home: "/", namespace: "/ns", releases: "/releases/" },
  en: { home: "/en/", namespace: "/en/ns/", releases: "/en/releases/" },
};

/** Return the localized route pair for a top-level product page.
 * 返回顶层产品页面的双语路由，供 canonical、alternate 与语言切换共同使用。
 */
export function localizedPathsFor(page: Page): LocalizedPaths {
  return { "zh-CN": paths["zh-CN"][page], en: paths.en[page] };
}
