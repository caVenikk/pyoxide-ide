# New Predefined Session popup

## Steps
1. Click the chevron next to "+" in the Terminal header.
2. Choose another shell than the default one.
3. On Windows open the popup again.
4. Choose "Settings".

## Expected
- Step 1: on macOS and Linux the popup lists the shells found (bash,
  zsh, fish, pwsh in the usual system and Homebrew folders), each by name,
  or `name (path)` when two share a name; a separator; "Settings".
- Step 2: a new tab named after the shell (for example "zsh") starts that
  shell.
- Step 3: Windows PowerShell, Command Prompt, PowerShell, Git Bash and
  the WSL distributions found; when there are several kinds, titled
  separators group them.
- Step 4: Settings opens on Tools > Terminal.

## Commands
`terminal.new-session-popup`, `terminal.new-tab {"shell": ...}`,
`terminal.state` (tabs, titles), `settings.state`.

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal` (shell
detection, New Predefined Session action). The popup look on each OS is to
verify against a reference screenshot.
