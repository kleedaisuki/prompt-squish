import type { Locale } from "./messages";

/** Shared product-shell navigation copy.
 * 产品外壳共用的导航文案。
 */
export interface ReferenceMessages {
  nav: {
    label: string;
    home: string;
    namespace: string;
    releases: string;
    footerLabel: string;
  };
}

/** Navigation labels used by every product route; page-specific prose lives beside its feature.
 * 所有产品路由共用的导航标签；页面专属文案保存在对应功能模块中。
 */
export const referenceMessages: Record<Locale, ReferenceMessages> = {
  "zh-CN": {
    nav: {
      label: "产品导航",
      home: "首页",
      namespace: "DSL 用户手册",
      releases: "发布记录",
      footerLabel: "项目资源",
    },
  },
  en: {
    nav: {
      label: "Product navigation",
      home: "Home",
      namespace: "DSL manual",
      releases: "Release log",
      footerLabel: "Project resources",
    },
  },
};
