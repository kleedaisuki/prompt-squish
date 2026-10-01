"""Generate a foreign-provider fixture using only the verified shipped v1.2 CLI.

This is a preparation job, never evidence that the current source passed tests.
The official binary stays on the hosted runner; only a tiny archive and its
provenance are uploaded for a mandatory new-reader regression fixture.
"""

import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parents[2]
NAME = "xmlsquish-1.2.0-x86_64-unknown-linux-gnu.tar.gz"
URL = "https://github.com/kleedaisuki/prompt-squish/releases/download/v1.2.0"
NS = "https://xmlsquish.moesegfault.dev/ns"


def download(directory: Path) -> tuple[Path, str]:
    """Verify the fixed official archive against its unique published SHA-256 entry."""
    with urllib.request.urlopen(f"{URL}/SHA256SUMS", timeout=60) as response:
        checksums = response.read().decode("utf-8")
    with urllib.request.urlopen(f"{URL}/{NAME}", timeout=120) as response:
        data = response.read()
    expected = [line.split()[0] for line in checksums.splitlines()
                if len(line.split()) == 2 and line.split()[1] == NAME]
    digest = hashlib.sha256(data).hexdigest()
    if len(expected) != 1 or expected[0] != digest:
        raise ValueError("shipped v1.2 archive checksum missing or mismatched")
    archive_path = directory / NAME
    archive_path.write_bytes(data)
    with tarfile.open(archive_path, "r:gz") as archive:
        members = [member for member in archive.getmembers()
                   if member.name == "xmlsquish-1.2.0-x86_64-unknown-linux-gnu/xmlsquish"]
        if len(members) != 1 or not members[0].isfile():
            raise ValueError("official archive omitted its single regular executable")
        source = archive.extractfile(members[0])
        if source is None:
            raise ValueError("official executable cannot be read")
        binary = directory / "xmlsquish"
        binary.write_bytes(source.read())
        binary.chmod(0o755)
    return binary, digest


def main() -> None:
    """Build the old foreign namespace twice and retain exact deterministic bytes."""
    directory = ROOT / ".temp/legacy-fixture-preparation"
    directory.mkdir(parents=True, exist_ok=True)
    binary, archive_digest = download(directory)
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if version != "xmlsquish 1.2.0":
        raise ValueError("official fixture writer version mismatch")
    producer = directory / "legacy-library"
    source = producer / "src"
    provider = producer / "vendor/provider"
    source.mkdir(parents=True)
    (provider / "src").mkdir(parents=True)
    (producer / "xmlsquish.toml").write_text(
        'manifest-version = 1\n[package]\nname = "legacy-library"\nversion = "1.0.0"\n'
        'source-root = "src"\n[target.library]\nentry = "src/main.xml"\nbackend = "sopack"\n'
        '[exports]\nmain = "src/main.xml"\n[dependencies.provider]\n'
        'package = "legacy-provider"\npath = "vendor/provider"\n', encoding="utf-8")
    (provider / "xmlsquish.toml").write_text(
        'manifest-version = 1\n[package]\nname = "legacy-provider"\nversion = "1.0.0"\n'
        'source-root = "src"\n[exports]\nmain = "src/lib.xml"\n', encoding="utf-8")
    (source / "main.xml").write_text(
        f'<xs:sopack xmlns:xs="{NS}" xmlns:m="urn:test:legacy-library" '
        'xmlns:p="urn:test:legacy-provider"><xs:import src="pkg:provider/main"/>'
        '<xs:macro name="m:bundle"><xs:expand ref="p:asset"/></xs:macro></xs:sopack>', encoding="utf-8")
    (provider / "src/lib.xml").write_text(
        f'<xs:module xmlns:xs="{NS}" xmlns:p="urn:test:legacy-provider">'
        '<xs:macro name="p:asset"><xs:asset path="owned.bin" name="legacy.bin"/>'
        '</xs:macro></xs:module>', encoding="utf-8")
    (provider / "src/owned.bin").write_bytes(b"legacy\x00\xffprovider")
    arguments = [str(binary), "build", "--offline", "--plain"]
    subprocess.run(arguments, cwd=producer, check=True)
    output = producer / "target/xmlsquish/artifacts/library.sopack"
    data = output.read_bytes()
    subprocess.run([str(binary), "clean", "--plain"], cwd=producer, check=True)
    subprocess.run(arguments, cwd=producer, check=True)
    if data != output.read_bytes():
        raise ValueError("shipped v1.2 fixture did not reproduce exact archive bytes")
    with zipfile.ZipFile(output) as archive:
        names = archive.namelist()
        if not any("_providers/" in name for name in names):
            raise ValueError("fixture did not exercise the old foreign-provider namespace")
    fixtures = ROOT / "tests/fixtures"
    fixtures.mkdir(parents=True, exist_ok=True)
    fixture = fixtures / "v1.2.0-library.sopack"
    fixture.write_bytes(data)
    provenance = {
        "schema": "xmlsquish.legacy-fixture.v1", "preparation_only": True,
        "writer_version": version, "official_url": f"{URL}/{NAME}",
        "official_archive_sha256": archive_digest,
        "writer_executable_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "fixture_sha256": hashlib.sha256(data).hexdigest(), "fixture_bytes": len(data),
        "foreign_provider_namespace_present": True, "deterministic_clean_rebuild": True,
    }
    (directory / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
