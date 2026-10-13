# Language Services status widget

## Steps
1. Open a project where no language service is enabled or found; open a
   `.py` file.
2. Enable ruff (and the type server), open `main.py`.
3. Hover the widget; click it.
4. Close all editors; click the widget again.
5. In the popup press the inline Restart Server button of a row.
6. Kill the server process from outside the IDE; open the popup.
7. Press Open Server Error Output, then Restart Server.
8. Click a row (not an inline button).

## Expected
- Step 1: no widget in the status bar; no balloon.
- Step 2: the widget appears right after the caret position widget and
  shows the icons of the services that serve the current file (up to 4;
  with more, 3 icons and `+N`).
- Step 3: tooltip `Language Services running on the current file:` with
  one line per service (`- Ruff 0.15.20`); the popup is titled
  "Language Services", has a "Running on Current File" section with rows
  such as `ruff 0.15.20` and a "Running on Other Files" section.
- Step 4: the widget shows the `{}` icon; "Running on Current File" has a
  disabled "No Services" row; the rows moved to "Running on Other Files"
  with the inline "Stop and Autorun Server as Needed" button.
- Step 5: balloon "Ruff server restarted"; the row reads
  `ruff 0.15.20 | Initializing…` and then the plain label.
- Step 6: the row reads `ruff 0.15.20 | Terminated` with an error mark on
  its icon and on the status bar icon; no balloon, no automatic restart.
- Step 7: an editor tab "Ruff Server Error Output" with the server's
  standard error; Restart Server starts it again.
- Step 8: settings of the external tools open (Pyoxide: until there is a
  settings window, the project settings file).

## Commands
`code.servers` (name, version, state, documents served),
`code.restart-servers {"server": ...}`, `code.stop-server {"server": ...}`.

## Source
https://www.jetbrains.com/help/pycharm/lsp-tools.html; IntelliJ Platform
Community (Apache-2.0) at `pycharm/2026.2.3`: `LanguageServiceWidget`,
`LspClientWidgetItem`, `LspWidgetInternalServiceImpl`,
`LspClientManagerImpl`, `LangBundle.properties`, `LspBundle.properties`.
