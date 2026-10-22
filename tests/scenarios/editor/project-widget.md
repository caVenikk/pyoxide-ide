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
5. In Projects and Files, point at the `alpha` row, click its "more"
   button (three dots); point at the `my-sample-app` row, click its cross;
   click the `my-sample-app` row itself.
6. Without a project, press Command+1 (Alt+1), then Down, Down, Up, Enter;
   then type `my` in the search field, press Up, Down, Option+Delete
   (Alt+Delete), and Escape twice.

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
- Projects and Files lists the recent projects, 10 below the line under
  the search field; `my-sample-app` is greyed, its avatar grey. A project
  in a Git work tree (at its folder or above) shows the branch below its
  folder, after the branch icon. The pointer over a row is a hand and
  shows a "more" button (a folder that exists) or a cross (a folder that
  is gone) at the row's right end. The "more" menu has Open Selected,
  Reveal in Finder (Show in Explorer), Copy Path (with the Copy Paths
  shortcut), New Project Group, Move to Group, Set Custom Project Icon…
  (greyed: not here yet) and Remove from Recent Projects… (with the Delete
  key); the button keeps its hover background while the menu is open. The
  cross asks "Remove 'my-sample-app' from recent projects list?" and
  Remove takes it off the list. A click on a gone folder's row asks
  "Reopen Project" ("The path … does not exist. If it is on a removable or
  network drive, please make sure that the drive is connected."): Cancel,
  or Remove From List (the default) takes it off the list; from the
  project widget's popup the same question has Remove From List and OK.
- The popup shows the branches too, as they were last looked at.
- The first recent project is selected when the list shows (its row is
  highlighted with its button); the pointer selects the row it is over.
  Up and Down move the selection from the list and from the search field,
  without wrapping; Home and End go to the first and the last row; Enter
  opens the selected project. Delete (Backspace, Command+Backspace) in the
  list and Option+Delete (Alt+Delete) in the search field ask "Remove '…'
  from recent projects list?"; Command+Shift+C (Ctrl+Shift+C) copies the
  folder. Typing keeps the selected row if it still matches, else selects
  the first match. Escape clears the search text, then moves to the list.
  The selection scrolls into view.
- The list is kept in `recent-projects.toml` of the configuration folder
  and survives a restart; a project keeps its colour.

## Commands
`project-widget.show`, `project-widget.state`, `project.open-recent
{"path": ...}`, `project.copy-recent-path {"path": ...}`,
`project.remove-recent {"path": ...}`, `project.reveal-recent {"path": ...}`,
`project.open-recent {"path": ..., "welcome": true}`, `welcome-projects.up`, `.down`,
`.first`, `.last`, `.open`, `.copy-path`, `.remove`, `.escape`,
`workbench.state` (`projects_and_files.selected`).

## Source
https://www.jetbrains.com/help/pycharm/opening-reopening-and-closing-projects.html,
https://www.jetbrains.com/help/pycharm/new-ui.html
