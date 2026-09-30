"""Guard CI trigger/job boundaries without a third-party YAML dependency.

This is a narrow regression guard, not a YAML validator. In particular, a textual
insertion before `performance:` must never insert job steps into dispatch inputs.
GitHub validates complete workflow semantics when accepting the checked-in file.
"""

from pathlib import Path
import re
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / "workflows" / "ci.yml"


def check_topology(document: str) -> None:
    """Require the maintained CI trigger and optional performance-job boundaries."""
    trigger = re.search(r"^on:\n(.*?)(?=^[^\s#])", document, re.MULTILINE | re.DOTALL)
    if trigger is None:
        raise ValueError("missing top-level on trigger mapping")
    dispatch = re.search(r"^  workflow_dispatch:\n(.*?)(?=^  \S|\Z)",
                         trigger.group(1), re.MULTILINE | re.DOTALL)
    if dispatch is None:
        raise ValueError("missing workflow_dispatch trigger mapping")
    if re.search(r"^\s*-", dispatch.group(1), re.MULTILINE):
        raise ValueError("job steps were inserted into workflow_dispatch inputs")
    if not re.search(r"^    inputs:\n      performance:\n", dispatch.group(1), re.MULTILINE):
        raise ValueError("performance must be a dispatch input below inputs")
    if not re.search(r"^        type: boolean$", dispatch.group(1), re.MULTILINE):
        raise ValueError("performance dispatch input must remain boolean")
    if not re.search(r"^        default: false$", dispatch.group(1), re.MULTILINE):
        raise ValueError("performance measurements must remain opt-in")
    jobs = re.search(r"^jobs:\n(.*)\Z", document, re.MULTILINE | re.DOTALL)
    if jobs is None:
        raise ValueError("missing top-level jobs mapping")
    names = re.findall(r"^  ([a-z][a-z-]*):$", jobs.group(1), re.MULTILINE)
    if names != ["rust-quality", "rust-test", "performance", "site"]:
        raise ValueError("CI jobs must remain unique and below the jobs mapping")
    performance = re.search(r"^  performance:\n(.*?)(?=^  \S|\Z)",
                            jobs.group(1), re.MULTILINE | re.DOTALL)
    if performance is None or "inputs.performance" not in performance.group(1):
        raise ValueError("performance job must be gated by the explicit dispatch input")


class WorkflowTopologyTests(unittest.TestCase):
    """Exercise the real workflow and reproduce the dispatch-input insertion defect."""

    def test_checked_in_workflow_has_expected_boundaries(self) -> None:
        """The lightweight CI suite checks the document that GitHub will dispatch."""
        check_topology(WORKFLOW.read_text(encoding="utf-8"))

    def test_job_step_inserted_before_input_is_rejected(self) -> None:
        """An unanchored performance replacement must fail before another dispatch."""
        source = WORKFLOW.read_text(encoding="utf-8")
        corrupted = source.replace(
            "      performance:\n",
            "      - name: Save reusable Rust state\n"
            "        uses: actions/cache/save@invalid\n"
            "      performance:\n", 1)
        with self.assertRaisesRegex(ValueError, "job steps"):
            check_topology(corrupted)

    def test_required_performance_default_is_rejected(self) -> None:
        """Routine fix iterations must not silently enable expensive measurements."""
        source = WORKFLOW.read_text(encoding="utf-8").replace("default: false", "default: true", 1)
        with self.assertRaisesRegex(ValueError, "opt-in"):
            check_topology(source)


if __name__ == "__main__":
    unittest.main()
