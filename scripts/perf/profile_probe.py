"""Compare default and ThinLTO builds of identical code with bounded small CLI probes.

The manual CI workflow supplies the compiled binaries and build wall times. This
script only launches them: no archive tails, baseline download, or network access.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import time

from measure import NS, ROOT, fixture, invoke, summary


def probe(arguments: argparse.Namespace, report: dict) -> None:
    """Alternate default/candidate subprocesses and reject changed prompt semantics."""
    scratch = arguments.scratch.resolve()
    if not scratch.is_relative_to(ROOT / ".temp"):
        raise ValueError("profile scratch must be below project .temp")
    scratch = scratch / f"run-{time.time_ns()}"
    binaries = [("candidate", arguments.candidate.resolve()),
                ("default", arguments.default.resolve())]
    projects = {}
    for label, binary in binaries:
        data = binary.read_bytes()
        report["binaries"][label] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        projects[label] = fixture(scratch / label, "prompt",
                                  f'<xs:entry xmlns:xs="{NS}"><Prompt>Hello tiny build</Prompt></xs:entry>')
        invoke(binary, ["build", "--offline", "--plain"], projects[label])
    for workload in ("startup", "tiny.cold", "tiny.warm"):
        samples = {label: [] for label, _ in binaries}
        for number in range(arguments.rounds):
            order = binaries if number % 2 == 0 else list(reversed(binaries))
            for label, binary in order:
                if workload == "tiny.cold":
                    shutil.rmtree(projects[label] / "target/xmlsquish")
                command = ["--version"] if workload == "startup" else ["build", "--offline", "--plain"]
                samples[label].append(invoke(binary, command, projects[label]))
        outputs = {}
        for label, values in samples.items():
            output = (projects[label] / "target/xmlsquish/artifacts/prompt.prompt").read_bytes()
            outputs[label] = output
            result = summary(values)
            result["output_sha256"] = hashlib.sha256(output).hexdigest()
            report["workloads"][f"{label}.{workload}"] = result
        if outputs["candidate"] != outputs["default"]:
            raise ValueError("profile change altered final prompt bytes")
        ratios = [candidate / default for candidate, default in
                  zip(samples["candidate"], samples["default"], strict=True)]
        report["paired_candidate_over_default"][workload] = {
            "samples_ratio": ratios, "median_ratio": statistics.median(ratios),
            "min_ratio": min(ratios), "max_ratio": max(ratios)}


def main() -> None:
    """Persist partial evidence on failure; hosted samples are descriptive only."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("default", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--build-times", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, default=ROOT / ".temp/profile-probe")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=20)
    arguments = parser.parse_args()
    if arguments.rounds < 1:
        parser.error("rounds must be positive")
    report = {"schema": 1, "commit": os.environ.get("GITHUB_SHA"), "platform": platform.platform(),
              "comparison": "same commit default release versus ThinLTO/codegen-units=1",
              "cold_definition": "empty project cache; OS page cache untouched",
              "builds": json.loads(arguments.build_times.read_text()),
              "binaries": {}, "workloads": {}, "paired_candidate_over_default": {}, "status": "running"}
    try:
        probe(arguments, report)
        report["status"] = "success"
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        arguments.report.parent.mkdir(parents=True, exist_ok=True)
        arguments.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
