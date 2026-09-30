"""Exercise v1.2 archive bytes and relocated SOPack dependencies through the real CLI."""

from pathlib import Path
import sys
import time
import zipfile


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "perf"))
from measure import NS, archive_check, fixture, invoke, sopack_reuse  # noqa: E402


def smoke_archives(binary: Path, scratch: Path) -> None:
    """Reject missing bytes, nondeterminism, entry/module confusion and relocation loss."""
    directory = scratch / f"archives-{time.time_ns()}"
    project = fixture(directory / "pack", "bundle",
                      f'<xs:pack xmlns:xs="{NS}">'
                      '<xs:asset path="payload.bin" name="assets/payload.bin"/>'
                      '<xs:include path="prompt.xml" name="instructions.prompt"/>'
                      '</xs:pack>', "pack")
    payload = b"\x00\xff\x80\r\nexact asset bytes\x00"
    (project / "payload.bin").write_bytes(payload)
    (project / "prompt.xml").write_text(
        f'<xs:entry xmlns:xs="{NS}"><Prompt>Hello   archive</Prompt></xs:entry>',
        encoding="utf-8")
    invoke(binary, ["build", "--offline", "--plain"], project)
    archive_check(binary, project, "bundle.pack")
    with zipfile.ZipFile(project / "target/xmlsquish/artifacts/bundle.pack") as archive:
        if archive.namelist() != ["assets/payload.bin", "instructions.prompt"]:
            raise ValueError("pack included unexpected or missing members")
        if archive.read("assets/payload.bin") != payload:
            raise ValueError("asset bytes were transformed")
        if archive.read("instructions.prompt") != b"<Prompt>Hello archive</Prompt>":
            raise ValueError("included entry did not use prompt backend semantics")
    consumer = sopack_reuse(binary, directory / "reusable", 3)
    invoke(binary, ["remove", "reusable"], consumer)
    if "[dependencies.reusable]" in (consumer / "xmlsquish.toml").read_text():
        raise ValueError("remove retained SOPack dependency declaration")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: archive_smoke.py BINARY SCRATCH")
    binary, scratch = (Path(argument).resolve() for argument in sys.argv[1:])
    scratch.mkdir(parents=True, exist_ok=True)
    smoke_archives(binary, scratch)
