# Tool windows and the main window chrome

## Steps
1. Open a project.
2. Toggle the project tool window with its stripe button, then with Cmd+1
   (macOS) or Alt+1 (Windows, Linux).
3. Focus the project tree and press Escape.
4. Hover the toolbar buttons and stripe buttons.
5. Switch between the dark and light theme.
6. Drag the gap between the project tool window and the editor.
7. Use the tool window header buttons (Select Opened File, Expand All,
   Collapse All, Hide) and Shift+Escape in the tree.

## Expected
- The main toolbar sits in the title bar area (macOS window buttons on its
  left, in the toolbar's vertical centre); the status bar is at the bottom
  on the frame background; the project tool window and the editor are
  rounded islands with a small gap between them and the frame.
- Step 2: the tool window hides and shows; the stripe button shows the
  selected state while it is open; the editor island grows to fill the
  space when it is hidden.
- Step 3: Escape moves focus back to the editor.
- Step 4: buttons get a rounded hover background and a tooltip with the
  action name and shortcut after a short delay.
- Step 5: every colour of the chrome and the editor changes at once.
- Step 6: the pointer is a left-right resize pointer over the gap; the tool
  window follows the mouse, no narrower than its minimum and leaving room
  for the editor.
- Step 7: each button runs its action; Hide and Shift+Escape hide the tool
  window and move the focus to the editor.

## Commands
`workbench.toggle-project`, `workbench.focus-editor`,
`workbench.hide-tool-window`, `project.tree.locate`,
`project.tree.expand-all`, `project.tree.collapse-all`,
`workbench.set-theme {"id": ...}`, `workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/tool-windows.html,
https://www.jetbrains.com/help/pycharm/new-ui.html
