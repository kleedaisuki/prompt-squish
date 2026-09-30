"""Download the official Linux v1.1.0 release and verify its published SHA-256.

Checksums verify bytes, not publisher identity. HTTPS and the fixed repository
release URL provide the trust boundary; never extract arbitrary archive paths.
"""

import hashlib
from pathlib import Path
import tarfile
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
NAME = "xmlsquish-1.1.0-x86_64-unknown-linux-gnu.tar.gz"
BASE = "https://github.com/kleedaisuki/prompt-squish/releases/download/v1.1.0"


def verify(data: bytes, checksums: str) -> None:
    """Require exactly one matching filename and digest before archive inspection."""
    matches = [line.split()[0] for line in checksums.splitlines()
               if len(line.split()) == 2 and line.split()[1] == NAME]
    if len(matches) != 1 or hashlib.sha256(data).hexdigest() != matches[0]:
        raise ValueError("official baseline checksum mismatch or missing digest")


def main() -> None:
    """Keep network files and the extracted sole executable inside project .cache."""
    directory = ROOT / ".cache" / "perf-baseline-v1.1.0"
    directory.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(f"{BASE}/SHA256SUMS", timeout=60) as response:
        checksums = response.read().decode("utf-8")
    with urllib.request.urlopen(f"{BASE}/{NAME}", timeout=120) as response:
        data = response.read()
    verify(data, checksums)
    archive_path = directory / NAME
    archive_path.write_bytes(data)
    with tarfile.open(archive_path, "r:gz") as archive:
        members = [member for member in archive.getmembers()
                   if member.name == "xmlsquish-1.1.0-x86_64-unknown-linux-gnu/xmlsquish"]
        if len(members) != 1 or not members[0].isfile():
            raise ValueError("baseline archive must contain one regular native binary")
        source = archive.extractfile(members[0])
        if source is None:
            raise ValueError("baseline executable cannot be read")
        binary = directory / "xmlsquish"
        binary.write_bytes(source.read())
        binary.chmod(0o755)
    print(f"Verified baseline {NAME}: {hashlib.sha256(data).hexdigest()}")


if __name__ == "__main__":
    main()
