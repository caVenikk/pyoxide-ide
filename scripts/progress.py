#!/usr/bin/env python3
# Feature parity report from the feature matrix in replica/features/ (ADR 0020).
#
# One CSV file per feature: replica/features/<area>/<name>.csv, a header line
# (feature,area,priority,original,clone,notes) and one row. The report is not
# committed: CI shows it in the job summary and publishes it from main.
#
#     python3 scripts/progress.py               # print the report (Markdown)
#     python3 scripts/progress.py --check-data  # validate the feature files (CI)
#     python3 scripts/progress.py --csv         # print the whole matrix as one CSV
#     python3 scripts/progress.py --site DIR    # write the published page and badge
#
# Scoring (same as skills/replica-diff/parity.py): weights must 3, should 2,
# could 1; yes 1, partial 0.5, no 0. Rows with original=no are own features and
# are not part of the parity score; clone=skip rows are excluded.
import argparse
import csv
import html
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FEATURES = ROOT / "replica" / "features"
COLUMNS = ["feature", "area", "priority", "original", "clone", "notes"]
W = {"must": 3, "should": 2, "could": 1}
C = {"yes": 1.0, "partial": 0.5, "no": 0.0}
CLONE = [*C, "skip"]
NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
MARK = {"yes": "✅", "partial": "🟡", "no": "⬜", "skip": "➖"}
LEGEND = ("✅ done · 🟡 partial · ⬜ not started · ➖ left out. Features marked (own) are"
          " additions beyond the reference and are not part of the parity score.")
LAYOUT = "expected replica/features/<area>/<name>.csv (lowercase letters, digits and dashes)"


def rel(path):
    return path.relative_to(ROOT).as_posix()


def read(path):
    """The row of one feature file, or None, and what is wrong with the file."""
    try:
        with open(path, newline="", encoding="utf-8-sig") as fh:
            lines = [line for line in csv.reader(fh) if any(c.strip() for c in line)]
    except (OSError, UnicodeDecodeError, csv.Error) as exc:
        return None, [f"{rel(path)}: {exc}"]
    if not lines or lines[0] != COLUMNS:
        return None, [f"{rel(path)}: the first line must be {','.join(COLUMNS)}"]
    if len(lines) != 2:
        return None, [f"{rel(path)}: one feature per file, found {len(lines) - 1} rows"]
    if len(lines[1]) != len(COLUMNS):
        return None, [f"{rel(path)}: {len(lines[1])} fields, expected {len(COLUMNS)}"]
    row = dict(zip(COLUMNS, (v.strip() for v in lines[1])))
    row["file"] = rel(path)
    bad = []
    if not row["feature"]:
        bad.append("feature is empty")
    if row["area"] != path.parent.name:
        bad.append(f"area '{row['area']}' differs from the folder '{path.parent.name}'")
    if row["priority"] not in W:
        bad.append(f"priority '{row['priority']}' is not {' / '.join(W)}")
    if row["original"] not in ("yes", "no"):
        bad.append(f"original '{row['original']}' is not yes / no")
    if row["clone"] not in CLONE:
        bad.append(f"clone '{row['clone']}' is not {' / '.join(CLONE)}")
    return (None if bad else row), [f"{rel(path)}: {b}" for b in bad]


def load():
    """All features, sorted by area and name, and the problems of replica/features/."""
    if not FEATURES.is_dir():
        return [], [f"{rel(FEATURES)}/ does not exist"]
    rows, problems = [], []
    for path in sorted(FEATURES.rglob("*")):
        parts = path.relative_to(FEATURES).parts
        if any(p.startswith(".") for p in parts):
            continue  # .DS_Store and the like, git-ignored
        if path.is_dir():
            if len(parts) != 1 or not NAME.fullmatch(path.name):
                problems.append(f"{rel(path)}: {LAYOUT}")
            continue
        if len(parts) != 2 or path.suffix != ".csv" or not NAME.fullmatch(path.stem):
            problems.append(f"{rel(path)}: {LAYOUT}")
            continue
        row, bad = read(path)
        problems += bad
        if row:
            rows.append(row)
    first = {}
    for r in rows:
        key = r["feature"].casefold()
        if key in first:
            problems.append(f"{r['file']}: the feature '{r['feature']}' is also in {first[key]}")
        first.setdefault(key, r["file"])
    if not rows and not problems:
        problems.append(f"{rel(FEATURES)}/ holds no features")
    rows.sort(key=lambda r: (r["area"], r["feature"].casefold()))
    return rows, problems


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


def by_area(rows):
    areas = {}
    for r in rows:
        areas.setdefault(r["area"], []).append(r)
    return areas


def counts(rs):
    own = [r for r in rs if r["original"] == "yes"]
    return [sum(1 for r in own if r["clone"] == s) for s in ("yes", "partial", "no")]


def summary(rows):
    out = [f"**Overall parity: {score(rows):.0f}%** `{bar(score(rows))}`", "",
           "| Area | Parity | Done | Partial | Todo |", "|---|---|---|---|---|"]
    for a, rs in by_area(rows).items():
        done, partial, todo = counts(rs)
        out.append(f"| {a} | {score(rs):.0f}% `{bar(score(rs), 10)}` | {done} | {partial} | {todo} |")
    return "\n".join(out)


def detail(rows):
    out = []
    for a, rs in by_area(rows).items():
        out += ["", f"## {a}", "", "| | Feature | Priority | Notes |", "|---|---|---|---|"]
        for r in rs:
            tag = " (own)" if r["original"] != "yes" else ""
            out.append(f"| {MARK[r['clone']]} | {r['feature']}{tag} | {r['priority']} | {r['notes']} |")
    return "\n".join(out)


def report(rows):
    return ("# Progress\n\n_Generated from `replica/features/` by `scripts/progress.py`._\n\n"
            + summary(rows) + "\n\n" + LEGEND + "\n" + detail(rows) + "\n")


def source():
    """Where the data comes from: the commit on GitHub when run in Actions."""
    server, repo, sha = (os.environ.get(k) for k in ("GITHUB_SERVER_URL", "GITHUB_REPOSITORY", "GITHUB_SHA"))
    if server and repo and sha:
        url = html.escape(f"{server}/{repo}/tree/{sha}/replica/features", quote=True)
        return f'<a href="{url}"><code>replica/features/</code></a> at <code>{html.escape(sha[:9])}</code>'
    return "<code>replica/features/</code>"


def page(rows):
    e = html.escape

    def table(head, body):
        th = "".join(f"<th>{h}</th>" for h in head)
        return f"<table><thead><tr>{th}</tr></thead><tbody>\n" + "\n".join(body) + "\n</tbody></table>"

    areas = by_area(rows)
    lines = []
    for a, rs in areas.items():
        done, partial, todo = counts(rs)
        lines.append(f'<tr><td><a href="#{e(a)}">{e(a)}</a></td><td>{score(rs):.0f}% <code>{bar(score(rs), 10)}</code></td>'
                     f'<td class="n">{done}</td><td class="n">{partial}</td><td class="n">{todo}</td></tr>')
    body = [f"<p><strong>Overall parity: {score(rows):.0f}%</strong> <code>{bar(score(rows))}</code></p>",
            table(["Area", "Parity", "Done", "Partial", "Todo"], lines), f"<p>{e(LEGEND)}</p>"]
    for a, rs in areas.items():
        items = [f"<tr><td>{MARK[r['clone']]}</td><td>{e(r['feature'])}{' (own)' if r['original'] != 'yes' else ''}</td>"
                 f"<td>{e(r['priority'])}</td><td>{e(r['notes'])}</td></tr>" for r in rs]
        body += [f'<h2 id="{e(a)}">{e(a)}</h2>', table(["", "Feature", "Priority", "Notes"], items)]
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pyoxide IDE progress</title>
<style>
:root {{ color-scheme: light dark; font-family: system-ui, sans-serif; }}
body {{ max-width: 64rem; margin: 2rem auto; padding: 0 1rem; line-height: 1.45; }}
table {{ border-collapse: collapse; width: 100%; margin: 0.5rem 0 1.5rem; }}
th, td {{ text-align: left; vertical-align: top; padding: 0.3rem 0.6rem; border-bottom: 1px solid #8885; }}
td.n {{ text-align: right; }}
code {{ font-family: ui-monospace, monospace; }}
</style>
</head>
<body>
<h1>Pyoxide IDE: feature parity</h1>
<p>Generated from {source()} by <code>scripts/progress.py</code>.</p>
{chr(10).join(body)}
</body>
</html>
"""


def badge(rows):
    """A shields.io endpoint badge: https://shields.io/badges/endpoint-badge."""
    p = score(rows)
    color = next(c for t, c in ((90, "brightgreen"), (75, "green"), (60, "yellowgreen"),
                                (40, "yellow"), (20, "orange"), (0, "red")) if p >= t)
    return json.dumps({"schemaVersion": 1, "label": "parity", "message": f"{p:.0f}%", "color": color}) + "\n"


def main():
    ap = argparse.ArgumentParser(description="Feature parity report from replica/features/.")
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--check-data", action="store_true", help="validate the feature files and exit")
    mode.add_argument("--csv", action="store_true", help="print the whole matrix as one CSV")
    mode.add_argument("--site", metavar="DIR", type=Path, help="write index.html and parity.json to DIR")
    args = ap.parse_args()
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    rows, problems = load()
    if problems:
        print("\n".join(problems), file=sys.stderr)
        print(f"{len(problems)} problem(s) in {rel(FEATURES)}/ (one file per feature, ADR 0020)",
              file=sys.stderr)
        sys.exit(1)
    if args.check_data:
        print(f"{rel(FEATURES)}/: {len(rows)} features in {len(by_area(rows))} areas, ok")
    elif args.csv:
        w = csv.DictWriter(sys.stdout, fieldnames=COLUMNS, extrasaction="ignore", lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    elif args.site:
        args.site.mkdir(parents=True, exist_ok=True)
        (args.site / "index.html").write_text(page(rows), encoding="utf-8")
        (args.site / "parity.json").write_text(badge(rows), encoding="utf-8")
    else:
        sys.stdout.write(report(rows))


main()
