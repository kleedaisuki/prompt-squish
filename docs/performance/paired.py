"""Shared mechanics for paired benchmark reports.

配对基准报告的共享机制。此模块不决定工作负载、等价性断言或报告格式。
"""

import hashlib
from pathlib import Path
from typing import Iterator


def executable_identity(path: Path) -> dict[str, str]:
    """Record the name and SHA-256 of the exact executable bytes.

    记录可执行文件的名称与精确字节的 SHA-256；不暴露本机绝对路径。
    """

    return {"name": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def alternating_rounds(count: int) -> Iterator[tuple[int, list[str]]]:
    """Yield one-based AB/BA rounds for sequential paired measurements.

    依次生成从 1 开始的 AB/BA 轮次；调用者负责验证正数轮数并执行测量。
    """

    for index in range(count):
        order = ["baseline", "candidate"] if index % 2 == 0 else ["candidate", "baseline"]
        yield index + 1, order
