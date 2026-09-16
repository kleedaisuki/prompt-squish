/** A downloadable native archive published by the release workflow.
 * 发布工作流生成的可下载原生安装包。 */
export interface ReleasePlatform {
  name: string;
  note: { en: string; "zh-CN": string };
  targets: ReadonlyArray<{ label: string; triple: string }>;
  extension: "zip" | "tar.gz";
}

/** Product-release facts shared by visible pages and structured metadata.
 * 页面与结构化元数据共享的产品发布事实。 */
export interface ProductRelease {
  version: string;
  tag: string;
  date: string;
  protocol: string;
  status: "stable";
  githubRelease: string;
  metadataPath: string;
  checksums: string;
  sourceInstall: string;
  platforms: ReadonlyArray<ReleasePlatform>;
}

export const repository = "https://github.com/kleedaisuki/prompt-squish";

const version = "1.0.1";
const tag = `v${version}`;
const downloadRoot = `${repository}/releases/download/${tag}`;

/** The current stable product release. / 当前稳定产品版本。 */
export const latestRelease: ProductRelease = {
  version,
  tag,
  date: "2026-09-16",
  protocol: "3.0",
  status: "stable",
  githubRelease: `${repository}/releases/tag/${tag}`,
  metadataPath: `/releases/${version}.json`,
  checksums: `${downloadRoot}/SHA256SUMS`,
  sourceInstall: `cargo install --git ${repository} --tag ${tag} --locked`,
  platforms: [
    {
      name: "Windows",
      note: { en: "Windows 10/11 · ZIP", "zh-CN": "Windows 10/11 · ZIP" },
      targets: [
        { label: "x64", triple: "x86_64-pc-windows-msvc" },
        { label: "ARM64", triple: "aarch64-pc-windows-msvc" },
      ],
      extension: "zip",
    },
    {
      name: "Linux",
      note: { en: "glibc 2.35+ · tar.gz", "zh-CN": "glibc 2.35+ · tar.gz" },
      targets: [
        { label: "x64", triple: "x86_64-unknown-linux-gnu" },
        { label: "ARM64", triple: "aarch64-unknown-linux-gnu" },
      ],
      extension: "tar.gz",
    },
    {
      name: "macOS",
      note: { en: "macOS 11+ · tar.gz", "zh-CN": "macOS 11+ · tar.gz" },
      targets: [
        { label: "Intel", triple: "x86_64-apple-darwin" },
        { label: "Apple Silicon", triple: "aarch64-apple-darwin" },
      ],
      extension: "tar.gz",
    },
  ],
};

/** Build the workflow-owned archive URL without duplicating its naming rule.
 * 复用工作流命名规则生成安装包 URL。 */
export function assetUrl(platform: ReleasePlatform, target: ReleasePlatform["targets"][number]): string {
  return `${downloadRoot}/xmlsquish-${version}-${target.triple}.${platform.extension}`;
}
