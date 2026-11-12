#!/usr/bin/env python3
# Fetches the ConPTY runtime of Windows Terminal (ADR 0014): `conpty.dll` and
# `OpenConsole.exe` (MIT, https://github.com/microsoft/terminal) from the
# NuGet package Microsoft.Windows.Console.ConPTY, checked against its pinned
# size and SHA-512, for the Windows release archive (next to the executable,
# where alacritty_terminal picks them up).
#
#     python3 scripts/fetch-conpty.py --dest <dir>           # writes conpty.dll, OpenConsole.exe
#     python3 scripts/fetch-conpty.py --dest <dir> --package <file.nupkg>   # a package downloaded before (still checked)
#
# Development builds use the system's ConPTY and need none of this. Nothing
# is committed. To move to a newer package: take its version, size and
# SHA-512 from the package's catalog entry on api.nuget.org, and update
# notices/packaging-conpty.md.
#
# Only the x64 files are taken: the only Windows architecture released so far.
# Standard library only. Exit code 1 on a failed download or check.
import argparse
import hashlib
import io
import sys
import urllib.request
import zipfile
from pathlib import Path

VERSION = "1.24.261001001"
URL = (
    "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/"
    f"{VERSION}/microsoft.windows.console.conpty.{VERSION}.nupkg"
)
SIZE = 1733607
SHA512 = (
    "263e7cb79524c99b4893feafa9c97744fb6d3104a3847243acea306c2976ee89"
    "f98d0bdda0775abfe94a5b015263bfad56bb1565fc9442c4f50a94e02bed54f2"
)


def download() -> bytes:
    print(f"Downloading {URL}")
    request = urllib.request.Request(URL, headers={"User-Agent": "pyoxide-fetch-conpty"})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read(SIZE + 1)


def check(data: bytes) -> None:
    if len(data) != SIZE:
        sys.exit(f"ConPTY package: {len(data)} bytes, expected {SIZE}")
    digest = hashlib.sha512(data).hexdigest()
    if digest != SHA512:
        sys.exit(f"ConPTY package: SHA-512 {digest}, expected {SHA512}")


def find(archive: zipfile.ZipFile, name: str) -> str:
    """The member `name` of the x64 runtime: the package keeps one copy per
    architecture, in a folder named after it."""
    hits = []
    for member in archive.namelist():
        parts = member.lower().split("/")
        if parts[-1] == name.lower() and any(p in ("x64", "win-x64") for p in parts[:-1]):
            hits.append(member)
    if len(hits) != 1:
        sys.exit(
            f"ConPTY package: expected one x64 {name}, found {hits or 'none'}; "
            f"members: {archive.namelist()}"
        )
    return hits[0]


def main() -> int:
    parser = argparse.ArgumentParser(description="Fetch conpty.dll and OpenConsole.exe")
    parser.add_argument("--dest", type=Path, required=True)
    parser.add_argument("--package", type=Path, help="use this .nupkg instead of downloading")
    args = parser.parse_args()
    data = args.package.read_bytes() if args.package else download()
    check(data)
    print(f"Microsoft.Windows.Console.ConPTY {VERSION}: SHA-512 verified")
    args.dest.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for name in ("conpty.dll", "OpenConsole.exe"):
            member = find(archive, name)
            (args.dest / name).write_bytes(archive.read(member))
            print(f"{member} -> {args.dest / name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
