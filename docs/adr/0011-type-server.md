# 0011. Type server for Python

- Status: proposed
- Date: 2026-10-13

## Context
Without its own engine (ADR 0010) Pyoxide needs one language server that
answers completion, documentation, signatures, navigation, references and
type diagnostics for Python, next to ruff for lint, quick fixes and
formatting. The reference integrates ty, Pyright, Basedpyright and Pyrefly
as optional tools and offers Pyrefly (and ty, behind a flag) as its type
engine.

The candidates were measured on 2026-10-13 with a small LSP driver on a
private mid-size web service (about 1,250 files, 78k lines, uv
environment with about 780 third-party modules; Apple M5, 16 GB), three
runs each, medians; completion over 19 typing positions per run (member
access, third-party objects, a broad two-letter prefix, auto-import).

| | ty 0.0.86 | basedpyright 1.40.2 | pyright 1.1.414 | pyrefly 1.3.2 |
|---|---|---|---|---|
| First diagnostics, 1.2k-line file (ms) | 240 | 960 | 887 | 222 |
| Diagnostics after an edit (ms) | 42 | 192 | 219 | 44 |
| Completion p50 / p95 (ms) | 12 / 138 | 64 / 398 | 36 / 78 | 36 / 49 |
| Steady memory, open files (MB) | 316 | 969 | 1020 | 574 |
| Whole project via LSP | 0.6 s | 5.8 s | 5.0 s | 0.6 s |
| Runtime | native binary | Node (bundled on PyPI) | Node | native binary |
| Position encodings | utf-8, utf-16, utf-32 | utf-16 | utf-16 | utf-16 |
| Pull diagnostics | yes | yes | yes | push only |
| Auto-import completion | yes | yes | partial | yes |
| Semantic tokens, inlay hints | yes, yes | yes, yes | no, no | yes, yes |
| Status | beta, 0.0.x, about two releases a week | stable | stable | stable 1.x |
| License | MIT | MIT | MIT | MIT |

Quality notes: all four put the expected item first after prefix
filtering; ty and pyrefly return nothing for a member access inside an
unclosed call (the editor closes brackets, as the reference does, which
avoids it); basedpyright's default rule set reports about 20k problems on
that project and needs `typeCheckingMode = "standard"`; pyrefly shows no
type errors in a project without its own configuration. ty's broad
two-letter completion takes about 0.5 s the first time (upstream
ty#3909).

## Options considered
1. ty: fastest, smallest, full feature set for the first version, same
   vendor and configuration conventions as ruff, a single binary per OS.
   Beta.
2. basedpyright: most mature inference and auto-import ranking; Node,
   about 1 GB per window, slower first diagnostics.
3. pyrefly: fast and stable, rich refactoring actions; utf-16 and push
   only, quiet in unconfigured projects.
4. pyright: no semantic tokens, inlay hints or import quick fixes.

## Decision
ty is the default type server (`[python.type-engine] name = "ty"`). The
LSP client stays generic, so basedpyright and pyrefly can be offered as
alternatives later through server specs, without changes to the platform
crates.

- A minimum ty version is checked at start (`ty --version`); an older one
  shows in the Language Services widget. An ignored integration test runs
  against the latest ty.
- Interpreter: the project interpreter goes to ty as
  `environment.python`; a change of interpreter restarts the type server.
- Diagnostics only for open files; a whole-project check is a later
  explicit action.

## Consequences
- Pyoxide follows ty's releases closely; behaviour changes in 0.0.x show
  up in the integration test.
- Completion asks again as the prefix grows (ty marks results
  incomplete); broad prefixes are cancelled when the user keeps typing.
- Memory of the server process (about 0.3 GB on the measured project) is
  reported next to the application's in `replica/perf.md`.
