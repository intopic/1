"""Fetch one pinned test fixture; verify crate and ELF before loading SBF."""
import hashlib
import io
from pathlib import Path
import tarfile
import urllib.request

URL = "https://static.crates.io/crates/litesvm/litesvm-0.18.1.crate"
CRATE_SHA = "b8fe187ac113cebaa49af5dce82dfd1084177e5434d78815f409a6a22676b294"
ELF_SHA = "7b44674b9704fa7640fbebfac68389f338b500b8c2bc29175c27ed66838b5a3c"
MEMBER = "litesvm-0.18.1/src/programs/elf/spl_token-3.5.0.so"


def fetch():
    target = Path(__file__).resolve().parent / "results" / "spl_token-3.5.0.so"
    target.parent.mkdir(exist_ok=True)
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == ELF_SHA:
        return target
    with urllib.request.urlopen(URL, timeout=60) as response:
        payload = response.read(10_000_000)
    assert hashlib.sha256(payload).hexdigest() == CRATE_SHA, "crate digest mismatch"
    with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
        # Read a named file into memory, never extract arbitrary archive paths.
        elf = archive.extractfile(MEMBER).read()
    assert hashlib.sha256(elf).hexdigest() == ELF_SHA, "ELF digest mismatch"
    target.write_bytes(elf)
    return target


if __name__ == "__main__":
    print(fetch())
