"""Validate same-runner process-pass pairing without compiling benchmark executables."""

from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/perf"))
try:
    from origin_compare import compare_passes, oracle_map
finally:
    sys.path.pop(0)


class OriginComparisonTests(unittest.TestCase):
    """Comparison identity and complete process pairs are mandatory, not assumptions."""

    def phase(self, number: int, version: str, elapsed: int, definitions: int = 128) -> dict:
        """Construct one valid fixture observation with explicit lifetime boundaries."""
        row = dict(schema="xmlsquish.mechanism.v1", suite="core", mode="latency",
                   workload="origin-fixture", mechanism="instantiate", dimensions={"definitions": definitions},
                   sample=0, iterations=1, elapsed_ns=elapsed, drop_included=False,
                   setup_excluded=True, timing="segmented_operation")
        return {"pass": number, "version": version, "rows": [row]}

    def test_pairs_process_medians_per_case(self) -> None:
        """AB/BA pass pairing keeps the two process-median ratios explicit."""
        phases = [self.phase(0, "baseline", 100), self.phase(0, "candidate", 50),
                  self.phase(1, "candidate", 90), self.phase(1, "baseline", 120)]
        result = compare_passes(phases)[0]
        self.assertEqual([item["candidate_over_baseline"] for item in result["passes"]], [0.5, 0.75])
        self.assertEqual(result["median_process_pass_ratio"], 0.625)

    def test_incomplete_process_pair_is_rejected(self) -> None:
        """A lone candidate is not before/after evidence."""
        with self.assertRaisesRegex(ValueError, "incomplete"):
            compare_passes([self.phase(0, "candidate", 50)])

    def test_changed_fixture_dimension_is_rejected(self) -> None:
        """Do not compare different definition counts just because mechanism names match."""
        with self.assertRaisesRegex(ValueError, "dimensions"):
            compare_passes([self.phase(0, "baseline", 100), self.phase(0, "candidate", 50, 1024)])

    def test_inconsistent_actual_outputs_are_rejected(self) -> None:
        """The same fixture cannot provide two different canonical provenance digests."""
        oracle = dict(suite="core", workload="fixture", dimensions={"definitions": 128},
                      document_sha256="a" * 64, trace_sha256="b" * 64, directives_sha256="c" * 64)
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            oracle_map([oracle, {**oracle, "trace_sha256": "d" * 64}])


if __name__ == "__main__":
    unittest.main()
