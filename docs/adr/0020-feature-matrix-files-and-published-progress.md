# 0020. Feature matrix as one file per feature; the progress report is published, not committed

- Status: accepted
- Date: 2026-10-19

## Context
Feature status is kept in a matrix (columns feature, area, priority, original,
clone, notes) from which `scripts/progress.py` computes the parity report. The
matrix was one file, `replica/features.csv` (183 rows), and the report was
committed twice: as `docs/PROGRESS.md` and as a generated block in
`README.md`. A pull request that changed a feature edited the matrix and
regenerated both files, and CI failed when they were out of date. Pull
requests are developed in parallel, so any two that changed a feature
conflicted in the generated files (the overall score and the area table change
with every row), and often in the matrix as well, where new rows were appended
at the end of an area or of the file. Each rebase costs a full CI run.

Constraints:
- `main` is protected by a ruleset: changes land only through pull requests
  as squash merges, with required checks (`check` on three platforms,
  `progress`, `integration`) and no bypass.
- Events caused by the workflow token (`GITHUB_TOKEN`) start no workflows, so
  a pull request a workflow opens with it never gets its required checks and
  cannot be merged. A personal access token or a GitHub App token would start
  them, at the price of a stored credential and a second full CI run (three
  platforms, about 30 minutes) after every merge.
- Git merges changes cleanly only when unchanged lines separate them: two rows
  inserted at the same place, or edits of neighbouring rows, conflict.
- The repository is public, so GitHub Pages is available.

## Options considered
Generated report:
1. Keep committing it. Every two pull requests that change a feature conflict.
2. A workflow on `main` regenerates the files and lands them through a bot
   pull request with auto-merge. With the workflow token the pull request
   never passes the required checks; with a stored token it costs a secret, a
   second CI run and an extra commit on `main` per merge.
3. A workflow pushes the regenerated files to `main` directly. Needs a bypass
   of the ruleset.
4. Do not commit the report. CI validates the data only, shows the report in
   the job summary of every run (pull requests included) and publishes it
   from `main` to GitHub Pages, together with a shields.io endpoint badge; the
   README keeps a static section with the link and the badge.

Matrix:
- A. One file per area, `replica/features/<area>.csv`. Fewer conflicts, but
  two features of one area still conflict when both append a row or edit
  neighbouring rows, and areas such as run-debug or vcs often have several
  pull requests open at once.
- B. One file per feature, `replica/features/<area>/<name>.csv`, a header and
  one row, like the notices of ADR 0019. Two pull requests touch the same file
  only when they change the same feature. The hand-kept row order is lost; the
  report sorts by area and feature name.
- C. One file kept sorted by feature name. Inserts land in different places,
  but neighbouring rows still conflict.

## Decision
Option 4 for the report and B for the matrix.

- `replica/features/<area>/<name>.csv` holds the header
  `feature,area,priority,original,clone,notes` and one row. The folder is the
  area and must equal the `area` column; `<name>` is a short slug of the
  feature name (lowercase letters, digits and dashes). The rows of
  `replica/features.csv` were moved unchanged: 183 features in 13 areas,
  overall parity 53% and every area score the same before and after.
- `scripts/progress.py` (standard library only) reads all the files:
  - without arguments it prints the report (Markdown);
  - `--check-data` validates the folder layout and file names, the header,
    one row of six fields per file, `area` equal to the folder, the values of
    `priority` (must / should / could), `original` (yes / no) and `clone`
    (yes / partial / no / skip), and that no feature name appears twice;
  - `--csv` prints the whole matrix as one CSV, for tools that read a single
    file such as `skills/replica-diff/parity.py`;
  - `--site <dir>` writes `index.html` (the report) and `parity.json` (the
    badge in the shields.io endpoint format).
- The `progress` CI job runs `--check-data` and appends the report to the job
  summary. The workflow `.github/workflows/progress.yml` runs on pushes to
  `main` that touch `replica/features/`, `scripts/progress.py` or the
  workflow, and on demand; it builds the site and deploys it with GitHub's
  Pages actions (`configure-pages`, `upload-pages-artifact`, `deploy-pages`).
  The repository's Pages source is "GitHub Actions".
- `docs/PROGRESS.md` and the generated README block are removed; the README
  shows the badge and links the published page.
- Counts written by hand that change with every feature are not kept in the
  docs: `docs/theme-mapping.md` no longer states the number of theme roles,
  `crates/platform/themes/src/roles.rs` is the list.

## Consequences
- Two pull requests that change or add different features touch different
  files and merge in either order without a rebase; the report cannot
  conflict because it is not in the repository.
- No stored token, no bypass and no second CI run: the deployment is one short
  job without a Rust build.
- The published page follows `main` with the delay of one Pages deployment. A
  pull request shows its effect on the report in the summary of its
  `progress` job.
- The README no longer shows the area table inline. The badge needs Pages and
  shields.io; until Pages is enabled the page does not exist and the badge
  shows an error.
- The report lists areas and features alphabetically instead of in the order
  of the old file.
- A repository has one Pages site: a later documentation site on Pages
  (release work) has to include the progress page; `progress.py --site`
  writes into any folder.
- Branches opened before this change that edit `replica/features.csv` move
  their edits into the feature files when they rebase and no longer
  regenerate `docs/PROGRESS.md` or the README block.
