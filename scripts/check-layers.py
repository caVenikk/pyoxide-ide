#!/usr/bin/env python3
# Layer rules of the workspace, checked from `cargo metadata`:
#
# - ADR 0005: platform crates never depend on product crates, and products
#   do not depend on each other.
# - ADR 0010 (language services): the model and the client know no UI
#   toolkit; protocol types stay inside the client; platform UI crates
#   depend on the code intelligence traits, never on the client; the
#   editor has diagnostic marks of its own and does not know the model.
#
#     python3 scripts/check-layers.py
#
# A crate under crates/platform/ is a platform crate; a crate under any other
# crates/<product>/ directory belongs to that product. Crates directly under
# crates/ are not classified yet and are only listed. Exit code 1 on a violation.
import json, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"

# ADR 0010.
TOOLKIT = "gpui"
NO_TOOLKIT = ["code-intel", "lsp-client", "py-lsp", "settings"]
CLIENT = "lsp-client"
PROTOCOL_TYPES = "lsp-types"
MODEL = "code-intel"
NOT_MODEL = ["editor"]


def layer(manifest: Path):
    rel = manifest.parent.relative_to(CRATES)
    if len(rel.parts) < 2:
        return None  # crates/<name>: not classified yet
    return rel.parts[0]  # "platform" or a product name


def main():
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT, capture_output=True, text=True, check=True).stdout)
    by_name = {}
    for p in meta["packages"]:
        by_name[p["name"]] = layer(Path(p["manifest_path"]))
    bad, unclassified = [], []
    for p in meta["packages"]:
        own = by_name[p["name"]]
        if own is None:
            unclassified.append(p["name"])
            continue
        for d in p["dependencies"]:
            dep = by_name.get(d["name"])
            if dep is None or dep == own:
                continue
            if own == "platform" and dep != "platform":
                bad.append(f"{p['name']} (platform) depends on {d['name']} ({dep})")
            elif own != "platform" and dep != "platform":
                bad.append(f"{p['name']} ({own}) depends on {d['name']} (another product: {dep})")

    # What each workspace crate uses to build (dev-dependencies left out):
    # its direct dependencies, and every crate it reaches in the workspace.
    direct = {
        p["name"]: {d["name"] for d in p["dependencies"] if d.get("kind") != "dev"}
        for p in meta["packages"]
    }

    def reach(name):
        seen, stack = set(), [name]
        while stack:
            for d in direct.get(stack.pop(), ()):
                if d in direct and d not in seen:
                    seen.add(d)
                    stack.append(d)
        return seen

    def uses(name, dependency):
        """`name` depends on `dependency`, directly or through workspace crates."""
        crates = {name} | reach(name)
        return dependency in crates or any(dependency in direct[c] for c in crates)

    for name in NO_TOOLKIT:
        if name in direct and uses(name, TOOLKIT):
            bad.append(f"{name} uses the UI toolkit ({TOOLKIT}) (ADR 0010)")
    for name in sorted(direct):
        if name != CLIENT and PROTOCOL_TYPES in direct[name]:
            bad.append(f"{name} depends on {PROTOCOL_TYPES}: protocol types stay in {CLIENT} (ADR 0010)")
        if by_name.get(name) == "platform" and uses(name, TOOLKIT) and uses(name, CLIENT):
            bad.append(f"{name} is a UI crate and depends on {CLIENT}: UI crates use {MODEL} (ADR 0010)")
    for name in NOT_MODEL:
        if name in direct and uses(name, MODEL):
            bad.append(f"{name} depends on {MODEL}: it has diagnostic marks of its own (ADR 0010)")

    if unclassified:
        print("not classified yet:", ", ".join(sorted(unclassified)))
    if bad:
        print("layer violations (ADR 0005, 0010):")
        for b in bad:
            print("  " + b)
        sys.exit(1)
    print("layers ok")


main()
