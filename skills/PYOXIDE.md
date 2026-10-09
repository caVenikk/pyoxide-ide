# Pyoxide adaptation of the Replica skill pack

The upstream pack targets web/mobile SaaS clones. In this repository it drives
the whole development of **Pyoxide IDE**, a Rust desktop IDE for Python whose
reference ("the original") is **PyCharm**. Each `SKILL.md` has a
"Pyoxide override" block right under its title; where the override and the
upstream text disagree, **the override wins**. Everything else upstream stays
as is (rules, formats, scripts).

## Vocabulary map
| upstream | here |
|---|---|
| backend | engine services: LSP/DAP clients, DB drivers (`db-*`), VCS, Docker, keychain (no auth/payments) |
| deploy | release: signed builds for Windows/macOS/Linux, GitHub Release (no hosting/domains) |
| landing, pricing, store listing | README, docs site, release notes (no pricing, no stores) |

## Allowed sources (in addition to upstream rules)
1. Public documentation, specifications, protocols (LSP, DAP, Oracle docs, ...).
2. Open-source code under compatible licenses, e.g. PyCharm/IntelliJ Community
   (Apache-2.0): reading for understanding is fine; copying or closely adapting
   code requires a `NOTICE` entry (source, commit, files, license, changes).
3. Observed behaviour of a maintainer's own licensed copy of PyCharm: screenshots and
   scenario recordings (input -> output) written to `replica/screens/` and
   `tests/scenarios/`. Reference screenshots are copyrighted material:
   they stay local (git-ignored), are never published, never shipped.
4. Exported settings from a maintainer's own copy (keymaps, colour schemes `.icls`, UI themes
   `.theme.json`, code styles, live templates) as an import format. Plugin jars
   may be opened only to extract such data files, never `.class` files.

## Forbidden (hard)
Decompiling, disassembling or reading bytecode/binaries/jars of PyCharm or any
Pro plugin, in any form; proprietary assets, icons, strings, names; pasting any
of that into prompts, issues, commits or docs. If unsure, stop and ask.

## Pipeline (one loop per feature area)
recon -> architect (once, then per area) -> design -> build -> backend (services)
-> test -> diff -> entrepreneur -> brand (sweep) -> launch -> deploy (release).
Feature areas: db, editor, python-intel, run-debug, vcs, db-gui, docker,
settings-import, packaging. Artifacts live in `replica/`.
