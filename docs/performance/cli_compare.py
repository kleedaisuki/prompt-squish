"""Compare real CLI batches without publishing source data. / 比较真实 CLI 批量任务而不公开源码。

Usage / 用法: python cli_compare.py BASELINE CANDIDATE SOURCE_DIR --report report.json --rounds 7
"""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


def is_output(path):
    """Identify generated artifacts case-insensitively. / 忽略大小写识别生成产物。"""
    return path.name.lower().endswith((".i.xml", ".o.xml"))


def sources(directory):
    """Reject links and select authored XML only. / 拒绝链接，仅选择手写 XML。"""
    result = []
    for path in directory.rglob("*"):
        if path.is_symlink() or (hasattr(path, "is_junction") and path.is_junction()):
            raise ValueError("Source tree contains links / 源码树包含链接")
        if path.is_file() and path.suffix.lower() == ".xml" and not is_output(path):
            result.append(path)
    if not result:
        raise ValueError("No XML sources / 未找到 XML 源码")
    return sorted(result)


def snapshot(directory, output):
    """Read exact artifact bytes outside timing. / 在计时区域外读取精确产物字节。"""
    return {path.relative_to(directory).as_posix(): path.read_bytes()
            for path in directory.rglob("*")
            if path.is_file() and path.suffix.lower() == ".xml" and is_output(path) == output}


def invoke(binary, directory):
    """Time process launch through termination; inspect outputs afterwards.
    测量进程启动至结束时间，之后检查输出。
    """
    for path in directory.rglob("*"):
        if path.is_file() and is_output(path):
            path.unlink()
    start = time.perf_counter_ns()
    process = subprocess.run([str(binary), "--color=never", "--debug", str(directory)],
                             capture_output=True)
    elapsed = time.perf_counter_ns() - start
    if process.returncode:
        raise RuntimeError(f"CLI failed with exit {process.returncode} / CLI 执行失败")
    artifacts = snapshot(directory, True)
    if not artifacts or not any(name.endswith(".o.xml") for name in artifacts):
        raise ValueError("CLI produced no final outputs / CLI 未生成最终产物")
    return elapsed, (process.returncode, process.stdout, process.stderr, artifacts)


def identity(path):
    """Publish binary identity without host paths. / 发布二进制身份而不暴露主机路径。"""
    return {"name": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    """Copy once, then compare both binaries on identical paths. / 仅复制一次，以相同路径比较二进制。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("source_dir", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=7)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive / 必须为正数")
    if args.source_dir.is_symlink() or (hasattr(args.source_dir, "is_junction") and args.source_dir.is_junction()):
        parser.error("Source root must not be a link / 源码根目录不能为链接")
    source_dir = args.source_dir.resolve(strict=True)
    binaries = {side: getattr(args, side).resolve(strict=True)
                for side in ("baseline", "candidate")}
    files = sources(source_dir)
    report = {"schema": 1, "platform": platform.platform(),
              "python": platform.python_version(),
              "executables": {side: identity(path) for side, path in binaries.items()},
              "input_xml_files": len(files), "warmups_per_binary": 1,
              "scope": "Wall-clock process time includes launch, discovery, compilation, "
                       "tokenization, artifact I/O, and diagnostics. Warm filesystem caches; "
                       "generated outputs removed outside timing. Exact stdout/stderr and all "
                       "intermediate/final artifact bytes compared on identical temporary paths. "
                       "Source bytes checked unchanged outside timing. No source data published.",
              "rounds": []}
    with tempfile.TemporaryDirectory(prefix="xmlsquish-cli-perf-") as temporary:
        directory = Path(temporary)
        for path in files:
            target = directory / path.relative_to(source_dir)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
        original = snapshot(directory, False)
        reference = None
        for side in ("baseline", "candidate"):
            _, observed = invoke(binaries[side], directory)
            if reference is not None and observed != reference:
                raise ValueError("Warmup results differ / 预热结果不一致")
            reference = observed
            if snapshot(directory, False) != original:
                raise ValueError("Sources changed / 源码被修改")
        for index in range(args.rounds):
            order = ["baseline", "candidate"] if index % 2 == 0 else ["candidate", "baseline"]
            measurements = {}
            for side in order:
                print(f"Round {index + 1}/{args.rounds}: {side}", flush=True)
                elapsed, observed = invoke(binaries[side], directory)
                if observed != reference:
                    raise ValueError("CLI results differ / CLI 结果不一致")
                if snapshot(directory, False) != original:
                    raise ValueError("Sources changed / 源码被修改")
                measurements[side] = elapsed
            report["rounds"].append({"index": index + 1, "order": order, "elapsed_ns": measurements,
                                     "candidate_over_baseline": measurements["candidate"] / measurements["baseline"]})
        report["artifact_files"] = len(reference[3])
        report["artifact_bytes"] = sum(map(len, reference[3].values()))
    report["median_ns"] = {side: statistics.median(row["elapsed_ns"][side] for row in report["rounds"])
                           for side in binaries}
    report["median_paired_ratio"] = statistics.median(row["candidate_over_baseline"] for row in report["rounds"])
    report["all_results_identical"] = True
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Report / 报告: {args.report}")


if __name__ == "__main__":
    main()
