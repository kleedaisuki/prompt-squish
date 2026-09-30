"""Check immutable release evidence without contacting GitHub."""

import unittest

from verify_ci import successful_ci


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


if __name__ == "__main__":
    unittest.main()
