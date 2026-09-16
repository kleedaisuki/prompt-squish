#!/usr/bin/env python3
"""Validate workspace dependency edges against the architectural contract.

根据架构契约验证 workspace 的直接依赖边。
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Iterable


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = Path(__file__).with_name("architecture_edges.txt")
Edge = tuple[str, str, str]


class ArchitectureCheckError(Exception):
    """Report invalid checker input or an unavailable metadata source.

    报告无效的检查器输入或不可用的元数据源。
    """


def parse_policy(lines: Iterable[str]) -> set[Edge]:
    """Parse a deterministic ``source -[kind]-> destination`` edge allowlist.

    解析确定性的 ``源 -[类别]-> 目标`` 依赖边允许列表。
    """

    edges: set[Edge] = set()
    for line_number, raw_line in enumerate(lines, start=1):
        line = raw_line.partition("#")[0].strip()
        if not line:
            continue
        source_and_kind, separator, destination = line.partition("]->")
        source, kind_separator, kind = source_and_kind.partition("-[")
        source = source.strip()
        kind = kind.strip()
        destination = destination.strip()
        if separator != "]->" or kind_separator != "-[" or not all((source, kind, destination)):
            raise ArchitectureCheckError(
                f"invalid policy line {line_number}: "
                "expected 'source -[kind]-> destination'"
            )
        if kind not in {"normal", "dev", "build"}:
            raise ArchitectureCheckError(
                f"invalid dependency kind on policy line {line_number}: {kind}"
            )
        edge = (source, kind, destination)
        if edge in edges:
            raise ArchitectureCheckError(
                f"duplicate policy edge on line {line_number}: "
                f"{source} -[{kind}]-> {destination}"
            )
        edges.add(edge)
    return edges


def workspace_edges(metadata: dict[str, Any]) -> set[Edge]:
    """Extract declared direct edges whose endpoints are workspace packages.

    提取起点和终点均为 workspace 包的已声明直接依赖边。
    """

    if metadata.get("version") != 1:
        raise ArchitectureCheckError("cargo metadata must use format version 1")

    packages = metadata.get("packages")
    member_ids = metadata.get("workspace_members")
    if not isinstance(packages, list) or not isinstance(member_ids, list):
        raise ArchitectureCheckError(
            "cargo metadata is missing packages or workspace_members"
        )

    members = {
        package["id"]: package
        for package in packages
        if isinstance(package, dict) and package.get("id") in member_ids
    }
    if len(members) != len(member_ids):
        raise ArchitectureCheckError(
            "cargo metadata does not describe every workspace member"
        )

    names = {package.get("name") for package in members.values()}
    if None in names or len(names) != len(members):
        raise ArchitectureCheckError("workspace package names must be present and unique")

    workspace_paths: dict[Path, str] = {}
    for package in members.values():
        manifest_path = package.get("manifest_path")
        if not isinstance(manifest_path, str):
            raise ArchitectureCheckError(
                f"cargo metadata package {package['name']} has no manifest path"
            )
        package_path = Path(manifest_path).parent.resolve()
        if package_path in workspace_paths:
            raise ArchitectureCheckError(
                f"workspace packages share a directory: {package_path}"
            )
        workspace_paths[package_path] = package["name"]

    edges: set[Edge] = set()
    for package in members.values():
        dependencies = package.get("dependencies")
        if not isinstance(dependencies, list):
            raise ArchitectureCheckError(
                f"cargo metadata package {package['name']} has no dependency list"
            )
        for dependency in dependencies:
            if not isinstance(dependency, dict):
                raise ArchitectureCheckError(
                    f"cargo metadata package {package['name']} has an invalid dependency"
                )
            dependency_path = dependency.get("path")
            if isinstance(dependency_path, str):
                # Resolve by package directory, not the dependency key. This handles
                # renamed crates and excludes same-named path dependencies outside
                # the workspace. / 按包目录而非依赖键解析，从而支持 crate 重命名，
                # 并排除 workspace 外同名的路径依赖。
                destination = workspace_paths.get(Path(dependency_path).resolve())
            else:
                destination = None
            if destination is not None:
                # In Cargo metadata, null represents an ordinary dependency.
                # 在 Cargo metadata 中，null 表示普通依赖。
                kind = dependency.get("kind") or "normal"
                if kind not in {"normal", "dev", "build"}:
                    raise ArchitectureCheckError(
                        f"cargo metadata has unknown dependency kind: {kind}"
                    )
                edges.add((package["name"], kind, destination))
    return edges


def load_metadata(path: Path | None) -> dict[str, Any]:
    """Load fixture metadata or invoke Cargo's stable, versioned JSON interface.

    加载 fixture 元数据，或调用 Cargo 稳定且带版本的 JSON 接口。
    """

    if path is not None:
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise ArchitectureCheckError(f"cannot read metadata {path}: {error}") from error
    else:
        cargo = os.environ.get("CARGO", "cargo")
        try:
            process = subprocess.run(
                [cargo, "metadata", "--format-version", "1", "--no-deps", "--locked"],
                cwd=REPOSITORY_ROOT,
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
            )
        except OSError as error:
            raise ArchitectureCheckError(f"cannot execute {cargo}: {error}") from error
        if process.returncode != 0:
            detail = process.stderr.strip() or "cargo metadata failed without a diagnostic"
            raise ArchitectureCheckError(detail)
        try:
            value = json.loads(process.stdout)
        except json.JSONDecodeError as error:
            raise ArchitectureCheckError(f"cargo returned invalid JSON: {error}") from error

    if not isinstance(value, dict):
        raise ArchitectureCheckError("cargo metadata root must be a JSON object")
    return value


def check(actual: set[Edge], allowed: set[Edge]) -> list[str]:
    """Return stable, actionable diagnostics for both sides of graph drift.

    为依赖图双向漂移返回稳定且可操作的诊断信息。
    """

    diagnostics = [
        f"forbidden internal dependency: {source} -[{kind}]-> {destination}"
        for source, kind, destination in sorted(actual - allowed)
    ]
    diagnostics.extend(
        f"allowed internal dependency is absent: {source} -[{kind}]-> {destination}"
        for source, kind, destination in sorted(allowed - actual)
    )
    return diagnostics


def parse_arguments() -> argparse.Namespace:
    """Parse command-line paths used by CI and focused fixture tests.

    解析 CI 与针对性 fixture 测试使用的命令行路径。
    """

    parser = argparse.ArgumentParser(
        description="Check workspace direct dependencies against the architecture policy."
    )
    parser.add_argument(
        "--metadata",
        type=Path,
        help="read cargo metadata JSON from this file instead of invoking Cargo",
    )
    parser.add_argument(
        "--policy",
        type=Path,
        default=DEFAULT_POLICY,
        help=f"edge allowlist (default: {DEFAULT_POLICY})",
    )
    return parser.parse_args()


def main() -> int:
    """Run the architecture check and return a process-oriented status code.

    运行架构检查并返回面向进程的状态码。
    """

    arguments = parse_arguments()
    try:
        allowed = parse_policy(arguments.policy.read_text(encoding="utf-8").splitlines())
        actual = workspace_edges(load_metadata(arguments.metadata))
    except (OSError, ArchitectureCheckError) as error:
        print(f"architecture check error: {error}", file=sys.stderr)
        return 2

    diagnostics = check(actual, allowed)
    if diagnostics:
        for diagnostic in diagnostics:
            print(diagnostic, file=sys.stderr)
        return 1

    print(f"architecture check passed: {len(actual)} internal direct edges")
    return 0


if __name__ == "__main__":
    sys.exit(main())
