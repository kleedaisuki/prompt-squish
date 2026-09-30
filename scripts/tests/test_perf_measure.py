"""Validate performance evidence and fixture policy without compiling Rust locally."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("measure", ROOT / "scripts/perf/measure.py")
measure = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(measure)
DOWNLOAD_SPEC = importlib.util.spec_from_file_location(
    "download_baseline", ROOT / "scripts/perf/download_baseline.py")
download = importlib.util.module_from_spec(DOWNLOAD_SPEC)
DOWNLOAD_SPEC.loader.exec_module(download)
SCRATCH = ROOT / ".temp" / "perf-tests"
SCRATCH.mkdir(parents=True, exist_ok=True)


class MeasurementTests(unittest.TestCase):
    """Raw observations and deterministic ZIP checks are part of the harness contract."""

    def test_statistics_keep_samples(self) -> None:
        """Do not discard variance or invent a benchmark confidence interval."""
        self.assertEqual(measure.summary([1, 100, 3]),
                         dict(samples_ns=[1, 100, 3], median_ns=3, min_ns=1, max_ns=100))

    def test_baseline_requires_unique_matching_checksum(self) -> None:
        """A digest for a different asset or ambiguous digest never authenticates bytes."""
        checksum = measure.hashlib.sha256(b"baseline").hexdigest()
        line = f"{checksum}  {download.NAME}\n"
        download.verify(b"baseline", line)
        for data, manifest in ((b"tampered", line), (b"baseline", line * 2),
                               (b"baseline", f"{checksum}  another.tar.gz\n")):
            with self.assertRaises(ValueError):
                download.verify(data, manifest)

    def test_fixture_selects_archive_backend(self) -> None:
        """The source root and target backend must not depend on CLI defaults."""
        with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
            project = measure.fixture(Path(temporary) / "pack", "bundle", "<pack/>", "pack")
            manifest = (project / "xmlsquish.toml").read_text()
            self.assertIn('backend = "pack"', manifest)
            self.assertIn('source-root = "."', manifest)

    def test_zip_check_rejects_mutable_timestamp(self) -> None:
        """Identical current bytes alone do not establish reproducibility."""
        with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
            project = Path(temporary)
            artifact = project / "target/xmlsquish/artifacts/bundle.pack"
            artifact.parent.mkdir(parents=True)
            with zipfile.ZipFile(artifact, "w") as archive:
                archive.writestr(zipfile.ZipInfo("asset", (2026, 1, 1, 0, 0, 0)), b"bytes")
            with self.assertRaisesRegex(ValueError, "timestamp"):
                measure.archive_check(Path("unused"), project, "bundle.pack")

    def test_cold_removal_is_outside_process_timing(self) -> None:
        """Cold samples clear only the project build root; warm samples leave it alone."""
        with patch.object(measure, "invoke", return_value=17) as invoke:
            with patch.object(measure.shutil, "rmtree") as remove:
                result = measure.measure(Path("binary"), Path("project"), 3, True)
            self.assertEqual(invoke.call_count, 4)
            self.assertEqual(remove.call_count, 3)
            self.assertEqual(result["samples_ns"], [17, 17, 17])


if __name__ == "__main__":
    unittest.main()
