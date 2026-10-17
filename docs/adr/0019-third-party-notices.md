# 0019. Third-party notices: one file per entry

- Status: accepted
- Date: 2026-10-17

## Context
`NOTICE` records everything copied or closely adapted from open-source code:
source URL, commit, files, licence and what changed. It had one entry per
change, 59 entries in about 1250 lines, and every pull request that adapted
code appended its entry to the end of the file. Pull requests are developed in
parallel, so two open pull requests that both add an entry conflict at the end
of the file, and each merge breaks the others.

Constraints:
- `main` is protected and merges are squash merges without bot pushes, so a
  generated file cannot be refreshed by CI after a merge.
- GitHub does not run custom merge drivers and does not honour a
  `merge=union` attribute in `.gitattributes`, so the pull request flow cannot
  merge a shared file automatically.
- A file `NOTICE` and a folder `notice/` cannot exist side by side on the
  case-insensitive file systems of macOS and Windows, which the repository
  must build on.
- Apache-2.0 asks that distributions carry the attribution notices, so a
  complete text must exist for releases.

## Options considered
1. Keep appending to `NOTICE`. Simple to read; conflicts on every merge.
2. One file per entry in a folder, and a complete `NOTICE` generated from them
   and committed with each change. The generated file is still touched by every
   pull request and conflicts exactly as before.
3. The same, with CI regenerating `NOTICE` after the merge. Not possible
   without a push to `main`.
4. A merge driver or `merge=union` for `NOTICE`. Not applied by GitHub.
5. One file per entry in a folder; the root `NOTICE` holds the header, a
   pointer to the folder and the trademarks, which do not change when an entry
   is added; the complete text is assembled by a script when it is needed.

## Decision
Option 5.

- `notices/` holds the text of `NOTICE` as files: `00-header.md` (the header),
  `zz-trademarks.md` (the trademarks) and one file per entry. An entry is a
  line `- <files or component>` followed by indented lines with the source
  URL, commit, files, licence and changes. The assembled order is the file name
  order. The entries that existed when the folder was introduced are named
  `NN-<area>-<name>.md` to keep their order; a new entry is a new file
  `<area>-<name>.md` (lowercase, digits and dashes), which sorts after them.
- `scripts/notice.py` (standard library only):
  - without arguments it writes the root `NOTICE` (header, pointer,
    trademarks);
  - `--check` fails when an entry is malformed (it must start with `- `, have
    indented continuation lines and no blank line, name a licence and contain a
    source URL) or the root `NOTICE` differs from what the script writes;
  - `--out <file>` writes the complete text (header, every entry, trademarks).
- CI runs `python3 scripts/notice.py --check` in the `progress` job.
- The folder is `notices/`, not `notice/`, because of the case-insensitive file
  systems above.
- Release packaging (#94) runs `scripts/notice.py --out` and ships the result as
  the `NOTICE` of every distribution.

## Consequences
- A pull request that adapts code adds one new file and touches no file that
  another pull request touches; two pull requests conflict only if they pick the
  same file name.
- The root `NOTICE` changes only with the header or the trademarks.
  A reader of the repository root sees the pointer and the folder; distributions
  carry the complete text.
- Pull requests opened before this change that append to `NOTICE` move their
  entry into a file in `notices/` when they rebase.
- Comments in the sources say "see NOTICE"; that stays correct, because
  `NOTICE` points to the entries.
- Every entry is checked for a licence and a source URL, which the header of
  `NOTICE` always asked for.
