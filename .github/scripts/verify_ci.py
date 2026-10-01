"""Require successful trusted CI for the immutable release commit, not its tag name."""

import json
import os
import re
import subprocess


REQUIRED_JOBS = {
    "Rust quality (Ubuntu, MSRV 1.88)",
    "Rust test (Linux, MSRV 1.88)",
    "Rust test (Windows, MSRV 1.88)",
    "Rust test (macOS, MSRV 1.88)",
    "Site",
}


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


def required_jobs_passed(jobs: list[dict]) -> bool:
    """Preparation-only success cannot authorize release when required checks skipped."""
    successful = {job.get("name") for job in jobs
                  if job.get("status") == "completed" and job.get("conclusion") == "success"}
    return REQUIRED_JOBS <= successful


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
    for run in runs:
        if not successful_ci([run], sha):
            continue
        response = subprocess.check_output(
            ["gh", "api", "--paginate", "--slurp",
             f"repos/{repository}/actions/runs/{run['id']}/jobs?filter=latest&per_page=100"],
            text=True, encoding="utf-8")
        jobs = [job for page in json.loads(response) for job in page["jobs"]]
        if required_jobs_passed(jobs):
            print(f"Verified complete exact-commit Continuous Integration for {sha}")
            return
    raise ValueError(f"no complete successful exact-commit CI for {sha}; preparation or skipped checks are not verification")


if __name__ == "__main__":
    main()
