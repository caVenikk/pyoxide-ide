# Terminal emulation

## Steps
1. Print the 16 ANSI colours, the 256-colour palette and a 24-bit colour
   gradient; print bold, italic, underlined and inverse text and a line
   with wide characters (CJK, emoji).
2. Run `vim`, `less` on a long file and `htop`; click inside `htop`;
   quit each.
3. Run `while true; do tput cols; tput lines; sleep 1; done` and resize
   the tool window.
4. Paste two lines (`echo one` and `echo two`) at the zsh or bash prompt.
5. Run `yes | head -n 200000` and type in the editor while it prints.
6. Hold Shift and drag over the `htop` screen.

## Expected
- Step 1: colours come from the theme's terminal keys (text and cell
  backgrounds separately), text styles render, wide characters take two
  cells and the following text stays aligned.
- Step 2: each program uses the alternate screen without a scrollbar;
  after quitting, the previous output and scroll position are back; the
  click reaches `htop` (mouse reporting).
- Step 3: the numbers follow the size of the tool window.
- Step 4: bracketed paste puts both lines at the prompt without running
  them.
- Step 5: the output keeps up, the scrollback is capped, typing in the
  editor stays smooth.
- Step 6: Shift selects text instead of sending the mouse to `htop`.

## Commands
`terminal.type {"text": ...}`, `terminal.paste`, `terminal.state`
(visible lines, cursor, alternate screen, size).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html,
https://invisible-island.net/xterm/ctlseqs/ctlseqs.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal` (Reworked
view), `DefaultColorSchemesManager.xml` (`BLOCK_TERMINAL_*` keys).
