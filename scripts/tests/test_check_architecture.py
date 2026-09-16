"""Focused black-box tests for the architectural edge checker.

架构依赖边检查器的针对性黑盒测试。
"""

from pathlib import Path
import subprocess
import sys
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).with_name("fixtures")


class ArchitectureCheckerTest(unittest.TestCase):
    """Verify rejection diagnostics at the command-line contract.

    在命令行契约层验证拒绝诊断。
    """

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
