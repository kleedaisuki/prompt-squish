"""Compare pre-repair and candidate production mechanisms on one hosted machine.

Each version compiles its own harness/API. Only a baseline-compatible archive
oracle helper/patch is injected into the reference benchmark, never candidate
production source or a harness calling APIs absent from the pinned baseline.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

from mechanisms import ROOT, aggregate, build, provenance, split_records
from io_counts import cli, probe as io_probe, latency_comparison


BASELINE = "1bbf6cd0ec9827a186da18fa0cbe8943257fa8fc"
AXES = {
    "core": {"definitions", "expansions", "called_last", "unused_payload_bytes",
             "selected_static_regions", "scalar_bytes", "shared_scalar_regions", "scalar_is_static",
             "unused_patterns", "unique_values", "units", "source_bytes"},
    "archive": {"members", "content_bytes", "name_bytes", "units", "providers", "import_edges",
                "source_bytes", "encoded_unit_bytes", "asset_bindings", "unique_asset_bytes",
                "expanded_asset_bytes"},
}
PRODUCTION_MAP = {
    "middle_prepared_input": "middle_shared_prepared_input",
    "link_prepared_input": "link_prepared_facts",
    "link_clone_plus_call": "link_prepared_clone_plus_call",
    "zip_strict_read_owned_members": "zip_read_direct_borrowed",
    "zip_write_borrowed_entries": "zip_write_direct_borrowed",
    "sopack_strict_read_relocate_and_expand": "sopack_read_shared_relocate",
    "sopack_write_normalize_and_transport": "sopack_write_borrowed_producer_graph",
}


def case(row: dict) -> dict:
    """Match input geometry, never synthetic algorithm rounds or transport namespaces."""
    return {"suite": row["suite"], "workload": row["workload"],
            "dimensions": {name: value for name, value in row["dimensions"].items()
                           if name in AXES[row["suite"]]}}


def identity(value: dict) -> str:
    """Use canonical JSON keys for deterministic comparison and report identities."""
    return json.dumps(value, sort_keys=True)


def fingerprint(oracle: dict) -> dict:
    """ZIP bytes remain exact; SOPack comparison uses full normalized semantic content."""
    return oracle.get("checksums") or {name: oracle[name] for name in (
        "document_sha256", "trace_sha256", "directives_sha256")}


def oracle_map(oracles: list[dict]) -> dict[str, dict]:
    """Require internally consistent actual outputs for one semantic fixture geometry."""
    result = {}
    for oracle in oracles:
        key, value = identity(case(oracle)), fingerprint(oracle)
        if key in result and result[key] != value:
            raise ValueError(f"inconsistent untimed oracle: {key}")
        result[key] = value
    return result


def measurement_key(row: dict, version: str, production: bool = True) -> str:
    """Explicitly map public production paths while preserving owned compatibility controls."""
    mechanism = row["mechanism"]
    if production and version == "baseline":
        mechanism = PRODUCTION_MAP.get(mechanism, mechanism)
    return identity({**case(row), "mode": row["mode"], "mechanism": mechanism,
                     "drop_included": row["drop_included"], "timing": row.get("timing", "batch"),
                     "setup_excluded": row.get("setup_excluded", True)})


def compare(phases: list[dict]) -> dict:
    """Pair independent process medians by identical case; new cases remain unpaired."""
    passes, results, controls, unpaired = {}, {}, {}, {}
    for phase in phases:
        passes.setdefault(phase["pass"], {})[phase["version"]] = phase
    for number, versions in passes.items():
        if set(versions) != {"baseline", "candidate"}:
            raise ValueError("incomplete full-repair process pass")
        before_oracles = oracle_map(versions["baseline"]["oracles"])
        after_oracles = oracle_map(versions["candidate"]["oracles"])
        common = before_oracles.keys() & after_oracles.keys()
        if not common:
            raise ValueError("no common semantic fixture oracles")
        for key in common:
            if before_oracles[key] != after_oracles[key]:
                raise ValueError(f"cross-version canonical output mismatch: {key}")
        for production, destination in ((True, results), (False, controls)):
            maps = {}
            for version, phase in versions.items():
                values = aggregate(phase["rows"])
                maps[version] = {}
                for row in values:
                    # Candidate owned wrappers remain separate compatibility controls.
                    if production and version == "candidate" and row["mechanism"] in PRODUCTION_MAP:
                        continue
                    key = measurement_key(row, version, production)
                    if key in maps[version]:
                        raise ValueError(f"ambiguous semantic measurement mapping: {key}")
                    maps[version][key] = row
            paired = maps["baseline"].keys() & maps["candidate"].keys()
            if production:
                unpaired[number] = {
                    version: [maps[version][key] for key in sorted(maps[version].keys() - paired)] for version in maps}
            for key in paired:
                before, after = maps["baseline"][key], maps["candidate"][key]
                if identity(case(before)) not in common:
                    raise ValueError(f"paired measurement lacks an independent equivalent-output oracle: {key}")
                record = destination.setdefault(key, {**json.loads(key), "passes": [],
                    "baseline_mechanism": before["mechanism"], "candidate_mechanism": after["mechanism"]})
                if after["mechanism"] in ("link_prepared_facts", "link_prepared_clone_plus_call"):
                    record["comparison_contract"] = "warm prepared-fact reuse; candidate cold preparation/specialization excluded and retained separately"
                elif after["mechanism"] == "middle_shared_prepared_input":
                    record["comparison_contract"] = "public middle-end ownership mechanism; not manager repeated specialization"
                metrics = ("elapsed_ns_per_operation",) if before["mode"] == "latency" else (
                    "allocation_calls_per_operation", "allocated_bytes_per_operation",
                    "peak_live_delta_bytes", "retained_live_delta_bytes")
                sample = {"pass": number, "metrics": {}}
                for metric in metrics:
                    old, new = before[metric]["median"], after[metric]["median"]
                    sample["metrics"][metric] = {"baseline": old, "candidate": new,
                                                "candidate_over_baseline": new / old if old else None}
                record["passes"].append(sample)
        for key, value in after_oracles.items():
            if key not in before_oracles:
                unpaired.setdefault("candidate_only_oracles", {})[key] = value
    for destination in (results, controls):
        for record in destination.values():
            record["median_process_pass_ratios"] = {}
            for metric in record["passes"][0]["metrics"]:
                values = [item["metrics"][metric]["candidate_over_baseline"] for item in record["passes"]]
                valid = [value for value in values if value is not None]
                record["median_process_pass_ratios"][metric] = statistics.median(valid) if valid else None
    return {"production_path_comparisons": list(results.values()),
            "same_public_api_compatibility_controls": list(controls.values()), "unpaired": unpaired}


def prepare_reference(directory: Path, report: dict) -> Path:
    """Retain version-specific baseline benchmarks with only an old-API oracle patch."""
    reference = ROOT / ".temp/full-repair-reference"
    if reference.exists():
        raise ValueError("reference worktree already exists; use a fresh hosted checkout")
    subprocess.run(["git", "worktree", "add", "--detach", str(reference), BASELINE], cwd=ROOT, check=True)
    helper = ROOT / "scripts/perf/archive_oracle.rs"
    destination = reference / "scripts/perf/archive_oracle.rs"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(helper.read_bytes())
    patch = ROOT / "scripts/perf/baseline_archive_oracle.patch"
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=reference, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=reference, check=True)
    report["baseline_benchmark_only_oracle_patch"] = {
        "helper_sha256": hashlib.sha256(helper.read_bytes()).hexdigest(),
        "patch_sha256": hashlib.sha256(patch.read_bytes()).hexdigest()}
    for profile in ("release", "bench"):
        baseline = tomllib.loads((reference / "Cargo.toml").read_text())["profile"].get(profile, {})
        candidate = tomllib.loads((ROOT / "Cargo.toml").read_text())["profile"].get(profile, {})
        if baseline != candidate:
            raise ValueError(f"different baseline/candidate {profile} profiles")
    return reference


def run(directory: Path, passes: int, report: dict, include_io: bool) -> None:
    """Compile selected executables and run three bounded AB/BA whole-process passes."""
    report.update(provenance=provenance(), baseline_commit=BASELINE,
                  pairing="same-runner AB/BA process medians; not lockstep operation samples",
                  explicit_production_mechanism_map=PRODUCTION_MAP,
                  warm_reuse_contract="link_prepared_facts excludes one-time preparation/specialization; candidate-only cold-tax rows are retained, never summed/subtracted to infer CLI speed", semantic_axis_allowlist={k: sorted(v) for k, v in AXES.items()})
    reference = prepare_reference(directory, report)
    binaries = {}
    for version, source, target in (("baseline", reference, ROOT / ".cache/full-repair-reference-target"),
                                    ("candidate", ROOT, ROOT / "target")):
        output = directory / version
        output.mkdir()
        metadata = {}
        start = time.perf_counter_ns()
        binaries[version] = build(output, metadata, source, target)
        report.setdefault("builds", {})[version] = {**metadata, "wall_ns": time.perf_counter_ns() - start,
            "binary_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries[version].items()}}
    phases = []
    environment = dict(os.environ)
    environment.pop("MECHANISM_ORIGIN_ONLY", None)
    for number in range(passes):
        order = ("baseline", "candidate") if number % 2 == 0 else ("candidate", "baseline")
        for position, version in enumerate(order):
            phase = {"pass": number, "position": position, "version": version, "rows": [], "oracles": []}
            for name in ("core_latency", "archive_latency", "core_allocations", "archive_allocations"):
                completed = subprocess.run([str(binaries[version][name])], cwd=ROOT, env=environment,
                                           capture_output=True, text=True, encoding="utf-8", timeout=300)
                filename = f"pass-{number}-{position}-{version}-{name}.jsonl"
                (directory / filename).write_text(completed.stdout, encoding="utf-8")
                if completed.returncode:
                    raise RuntimeError(f"{version} {name} failed: {completed.stderr}")
                rows, oracles = split_records([json.loads(line) for line in completed.stdout.splitlines() if line.strip()])
                if not rows:
                    raise ValueError(f"no measured rows from {version} {name}")
                phase["rows"] += rows
                phase["oracles"] += oracles
            phases.append(phase)
            report.setdefault("phases", []).append({**phase, "rows": len(phase["rows"]), "oracles": len(phase["oracles"])})
    report.update(compare(phases))
    if include_io:
        report["separate_instrumented_io_counts"] = {}
        cli_binaries = {version: cli(source, target) for version, source, target in (
            ("baseline", reference, ROOT / ".cache/full-repair-reference-target"),
            ("candidate", ROOT, ROOT / "target"))}
        report["uninstrumented_cli_latency"] = latency_comparison(cli_binaries, directory / "cli-latency", passes)
        for version, source, target in (("baseline", reference, ROOT / ".cache/full-repair-reference-target"),
                                        ("candidate", ROOT, ROOT / "target")):
            binary = cli_binaries[version]
            report["separate_instrumented_io_counts"][version] = io_probe(binary, directory / f"{version}-io")
        before = report["separate_instrumented_io_counts"]["baseline"]["cases"]
        after = report["separate_instrumented_io_counts"]["candidate"]["cases"]
        for name in before.keys() & after.keys():
            if before[name]["cold"]["artifacts"] != after[name]["cold"]["artifacts"]:
                raise ValueError(f"cross-version CLI product changed in IO probe: {name}")
        completed = subprocess.run(["cargo", "+1.88.0", "run", "-p", "squish-store", "--example",
                                    "io_probe", "--release", "--locked", "--", ".temp/store-io-probe"],
                                   cwd=ROOT, capture_output=True, text=True, encoding="utf-8", check=True)
        report["separate_candidate_store_primitive_counts"] = json.loads(completed.stdout.strip())


def main() -> None:
    """Keep raw small evidence and partial failures under repository .temp only."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=ROOT / ".temp/full-repair-comparison")
    parser.add_argument("--passes", type=int, default=3)
    parser.add_argument("--io", action="store_true", help="collect separate CLI strace/lifecycle and store-primitive counts")
    arguments = parser.parse_args()
    directory = arguments.directory.resolve()
    if not directory.is_relative_to(ROOT / ".temp") or not 1 <= arguments.passes <= 5:
        parser.error("directory must be project .temp and passes must be between 1 and 5")
    directory.mkdir(parents=True, exist_ok=True)
    report = {"schema": "xmlsquish.full-repair-comparison.v1", "status": "running"}
    try:
        run(directory, arguments.passes, report, arguments.io)
        report["status"] = "success"
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        (directory / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
