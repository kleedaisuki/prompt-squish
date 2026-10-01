"""Compare pinned pre-index and candidate origin mechanisms on one hosted runner.

Both executables use identical current benchmark-only source, isolated Cargo
targets and the same shipping bench profile. Pairing is between process-pass
medians for each identical case, not a claim that independent operations ran as
lockstep paired samples. No production source is overlaid into the reference.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import time
import tomllib

from mechanisms import ROOT, aggregate, provenance, split_records


REFERENCE = "54fe53f6275b004266cf946fb8fe2a352f196fa7"
HARNESS_FILES = (
    "scripts/perf/mechanism_support.rs",
    "crates/squish-link/benches/core_latency.rs",
    "crates/squish-link/benches/support/core_suite.rs",
)


def build(source: Path, target: Path, errors: Path) -> Path:
    """Compile only core_latency, reading its actual executable from Cargo JSON."""
    arguments = ["cargo", "+1.88.0", "bench", "-p", "squish-link", "--bench", "core_latency",
                 "--features", "mechanism-benchmarks", "--locked", "--no-run", "--message-format=json"]
    executable = None
    environment = {**os.environ, "CARGO_TARGET_DIR": str(target)}
    with errors.open("w", encoding="utf-8") as stream:
        process = subprocess.Popen(arguments, cwd=source, env=environment, stdout=subprocess.PIPE,
                                   stderr=stream, text=True, encoding="utf-8")
        assert process.stdout is not None
        for line in process.stdout:
            message = json.loads(line)
            if message.get("reason") == "compiler-artifact" and message.get("target", {}).get("name") == "core_latency":
                executable = message.get("executable") or executable
            if message.get("reason") == "compiler-message" and message.get("message", {}).get("rendered"):
                print(message["message"]["rendered"], end="")
        if process.wait() or executable is None:
            raise RuntimeError(f"core_latency build failed in {source}")
    return Path(executable)


def key(row: dict) -> str:
    """Require identical fixtures, mechanisms and lifetime boundaries across versions."""
    return json.dumps({name: row.get(name) for name in (
        "suite", "workload", "mechanism", "dimensions", "drop_included", "timing", "setup_excluded")},
        sort_keys=True)


def compare_passes(phases: list[dict]) -> list[dict]:
    """Pair per-case medians from adjacent AB/BA process passes, retaining each ratio."""
    passes = {}
    for phase in phases:
        medians = {key(row): row for row in aggregate(phase["rows"])}
        passes.setdefault(phase["pass"], {})[phase["version"]] = medians
    comparisons = {}
    for number, versions in passes.items():
        if set(versions) != {"baseline", "candidate"}:
            raise ValueError("incomplete baseline/candidate process pass")
        if set(versions["baseline"]) != set(versions["candidate"]):
            raise ValueError("baseline/candidate fixture dimensions or mechanisms differ")
        for identity, baseline in versions["baseline"].items():
            candidate = versions["candidate"][identity]
            before = baseline["elapsed_ns_per_operation"]["median"]
            after = candidate["elapsed_ns_per_operation"]["median"]
            if before <= 0:
                raise ValueError("zero baseline latency is not a meaningful ratio")
            result = comparisons.setdefault(identity, {**json.loads(identity), "passes": []})
            result["passes"].append({"pass": number, "baseline_median_ns": before,
                                     "candidate_median_ns": after, "candidate_over_baseline": after / before})
    for result in comparisons.values():
        ratios = [value["candidate_over_baseline"] for value in result["passes"]]
        result.update(median_process_pass_ratio=statistics.median(ratios),
                      min_process_pass_ratio=min(ratios), max_process_pass_ratio=max(ratios))
    return list(comparisons.values())


def oracle_map(oracles: list[dict]) -> dict[str, dict]:
    """Identify untimed actual-output fingerprints without derived algorithm cost fields."""
    result = {}
    for oracle in oracles:
        identity = json.dumps({name: oracle[name] for name in ("suite", "workload", "dimensions")},
                              sort_keys=True)
        digests = {name: oracle[name] for name in ("document_sha256", "trace_sha256", "directives_sha256")}
        if identity in result and result[identity] != digests:
            raise ValueError("one fixture emitted inconsistent canonical output fingerprints")
        result[identity] = digests
    return result


def run(arguments: argparse.Namespace, directory: Path, report: dict) -> None:
    """Prepare an immutable reference tree with benchmark-only overlays, then alternate."""
    report["provenance"] = provenance()
    reference = ROOT / ".temp/mechanism-origin-reference"
    if reference.exists():
        raise ValueError("reference worktree already exists; use a fresh hosted checkout")
    subprocess.run(["git", "worktree", "add", "--detach", str(reference), arguments.baseline],
                   cwd=ROOT, check=True)
    hashes = {}
    for name in HARNESS_FILES:
        destination = reference / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, destination)
        hashes[name] = hashlib.sha256(destination.read_bytes()).hexdigest()
    candidate_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    baseline_manifest = tomllib.loads((reference / "Cargo.toml").read_text(encoding="utf-8"))
    for profile in ("release", "bench"):
        before = baseline_manifest.get("profile", {}).get(profile, {})
        after = candidate_manifest.get("profile", {}).get(profile, {})
        if before != after:
            raise ValueError(f"baseline/candidate {profile} profiles differ")
    report.update(baseline_commit=arguments.baseline, identical_benchmark_source_sha256=hashes,
                  pairing="adjacent AB/BA process-pass medians per identical case; not lockstep operation samples",
                  oracle="identical untimed core fixture/document/provenance assertions executed in every process")
    binaries = {}
    for version, source, target in (
        ("baseline", reference, ROOT / ".cache/origin-reference-target"),
        ("candidate", ROOT, ROOT / "target"),
    ):
        start = time.perf_counter_ns()
        binary = build(source, target, directory / f"{version}-cargo-stderr.txt")
        binaries[version] = binary
        report.setdefault("builds", {})[version] = {
            "wall_ns": time.perf_counter_ns() - start,
            "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "bytes": binary.stat().st_size}
    environment = {**os.environ, "MECHANISM_ORIGIN_ONLY": "1"}
    phases = []
    for number in range(arguments.passes):
        order = ("baseline", "candidate") if number % 2 == 0 else ("candidate", "baseline")
        for position, version in enumerate(order):
            completed = subprocess.run([str(binaries[version])], cwd=ROOT, env=environment,
                                       capture_output=True, text=True, encoding="utf-8", timeout=300)
            filename = f"pass-{number}-{position}-{version}.jsonl"
            (directory / filename).write_text(completed.stdout, encoding="utf-8")
            if completed.returncode:
                raise RuntimeError(f"{version} origin oracle/measurement failed: {completed.stderr}")
            records = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
            rows, oracles = split_records(records)
            if not rows:
                raise ValueError("empty targeted origin observations")
            for row in rows:
                if row["mode"] != "latency":
                    raise ValueError("origin comparison must use uninstrumented latency binaries")
            fingerprints = oracle_map(oracles)
            if not fingerprints:
                raise ValueError("targeted core suite emitted no independent output fingerprints")
            if "canonical_output_oracles" not in report:
                report["canonical_output_oracles"] = fingerprints
            elif report["canonical_output_oracles"] != fingerprints:
                raise ValueError("canonical document/trace/directive output differs between process versions")
            phase = {"pass": number, "position": position, "version": version, "file": filename, "rows": rows}
            phases.append(phase)
            report.setdefault("phases", []).append({**phase, "rows": len(rows)})
    report["comparisons"] = compare_passes(phases)


def main() -> None:
    """Write bounded small evidence below project .temp, including failures."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default=REFERENCE)
    parser.add_argument("--passes", type=int, default=3)
    parser.add_argument("--directory", type=Path, default=ROOT / ".temp/origin-comparison")
    arguments = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", arguments.baseline):
        parser.error("baseline must be a full immutable commit SHA")
    if not 1 <= arguments.passes <= 10:
        parser.error("passes must be between 1 and 10")
    directory = arguments.directory.resolve()
    if not directory.is_relative_to(ROOT / ".temp"):
        parser.error("comparison directory must be below project .temp")
    directory.mkdir(parents=True, exist_ok=True)
    report = {"schema": "xmlsquish.origin-comparison.v1", "status": "running"}
    try:
        run(arguments, directory, report)
        report["status"] = "success"
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        (directory / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
