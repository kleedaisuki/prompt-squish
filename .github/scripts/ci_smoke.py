"""Exercise the installed root CLI contract. / 验证已构建根 CLI 的进程契约。"""

import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile


COMMANDS = ("build", "fmt", "add", "remove", "inspect")


def invoke(binary: Path, *arguments: str, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    """Capture one CLI process without hiding failures. / 捕获一次 CLI 进程且不隐藏失败。"""
    return subprocess.run(
        [str(binary), *arguments],
        cwd=cwd,
        check=False,
        text=True,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def run(binary: Path, *arguments: str, cwd: Path | None = None) -> str:
    """Run one successful CLI operation and return stdout. / 执行成功的 CLI 操作并返回标准输出。"""
    result = invoke(binary, *arguments, cwd=cwd)
    if result.returncode:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(arguments)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout


def assert_help_contract(help_text: str) -> None:
    """Require every project-manager command in bare help. / 要求裸帮助列出所有项目管理命令。"""
    for command in COMMANDS:
        if not re.search(rf"^  {command}\s", help_text, re.MULTILINE):
            raise RuntimeError(f"bare help omits direct command: {command}")


def smoke(binary: Path, scratch: Path) -> None:
    """Format, build, and inspect a disposable project. / 格式化、构建并检查一次性项目。"""
    if not re.fullmatch(r"xmlsquish \d+\.\d+\.\d+(?:[-+][^\s]+)?", run(binary, "--version").strip()):
        raise RuntimeError("unexpected version contract")
    assert_help_contract(run(binary))
    for command in COMMANDS:
        run(binary, command, "--help")
    usage = invoke(binary, "legacy.xml", "--message-format=json")
    if usage.returncode != 2 or usage.stderr:
        raise RuntimeError("JSON usage failure violated its exit/stream contract")
    records = [json.loads(line) for line in usage.stdout.splitlines()]
    if len(records) != 2 or [record.get("kind") for record in records] != ["diagnostic", "finished"]:
        raise RuntimeError("JSON usage failure omitted its diagnostic or terminal record")
    if records[0].get("phase") != "parse" or records[0].get("exit_code") != 2:
        raise RuntimeError("JSON usage diagnostic omitted parse-phase exit metadata")
    if records[1].get("status") != "failed" or records[1].get("exit_code") != 2:
        raise RuntimeError("JSON usage terminal record omitted failure metadata")

    with tempfile.TemporaryDirectory(prefix="root-cli-", dir=scratch) as directory:
        project = Path(directory)
        (project / "src").mkdir()
        (project / ".xmlsquish").mkdir()
        (project / ".xmlsquish" / "config.toml").write_text(
            '[source]\ncache-root = "cache/sources"\n'
            '[manager]\nstorage-root = "cache/state"\n',
            encoding="utf-8",
        )
        (project / "xmlsquish.toml").write_text(
            'manifest-version = 1\n'
            '[package]\nname = "ci-fixture"\nversion = "1.0.0"\n\n'
            '[target.chat]\nentry = "src/main.xml"\n',
            encoding="utf-8",
        )
        source = project / "src" / "main.xml"
        source.write_text(
            "<xs:entry  xmlns:xs = 'https://xmlsquish.moesegfault.dev/ns' >"
            "<message>Hello   CI</message></xs:entry >",
            encoding="utf-8",
        )

        difference = invoke(binary, "fmt", "--diff", "--plain", cwd=project)
        if difference.returncode != 1 or "--- " not in difference.stdout or "+++ " not in difference.stdout:
            raise RuntimeError("human fmt --diff did not emit a unified diff on stdout")
        run(binary, "fmt", "--plain", cwd=project)
        run(binary, "build", "--plain", cwd=project)
        prompts = list((project / "target" / "xmlsquish").rglob("*.prompt"))
        if not prompts:
            raise RuntimeError("bare build did not publish its default .prompt artifact")
        run(binary, "build", "--emit=ir", "--plain", cwd=project)
        if not list((project / "target" / "xmlsquish").rglob("*.xsir")):
            raise RuntimeError("explicit IR build did not publish an .xsir artifact")
        inspected = run(binary, "inspect", "link", "chat", "--format=json", cwd=project)
        if json.loads(inspected).get("view") != "link":
            raise RuntimeError("inspect did not return the requested link view")


def main() -> None:
    """Validate arguments before creating fixture state. / 创建夹具状态前校验参数。"""
    if len(sys.argv) != 3:
        raise SystemExit("usage: ci_smoke.py BINARY SCRATCH")
    binary = Path(sys.argv[1]).resolve()
    scratch = Path(sys.argv[2]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    smoke(binary, scratch)


if __name__ == "__main__":
    main()
