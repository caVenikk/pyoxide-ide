#!/usr/bin/env python3
"""Generate docs/PROGRESS.md and the README progress block from replica/features.csv.

    python3 scripts/progress.py           # rewrite the generated files
    python3 scripts/progress.py --check   # exit 1 if they are out of date (CI)

Scoring (same as skills/replica-diff/parity.py): weights must 3, should 2,
could 1; yes 1, partial 0.5, no 0. Rows with original=no are own features and
are not part of the parity score; clone=skip rows are excluded.
"""
import csv, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
W = {"must": 3, "should": 2, "could": 1}
C = {"yes": 1.0, "partial": 0.5, "no": 0.0}
START, END = "<!-- progress:start -->", "<!-- progress:end -->"


def load():
    with open(ROOT / "replica/features.csv", newline="", encoding="utf-8") as fh:
        return [{k.strip().lower(): (v or "").strip() for k, v in r.items()} for r in csv.DictReader(fh) if r.get("feature")]


def score(rows):
    num = den = 0.0
    for r in rows:
        if r["original"] != "yes" or r["clone"] == "skip":
            continue
        w = W[r["priority"]]
        num += w * C[r["clone"]]
        den += w
    return 100.0 * num / den if den else 0.0


def bar(p, n=20):
    f = round(p / 100 * n)
    return "█" * f + "░" * (n - f)


def summary(rows):
    areas = {}
    for r in rows:
        areas.setdefault(r["area"], []).append(r)
    out = [f"**Overall parity: {score(rows):.0f}%** `{bar(score(rows))}`", "",
           "| Area | Parity | Done | Partial | Todo |", "|---|---|---|---|---|"]
    for a, rs in areas.items():
        own = [r for r in rs if r["original"] == "yes"]
        n = lambda s: sum(1 for r in own if r["clone"] == s)
        out.append(f"| {a} | {score(rs):.0f}% `{bar(score(rs), 10)}` | {n('yes')} | {n('partial')} | {n('no')} |")
    return "\n".join(out)


def detail(rows):
    mark = {"yes": "✅", "partial": "🟡", "no": "⬜", "skip": "➖"}
    out = []
    for a in dict.fromkeys(r["area"] for r in rows):
        out += ["", f"## {a}", "", "| | Feature | Priority | Notes |", "|---|---|---|---|"]
        for r in rows:
            if r["area"] == a:
                tag = " (own)" if r["original"] != "yes" else ""
                out.append(f"| {mark[r['clone']]} | {r['feature']}{tag} | {r['priority']} | {r['notes']} |")
    return "\n".join(out)


def build():
    rows = load()
    progress = ("# Progress\n\n_Generated from `replica/features.csv` by `scripts/progress.py`; do not edit by hand._\n\n"
                + summary(rows) + "\n\n✅ done · 🟡 partial · ⬜ not started. Features marked (own) are additions beyond the reference and are not part of the parity score."
                + "\n" + detail(rows) + "\n")
    readme = (ROOT / "README.md").read_text()
    block = f"{START}\n{summary(rows)}\n\nPer-feature status: [docs/PROGRESS.md](docs/PROGRESS.md)\n{END}"
    if START in readme:
        a, rest = readme.split(START, 1)
        readme = a + block + rest.split(END, 1)[1]
    else:
        readme = readme.replace("## Workspace", f"## Progress\n\n{block}\n\n## Workspace", 1)
    return progress, readme


def main():
    progress, readme = build()
    files = {ROOT / "docs/PROGRESS.md": progress, ROOT / "README.md": readme}
    if "--check" in sys.argv:
        bad = [str(p.relative_to(ROOT)) for p, t in files.items() if not p.exists() or p.read_text() != t]
        if bad:
            print("out of date, run python3 scripts/progress.py:", ", ".join(bad)); sys.exit(1)
        return
    for p, t in files.items():
        p.write_text(t)


main()
