# Python syntax highlighting

## Steps
1. Open a Python file with: keywords, a decorated class with methods,
   `self` and `cls`, `__init__`, strings with escapes, f-strings with
   expressions, numbers, comments with `TODO`, docstrings, `True`/`None`,
   built-ins (`len`, `print`), keyword arguments, an UPPER_CASE constant.
2. Type an opening triple quote in the middle of the file, then close it.

## Expected
- Each kind of token has the colour and font style of its text role in the
  current theme (docs/theme-mapping.md): keyword, string, escape, number,
  comment, docstring, todo, function declaration, method call, class,
  decorator, `self`, dunder, constant, built-in, keyword argument.
- Step 2: while the string is open, the rest of the file turns into a
  string; after closing it, highlighting returns. Updates arrive without
  freezing the editor; typing never waits for the highlighter.
- Switching between the dark and light theme recolours the text at once.

## Commands
`project.open-file`, `editor.type`, `workbench.set-theme {"id": ...}`,
`editor.highlights {"line": ...}`.

## Source
Observed behaviour; text roles in docs/theme-mapping.md.
