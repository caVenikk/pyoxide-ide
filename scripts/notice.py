#!/usr/bin/env python3
# Assemble NOTICE from the files in notices/ (ADR 0019).
#
#     python3 scripts/notice.py             # rewrite the root NOTICE
#     python3 scripts/notice.py --check     # exit 1 if a file in notices/ is malformed
#                                           # or the root NOTICE is out of date (CI)
#     python3 scripts/notice.py --out FILE  # write the complete text, with every entry,
#                                           # for distributions
#
# notices/00-header.md is the header, notices/zz-trademarks.md the trademarks,
# every other notices/*.md is one entry, in file name order. The root NOTICE
# holds the header, a pointer to notices/ and the trademarks only, so adding
# an entry never changes it. Standard library only.
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIR = ROOT / "notices"
NOTICE = ROOT / "NOTICE"
HEADER = "00-header.md"
TRADEMARKS = "zz-trademarks.md"

# Lowercase, so that the file name order is the same on every platform.
NAME = re.compile(r"[a-z0-9][a-z0-9-]*\.md")
# Add a name here when an entry needs a licence that is not listed.
LICENCE = re.compile(
    r"\b(Apache-2\.0|MIT|BSD-[23]-Clause|ISC|MPL-2\.0|CC0-1\.0|Unlicense|Zlib"
    r"|SIL Open Font License)\b"
)
URL = re.compile(r"https?://[^\s)]+")

POINTER = (
    "The full list of third-party notices is in the notices/ folder, one file per\n"
    "entry. `python3 scripts/notice.py --out <file>` assembles the complete text\n"
    "(this header, every entry, the trademarks) for distributions."
)


def read(path):
    return path.read_bytes().decode("utf-8").replace("\r\n", "\n").strip("\n")


def check_entry(text):
    problems = []
    lines = text.split("\n")
    if not lines[0].startswith("- ") or not lines[0][2:].strip():
        problems.append("the first line must be '- ' and the files or the component")
    if len(lines) < 2:
        problems.append("the source, licence and changes must follow on indented lines")
    if "" in lines:
        problems.append("blank line: one file is one entry")
    elif any(not line.startswith("  ") for line in lines[1:]):
        problems.append("lines after the first must be indented by two spaces")
    if not LICENCE.search(text):
        problems.append("no licence named (for example Apache-2.0)")
    if not URL.search(text):
        problems.append("no source URL")
    return problems


def collect():
    """Return (header, entries, trademarks, problems)."""
    header = trademarks = None
    entries = []
    problems = []
    for path in sorted(DIR.iterdir(), key=lambda p: p.name):
        name = path.name
        if name.startswith("."):
            continue
        if not path.is_file() or not NAME.fullmatch(name):
            problems.append(
                f"notices/{name}: unexpected file; names are lowercase letters, "
                "digits and dashes, with the .md extension"
            )
            continue
        text = read(path)
        if name == HEADER:
            header = text
        elif name == TRADEMARKS:
            trademarks = text
        else:
            problems += [f"notices/{name}: {p}" for p in check_entry(text)]
            entries.append(text)
    for name, found in ((HEADER, header), (TRADEMARKS, trademarks)):
        if not found:
            problems.append(f"notices/{name}: missing or empty")
    return header, entries, trademarks, problems


def assemble(parts):
    return "\n\n".join(parts) + "\n"


def main():
    ap = argparse.ArgumentParser(description="Assemble NOTICE from notices/.")
    group = ap.add_mutually_exclusive_group()
    group.add_argument(
        "--check", action="store_true", help="exit 1 if notices/ or NOTICE is not right"
    )
    group.add_argument("--out", metavar="FILE", help="write the complete text to FILE")
    args = ap.parse_args()

    header, entries, trademarks, problems = collect()
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1

    if args.out:
        out = Path(args.out)
        if out.resolve() == NOTICE.resolve():
            print("--out must not be the root NOTICE (it holds the short form)", file=sys.stderr)
            return 1
        out.write_bytes(assemble([header, *entries, trademarks]).encode("utf-8"))
        print(f"{out}: {len(entries)} entries")
        return 0

    want = assemble([header, POINTER, trademarks])
    have = NOTICE.read_bytes().decode("utf-8").replace("\r\n", "\n") if NOTICE.exists() else None
    if args.check:
        if have != want:
            print("NOTICE is out of date: run python3 scripts/notice.py", file=sys.stderr)
            return 1
        print(f"notices/: {len(entries)} entries well-formed, NOTICE up to date")
        return 0
    if have == want:
        print("NOTICE is up to date")
    else:
        NOTICE.write_bytes(want.encode("utf-8"))
        print("NOTICE updated")
    return 0


if __name__ == "__main__":
    sys.exit(main())
