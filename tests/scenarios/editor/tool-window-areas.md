# Tool window areas and stripes

## Steps
1. Open a project. Look at the left and right stripes.
2. Press Cmd+4 (macOS) or Alt+4 (Windows, Linux), then drag the gap above
   the Run tool window up.
3. Show another tool window of the bottom area (a stripe button of the
   lower group of the left stripe, or `workbench.activate-tool-window`),
   then press Cmd+4 again.
4. Show another tool window of the left area (a stripe button of the
   upper group), then press Cmd+1 (Alt+1).
5. With a tool window focused, press Shift+Escape.
6. Click the stripe button of a tool window that is shown.

## Expected
- Step 1: the left stripe shows the left area's tool windows at the top
  (Project, Commit, Structure) and the bottom area's at the bottom
  (the product's, Services, Terminal, Problems, Version Control); the
  right stripe shows Notifications. Run has no button before its first
  use; Debug gets one after the first debug session.
- Step 2: Run opens below the editor across the full width, under the
  project tool window, with the focus in it; its button appears at the top
  of the lower group and shows the selected state.
- Step 3: the other tool window replaces Run in the bottom area at the same
  height; only its stripe button is selected; Cmd+4 brings Run back at
  that height.
- Step 4: the other tool window replaces Project in the left area at the
  same width (as Commit does); Cmd+1 brings Project back.
- Step 5: the focused tool window hides and the focus returns to the
  editor; the other areas stay as they are.
- Step 6: the tool window hides.

## Commands
`workbench.activate-tool-window {"id": ...}`,
`workbench.toggle-tool-window {"id": ...}`, `workbench.toggle-project`,
`workbench.toggle-run`, `workbench.hide-tool-window`, `workbench.state`
(`tool_windows`, `tool_window_areas`).

## Source
https://www.jetbrains.com/help/pycharm/tool-windows.html,
https://www.jetbrains.com/help/pycharm/new-ui.html; IntelliJ Community
`defaultToolWindowLayoutProvider.kt` (new UI default stripes),
`WindowInfoImpl` (default weight 0.33), `HideToolWindowAction`.
