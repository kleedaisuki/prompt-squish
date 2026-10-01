"""Check mechanism JSONL semantics without compiling or running a Rust benchmark."""

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("mechanisms", ROOT / "scripts/perf/mechanisms.py")
mechanisms = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mechanisms)


class MechanismReportTests(unittest.TestCase):
    """Latency and allocation evidence must not be silently conflated or misnormalized."""

    def latency(self, sample: int = 0) -> dict:
        """Construct a small schema-complete latency observation."""
        return dict(schema=mechanisms.SCHEMA, suite="core", mode="latency", workload="owned",
                    mechanism="actual-api", dimensions={"definitions": 64}, sample=sample,
                    iterations=4, drop_included=True, elapsed_ns=120)

    def test_latency_normalizes_batch_but_preserves_samples(self) -> None:
        """Per-operation latency divides elapsed time by its actual adaptive batch size."""
        rows = [self.latency(), {**self.latency(1), "elapsed_ns": 200}]
        result = mechanisms.aggregate(rows)[0]["elapsed_ns_per_operation"]
        self.assertEqual(result["samples"], [30, 50])
        self.assertEqual(result["median"], 40)

    def test_instrumented_latency_is_rejected(self) -> None:
        """A counting allocator sample is never evidence of uninstrumented speed."""
        with self.assertRaisesRegex(ValueError, "contaminate"):
            mechanisms.validate({**self.latency(), "allocation_calls": 1})

    def test_peak_is_not_divided_by_iterations(self) -> None:
        """Peak extra live memory is a batch peak, not allocation traffic per operation."""
        row = self.latency()
        row.pop("elapsed_ns")
        row.update(mode="allocations", allocation_calls=8, allocated_bytes=400,
                   deallocation_calls=8, reallocation_calls=0, peak_live_delta_bytes=80,
                   retained_live_delta_bytes=0)
        result = mechanisms.aggregate([row])[0]
        self.assertEqual(result["allocation_calls_per_operation"]["median"], 2)
        self.assertEqual(result["peak_live_delta_bytes"]["median"], 80)

    def test_duplicate_sample_is_rejected(self) -> None:
        """Concatenating repeated runs must not accidentally double-count the same sample ID."""
        with self.assertRaisesRegex(ValueError, "duplicate"):
            mechanisms.aggregate([self.latency(), self.latency()])

    def test_drop_policy_separates_owned_and_borrowed_controls(self) -> None:
        """Different destruction boundaries cannot be merged into one statistic."""
        row = self.latency()
        self.assertEqual(len(mechanisms.aggregate([row, {**row, "drop_included": False}])), 2)


if __name__ == "__main__":
    unittest.main()
