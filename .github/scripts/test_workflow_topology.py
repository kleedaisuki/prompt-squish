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
    performance_input = re.search(r"^      performance:\n(.*?)(?=^      \S|\Z)",
                                 dispatch.group(1), re.MULTILINE | re.DOTALL)
    if performance_input is None or not re.search(r"^        type: boolean$",
                                                 performance_input.group(1), re.MULTILINE):
        raise ValueError("performance dispatch input must remain boolean")
    if not re.search(r"^        default: false$", performance_input.group(1), re.MULTILINE):
        raise ValueError("performance measurements must remain opt-in")
    site_input = re.search(r"^      site:\n(.*?)(?=^      \S|\Z)",
                           dispatch.group(1), re.MULTILINE | re.DOTALL)
    if site_input is None or not re.search(r"^        type: boolean$", site_input.group(1),
                                           re.MULTILINE):
        raise ValueError("site must remain a boolean dispatch input")
    if not re.search(r"^        default: true$", site_input.group(1), re.MULTILINE):
        raise ValueError("site validation must remain enabled by default")
    jobs = re.search(r"^jobs:\n(.*)\Z", document, re.MULTILINE | re.DOTALL)
    if jobs is None:
        raise ValueError("missing top-level jobs mapping")
    names = re.findall(r"^  ([a-z][a-z-]*):$", jobs.group(1), re.MULTILINE)
    if names != ["rust-quality", "rust-test", "performance", "profile", "mechanisms", "site"]:
        raise ValueError("CI jobs must remain unique and below the jobs mapping")
    mechanisms_input = re.search(r"^      mechanisms:\n(.*?)(?=^      \S|\Z)",
                                 dispatch.group(1), re.MULTILINE | re.DOTALL)
    if mechanisms_input is None or "        default: false" not in mechanisms_input.group(1):
        raise ValueError("mechanism evidence must remain explicitly opt-in")
    mechanisms = re.search(r"^  mechanisms:\n(.*?)(?=^  \S|\Z)",
                           jobs.group(1), re.MULTILINE | re.DOTALL)
    if mechanisms is None or "    needs: rust-quality" not in mechanisms.group(1) or (
        "if: github.event_name == 'workflow_dispatch' && inputs.mechanisms" not in mechanisms.group(1)
    ):
        raise ValueError("mechanism evidence must require manual input and successful quality")
    profile_input = re.search(r"^      profile:\n(.*?)(?=^      \S|\Z)",
                              dispatch.group(1), re.MULTILINE | re.DOTALL)
    if profile_input is None or "        default: false" not in profile_input.group(1):
        raise ValueError("profile experiment must remain explicitly opt-in")
    profile = re.search(r"^  profile:\n(.*?)(?=^  \S|\Z)",
                        jobs.group(1), re.MULTILINE | re.DOTALL)
    if profile is None or "    needs: rust-quality" not in profile.group(1) or (
        "if: github.event_name == 'workflow_dispatch' && inputs.profile" not in profile.group(1)
    ):
        raise ValueError("profile experiment must require manual input and successful quality")
    quality = re.search(r"^  rust-quality:\n(.*?)(?=^  \S|\Z)",
                        jobs.group(1), re.MULTILINE | re.DOTALL)
    native = re.search(r"^  rust-test:\n(.*?)(?=^  \S|\Z)",
                       jobs.group(1), re.MULTILINE | re.DOTALL)
    if quality is None or "compiled: ${{ steps.compile.outcome }}" not in quality.group(1):
        raise ValueError("quality job must publish the actual compile outcome")
    if not re.search(r"^        id: compile$", quality.group(1), re.MULTILINE):
        raise ValueError("compile outcome must belong to the real MSRV check step")
    if native is None or (
        "if: always() && !cancelled() && needs.rust-quality.outputs.compiled == 'success'"
        not in native.group(1)
    ):
        raise ValueError("native tests must require successful compilation even after lint failure")
    performance = re.search(r"^  performance:\n(.*?)(?=^  \S|\Z)",
                            jobs.group(1), re.MULTILINE | re.DOTALL)
    if performance is None or "inputs.performance" not in performance.group(1):
        raise ValueError("performance job must be gated by the explicit dispatch input")
    site = re.search(r"^  site:\n(.*?)(?=^  \S|\Z)", jobs.group(1), re.MULTILINE | re.DOTALL)
    if site is None or not re.search(
        r"^    if: github.event_name != 'workflow_dispatch' \|\| inputs.site$",
        site.group(1), re.MULTILINE,
    ):
        raise ValueError("site validation must run for all push and pull-request events")


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

    def test_site_disabled_by_default_is_rejected(self) -> None:
        """Only explicit Rust-only dispatches may omit repeated site validation."""
        source = WORKFLOW.read_text(encoding="utf-8").replace("default: true", "default: false", 1)
        with self.assertRaisesRegex(ValueError, "enabled by default"):
            check_topology(source)

    def test_site_push_or_pull_request_skip_is_rejected(self) -> None:
        """The site optimization must not weaken normal main/PR coverage."""
        source = WORKFLOW.read_text(encoding="utf-8").replace(
            "github.event_name != 'workflow_dispatch' || inputs.site", "inputs.site", 1)
        with self.assertRaisesRegex(ValueError, "push and pull-request"):
            check_topology(source)


if __name__ == "__main__":
    unittest.main()
