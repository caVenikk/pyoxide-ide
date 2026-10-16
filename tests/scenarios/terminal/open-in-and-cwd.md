# Open In > Terminal, start directory and restored tabs

## Steps
1. In the project tree right-click a sub-folder and choose Open In >
   Terminal; do the same on a file in another folder.
2. Set Settings > Tools > Terminal > Start directory to a sub-folder;
   open a new tab.
3. In a tab run `cd src`; rename another tab to `server`.
4. Close and reopen the project; open the terminal.
5. Drag a folder from the project tree onto the tab strip, then onto the
   output.

## Expected
- Step 1: a new tab opens in that folder; for a file, in its folder.
- Step 2: new tabs start in the configured folder; an empty field means
  the default (shown greyed: the project root).
- Step 3: the tracked working directory follows `cd` (shell integration
  on).
- Step 4: the tabs come back with their names and their last working
  directories, the first tab selected, without the focus.
- Step 5: on the tab strip a new tab opens in that folder; on the output
  the folder's path is pasted at the prompt.

## Commands
`terminal.open-in {"path": ...}`, `terminal.new-tab`,
`terminal.rename-tab {"name": ...}`, `terminal.state` (tabs, working
directories).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html,
https://www.jetbrains.com/help/pycharm/settings-tools-terminal.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal`
(persistent tabs, Open In action, start directory),
`TerminalProjectNonSharedOptionsProvider`.
