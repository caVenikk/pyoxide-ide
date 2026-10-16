# Main toolbar and status bar in a narrow window

## Steps
1. Open a project located in a deep folder and open a file of it, so the
   main toolbar shows the project name, the file's path in the middle and
   the run widget with "Current File".
2. Make the window narrower step by step, down to its smallest width.

## Expected
Main toolbar (continuous while resizing, no jumps, nothing overlaps):
- While there is room on both sides the file widget is centred in the
  toolbar; otherwise it keeps two gaps (20 px) from the groups beside it.
- First the texts give way: the widest widget shrinks until it is as narrow
  as the next one, then both shrink together. The project name and the
  file path lose characters from the end and get "..." (at least 4
  characters stay: "skin...", "2026..."); the run configuration is cut in
  the middle with "…" (at least 5 characters stay: "Curre…").
- With every text at its minimum the widgets keep their places and the
  right part of the toolbar slides under the window's right edge pixel by
  pixel; a widget can be cut in the middle.

Status bar:
- Nothing shrinks. The breadcrumbs are cut where the widgets on the right
  begin, pixel by pixel.
- With the breadcrumbs gone, the widgets keep their width and slide past
  the window's left edge, cut there, possibly in the middle of a widget.

## Commands
`project.open-folder {"path": ...}`, `project.open-file {"path": ...}`,
`workbench.state`.

## Source
observed (PyCharm 2026.2.3, four window widths); IntelliJ Platform
Community at `pycharm/2026.2.3`: `MainToolbar.kt`, `HorizontalLayout.kt`,
`CompressingLayoutStrategy.kt`, `DefaultCutStrategy.java`,
`AbstractToolbarComboUI.kt`, `RedesignedRunWidget.kt`, `IdeStatusBarImpl.kt`.
