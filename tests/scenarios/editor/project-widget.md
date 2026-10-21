# Project widget and recent projects

## Steps
1. Start the IDE with a fresh configuration folder and open three folders
   one after the other: `alpha`, `my-sample-app`, `beta` (the last one
   stays open).
2. Click the project widget (the project's name at the left of the main
   toolbar).
3. Type `alp` and press Enter.
4. Delete the `my-sample-app` folder on disk, quit, start the IDE without
   a project.
5. In Projects and Files, point at the `alpha` row, click its gear; point
   at the `my-sample-app` row, click its cross.

## Expected
- The toolbar shows the project's avatar (its initials on a gradient in
  the project's colour) and a band of that colour fading from the avatar
  along the toolbar.
- The popup opens below the widget: Open…, Clone Repository… (disabled),
  "Open Projects" with `beta`, "Recent Projects" with `my-sample-app` and
  `alpha` (opened last first). Each row shows the avatar, the name and
  the folder, `~` for the home folder. The widget stays highlighted while
  the popup is open; clicking it again closes the popup.
- Typing filters the rows by name or folder; Enter opens `alpha` in the
  window and it goes first in the recent projects.
- Projects and Files lists the recent projects; `my-sample-app` is greyed.
  The pointer over a row shows a gear (a folder that exists) or a cross
  (a folder that is gone). The gear's menu has Open Selected, Copy Path
  and Remove from Recent Projects…; the cross asks "Remove 'my-sample-app'
  from recent projects list?" and Remove takes it off the list.
- The list is kept in `recent-projects.toml` of the configuration folder
  and survives a restart; a project keeps its colour.

## Commands
`project-widget.show`, `project-widget.state`, `project.open-recent
{"path": ...}`, `project.copy-recent-path {"path": ...}`,
`project.remove-recent {"path": ...}`, `workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/opening-reopening-and-closing-projects.html,
https://www.jetbrains.com/help/pycharm/new-ui.html
