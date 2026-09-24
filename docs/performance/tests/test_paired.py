"""Verify shared paired benchmark mechanics without invoking binaries.

无需启动二进制即可验证配对基准的共享机制。
"""

import hashlib
from pathlib import Path
import sys
import tempfile
import unittest


# Direct script execution resolves sibling modules; mirror that import path here.
# 直接执行脚本时可解析同级模块；此处复用同一路径规则。
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from paired import alternating_rounds, executable_identity  # noqa: E402
sys.path.pop(0)


class PairedMechanicsTests(unittest.TestCase):
    """Keep report-facing identity and order deterministic.

    保持面向报告的二进制身份和轮次顺序确定不变。
    """

    def test_executable_identity_uses_bytes_not_path(self) -> None:
        """Hash exact bytes without publishing host paths. / 哈希精确字节且不公开主机路径。"""

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate.exe"
            path.write_bytes(b"\x00\xffbinary")
            self.assertEqual(
                executable_identity(path),
                {"name": "candidate.exe", "sha256": hashlib.sha256(b"\x00\xffbinary").hexdigest()},
            )

    def test_alternating_rounds_preserve_one_based_ab_ba_order(self) -> None:
        """Preserve report indices and process order. / 保持报告索引与进程顺序。"""

        self.assertEqual(
            list(alternating_rounds(3)),
            [
                (1, ["baseline", "candidate"]),
                (2, ["candidate", "baseline"]),
                (3, ["baseline", "candidate"]),
            ],
        )
        self.assertEqual(list(alternating_rounds(0)), [])


if __name__ == "__main__":
    unittest.main()
