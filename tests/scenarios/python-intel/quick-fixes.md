# Quick fixes and intention actions

## Steps
1. On the unused `import os` press Alt+Enter (Option+Enter).
2. Open the submenu of the first item (Right arrow).
3. Choose "Disable for this file"; undo; choose "Disable for this line";
   undo; choose "Disable for this project".
4. Press Alt+Enter on a line without problems.
5. Use a class from another module without importing it; press
   Alt+Enter on it.

## Expected
- Step 1: the popup lists quick fixes first, each with a bulb icon, then a
  separator and the intentions without icon sorted by name; the top item reads
  "Ruff (unused-import): Remove unused import: `os`" (the rule code
  replaced by its name); Enter applies it as one undo step.
- Step 2: sub-options "Disable for this file", "Disable for this line",
  "Disable for this project" (cross icon on the last), in this order.
- Step 3: for this file, line 1 becomes `# ruff: noqa: F401` (an existing
  `# ruff: noqa:` comment is extended); for this line, `# noqa: F401` is
  appended; for this project, `F401` is added to `ignore` in
  `[tool.ruff.lint]` of `pyproject.toml` (or `[lint]` of `ruff.toml` /
  `.ruff.toml`; a `ruff.toml` is created next to the file if none
  exists). The problem disappears.
- Step 4: intentions "Ruff: Fix all auto-fixable problems" and "Ruff:
  Organize imports".
- Step 5: the type server's import fix ("Import ..." / add import) is
  offered; applying it adds the import line.

## Commands
`code.show-intentions`, `code.state` (`popup.items[]`).

## Source
https://www.jetbrains.com/help/pycharm/intention-actions.html; ruff
`ruff_server` code actions; IntelliJ Platform Community (Apache-2.0) at
`pycharm/2026.2.3`: `RuffLspClient.customizeQuickFixes`,
`RuffDisableRuleIntentionAction`, `RuffDisableRuleForFileIntentionAction`,
`LspIntentionActionService`.
