"""Compare alternating compiler benchmark binaries. / 交替比较编译器基准二进制。

Usage / 用法: python compare.py BASELINE CANDIDATE --report paired-results.json --rounds 3
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess


def run(binary):
    """Collect one isolated benchmark process. / 收集一个独立基准进程的结果。"""
    command = [str(binary), "compiler::perf_tests::compiler_baseline", "--ignored",
               "--nocapture", "--test-threads=1"]
    process = subprocess.run(command, capture_output=True, text=True, encoding="utf-8")
    if process.returncode:
        raise RuntimeError(f"Benchmark failed / 基准失败: {command}\n"
                           f"{process.stdout}\n{process.stderr}")
    records = {}
    for line in process.stdout.splitlines():
        match = re.search(r"\b(PERF(?:_PHASE)?) (\{.*\})\s*$", line)
        if not match:
            continue
        row = json.loads(match[2])
        key = (row["name"], row.get("phase", "compile"))
        if key in records or not row["samples_ns"]:
            raise ValueError(f"Duplicate or empty measurement / 重复或空测量: {key}")
        if not all(isinstance(value, int) and value > 0 for value in row["samples_ns"]):
            raise ValueError(f"Invalid sample / 非法样本: {key}")
        records[key] = row
    if not records:
        raise ValueError("No benchmark records / 未找到基准记录")
    return records


def check_pair(baseline, candidate):
    """Reject corpus or artifact mismatches. / 拒绝语料或产物不一致的比较。"""
    if baseline.keys() != candidate.keys():
        raise ValueError("Benchmark case sets differ / 基准案例集合不一致")
    fields = ("output_bytes", "intermediate_bytes", "output_fnv", "intermediate_fnv")
    for key, before in baseline.items():
        after = candidate[key]
        if before["batch"] != after["batch"]:
            raise ValueError(f"Batch differs / 批量大小不同: {key}")
        if key[1] == "compile" and any(before[field] != after[field] for field in fields):
            raise ValueError(f"Output/IR differs / 输出或 IR 不同: {key}")


def executable(path):
    """Record exact executable identity. / 记录精确的可执行文件身份。"""
    return {"name": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def rust_version():
    """Record installed rustc, not inferred binary provenance. / 记录当前 rustc，不推断构建来源。"""
    try:
        return subprocess.check_output(["rustc", "--version"], text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def main():
    """Alternate process order and retain paired raw samples. / 交替进程顺序并保留配对原始样本。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive / 必须为正数")
    paths = {name: getattr(args, name).resolve(strict=True)
             for name in ("baseline", "candidate")}
    report = {
        "schema": 1,
        "executables": {name: executable(path) for name, path in paths.items()},
        "source": {"baseline": "7c1e10d with identical benchmark harness",
                   "candidate": "working tree; executable SHA256 identifies measured build"},
        "environment": {"platform": platform.platform(), "machine": platform.machine(),
                        "processor": platform.processor(), "python": platform.python_version(),
                        "installed_rustc": rust_version(), "cwd": "repository root",
                        "samples_override": os.environ.get("XMLSQUISH_PERF_SAMPLES")},
        "method": "Alternate AB/BA process order; three harness warmups; medians are descriptive, "
                  "not confidence intervals. Ratio is candidate/baseline; below 1 is faster.",
        "scope": "In-memory full compiler includes result destruction and provenance serialization, "
                 "excludes CLI I/O and final squish. Discovery and prepared expansion phases run "
                 "independently and are not additive. Artifacts compared by lengths and FNV-1a "
                 "fingerprints, not a cryptographic equivalence proof.",
        "rounds": [],
    }
    reference = None
    for index in range(args.rounds):
        order = ["baseline", "candidate"] if index % 2 == 0 else ["candidate", "baseline"]
        pair = {}
        for name in order:
            print(f"Round {index + 1}/{args.rounds}: {name}", flush=True)
            pair[name] = run(paths[name])
        check_pair(pair["baseline"], pair["candidate"])
        if reference is not None:
            check_pair(reference, pair["baseline"])
        reference = pair["baseline"]
        report["rounds"].append({"index": index + 1, "order": order,
                                 "measurements": {name: list(rows.values())
                                                  for name, rows in pair.items()}})
    summaries = []
    for name, phase in sorted(reference):
        medians = {"baseline": [], "candidate": []}
        for round_data in report["rounds"]:
            for side in medians:
                row = next(row for row in round_data["measurements"][side]
                           if (row["name"], row.get("phase", "compile")) == (name, phase))
                medians[side].append(statistics.median(row["samples_ns"]))
        ratios = [after / before for before, after in zip(medians["baseline"], medians["candidate"])]
        summaries.append({"name": name, "phase": phase, "round_medians_ns": medians,
                          "round_candidate_over_baseline": ratios,
                          "median_paired_ratio": statistics.median(ratios)})
    report["summary"] = summaries
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Report / 报告: {args.report}")


if __name__ == "__main__":
    main()
