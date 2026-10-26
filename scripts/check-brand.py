#!/usr/bin/env python3
# The product name lives in one place (docs/rebrand.md): the `brand` crate
# (crates/python/brand) for the product layer, `app-info` (crates/platform/
# app-info) for what the platform learns at startup. This check fails on the
# name anywhere else in the code of crates/:
#
# - "Pyoxide" / "pyoxide" / "PYOXIDE" (display names, slugs, environment
#   variable names, keychain services, folder names) in a string, a name or
#   a non-comment line of a data file;
# - not in comments and doc comments (a sweep of those is part of a rebrand,
#   docs/rebrand.md), not in test code (tests/ folders, tests.rs, everything
#   from the first #[cfg(test)] of a source file, examples/), not in
#   Cargo.toml (package and binary names are a manual step of a rebrand), and
#   not in the allow-list below (each entry says why).
#
#     python3 scripts/check-brand.py
#     python3 scripts/check-brand.py --self-test   # only the checker's own tests
#
# Exit code 1 on a stray occurrence. Standard library only.
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"

NAME = re.compile(r"pyoxide", re.IGNORECASE)
# Rust paths of the product crates (`pyoxide_ide::product()`): a crate name,
# renamed with its package.
CRATE_PATH = re.compile(r"\bpyoxide_(ide|cli)\b")

# The brand module: the one place the name is spelled out.
BRAND_DIR = "crates/python/brand/"

# Allowed occurrences: (path prefix, regex matched against the offending
# line, why). Keep this short: an entry is a name the product does not own.
ALLOW = [
    (
        "crates/database/db-ui/src/store.rs",
        r'"pyoxide"',
        "the fake data source's database name, a fixture that mirrors the development database",
    ),
    (
        "crates/database/db-ui/src/dialog/mod.rs",
        r"pyoxide-local",
        "a data source name in a command example",
    ),
    (
        "crates/database/db-ui/src/explorer.rs",
        r"pyoxide-local",
        "a data source name in a command example",
    ),
]

RUST_SUFFIXES = {".rs"}
DATA_SUFFIXES = {".toml", ".py", ".sh", ".bash", ".zsh", ".fish", ".ps1", ".scm", ".json"}
# Files of the shell integration have no suffix (zshrc, zshenv, ...).
SCRIPT_NAMES = {"zshrc", "zshenv", "zlogin", "zprofile"}
SKIP_DIRS = {"tests", "examples", "benches", "target", "node_modules"}
SKIP_NAMES = {"Cargo.toml", "tests.rs"}


def strip_rust(text: str) -> list[str]:
    """The code of a Rust source with comments blanked out: string and char
    literals kept, `//` and `/* */` comments removed. One entry per line."""
    out = []
    line = []
    i, n = 0, len(text)
    block = 0  # nesting of /* */
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if block:
            if c == "/" and nxt == "*":
                block += 1
                i += 2
            elif c == "*" and nxt == "/":
                block -= 1
                i += 2
            else:
                if c == "\n":
                    out.append("".join(line))
                    line = []
                i += 1
            continue
        if c == "/" and nxt == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if c == "/" and nxt == "*":
            block = 1
            i += 2
            continue
        if c == "r" and (nxt == '"' or nxt == "#") and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            # raw string r"..." or r#"..."#
            j = i + 1
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                end = '"' + "#" * hashes
                k = text.find(end, j + 1)
                k = n if k < 0 else k + len(end)
                chunk = text[i:k]
                parts = chunk.split("\n")
                line.append(parts[0])
                for p in parts[1:]:
                    out.append("".join(line))
                    line = [p]
                i = k
                continue
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            chunk = text[i : j + 1]
            parts = chunk.split("\n")
            line.append(parts[0])
            for p in parts[1:]:
                out.append("".join(line))
                line = [p]
            i = j + 1
            continue
        if c == "'":
            # a char literal ('"', '\'') or a lifetime ('a)
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:])
            if m:
                line.append(m.group(0))
                i += len(m.group(0))
                continue
        if c == "\n":
            out.append("".join(line))
            line = []
        else:
            line.append(c)
        i += 1
    out.append("".join(line))
    return out


def rust_findings(text: str) -> list[tuple[int, str]]:
    """(line number, line) of each occurrence in the non-test code."""
    raw = text.split("\n")
    code = strip_rust(text)
    found = []
    for no, line in enumerate(code, 1):
        raw_line = raw[no - 1] if no - 1 < len(raw) else line
        if re.match(r"\s*#\[cfg\(test\)\]", raw_line):
            break  # the rest of the file is test code
        if NAME.search(CRATE_PATH.sub("", line)):
            found.append((no, raw_line.strip()))
    return found


def data_findings(text: str) -> list[tuple[int, str]]:
    found = []
    for no, line in enumerate(text.split("\n"), 1):
        stripped = line.lstrip()
        if stripped.startswith(("#", ";", "<!--", "//", "REM ")):
            continue  # a comment line
        if NAME.search(line):
            found.append((no, line.strip()))
    return found


def allowed(rel: str, line: str) -> bool:
    if rel.startswith(BRAND_DIR):
        return True
    return any(rel.startswith(p) and re.search(rx, line) for p, rx, _ in ALLOW)


def scan_file(path: Path, rel: str) -> list[tuple[int, str]]:
    try:
        text = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        return []
    if path.suffix in RUST_SUFFIXES:
        found = rust_findings(text)
    elif path.suffix in DATA_SUFFIXES or path.name in SCRIPT_NAMES:
        found = data_findings(text)
    else:
        return []
    return [(no, line) for no, line in found if not allowed(rel, line)]


def scan(root: Path) -> list[str]:
    problems = []
    for path in sorted((root / "crates").rglob("*")):
        if not path.is_file() or path.name in SKIP_NAMES:
            continue
        rel_parts = path.relative_to(root).parts
        if any(p in SKIP_DIRS for p in rel_parts[:-1]):
            continue
        rel = "/".join(rel_parts)
        for no, line in scan_file(path, rel):
            problems.append(f"{rel}:{no}: {line[:140]}")
    return problems


def self_test() -> None:
    def rs(src: str) -> list[int]:
        return [no for no, _ in rust_findings(src)]

    assert rs('let a = "pyoxide";') == [1]
    assert rs('let a = "x"; // pyoxide in a comment') == []
    assert rs("/// the Pyoxide docs\nfn f() {}") == []
    assert rs("/* pyoxide\n pyoxide */ let a = 1;\nlet b = \"PYOXIDE_X\";") == [3]
    assert rs('let url = "https://github.com/x/pyoxide-ide";') == [1]
    assert rs('let s = r#"say "Pyoxide""#;') == [1]
    assert rs("let c = '\"'; let d = \"ok\";") == []
    assert rs("fn f<'a>(x: &'a str) { pyoxide_ide::run(); }") == []
    assert rs('let a = 1;\n#[cfg(test)]\nmod tests { fn f() { "pyoxide"; } }') == []
    assert rs('let a = concat!(env_prefix!(), "_X");') == []
    assert [no for no, _ in data_findings("# Pyoxide keymap\nname = 'pyoxide'\n")] == [2]
    assert allowed("crates/python/brand/src/lib.rs", "x pyoxide")
    assert not allowed("crates/platform/ui/src/lib.rs", '"pyoxide"')
    assert allowed("crates/database/db-ui/src/store.rs", 'vec!["pyoxide".into()]')


def main() -> int:
    self_test()
    if "--self-test" in sys.argv:
        print("check-brand: self-test ok")
        return 0
    problems = scan(ROOT)
    if problems:
        print("check-brand: the product name outside the brand module (docs/rebrand.md):")
        for p in problems:
            print("  " + p)
        print(
            "Take the name from `brand` (product crates) or `app_info` (platform crates), "
            "or add an entry with a reason to ALLOW in scripts/check-brand.py."
        )
        return 1
    print("check-brand: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
