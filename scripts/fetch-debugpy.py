#!/usr/bin/env python3
# Fetches the debugger helper of Pyoxide IDE (ADR 0015): the pure-Python
# wheel of debugpy 1.8.21 from PyPI, checked against its pinned size and
# SHA-256, unpacked unmodified (every file checked against the wheel's
# RECORD) into a helper folder.
#
#     python3 scripts/fetch-debugpy.py                  # into the app cache, once per machine
#     python3 scripts/fetch-debugpy.py --dest <dir>     # release packaging (the bundle's helpers/debugpy)
#     python3 scripts/fetch-debugpy.py --wheel <file>   # a wheel downloaded before (still checked)
#     python3 scripts/fetch-debugpy.py --print-dest     # where the app cache folder is
#
# The app cache folder is `helpers/debugpy-1.8.21` in `PYOXIDE_CACHE_DIR`,
# else in `pyoxide` of the user's cache folder (`~/Library/Caches` on macOS,
# `$XDG_CACHE_HOME` or `~/.cache` on Linux, `%LOCALAPPDATA%` on Windows),
# where the IDE looks after its own bundle (crates/python/py-debug).
#
# The folder is never committed. Licences: debugpy is MIT; its vendored
# pydevd is EPL-2.0 and other vendored parts are listed in
# `debugpy/ThirdPartyNotices.txt`; release packaging ships the folder as it
# is unpacked here, with the notices kept (see NOTICE).
#
# Standard library only. Exit code 1 on a failed download or check.
import argparse
import base64
import csv
import hashlib
import io
import os
import shutil
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

VERSION = "1.8.21"
WHEEL = f"debugpy-{VERSION}-py2.py3-none-any.whl"
URL = (
    "https://files.pythonhosted.org/packages/95/51/"
    "67e7cf11a53e40694f720457d5b3a1cdaaa3d5a9a633e482f225456b93ff/" + WHEEL
)
SHA256 = "b1e37d333663c8851516a47364ef473da127f9caebe4417e6df6f5825a7e9a92"
SIZE = 5352888
DIST_INFO = f"debugpy-{VERSION}.dist-info"
# What makes a folder a helper (crates/python/py-debug/src/helper.rs).
MARKERS = (f"{DIST_INFO}/RECORD", "debugpy/adapter/__main__.py")


def cache_dir() -> Path:
    """The application cache folder, as the IDE computes it."""
    override = os.environ.get("PYOXIDE_CACHE_DIR")
    if override:
        return Path(override)
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA")
        if not base:
            sys.exit("LOCALAPPDATA is not set; pass --dest")
        return Path(base) / "pyoxide"
    home = Path.home()
    if sys.platform == "darwin":
        return home / "Library" / "Caches" / "pyoxide"
    xdg = os.environ.get("XDG_CACHE_HOME")
    base = Path(xdg) if xdg and Path(xdg).is_absolute() else home / ".cache"
    return base / "pyoxide"


def default_dest() -> Path:
    return cache_dir() / "helpers" / f"debugpy-{VERSION}"


def is_helper(folder: Path) -> bool:
    return all((folder / m).is_file() for m in MARKERS)


def download() -> bytes:
    print(f"Downloading {URL}")
    request = urllib.request.Request(URL, headers={"User-Agent": "pyoxide-fetch-debugpy"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read(SIZE + 1)
    return data


def check_wheel(data: bytes) -> None:
    if len(data) != SIZE:
        sys.exit(f"{WHEEL}: {len(data)} bytes, expected {SIZE}")
    digest = hashlib.sha256(data).hexdigest()
    if digest != SHA256:
        sys.exit(f"{WHEEL}: SHA-256 {digest}, expected {SHA256}")


def safe_name(name: str) -> PurePosixPath:
    """A member name as a relative path inside the folder, or exit."""
    path = PurePosixPath(name)
    if (
        not name
        or path.is_absolute()
        or "\\" in name
        or ":" in name
        or any(part in ("", ".", "..") for part in path.parts)
    ):
        sys.exit(f"{WHEEL}: unsafe member name {name!r}")
    return path


def record_hashes(archive: zipfile.ZipFile) -> dict:
    """The wheel's RECORD: member name -> (urlsafe base64 SHA-256, size)."""
    rows = {}
    text = archive.read(f"{DIST_INFO}/RECORD").decode("utf-8")
    for row in csv.reader(io.StringIO(text)):
        if not row:
            continue
        name, digest, size = (row + ["", ""])[:3]
        rows[name] = (digest, size)
    return rows


def unpack(data: bytes, target: Path) -> int:
    """Unpacks the wheel into `target`, checking every file; the count."""
    count = 0
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        record = record_hashes(archive)
        names = {info.filename for info in archive.infolist() if not info.is_dir()}
        missing = sorted(set(record) - names)
        if missing:
            sys.exit(f"{WHEEL}: files of RECORD are missing: {missing[:5]}")
        for info in archive.infolist():
            if info.is_dir():
                continue
            relative = safe_name(info.filename)
            content = archive.read(info)
            digest, size = record.get(info.filename, ("", ""))
            if info.filename != f"{DIST_INFO}/RECORD":
                if not digest.startswith("sha256="):
                    sys.exit(f"{WHEEL}: {info.filename} is not in RECORD")
                actual = base64.urlsafe_b64encode(hashlib.sha256(content).digest())
                if actual.rstrip(b"=").decode() != digest[len("sha256=") :]:
                    sys.exit(f"{WHEEL}: {info.filename} does not match RECORD")
                if size and int(size) != len(content):
                    sys.exit(f"{WHEEL}: {info.filename} has the wrong size")
            out = target.joinpath(*relative.parts)
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(content)
            count += 1
    return count


def main() -> int:
    parser = argparse.ArgumentParser(
        description=f"Fetch debugpy {VERSION} (the debugger helper of Pyoxide IDE)"
    )
    parser.add_argument("--dest", type=Path, help="the helper folder (default: the app cache)")
    parser.add_argument("--wheel", type=Path, help=f"use this {WHEEL} instead of downloading")
    parser.add_argument("--force", action="store_true", help="replace an existing folder")
    parser.add_argument("--print-dest", action="store_true", help="print the default folder")
    args = parser.parse_args()

    if args.print_dest:
        print(default_dest())
        return 0
    dest = (args.dest or default_dest()).resolve()
    if dest.exists() and not args.force:
        if is_helper(dest):
            print(f"debugpy {VERSION} is already in {dest}")
            return 0
        sys.exit(f"{dest} exists and is not a debugpy {VERSION} helper; use --force")

    data = args.wheel.read_bytes() if args.wheel else download()
    check_wheel(data)
    print(f"{WHEEL}: SHA-256 {SHA256} verified")

    dest.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{dest.name}.", dir=dest.parent))
    try:
        count = unpack(data, staging)
        if not is_helper(staging):
            sys.exit(f"{WHEEL}: not a debugpy helper after unpacking")
        if dest.exists():
            shutil.rmtree(dest)
        staging.rename(dest)
    finally:
        if staging.exists():
            shutil.rmtree(staging, ignore_errors=True)
    print(f"Unpacked {count} files into {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
