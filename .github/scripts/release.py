"""Package native binaries and publish a complete matrix. / 原生二进制打包及完整矩阵发布。

Run from the checkout parent: / 从检出目录的父目录运行：
    RELEASE_TAG=v1.0.4 RELEASE_TARGET=x86_64-unknown-linux-gnu \
      python automation/.github/scripts/release.py package source dist

Version-contract check: / 版本契约检查：
    python .github/scripts/release.py verify . v1.0.4
"""

import hashlib
import gzip
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import zipfile


# Closed matrix prevents silently publishing a partial release. / 封闭矩阵防止不完整发布。
TARGETS = (
    "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc",
    "x86_64-apple-darwin", "aarch64-apple-darwin",
)
COMMANDS = ("new", "build", "clean", "fmt", "add", "remove", "inspect")
STABLE_TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def run(*args, cwd=None):
    """Run a checked command without shell interpolation. / 无 shell 插值地执行并检查命令。"""
    return subprocess.check_output(args, cwd=cwd, text=True, encoding="utf-8").strip()


def archive_name(version, target):
    """Return the public stable asset contract. / 返回稳定的公开资产命名。"""
    suffix = "zip" if "windows" in target else "tar.gz"
    return f"xmlsquish-{version}-{target}.{suffix}"


def digest(path):
    """Hash the exact published bytes. / 对发布字节计算摘要。"""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_repository(source, tag=None):
    """Keep release metadata mutually consistent. / 保持发布元数据彼此一致。"""
    manifest = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["package"]["version"]
    expected_tag = tag or f"v{version}"
    if not STABLE_TAG.fullmatch(expected_tag):
        raise ValueError("expected a stable vMAJOR.MINOR.PATCH tag")
    if expected_tag != f"v{version}":
        raise ValueError("Cargo package version does not match release tag")

    lock = tomllib.loads((source / "Cargo.lock").read_text(encoding="utf-8"))
    roots = [
        package for package in lock.get("package", [])
        if package.get("name") == manifest["package"]["name"] and "source" not in package
    ]
    if len(roots) != 1 or roots[0].get("version") != version:
        raise ValueError("Cargo.lock root package version does not match Cargo.toml")

    changelog = (source / "CHANGELOG.md").read_text(encoding="utf-8")
    release_path = source / "docs" / "releases" / f"{version}.md"
    required_changelog_fragments = (
        f"## [{version}]",
        f"(docs/releases/{version}.md)",
        f"[{version}]: https://github.com/kleedaisuki/prompt-squish/releases/tag/{expected_tag}",
    )
    if any(fragment not in changelog for fragment in required_changelog_fragments):
        raise ValueError("CHANGELOG.md does not contain the current release contract")
    if not release_path.is_file():
        raise ValueError(f"missing release notes: {release_path.relative_to(source)}")
    notes = release_path.read_text(encoding="utf-8")
    if f"# xmlsquish {version}" not in notes or f"`{expected_tag}`" not in notes:
        raise ValueError("release notes do not match the current package version and tag")
    print(f"Verified release metadata for {expected_tag}")
    return version


def third_party_licenses(root, metadata):
    """Retain shipped dependency and standard-library legal notices. / 保留依赖与标准库法律声明。"""
    destination = root / "THIRD_PARTY_LICENSES"
    destination.mkdir()
    inventory = []
    for dependency in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if dependency["id"] in metadata["workspace_members"]:
            continue
        package_root = Path(dependency["manifest_path"]).parent
        name = f"{dependency['name']}-{dependency['version']}"
        copied = []
        for file in sorted(package_root.rglob("*")):
            if file.is_file() and file.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT")):
                relative = file.relative_to(package_root)
                output = destination / name / relative
                output.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(file, output)
                copied.append(relative.as_posix())
        if not copied:
            # Some crates omit repository-root licenses from the crates.io archive.
            # 部分 crate 未将仓库根许可证打包；仅使用经审核的固定版本补充文件。
            supplement = Path(__file__).resolve().parents[1] / "licenses" / name
            if not supplement.is_dir():
                raise ValueError(f"dependency ships no legal notices; review before release: {name}")
            shutil.copytree(supplement, destination / name)
            copied = sorted(file.relative_to(supplement).as_posix() for file in supplement.rglob("*") if file.is_file())
        inventory.append({"package": name, "license": dependency.get("license"), "source": dependency.get("source"), "notices": copied})
    (destination / "inventory.json").write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    sysroot = Path(run("rustc", "+1.88.0", "--print", "sysroot"))
    rust_docs = sysroot / "share" / "doc" / "rust"
    rust_destination = destination / "rust-1.88.0"
    shutil.copytree(rust_docs / "licenses", rust_destination / "licenses")
    for name in ("COPYRIGHT.html", "COPYRIGHT-library.html"):
        shutil.copyfile(rust_docs / name, rust_destination / name)


def package(source, dist, tag):
    """Smoke-test the native binary before packaging. / 原生执行验证后才允许打包。"""
    target = os.environ["RELEASE_TARGET"]
    if target not in TARGETS:
        raise ValueError(f"unsupported target: {target}")
    version = verify_repository(source, tag)
    metadata = json.loads(run("cargo", "+1.88.0", "metadata", "--locked", "--filter-platform", target, "--format-version=1", cwd=source))
    package_info = next(p for p in metadata["packages"] if p["name"] == "xmlsquish")
    if package_info["version"] != version:
        raise ValueError("Cargo package version does not match release tag")
    executable = "xmlsquish.exe" if "windows" in target else "xmlsquish"
    binary = source / "target" / target / "release" / executable
    if run(str(binary), "--version") != f"xmlsquish {version}":
        raise ValueError("binary version does not match release tag")
    with tempfile.TemporaryDirectory() as directory:
        staging = Path(directory)
        help_text = run(str(binary))
        for command in COMMANDS:
            if not re.search(rf"^  {command}\s", help_text, re.MULTILINE):
                raise ValueError(f"binary help omits direct command: {command}")
            run(str(binary), command, "--help")
        # Exercise project creation and the public .prompt artifact through the installed binary.
        # 通过已安装二进制验证项目创建与公开 .prompt 产物。
        run(str(binary), "new", "release-smoke", "--vcs=none", cwd=staging)
        project = staging / "release-smoke"
        if not (project / "xmlsquish.toml").is_file() or not (project / "src" / "prompt.xml").is_file():
            raise ValueError("native new smoke test omitted canonical project files")
        if (project / ".git").exists() or (project / ".gitignore").exists():
            raise ValueError("native new --vcs=none smoke test created Git state")
        if (project / "xmlsquish.lock").exists():
            raise ValueError("native new smoke test unexpectedly created a lockfile")
        run(str(binary), "fmt", "--check", "--plain", cwd=project)
        source_file = project / "src" / "prompt.xml"
        source_file.write_text(
            "<xs:entry  xmlns:xs = 'https://xmlsquish.moesegfault.dev/ns' >"
            "<Prompt><System>Hello   release</System></Prompt></xs:entry >",
            encoding="utf-8",
        )
        run(str(binary), "fmt", "--plain", cwd=project)
        run(str(binary), "build", "--offline", "--plain", cwd=project)
        target_root = project / "target" / "xmlsquish"
        artifacts = target_root / "artifacts"
        prompt = artifacts / "prompt.prompt"
        if not prompt.is_file():
            raise ValueError("native build did not publish target/xmlsquish/artifacts/prompt.prompt")
        if not (target_root / "cache" / "cas").is_dir() or not (target_root / "cache" / "actions.sqlite3").is_file():
            raise ValueError("native build did not create its project-local cache")
        if not (target_root / "metadata" / "layout.json").is_file():
            raise ValueError("native build did not create its layout discriminator")
        run(str(binary), "build", "--emit=ir", "--offline", "--plain", cwd=project)
        ir = list((artifacts / "ir").rglob("*.xsir"))
        if len(ir) != 1:
            raise ValueError("native explicit IR build smoke test did not publish exactly one stable IR artifact")
        exposed_private = [
            path for path in artifacts.rglob("*")
            if path.name == ".squish-publish"
            or re.fullmatch(r"[0-9a-f]{32,}", path.name)
            or path.name.endswith((".xsmap", ".build.json"))
        ]
        if exposed_private or (artifacts / "target" / "xmlsquish").exists():
            raise ValueError("native build exposes private publication state below artifacts")
        inspected = json.loads(run(str(binary), "inspect", "link", "prompt", "--format=json", cwd=project))
        if inspected.get("view") != "link":
            raise ValueError("native inspect smoke test returned the wrong view")
        run(str(binary), "clean", "--plain", cwd=project)
        if target_root.exists():
            raise ValueError("native clean smoke test retained the project-local build root")
        run(str(binary), "build", "--offline", "--plain", cwd=project)
        if not prompt.is_file():
            raise ValueError("native project did not rebuild after clean")
        root = staging / f"xmlsquish-{version}-{target}"
        root.mkdir()
        shutil.copy2(binary, root / executable)
        shutil.copy2(source / "LICENSE", root / "LICENSE")
        third_party_licenses(root, metadata)
        (root / "README.txt").write_text(
            f"xmlsquish {version} ({target})\n\n"
            "EN: Extract the archive and place xmlsquish (xmlsquish.exe on Windows)\n"
            "in a directory on PATH. Run xmlsquish --version, then run xmlsquish build\n"
            "from a project containing xmlsquish.toml.\n"
            "ZH: 解压后将 xmlsquish（Windows 为 xmlsquish.exe）放入 PATH 目录，\n"
            "运行 xmlsquish --version 验证，再在含 xmlsquish.toml 的项目中运行\n"
            "xmlsquish build。\n\n"
            "Linux: glibc >= 2.35. macOS: 11.0+. Windows: MSVC desktop target.\n"
            "Unsigned, not notarized. SHA256SUMS verifies integrity, not identity.\n"
            "二进制未签名、未经 Apple 公证。SHA256SUMS 校验完整性，不证明身份。\n\n"
            f"Source / 对应源码: https://github.com/kleedaisuki/prompt-squish/tree/{tag}\n"
            "Documentation / 文档: https://xmlsquish.moesegfault.dev/\n"
            "License / 许可证: GPL-3.0-or-later (see LICENSE).\n"
            "Dependency notices / 依赖法律声明: THIRD_PARTY_LICENSES/\n", encoding="utf-8")
        output = dist / archive_name(version, target)
        epoch = int(run("git", "show", "-s", "--format=%ct", "HEAD", cwd=source))
        if "windows" in target:
            with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
                for file in sorted(p for p in root.rglob("*") if p.is_file()):
                    info = zipfile.ZipInfo(f"{root.name}/{file.relative_to(root).as_posix()}", time.gmtime(max(epoch, 315532800))[:6])
                    info.create_system = 3
                    info.external_attr = 0o100644 << 16
                    info.compress_type = zipfile.ZIP_DEFLATED
                    archive.writestr(info, file.read_bytes())
        else:
            with output.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as archive:
                    for file in sorted(p for p in root.rglob("*") if p.is_file()):
                        info = tarfile.TarInfo(f"{root.name}/{file.relative_to(root).as_posix()}")
                        info.size = file.stat().st_size
                        info.mtime = epoch
                        info.mode = 0o755 if file.name == executable else 0o644
                        with file.open("rb") as content:
                            archive.addfile(info, content)
        print(f"Packaged {output.name}: {digest(output)}")


def publish(source, dist, tag):
    """Publish only complete assets; never overwrite existing bytes. / 仅发布完整资产且禁止覆盖。"""
    expected = {archive_name(tag[1:], target) for target in TARGETS}
    if {p.name for p in dist.iterdir()} != expected:
        raise ValueError("release assets do not match all six expected targets")
    assets = sorted(dist.iterdir())
    manifest = dist / "SHA256SUMS"
    manifest.write_text("".join(f"{digest(path)}  {path.name}\n" for path in assets), encoding="utf-8", newline="\n")
    # Refetch only the existing tag; no tag creation or movement is permitted.
    # 仅抓取既有标签，不创建或移动标签。
    run("git", "fetch", "origin", f"refs/tags/{tag}", cwd=source)
    if run("git", "rev-parse", "FETCH_HEAD^{commit}", cwd=source) != os.environ["RELEASE_SHA"]:
        raise ValueError("release tag moved during build")
    releases = json.loads(run("gh", "api", "--paginate", "--slurp", f"repos/{os.environ['GH_REPO']}/releases"))
    release = next((r for page in releases for r in page if r["tag_name"] == tag), None)
    if release is None:
        notes = source / "docs" / "releases" / f"{tag[1:]}.md"
        run("gh", "release", "create", tag, "--verify-tag", "--draft", "--title", f"xmlsquish {tag[1:]}", "--notes-file", str(notes))
        existing = set()
    else:
        existing = {asset["name"] for asset in release["assets"]}
    # Check every existing asset before uploading anything, including on retries.
    # 重试时也先检查全部既有资产，再上传任何新增文件。
    for asset in [*assets, manifest]:
        if asset.name in existing:
            with tempfile.TemporaryDirectory() as directory:
                run("gh", "release", "download", tag, "--pattern", asset.name, "--dir", directory)
                if digest(Path(directory) / asset.name) != digest(asset):
                    raise ValueError(f"refusing to replace existing asset: {asset.name}")
            print(f"Already published identical asset: {asset.name}")
    for asset in [*assets, manifest]:
        if asset.name not in existing:
            run("gh", "release", "upload", tag, str(asset))
            print(f"Published {asset.name}")
    if release is None or release["draft"]:
        run("gh", "release", "edit", tag, "--draft=false")


def main():
    """Validate CLI and environment before filesystem writes. / 写入前验证命令与环境。"""
    if len(sys.argv) in (3, 4) and sys.argv[1] == "verify":
        source = Path(sys.argv[2]).resolve()
        tag = sys.argv[3] if len(sys.argv) == 4 else None
        verify_repository(source, tag)
        return
    if len(sys.argv) != 4 or sys.argv[1] not in ("package", "publish"):
        raise SystemExit("usage: release.py verify SOURCE [TAG] | {package|publish} SOURCE DIST")
    tag = os.environ["RELEASE_TAG"]
    if not STABLE_TAG.fullmatch(tag):
        raise ValueError("expected a stable vMAJOR.MINOR.PATCH tag")
    if tuple(map(int, tag[1:].split("."))) < (0, 3, 0):
        raise ValueError("binary automation supports v0.3.0 and later")
    source, dist = (Path(value).resolve() for value in sys.argv[2:])
    dist.mkdir(parents=True, exist_ok=True)
    {"package": package, "publish": publish}[sys.argv[1]](source, dist, tag)


if __name__ == "__main__":
    main()
