"""Collect CLI syscall and lifecycle counts separately from latency measurements.

strace changes execution cost: its timings are intentionally not interpreted as
latency. Tracing-off syscall probes and separate tracing-on lifecycle probes use
fresh project-cache states. Store-example counters are a separate primitive probe,
not mislabeled as all SQLite statements issued by the CLI.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess

from measure import NS, fixture, invoke


SYSCALLS = "read,pread64,write,pwrite64,openat,close,fsync,fdatasync,rename,renameat,renameat2,unlink,unlinkat,statx,newfstatat"


def cli(source: Path, target: Path) -> Path:
    """Build only the selected CLI with the same shipping profile as the mechanism binaries."""
    subprocess.run(["cargo", "+1.88.0", "build", "--release", "--bin", "xmlsquish", "--locked"],
                   cwd=source, env={**os.environ, "CARGO_TARGET_DIR": str(target)}, check=True)
    return target / "release/xmlsquish"


def parse_strace(text: str) -> dict:
    """Extract counts/errors only; instrumented percentages and durations are discarded."""
    result = {}
    for line in text.splitlines():
        values = line.split()
        if len(values) in (5, 6) and re.fullmatch(r"[a-z][a-z0-9_]*", values[-1]):
            if values[-1] == "total":
                continue
            result[values[-1]] = {"calls": int(values[3]), "errors": int(values[4]) if len(values) == 6 else 0}
    if not result:
        raise ValueError("strace emitted no recognized syscall count rows")
    return result


def artifacts(project: Path) -> dict:
    """Retain output byte fingerprints without uploading binary artifacts."""
    root = project / "target/xmlsquish/artifacts"
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in root.rglob("*") if path.is_file() and path.suffix in (".prompt", ".pack")}


def prepare(binary: Path, directory: Path) -> dict[str, Path]:
    """Create tiny, large-asset and mostly-unreachable compiled-dependency fixtures."""
    tiny = fixture(directory / "tiny", "prompt",
                   f'<xs:entry xmlns:xs="{NS}"><Prompt>tiny IO probe</Prompt></xs:entry>')
    pack = fixture(directory / "large-assets", "bundle",
                   f'<xs:pack xmlns:xs="{NS}"><xs:asset path="large.bin" name="large.bin"/></xs:pack>', "pack")
    (pack / "large.bin").write_bytes(bytes(range(256)) * (32 * 1024))
    producer = fixture(directory / "library", "library",
                       f'<xs:sopack xmlns:xs="{NS}">'
                       + ''.join(f'<xs:import src="unit{i:03}.xml"/>' for i in range(32)) + '</xs:sopack>', "sopack")
    manifest = producer / "xmlsquish.toml"
    manifest.write_text(manifest.read_text().replace('name = "perf-library"', 'name = "io-library"')
                        + '\n[exports]\nmain = "unit000.xml"\n', encoding="utf-8")
    for number in range(32):
        body = '<xs:macro name="m:used">small reachable module</xs:macro>' if number == 0 else (
            f'<xs:macro name="m:unused{number}"><xs:asset path="unused.bin" name="unused{number}.bin"/></xs:macro>')
        (producer / f"unit{number:03}.xml").write_text(
            f'<xs:module xmlns:xs="{NS}" xmlns:m="urn:io:library">{body}</xs:module>', encoding="utf-8")
    (producer / "unused.bin").write_bytes(bytes(range(256)) * (32 * 1024))
    invoke(binary, ["build", "--offline", "--plain"], producer)
    consumer = fixture(directory / "small-reachable", "prompt",
                       f'<xs:entry xmlns:xs="{NS}" xmlns:m="urn:io:library">'
                       '<xs:import src="pkg:io-library/main"/><Prompt><xs:expand ref="m:used"/></Prompt></xs:entry>')
    (consumer / "deps").mkdir()
    shutil.copyfile(producer / "target/xmlsquish/artifacts/library.sopack", consumer / "deps/library.sopack")
    shutil.rmtree(producer)
    invoke(binary, ["add", "io-library", "--path", "deps/library.sopack"], consumer)
    return {"tiny": tiny, "large_assets": pack, "sopack_small_reachable": consumer}


def probe(binary: Path, directory: Path) -> dict:
    """Measure default-off syscall counts and separate optional events with explicit states."""
    if shutil.which("strace") is None:
        raise ValueError("hosted IO counts require strace; install it on the Linux runner")
    report = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "syscall_probe_trace": "off", "instrumented_timings_used": False, "cases": {}}
    for name, project in prepare(binary, directory).items():
        observations = {}
        for state in ("cold", "warm"):
            if state == "cold":
                shutil.rmtree(project / "target/xmlsquish", ignore_errors=True)
            output = directory / f"{name}-{state}-strace.txt"
            subprocess.run(["strace", "-f", "-qq", "-c", "-e", f"trace={SYSCALLS}", "-o", str(output),
                            str(binary), "build", "--offline", "--plain", "--trace=off"], cwd=project,
                           env={**os.environ, "XMLSQUISH_TRACE": "off", "NO_COLOR": "1"},
                           capture_output=True, check=True, timeout=120)
            observations[state] = {"syscalls": parse_strace(output.read_text()), "artifacts": artifacts(project)}
        if observations["cold"]["artifacts"] != observations["warm"]["artifacts"]:
            raise ValueError(f"cold/warm IO probe changed product bytes: {name}")
        # Separate cold/warm runs: the trace file's own writes are not in syscall counts.
        shutil.rmtree(project / "target/xmlsquish")
        seen_invocations = set()
        for state in ("cold", "warm"):
            invoke(binary, ["build", "--offline", "--plain", "--trace=events"], project)
            records = [json.loads(line) for path in (project / "target/xmlsquish/metadata/traces").glob("*.jsonl")
                       for line in path.read_text().splitlines() if line.strip()]
            invocations = {record["invocation"] for record in records if record.get("kind") == "command_start"}
            added = invocations - seen_invocations
            if len(added) != 1:
                raise ValueError("trace probe must identify exactly one new invocation")
            current = added.pop()
            seen_invocations = invocations
            selected = [record for record in records if record.get("invocation") == current]
            kinds, events = {}, {}
            for record in selected:
                kinds[record["kind"]] = kinds.get(record["kind"], 0) + 1
                event = record.get("data", {}).get("event", {}).get("payload", {}).get("type")
                if event:
                    events[event] = events.get(event, 0) + 1
            observations[state]["separate_trace"] = {"record_counts": kinds, "lifecycle_event_counts": events}
        report["cases"][name] = observations
    return report


def latency_comparison(binaries: dict[str, Path], directory: Path, passes: int) -> dict:
    """Measure complete CLI processes without strace/tracing on the same runner.

    Project-cache cold reset and warm priming are outside timed regions. Cold
    means project cache, not OS page cache. Every output is checked outside timing.
    Process groups alternate AB/BA; medians are descriptive, not significance tests.
    """
    directory.mkdir(parents=True, exist_ok=False)
    projects = {version: prepare(binary, directory / version) for version, binary in binaries.items()}
    expected = {}
    for version, cases in projects.items():
        for name, project in cases.items():
            invoke(binaries[version], ["build", "--offline", "--plain", "--trace=off"], project)
            value = artifacts(project)
            if not value or (name in expected and expected[name] != value):
                raise ValueError(f"cross-version CLI fixture output mismatch: {name}")
            expected[name] = value
    rows = []
    for number in range(passes):
        order = ("baseline", "candidate") if number % 2 == 0 else ("candidate", "baseline")
        for position, version in enumerate(order):
            binary = binaries[version]
            for name in ("startup", *projects[version]):
                states = ("process",) if name == "startup" else ("cold", "warm")
                project = directory if name == "startup" else projects[version][name]
                for state in states:
                    arguments = ["--version"] if name == "startup" else ["build", "--offline", "--plain", "--trace=off"]
                    if state == "warm":
                        invoke(binary, arguments, project)
                    samples = []
                    for _ in range(5):
                        if state == "cold":
                            shutil.rmtree(project / "target/xmlsquish", ignore_errors=True)
                        samples.append(invoke(binary, arguments, project))
                        if name != "startup" and artifacts(project) != expected[name]:
                            raise ValueError(f"CLI product changed after measured operation: {version}/{name}/{state}")
                    rows.append(dict(pass_index=number, position=position, version=version,
                                     workload=name, state=state, elapsed_ns_samples=samples,
                                     median_elapsed_ns=statistics.median(samples)))
    paired = []
    for number in range(passes):
        before = {(row["workload"], row["state"]): row for row in rows
                  if row["pass_index"] == number and row["version"] == "baseline"}
        after = {(row["workload"], row["state"]): row for row in rows
                 if row["pass_index"] == number and row["version"] == "candidate"}
        for key, old in before.items():
            new = after[key]
            paired.append(dict(pass_index=number, workload=key[0], state=key[1],
                               baseline_median_ns=old["median_elapsed_ns"],
                               candidate_median_ns=new["median_elapsed_ns"],
                               candidate_over_baseline=new["median_elapsed_ns"] / old["median_elapsed_ns"]))
    return dict(process_spawn_included=True, fixture_setup_excluded=True, trace="off",
                strace_used=False, cold_state="project cache empty; OS page cache uncontrolled",
                repetitions_per_process_group=5, raw_process_groups=rows, paired_process_groups=paired,
                product_sha256=expected)
