"""Focused tests for the architectural edge checker.

架构依赖边检查器的针对性测试。
"""

from pathlib import Path
import subprocess
import sys
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).with_name("fixtures")
# Import the sibling checker without making scripts a production package.
# 导入同级检查器，同时避免把 scripts 变成生产包。
sys.path.insert(0, str(SCRIPTS))
import check_architecture as checker  # noqa: E402
sys.path.pop(0)


def metadata_with(dependencies: list[dict[str, object]]) -> dict[str, object]:
    """Build minimal format-version-1 metadata for function-level tests.

    为函数级测试构造最小的 format-version-1 元数据。
    """

    return {
        "version": 1,
        "workspace_members": ["core-id", "adapter-id"],
        "packages": [
            {
                "id": "core-id",
                "name": "squish-core",
                "manifest_path": str(FIXTURES / "core" / "Cargo.toml"),
                "dependencies": dependencies,
            },
            {
                "id": "adapter-id",
                "name": "squish-adapter",
                "manifest_path": str(FIXTURES / "adapter" / "Cargo.toml"),
                "dependencies": [],
            },
        ],
    }


class ArchitectureCheckerTest(unittest.TestCase):
    """Verify extraction boundaries and command-line rejection diagnostics.

    验证提取边界与命令行契约层的拒绝诊断。
    """

    def test_target_predicates_collapse_to_one_declared_edge(self) -> None:
        """Target-specific declarations collapse when endpoints and kind match.

        当端点与类别相同时，目标特定声明折叠为一条边。
        """

        adapter_path = str(FIXTURES / "adapter")
        dependencies = [
            {
                "name": "squish-adapter",
                "path": adapter_path,
                "source": None,
                "kind": None,
                "target": target,
            }
            for target in ("cfg(unix)", "cfg(windows)")
        ]

        self.assertEqual(
            checker.workspace_edges(metadata_with(dependencies)),
            {("squish-core", "normal", "squish-adapter")},
        )

    def test_unknown_dependency_kind_is_rejected(self) -> None:
        """An unknown future Cargo dependency kind cannot bypass policy.

        未知的未来 Cargo 依赖类别不能绕过策略。
        """

        dependency = {
            "name": "squish-adapter",
            "path": str(FIXTURES / "adapter"),
            "source": None,
            "kind": "peer",
            "target": None,
        }

        with self.assertRaisesRegex(
            checker.ArchitectureCheckError,
            "unknown dependency kind: peer",
        ):
            checker.workspace_edges(metadata_with([dependency]))

    def test_forbidden_edge_reports_both_endpoints(self) -> None:
        """A renamed, optional target edge names its source and destination.

        被禁止的重命名可选目标边会同时报告其源 crate 与目标 crate。
        """

        process = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "check_architecture.py"),
                "--metadata",
                str(FIXTURES / "metadata-forbidden-edge.json"),
                "--policy",
                str(FIXTURES / "no-edges-policy.txt"),
            ],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )

        self.assertEqual(process.returncode, 1)
        self.assertIn(
            "forbidden internal dependency: squish-core -[normal]-> squish-adapter",
            process.stderr,
        )


if __name__ == "__main__":
    unittest.main()
