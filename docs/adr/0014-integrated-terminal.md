# 0014. Integrated terminal

- Status: proposed
- Date: 2026-10-16

## Context
Daily Python work runs `uv`, `git`, test and migration commands in a shell.
The reference IDE has a Terminal tool window (Option+F12 / Alt+F12) with
tabs, the project's virtual environment activated in every new tab, links
to files, find, and shell integration that reports the prompt, the command
and the working directory (#89).

The reference uses pty4j for the pseudo-terminal and JediTerm (Java) as the
emulator, and has two engines: "Classic" and "Reworked 2025", the default
since 2025.2. The reworked engine paints JediTerm's screen into an IntelliJ
editor switched to a character-grid mode and draws command blocks from
its shell integration. Pyoxide clones the reworked engine's look and
behaviour only; there is no engine choice.

Constraints: no JVM, external processes only behind protocols (a shell over
a pseudo-terminal is one), nothing blocks the UI thread, Windows, macOS and
Linux, platform crates know no language (ADR 0005). Full-screen programs
(vim, htop, less) need exact cells, the alternate screen, mouse reporting
and bracketed paste.

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
plugins/terminal (TerminalOptionsProvider, LocalOptionsConfigurer,
LocalTerminalStartCommandBuilder, TerminalEventDispatcher, the shell
integration scripts), python/terminal (PyVirtualEnvTerminalCustomizer),
platform/platform-resources/src/keymaps; the reference's terminal help pages.

## Options considered
| option | licence | verdict |
|---|---|---|
| `alacritty_terminal` 0.26 (Alacritty's core: `Term`, grid and scrollback, alternate screen, modes, SGR 16/256/24-bit, OSC 0/2/4/8/10-12/52, cursor shapes, damage, `tty` with openpty and ConPTY) | Apache-2.0 | **chosen** for emulator and PTY |
| `portable-pty` 0.9 (wezterm's PTY only, blocking read/write) | MIT | fallback if alacritty's `tty` falls short |
| `wezterm-term` | MIT | not published on crates.io; heavy wezterm dependencies |
| `termwiz` 0.23 | MIT | a TUI toolkit, no embeddable terminal with scrollback |
| `vt100` 0.16 | MIT | screen scraping without selection or search; tests at most |
| `libghostty-vt` | MIT | pre-release C API, needs the Zig toolchain on every CI OS, Windows untested |
| pty4j + JediTerm (the reference) | EPL-1.0 / LGPL-3.0 or Apache-2.0 | context only: JVM |
| Zed's `terminal` crates | GPL-3.0 | never read |

For the view: painting the screen with the editor crate (as the reference
paints into its editor) or a grid element of its own.

## Decision
Three layers, one engine.

### PTY
`alacritty_terminal::tty`: openpty on macOS and Linux, ConPTY on Windows.
ConPTY needs Windows 10 1809 (build 17763) or later; the version is checked
before a session is created, because the crate asserts when creation
fails, and an older system gets an error in the tab instead.

ConPTY is the known bottleneck on Windows: the console host re-encodes the
screen, and the ConPTY built into Windows 10 is older and slower than
current releases. Release builds ship `conpty.dll` and `OpenConsole.exe`
from Windows Terminal (MIT, NOTICE entry) next to the executable, where
`alacritty_terminal` picks them up; the reference also bundles a ConPTY of
its own. Development builds use the system's ConPTY. Packaging the two
files is part of the release work (#94).

`tty::setup_env` is never called (it changes the application's
environment); the environment is passed per session.

### Emulator
`alacritty_terminal::Term` with its `vte` parser, pinned `=0.26.0`.
Alacritty's own event loop is not used. Per session, a reader thread reads
the PTY, pre-scans the bytes for the sequences `Term` ignores (OSC 7 working
directory, OSC 133 prompt and command marks, Pyoxide's own integration
events) and feeds them to the parser under the terminal's lock; a writer
path sends input, and resizes and the child's exit are handled there too.
Selection and search use the crate's `Selection` and regex search. The UI
thread takes a snapshot of the visible cells, the cursor, the modes, the
selection and the matches under the lock, at most once per frame, and
paints from it. Types of the crate do not leave the `terminal` crate.
Scrollback: 10 000 lines by default (the reference keeps 1024 KB of output;
the crate counts lines), measured against the idle memory budget.

### Engine
One engine with the look and behaviour of the reference's "Reworked 2025"
terminal:

- **View**: a GPUI grid element in a `terminal-ui` crate, not the editor.
  Per visible row it places runs of equal style at `column × cell width`
  (split at wide and fallback glyphs to keep cells aligned), merges
  background quads, and draws the cursor (block, underline, bar; hollow
  when unfocused), the selection, search matches, the hovered link and
  command separators. Pyoxide's editor has no character-grid mode;
  adding one would put terminal concerns (cell alignment, wide cells,
  alternate screen, mouse reporting) into the most performance-sensitive
  crate for one user, and the snapshot already is a grid. Repaint is
  coalesced to one per frame, as the Run console does.
- **Shell integration**: Pyoxide's own scripts for zsh (`ZDOTDIR`), bash
  (`--rcfile`), fish (`--init-command`) and PowerShell (`-File`), emitting
  standard OSC 7 and OSC 133. The injection mechanism is adapted from the
  Community scripts (NOTICE entry with commit and files if adapted
  closely). Scripts are embedded and written to a versioned cache folder.
  They source the activation script named by the session (below) after the
  user's rc files and before the first prompt, so `pyenv` or `conda init`
  in rc files cannot undo it. The "Shell integration" setting turns them
  off; cmd.exe and unknown shells run without them.
- **Start rules** as the reference: shell from the settings, else
  `powershell.exe` on Windows and `$SHELL`, `/bin/zsh` (macOS), `/bin/bash`,
  `/bin/sh` elsewhere; `--login` for local macOS shells and `-i` for
  bash, zsh and fish; working directory: the requested folder, else the
  "Start directory" setting, else the product's default, else the project
  root, else home. Environment: the parent's, `TERM=xterm-256color` (not on
  Windows), `COLORTERM=truecolor`, a product-name variable for the terminal
  emulator (its value goes through the rebrand, #93), the project's
  variables, the integration's variables.
- **Product hook**: `terminal::ShellCustomizer` gives environment variables,
  a script to source with its arguments, and a default working directory
  for a session. The Python product activates the interpreter of the
  module that owns the working directory: the venv's `activate`,
  `activate.fish`, `Activate.ps1` (uv venvs included); for shells without
  integration the variables of the activation script are merged into the
  process environment. Conda comes later. "Activate virtualenv" is a
  project setting, on by default.
- **Keys**: with "Override IDE shortcuts" on (the default) the terminal's
  key context wins over every binding except the reference's allowed list
  (tool window activation, Search Everywhere, Find in Files, Settings, tab
  and terminal actions); xterm encoding of keys, mouse (SGR), bracketed
  paste and Option as Meta is the `terminal` crate's.

### Crates
| crate | layer | GPUI | role |
|---|---|---|---|
| `terminal` | platform | no | session (PTY, `Term`, reader and writer threads, OSC pre-scan), snapshot types, input encoding, start rules, shell detection, integration scripts, `ShellCustomizer`, `terminal.*` engine commands |
| `terminal-ui` | platform | yes | grid element, session view (input, selection, find, links), Terminal tool window content |
| `pyoxide-ide` (extended) | python | yes | venv and uv activation, the "Activate virtualenv" setting |

`scripts/check-layers.py`: `terminal` knows no toolkit, only `terminal`
depends on `alacritty_terminal`, the workbench only embeds `terminal-ui`.

### Storage
Application settings (shell path, font, toggles) in the application
settings store (ADR 0017); the start directory, the project's variables
and the open tabs (name, user-defined flag, shell, last working directory)
in the personal project state (ADR 0009, `[terminal]`). Imported once from
`.idea/workspace.xml` (start directory, variables) and
`.idea/python-terminal.xml` (Activate virtualenv); `.idea` is never written.

## Consequences
- Two new platform crates and one pinned 0.x dependency; most of its
  dependencies (`parking_lot`, `polling`, `regex-automata`, `rustix`,
  `windows-sys`) are in the lock file already. An upgrade is deliberate.
- PTY tests run on all three CI systems with `process-test-helper`-style
  programs instead of real shells; shell-integration tests that need zsh,
  bash, fish or pwsh are `#[ignore]` and documented.
- A headless throughput benchmark (large output through the PTY into
  `Term`) runs in CI on all three systems, on Windows with the system's
  and the bundled ConPTY.
- The grid view issue has a checkpoint with measurements: throughput
  (`cat` of a large file), keystroke-to-paint latency and scrolling,
  compared with a native terminal and the reference on the same machine;
  the budgets then go into `docs/performance.md`. Flooding output must not
  slow typing in the editor.
- Release builds for Windows carry two more binaries from Windows Terminal
  (MIT).
- Theme keys `BLOCK_TERMINAL_*` (foreground and background per ANSI colour,
  default colours, search entries, separators) are mapped in
  `docs/theme-mapping.md`; the reference's minimum contrast ratio (4.5)
  uses `themes::contrast`.
- Not in the first version: splits, Move to Editor, command completion,
  Paste from History, conda activation, SSH and WSL sessions.
