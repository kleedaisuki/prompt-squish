"""Check immutable release evidence without contacting GitHub."""

import unittest

from verify_ci import REQUIRED_JOBS, required_jobs_passed, successful_ci


class CiGateTests(unittest.TestCase):
    """Wrong commits, workflows, events and incomplete runs must never pass."""

    def test_exact_branch_ci_passes(self) -> None:
        """Successful manual CI is valid evidence before merging a release branch."""
        run = dict(head_sha="abc", path=".github/workflows/ci.yml",
                   event="workflow_dispatch", status="completed", conclusion="success")
        self.assertTrue(successful_ci([run], "abc"))
        for field, wrong in (("head_sha", "def"), ("path", ".github/workflows/release.yml"),
                             ("event", "pull_request"), ("status", "in_progress"),
                             ("conclusion", "failure")):
            with self.subTest(field=field):
                self.assertFalse(successful_ci([{**run, field: wrong}], "abc"))

    def test_absent_evidence_fails(self) -> None:
        """API emptiness is not permission to publish."""
        self.assertFalse(successful_ci([], "abc"))

    def test_preparation_success_is_not_release_verification(self) -> None:
        """Every required quality/native/site job must succeed, not merely the workflow."""
        jobs = [dict(name=name, status="completed", conclusion="success") for name in REQUIRED_JOBS]
        self.assertTrue(required_jobs_passed(jobs))
        self.assertFalse(required_jobs_passed([
            dict(name="Preparation only - shipped v1.2 fixture", status="completed", conclusion="success")]))
        for required in REQUIRED_JOBS:
            changed = [{**job, "conclusion": "skipped"} if job["name"] == required else job for job in jobs]
            self.assertFalse(required_jobs_passed(changed))


if __name__ == "__main__":
    unittest.main()
