# Command blocks in the terminal

## Steps
1. In a shell with the integration, run `true`, then `ls`, then
   `false`, then `echo done`.
2. Scroll up a little and back.
3. Resize the window narrower and wider.
4. Run `clear` (Cmd+K / Ctrl+L).
5. Open `vim` (or `less`) and leave it.

## Expected
- Step 1: a one pixel line separates each command block from the one
  above; the first block has none; the block of `false` has a red marker
  of rounded ends at its left edge from its first text line to its last.
- Step 2: separators and the marker move with their blocks.
- Step 3: after the width changes the blocks are laid out again from the
  next prompt on (blocks of the old width are dropped).
- Step 4: the screen is empty except for the new prompt; no separator
  above it.
- Step 5: the full screen program has no separators; they are back after
  it.

## Commands
`terminal.type {"text": ...}`, `terminal.state` (`blocks`: visible blocks
with their rows, `separated`, `failed`).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `TerminalBlocksDecorator`,
`TerminalUi` (insets and marker constants).
