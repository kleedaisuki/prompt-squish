/** A downloadable native archive published by the release workflow.
 * 发布工作流生成的可下载原生安装包。 */
export interface ReleasePlatform {
  name: string;
  note: { en: string; "zh-CN": string };
  targets: ReadonlyArray<{ label: string; triple: string }>;
  extension: "zip" | "tar.gz";
}

/** A release with workflow-built archives and a checksum manifest.
 * 带工作流原生归档与校验清单的版本。 */
export interface NativeAcquisition {
  kind: "native";
  checksums: string;
  sourceInstall: string;
  platforms: ReadonlyArray<ReleasePlatform>;
}

/** A historical release acquired only from its immutable source tag.
 * 只能从不可变源码标签获取的历史版本。 */
export interface SourceAcquisition {
  kind: "source";
  sourceInstall: string;
  minimumRust: string;
}

export type ReleaseAcquisition = NativeAcquisition | SourceAcquisition;
export type ReleaseVersion = "1.0.4" | "1.0.2" | "1.0.1" | "1.0.0" | "0.3.0" | "0.2.0";

/** Locale-neutral facts for one immutable product release.
 * 单个不可变产品版本的非本地化事实。 */
export interface ProductRelease {
  version: ReleaseVersion;
  tag: `v${ReleaseVersion}`;
  date: string;
  protocol?: string;
  status: "current" | "historical";
  githubRelease: string;
  metadataPath?: `/releases/${ReleaseVersion}.json`;
  acquisition: ReleaseAcquisition;
}

export const repository = "https://github.com/kleedaisuki/prompt-squish";

const nativePlatforms: ReadonlyArray<ReleasePlatform> = [
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
];

function nativeAcquisition(version: ReleaseVersion): NativeAcquisition {
  const tag = `v${version}`;
  const downloadRoot = `${repository}/releases/download/${tag}`;
  return {
    kind: "native",
    checksums: `${downloadRoot}/SHA256SUMS`,
    sourceInstall: `cargo install --git ${repository} --tag ${tag} --locked`,
    platforms: nativePlatforms,
  };
}

/** Reverse-chronological catalog; ordering is product chronology, never translation order.
 * 逆时间顺序目录；产品顺序绝不依赖翻译数组位置。 */
export const releases: ReadonlyArray<ProductRelease> = [
  {
    version: "1.0.4",
    tag: "v1.0.4",
    date: "2026-09-20",
    protocol: "3.1",
    status: "current",
    githubRelease: `${repository}/releases/tag/v1.0.4`,
    metadataPath: "/releases/1.0.4.json",
    acquisition: nativeAcquisition("1.0.4"),
  },
  {
    version: "1.0.2",
    tag: "v1.0.2",
    date: "2026-09-17",
    protocol: "3.1",
    status: "historical",
    githubRelease: `${repository}/releases/tag/v1.0.2`,
    metadataPath: "/releases/1.0.2.json",
    acquisition: nativeAcquisition("1.0.2"),
  },
  {
    version: "1.0.1",
    tag: "v1.0.1",
    date: "2026-09-16",
    protocol: "3.0",
    status: "historical",
    githubRelease: `${repository}/releases/tag/v1.0.1`,
    metadataPath: "/releases/1.0.1.json",
    acquisition: nativeAcquisition("1.0.1"),
  },
  {
    version: "1.0.0",
    tag: "v1.0.0",
    date: "2026-09-15",
    protocol: "2.1",
    status: "historical",
    githubRelease: `${repository}/releases/tag/v1.0.0`,
    metadataPath: "/releases/1.0.0.json",
    acquisition: nativeAcquisition("1.0.0"),
  },
  {
    version: "0.3.0",
    tag: "v0.3.0",
    date: "2026-09-11",
    status: "historical",
    githubRelease: `${repository}/releases/tag/v0.3.0`,
    acquisition: nativeAcquisition("0.3.0"),
  },
  {
    version: "0.2.0",
    tag: "v0.2.0",
    date: "2026-09-11",
    status: "historical",
    githubRelease: `${repository}/releases/tag/v0.2.0`,
    acquisition: {
      kind: "source",
      minimumRust: "1.88",
      sourceInstall: `cargo install --git ${repository} --tag v0.2.0 --locked`,
    },
  },
];

/** The current stable product release. / 当前稳定产品版本。 */
export const latestRelease: ProductRelease = releases[0];

/** Resolve one catalog record without accepting arbitrary version strings.
 * 只解析目录内版本，拒绝任意版本字符串。 */
export function releaseByVersion(version: string): ProductRelease | undefined {
  return releases.find((release) => release.version === version);
}

/** Build one workflow-owned archive URL from the selected release.
 * 按指定版本生成工作流所拥有的归档 URL。 */
export function assetUrl(
  release: ProductRelease,
  platform: ReleasePlatform,
  target: ReleasePlatform["targets"][number],
): string {
  if (release.acquisition.kind !== "native") {
    throw new TypeError(`Release ${release.version} has no native archives`);
  }
  return `${repository}/releases/download/${release.tag}/xmlsquish-${release.version}-${target.triple}.${platform.extension}`;
}
