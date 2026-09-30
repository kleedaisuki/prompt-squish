"""Measure real process startup and full build costs without claiming noisy CI speedups.

All fixtures live below the repository's .temp or .cache. Cold means an empty
project cache, not a dropped OS page cache. Optional baselines compare only the
stable prompt path; new archive workloads report absolute candidate scaling.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import time
import zipfile


ROOT = Path(__file__).resolve().parents[2]
NS = "https://xmlsquish.moesegfault.dev/ns"


def invoke(binary: Path, arguments: list[str], cwd: Path) -> int:
    """Measure launch through exit, capturing all streams without rendering costs."""
    environment = {**os.environ, "XMLSQUISH_TRACE": "off", "NO_COLOR": "1"}
    start = time.perf_counter_ns()
    result = subprocess.run([str(binary), *arguments], cwd=cwd, env=environment,
                            capture_output=True, timeout=120)
    duration = time.perf_counter_ns() - start
    if result.returncode:
        raise RuntimeError(f"{arguments}: exit {result.returncode}\n"
                           f"{result.stdout.decode('utf-8', errors='replace')}\n"
                           f"{result.stderr.decode('utf-8', errors='replace')}")
    return duration


def fixture(directory: Path, name: str, source: str, backend: str = "squish") -> Path:
    """Create a single explicitly selected unit and no ambient dependency state."""
    directory.mkdir(parents=True, exist_ok=False)
    (directory / "xmlsquish.toml").write_text(
        f'manifest-version = 1\n[package]\nname = "perf-{name}"\n'
        'version = "1.0.0"\nsource-root = "."\n'
        f'[target.{name}]\nentry = "{name}.xml"\nbackend = "{backend}"\n', encoding="utf-8")
    (directory / f"{name}.xml").write_text(source, encoding="utf-8")
    return directory


def summary(samples: list[int]) -> dict:
    """Retain raw samples and robust descriptive statistics, not significance claims."""
    return {"samples_ns": samples, "median_ns": statistics.median(samples),
            "min_ns": min(samples), "max_ns": max(samples)}


def measure(binary: Path, project: Path, rounds: int, cold: bool) -> dict:
    """Measure the entire CLI; clearing project state happens outside timing."""
    arguments = ["build", "--offline", "--plain"]
    invoke(binary, arguments, project)
    samples = []
    for _ in range(rounds):
        if cold:
            shutil.rmtree(project / "target" / "xmlsquish")
        samples.append(invoke(binary, arguments, project))
    return summary(samples)


def archive_check(binary: Path, project: Path, name: str) -> dict:
    """Compare cold rebuilt ZIP bytes and reject mutable timestamps or unsafe names."""
    output = project / "target" / "xmlsquish" / "artifacts" / name
    before = output.read_bytes()
    with zipfile.ZipFile(output) as archive:
        members = archive.namelist()
        if len(members) != len(set(members)) or members != sorted(members):
            raise ValueError("ZIP members must be unique and lexically ordered")
        for entry in archive.infolist():
            if entry.date_time != (1980, 1, 1, 0, 0, 0):
                raise ValueError("ZIP timestamp is not the fixed DOS epoch")
            if entry.filename.startswith("/") or ".." in entry.filename.split("/"):
                raise ValueError("ZIP contains an unsafe member path")
    shutil.rmtree(project / "target" / "xmlsquish")
    invoke(binary, ["build", "--offline", "--plain"], project)
    if before != output.read_bytes():
        raise ValueError(f"cold rebuild changed reproducible bytes: {name}")
    return {"bytes": len(before), "members": len(members),
            "sha256": hashlib.sha256(before).hexdigest()}


def run(arguments: argparse.Namespace, report: dict) -> None:
    """Build tiny and archive scaling fixtures, preserving results from completed probes."""
    scratch = arguments.scratch.resolve()
    if not any(scratch.is_relative_to(ROOT / folder) for folder in (".temp", ".cache")):
        raise ValueError("scratch must be below repository .temp or .cache")
    scratch.mkdir(parents=True, exist_ok=True)
    # A unique run directory prevents accidentally deleting another measurement.
    scratch = scratch / f"run-{time.time_ns()}"
    scratch.mkdir()
    prompt = f'<xs:entry xmlns:xs="{NS}"><Prompt>Hello tiny build</Prompt></xs:entry>'
    binaries = [("candidate", arguments.binary.resolve())]
    if arguments.baseline:
        binaries.append(("baseline", arguments.baseline.resolve()))
    projects = {}
    for label, binary in binaries:
        tiny = fixture(scratch / label, "prompt", prompt)
        if label == "baseline":
            manifest = tiny / "xmlsquish.toml"
            manifest.write_text(manifest.read_text().replace('backend = "squish"\n', ''),
                                encoding="utf-8")
        report["binaries"][label] = {"sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                                    "version": subprocess.check_output(
                                        [str(binary), "--version"], text=True).strip()}
        projects[label] = tiny
        invoke(binary, ["build", "--offline", "--plain"], tiny)
    for workload in ("startup", "tiny.cold", "tiny.warm"):
        samples = {label: [] for label, _ in binaries}
        for round_number in range(arguments.rounds):
            order = binaries if round_number % 2 == 0 else list(reversed(binaries))
            for label, binary in order:
                if workload == "tiny.cold":
                    shutil.rmtree(projects[label] / "target/xmlsquish")
                command = ["--version"] if workload == "startup" else ["build", "--offline", "--plain"]
                samples[label].append(invoke(binary, command, projects[label]))
        for label, values in samples.items():
            report["workloads"][f"{label}.{workload}"] = summary(values)
        if "baseline" in samples:
            ratios = [
                candidate / baseline for candidate, baseline in
                zip(samples["candidate"], samples["baseline"], strict=True)]
            report.setdefault("paired_candidate_over_baseline", {})[workload] = {
                "samples_ratio": ratios, "median_ratio": statistics.median(ratios),
                "min_ratio": min(ratios), "max_ratio": max(ratios)}
    binary = arguments.binary.resolve()
    for count in (1, 64, 2048):
        macros = ''.join(f'<xs:macro name="m:item{i}"><Item>{i}</Item></xs:macro>'
                         for i in range(count))
        expansions = ''.join(f'<xs:expand ref="m:item{i}"/>' for i in range(count))
        project = fixture(scratch / f"many-macros-{count}", "prompt",
                          f'<xs:entry xmlns:xs="{NS}" xmlns:m="urn:perf:index">'
                          f'<xs:import src="macros.xml"/><Prompt>{expansions}</Prompt></xs:entry>')
        (project / "macros.xml").write_text(
            f'<xs:module xmlns:xs="{NS}" xmlns:m="urn:perf:index">{macros}</xs:module>',
            encoding="utf-8")
        report["workloads"][f"candidate.prompt.distinct-macros-{count}"] = measure(
            binary, project, arguments.rounds, True)
        invocations = ''.join('<xs:expand ref="m:match"><xs:arg name="value" '
                              'value="literal regex input"/></xs:expand>' for _ in range(count))
        project = fixture(scratch / f"regex-{count}", "prompt",
                          f'<xs:entry xmlns:xs="{NS}" xmlns:m="urn:perf:regex">'
                          '<xs:import src="macros.xml"/>'
                          f'<Prompt>{invocations}</Prompt></xs:entry>')
        (project / "macros.xml").write_text(
            f'<xs:module xmlns:xs="{NS}" xmlns:m="urn:perf:regex">'
            '<xs:macro name="m:match"><xs:param name="value"/>'
            '<xs:ifr get="arg.value" pattern="^literal.*input$"><Match/></xs:ifr>'
            '</xs:macro></xs:module>', encoding="utf-8")
        report["workloads"][f"candidate.prompt.literal-regex-{count}"] = measure(
            binary, project, arguments.rounds, True)
    for count in (1, 64, 512):
        assets = ''.join(f'<xs:asset path="payload.bin" name="assets/{i:04}.bin"/>'
                         for i in range(count))
        project = fixture(scratch / f"pack-{count}", "bundle",
                          f'<xs:pack xmlns:xs="{NS}">{assets}</xs:pack>', "pack")
        (project / "payload.bin").write_bytes(bytes(range(256)) * 256)
        result = measure(binary, project, arguments.rounds, True)
        result.update(archive_check(binary, project, "bundle.pack"))
        report["workloads"][f"candidate.pack.assets-{count}"] = result
    for count in (1, 64, 512):
        project = sopack_reuse(binary, scratch / f"sopack-{count}", count)
        result = measure(binary, project, arguments.rounds, True)
        result.update(archive_check(binary, project, "bundle.pack"))
        report["workloads"][f"candidate.sopack.macro-reuse-{count}"] = result
    # Phase evidence is collected separately so tracing never contaminates latency.
    traced = projects["candidate"]
    shutil.rmtree(traced / "target/xmlsquish")
    invoke(binary, ["build", "--offline", "--plain", "--trace=events"], traced)
    traces = sorted((traced / "target/xmlsquish/metadata/traces").glob("*.jsonl"))
    if not traces:
        raise ValueError("opt-in events tracing did not persist pipeline evidence")
    report["untimed_pipeline_trace"] = [json.loads(line) for path in traces
                                         for line in path.read_text(encoding="utf-8").splitlines()
                                         if line.strip()]


def sopack_reuse(binary: Path, directory: Path, count: int) -> Path:
    """Relocate an immutable compiled dependency and exercise definition-site assets."""
    producer = fixture(directory / "producer", "library",
                       f'<xs:sopack xmlns:xs="{NS}"><xs:import src="macros.xml"/></xs:sopack>',
                       "sopack")
    manifest = producer / "xmlsquish.toml"
    # `add NAME --path` requires the actual package name unless --rename is used.
    manifest.write_text(manifest.read_text().replace('name = "perf-library"',
                                                    'name = "reusable"')
                        + '\n[exports]\nmain = "macros.xml"\n', encoding="utf-8")
    macros = ''.join(
        f'<xs:macro name="m:asset{i}"><xs:asset path="payload.bin" '
        f'name="assets/{i:04}.bin"/></xs:macro>' for i in range(count))
    (producer / "macros.xml").write_text(
        f'<xs:module xmlns:xs="{NS}" xmlns:m="urn:perf:reusable">{macros}</xs:module>',
        encoding="utf-8")
    (producer / "payload.bin").write_bytes(bytes(range(256)) * 256)
    invoke(binary, ["build", "--offline", "--plain"], producer)
    archive_check(binary, producer, "library.sopack")
    relocated = directory / "relocated" / "library.sopack"
    relocated.parent.mkdir()
    shutil.copyfile(producer / "target/xmlsquish/artifacts/library.sopack", relocated)
    immutable_digest = hashlib.sha256(relocated.read_bytes()).digest()
    shutil.rmtree(producer)
    expansions = ''.join(f'<xs:expand ref="m:asset{i}"/>' for i in range(count))
    consumer = fixture(directory / "consumer", "bundle",
                       f'<xs:pack xmlns:xs="{NS}" xmlns:m="urn:perf:reusable">'
                       f'<xs:import src="pkg:reusable/main"/>{expansions}</xs:pack>', "pack")
    invoke(binary, ["add", "reusable", "--path", str(relocated)], consumer)
    invoke(binary, ["build", "--offline", "--plain"], consumer)
    with zipfile.ZipFile(consumer / "target/xmlsquish/artifacts/bundle.pack") as archive:
        if len(archive.namelist()) != count:
            raise ValueError("SOPack macro reuse did not emit every caller asset")
        for name in archive.namelist():
            if archive.read(name) != bytes(range(256)) * 256:
                raise ValueError("relocated SOPack macro lost definition-site asset bytes")
    if hashlib.sha256(relocated.read_bytes()).digest() != immutable_digest:
        raise ValueError("build mutated its immutable SOPack dependency")
    return consumer


def main() -> None:
    """Write provenance and partial failure evidence even if a measured build fails."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--scratch", type=Path, default=ROOT / ".temp" / "performance")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=5)
    arguments = parser.parse_args()
    if arguments.rounds < 1:
        parser.error("rounds must be positive")
    report = {"schema": 1, "platform": platform.platform(), "python": platform.python_version(),
              "commit": os.environ.get("GITHUB_SHA"), "runner": os.environ.get("RUNNER_NAME"),
              "cold_definition": "empty project cache; OS page cache untouched",
              "binaries": {}, "workloads": {}, "status": "running"}
    try:
        run(arguments, report)
        report["status"] = "success"
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        arguments.report.parent.mkdir(parents=True, exist_ok=True)
        arguments.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
