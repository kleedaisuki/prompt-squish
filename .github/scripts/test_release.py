"""Test release metadata gates. / 测试发布元数据门禁。"""

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("release.py")
SPEC = importlib.util.spec_from_file_location("xmlsquish_release", SCRIPT)
release = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(release)


class ReleaseMetadataTests(unittest.TestCase):
    """Exercise repository and mismatch contracts. / 验证仓库与版本失配契约。"""

    def write_fixture(self, root: Path, *, manifest="1.0.4", lock="1.0.4") -> None:
        """Create the smallest complete release tree. / 创建最小完整发布树。"""
        (root / "docs" / "releases").mkdir(parents=True)
        (root / "Cargo.toml").write_text(
            f'[package]\nname = "xmlsquish"\nversion = "{manifest}"\n', encoding="utf-8"
        )
        (root / "Cargo.lock").write_text(
            f'[[package]]\nname = "xmlsquish"\nversion = "{lock}"\n', encoding="utf-8"
        )
        (root / "CHANGELOG.md").write_text(
            f"## [{manifest}]\n[notes](docs/releases/{manifest}.md)\n"
            f"[{manifest}]: https://github.com/kleedaisuki/prompt-squish/releases/tag/v{manifest}\n",
            encoding="utf-8",
        )
        (root / "docs" / "releases" / f"{manifest}.md").write_text(
            f"# xmlsquish {manifest}\n\nTag: `v{manifest}`\n", encoding="utf-8"
        )

    def test_repository_contract_accepts_matching_release(self) -> None:
        """A coherent release passes. / 一致的发布应通过。"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_fixture(root)
            self.assertEqual(release.verify_repository(root, "v1.0.4"), "1.0.4")

    def test_repository_contract_rejects_tag_mismatch(self) -> None:
        """A moved version/tag boundary fails. / 版本与标签错配必须失败。"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_fixture(root)
            with self.assertRaisesRegex(ValueError, "version does not match release tag"):
                release.verify_repository(root, "v1.0.3")

    def test_repository_contract_rejects_stale_lockfile(self) -> None:
        """A stale root lock entry fails. / 过期的根锁文件条目必须失败。"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_fixture(root, lock="1.0.3")
            with self.assertRaisesRegex(ValueError, "Cargo.lock root package version"):
                release.verify_repository(root, "v1.0.4")


if __name__ == "__main__":
    unittest.main()
