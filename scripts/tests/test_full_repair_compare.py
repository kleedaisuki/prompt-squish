"""Validate explicit full-repair mappings without invoking Cargo or native binaries."""

from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/perf"))
try:
    from full_repair_compare import compare
finally:
    sys.path.pop(0)


class FullRepairComparisonTests(unittest.TestCase):
    """Derived-cost dimensions cannot fabricate input differences or false speed pairs."""

    def phase(self, version: str, mechanism: str, elapsed: int, derived: int = 1) -> dict:
        """Construct one same-input case with algorithm-dependent metadata excluded."""
        dimensions = {"definitions": 128, "expansions": 64, "units": 2, "source_bytes": 100,
                      "compiled_regexes": derived}
        row = dict(schema="xmlsquish.mechanism.v1", suite="core", workload="same-input",
                   mode="latency", mechanism=mechanism, dimensions=dimensions, sample=0,
                   iterations=1, elapsed_ns=elapsed, drop_included=False, setup_excluded=True,
                   timing="segmented_operation")
        oracle = dict(suite="core", workload="same-input", dimensions=dimensions,
                      document_sha256="a" * 64, trace_sha256="b" * 64, directives_sha256="c" * 64)
        return {"pass": 0, "version": version, "rows": [row], "oracles": [oracle]}

    def test_equivalent_production_mapping_is_explicit(self) -> None:
        """Owned/shared production paths pair by a reviewed map, not by best-case search."""
        result = compare([self.phase("baseline", "link_prepared_input", 100),
                          self.phase("candidate", "link_prepared_facts", 50, 0)])
        row = result["production_path_comparisons"][0]
        self.assertEqual(row["baseline_mechanism"], "link_prepared_input")
        self.assertEqual(row["candidate_mechanism"], "link_prepared_facts")
        self.assertEqual(row["median_process_pass_ratios"]["elapsed_ns_per_operation"], 0.5)

    def test_changed_output_is_not_accepted_for_speed(self) -> None:
        """Equivalent workload geometry alone never excuses changed complete provenance."""
        baseline = self.phase("baseline", "link_prepared_input", 100)
        candidate = self.phase("candidate", "link_prepared_facts", 50)
        candidate["oracles"][0]["trace_sha256"] = "d" * 64
        with self.assertRaisesRegex(ValueError, "output mismatch"):
            compare([baseline, candidate])

    def test_new_case_is_unpaired(self) -> None:
        """A candidate-only primitive is reported, not assigned an invented baseline."""
        candidate = self.phase("candidate", "new_digest_only_stage", 1)
        result = compare([self.phase("baseline", "link_prepared_input", 100), candidate])
        self.assertEqual(result["production_path_comparisons"], [])
        self.assertTrue(result["unpaired"][0]["candidate"])


if __name__ == "__main__":
    unittest.main()
