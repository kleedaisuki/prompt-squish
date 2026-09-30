"""Require successful trusted CI for the immutable release commit, not its tag name."""

import json
import os
import re
import subprocess


def successful_ci(runs: list[dict], sha: str) -> bool:
    """Accept completed branch CI only; pull-request merge runs are not release evidence."""
    return any(
        run.get("head_sha") == sha
        and run.get("path") == ".github/workflows/ci.yml"
        and run.get("event") in ("push", "workflow_dispatch")
        and run.get("status") == "completed"
        and run.get("conclusion") == "success"
        for run in runs
    )


def main() -> None:
    """Query the named CI workflow using read-only Actions permission and fail closed."""
    sha, repository = os.environ["RELEASE_SHA"], os.environ["GH_REPO"]
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("release SHA must be a full immutable commit ID")
    response = subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp",
         f"repos/{repository}/actions/workflows/ci.yml/runs?head_sha={sha}&per_page=100"],
        text=True, encoding="utf-8",
    )
    runs = [run for page in json.loads(response) for run in page["workflow_runs"]]
    if not successful_ci(runs, sha):
        raise ValueError(f"no successful exact-commit CI for {sha}; dispatch ci.yml and retry")
    print(f"Verified successful Continuous Integration for {sha}")


if __name__ == "__main__":
    main()
