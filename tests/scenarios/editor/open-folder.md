# Open a folder as a project

## Steps
1. Start the IDE without a project.
2. Open a folder that contains `src/app.py`, `README.md`, a `.git` folder,
   a `.venv` folder and a `__pycache__` folder.
3. Look at the main toolbar and the project tool window.
4. Expand `src`.

## Expected
- The project widget in the main toolbar shows the folder name with a
  coloured badge of its initials.
- The project tool window is open and shows the folder as the root row,
  expanded, with its absolute path in muted text after the name.
- Folders come before files, both sorted by name, case-insensitively.
- `.git` is not shown; `.venv` and `__pycache__` are shown (as excluded or
  ignored folders) but never expanded automatically.
- Expanding `src` shows `app.py` with a Python file icon.
- Opening a folder of 50,000 files shows the tree at once; the UI keeps
  responding while folders load.

## Commands
`project.open-folder {"path": ...}`, `project.tree.expand {"path": ...}`,
`workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/opening-reopening-and-closing-projects.html,
https://www.jetbrains.com/help/pycharm/project-tool-window.html
