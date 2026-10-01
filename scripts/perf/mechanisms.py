"""Build four opt-in mechanism executables and collect small reproducible reports.

No v1.1 CLI comparison occurs: these are in-process probes of the released v1.2
mechanisms and explicitly labeled naive controls on the same source/runner.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time
import tomllib


ROOT = Path(__file__).resolve().parents[2]
TARGETS = {"core_latency", "core_allocations", "archive_latency", "archive_allocations"}
SCHEMA = "xmlsquish.mechanism.v1"


def command(*arguments: str) -> str:
    """Read small provenance values without shell interpolation."""
    return subprocess.check_output(arguments, cwd=ROOT, text=True, encoding="utf-8").strip()


def validate(row: dict) -> None:
    """Reject mixed timing/allocation modes, malformed dimensions and invalid counts."""
    if row.get("schema") != SCHEMA or row.get("mode") not in ("latency", "allocations"):
        raise ValueError("unknown mechanism report schema or mode")
    for name in ("suite", "workload", "mechanism"):
        if not isinstance(row.get(name), str) or not row[name]:
            raise ValueError(f"missing nonempty {name} label")
    for name in ("sample", "iterations"):
        if type(row.get(name)) is not int or row[name] < (1 if name == "iterations" else 0):
            raise ValueError(f"invalid {name}")
    dimensions = row.get("dimensions")
    if not isinstance(dimensions, dict) or any(type(value) is not int or value < 0
                                             for value in dimensions.values()):
        raise ValueError("dimensions must contain nonnegative integer values")
    if type(row.get("drop_included")) is not bool:
        raise ValueError("result destruction policy must be explicit")
    if row["mode"] == "latency":
        if type(row.get("elapsed_ns")) is not int or row["elapsed_ns"] < 0:
            raise ValueError("latency row requires elapsed_ns")
        if "allocation_calls" in row:
            raise ValueError("counting allocator must not contaminate latency evidence")
    else:
        if "elapsed_ns" in row:
            raise ValueError("instrumented timings are not latency evidence")
        for name in ("allocation_calls", "allocated_bytes", "deallocation_calls",
                     "reallocation_calls", "peak_live_delta_bytes", "retained_live_delta_bytes"):
            if type(row.get(name)) is not int or row[name] < 0:
                raise ValueError(f"invalid allocation counter {name}")


def aggregate(rows: list[dict]) -> list[dict]:
    """Group identical mechanism dimensions and retain descriptive per-operation samples."""
    groups = {}
    for row in rows:
        validate(row)
        identity = (row["suite"], row["mode"], row["workload"], row["mechanism"],
                    json.dumps(row["dimensions"], sort_keys=True), row["drop_included"],
                    row.get("timing", "batch"), row.get("setup_excluded", True))
        groups.setdefault(identity, []).append(row)
    result = []
    for identity, samples in groups.items():
        if len({row["sample"] for row in samples}) != len(samples):
            raise ValueError("duplicate sample index for one mechanism/dimension combination")
        record = {"suite": identity[0], "mode": identity[1], "workload": identity[2],
                  "mechanism": identity[3], "dimensions": json.loads(identity[4]),
                  "drop_included": identity[5], "timing": identity[6], "setup_excluded": identity[7]}
        metrics = ("elapsed_ns",) if identity[1] == "latency" else (
            "allocation_calls", "allocated_bytes", "deallocation_calls", "reallocation_calls")
        for metric in metrics:
            values = [row[metric] / row["iterations"] for row in samples]
            record[metric + "_per_operation"] = {
                "samples": values, "median": statistics.median(values),
                "min": min(values), "max": max(values)}
        if identity[1] == "allocations":
            for metric in ("peak_live_delta_bytes", "retained_live_delta_bytes"):
                values = [row[metric] for row in samples]
                record[metric] = {"samples": values, "median": statistics.median(values),
                                  "min": min(values), "max": max(values)}
        result.append(record)
    return result


def build(directory: Path, report: dict) -> dict[str, Path]:
    """Compile only the four opt-in targets and resolve their actual Cargo artifact paths."""
    arguments = ["cargo", "+1.88.0", "bench", "--locked", "--no-run", "--message-format=json",
                 "-p", "squish-link", "-p", "squish-backend", "--features",
                 "squish-link/mechanism-benchmarks,squish-backend/mechanism-benchmarks"]
    for name in sorted(TARGETS):
        arguments += ["--bench", name]
    artifacts = {}
    with (directory / "cargo-stderr.txt").open("w", encoding="utf-8") as errors:
        process = subprocess.Popen(arguments, cwd=ROOT, stdout=subprocess.PIPE, stderr=errors,
                                   text=True, encoding="utf-8")
        assert process.stdout is not None
        for line in process.stdout:
            message = json.loads(line)
            if message.get("reason") == "compiler-artifact" and message.get("executable"):
                name = message["target"]["name"]
                if name in TARGETS:
                    artifacts[name] = Path(message["executable"])
                    report.setdefault("cargo_artifact_profiles", {})[name] = message.get("profile", {})
            if message.get("reason") == "compiler-message":
                rendered = message.get("message", {}).get("rendered")
                if rendered:
                    print(rendered, end="")
        if process.wait():
            raise RuntimeError("mechanism benchmark build failed; see hosted compiler diagnostics")
    if set(artifacts) != TARGETS:
        raise ValueError(f"missing benchmark executables: {TARGETS - set(artifacts)}")
    return artifacts


def provenance() -> dict:
    """Describe the actual source, released tag, machine, toolchain and shipping profile."""
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    tag = command("git", "rev-parse", "--verify", "refs/tags/v1.2.0^{commit}")
    cpu = []
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.is_file():
        cpu = sorted({line.split(":", 1)[1].strip() for line in cpuinfo.read_text().splitlines()
                      if line.startswith("model name")})
    return {"source_commit": command("git", "rev-parse", "HEAD"), "released_v1_2_tag_commit": tag,
            "source_version": manifest["package"]["version"], "platform": platform.platform(),
            "cpu_models": cpu, "logical_cpus": os.cpu_count(), "python": platform.python_version(),
            "rustc": command("rustc", "+1.88.0", "-vV"),
            "profile_name": "bench (inherits release unless explicitly overridden)",
            "release_profile": manifest.get("profile", {}).get("release", {}),
            "bench_profile_overrides": manifest.get("profile", {}).get("bench", {}),
            "profile_env": {key: value for key, value in os.environ.items()
                            if key.startswith("CARGO_PROFILE_")},
            "rustflags": os.environ.get("RUSTFLAGS", ""),
            "sampling_env": {key: value for key, value in os.environ.items()
                             if key.startswith("MECHANISM_")},
            "github_run_id": os.environ.get("GITHUB_RUN_ID")}


def collect(directory: Path, report: dict) -> None:
    """Run latency binaries without global instrumentation, then allocation-only passes."""
    report["provenance"] = provenance()
    start = time.perf_counter_ns()
    binaries = build(directory, report)
    report["build_wall_ns"] = time.perf_counter_ns() - start
    all_rows = []
    for name in ("core_latency", "archive_latency", "core_allocations", "archive_allocations"):
        binary = binaries[name]
        report.setdefault("binaries", {})[name] = {"sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                                                   "bytes": binary.stat().st_size}
        result = subprocess.run([str(binary)], cwd=ROOT, capture_output=True,
                                text=True, encoding="utf-8", timeout=300)
        (directory / f"{name}.jsonl").write_text(result.stdout, encoding="utf-8")
        if result.returncode:
            raise RuntimeError(f"{name} failed ({result.returncode}): {result.stderr}")
        rows = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        if not rows:
            raise ValueError(f"{name} emitted no mechanism samples")
        expected_mode = "latency" if name.endswith("latency") else "allocations"
        for row in rows:
            validate(row)
            if row["mode"] != expected_mode:
                raise ValueError(f"wrong instrumentation mode in {name}")
        all_rows += rows
        report["summary"] = aggregate(all_rows)


def main() -> None:
    """Keep all experimental state project-local and persist partial failure metadata."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=ROOT / ".temp/mechanisms")
    parser.add_argument("--summarize", nargs="+", type=Path,
                        help="validate and summarize existing JSONL without building Rust")
    arguments = parser.parse_args()
    directory = arguments.directory.resolve()
    if not directory.is_relative_to(ROOT / ".temp"):
        parser.error("directory must be below project .temp")
    directory.mkdir(parents=True, exist_ok=True)
    report = {"schema": "xmlsquish.mechanism.report.v1", "status": "running", "summary": []}
    try:
        if arguments.summarize:
            rows = [json.loads(line) for path in arguments.summarize
                    for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
            report["summary"] = aggregate(rows)
        else:
            collect(directory, report)
        report["status"] = "success"
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        (directory / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
