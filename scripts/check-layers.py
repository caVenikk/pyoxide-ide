#!/usr/bin/env python3
# Layer rule of ADR 0005: platform crates never depend on product crates, and
# products do not depend on each other.
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
    if unclassified:
        print("not classified yet:", ", ".join(sorted(unclassified)))
    if bad:
        print("layer violations (ADR 0005):")
        for b in bad:
            print("  " + b)
        sys.exit(1)
    print("layers ok")


main()
