# Code completion

## Steps
1. In a method type `self.` and wait.
2. Type two letters of a member name; then one more.
3. Move with Up/Down, Page Up/Down; press Enter on a method item.
4. Type `os.pa`, select `path` and press Tab inside an existing
   identifier.
5. Type the start of a class name that is not imported in the file and
   accept it.
6. Press Escape; press Ctrl+Space with the caret after a letter.
7. Type inside a string literal and inside a comment.
8. Type `print(` inside an unclosed call and then `self.`.

## Expected
- Step 1: the lookup opens below the caret (`.` is a trigger character);
  the first row is selected; rows show a kind icon, the name, grey tail
  text after it and right-aligned grey type text.
- Step 2: matched characters are highlighted; typing narrows the list on
  the client without a new request unless the server marked the result
  incomplete; the popup opens within 100 ms after the server answers.
- Step 3: Enter inserts the name; a function or method also gets
  parentheses with the caret between them (Python insert handler).
- Step 4: Tab replaces the identifier to the right of the caret.
- Step 5: the name is inserted and the import line is added at the top,
  together in one undo step.
- Step 6: Escape closes the popup; Ctrl+Space opens it explicitly.
- Step 7: no auto-popup inside strings and comments.
- Step 8: the bracket is closed automatically, so the server still
  completes `self.`.

## Commands
`code.complete`, `completion.accept`, `completion.replace`,
`completion.cancel`, `code.state` (`completion.items[]`, `selected`).

## Source
https://www.jetbrains.com/help/pycharm/auto-completing-code.html; IntelliJ
Platform Community (Apache-2.0) at `pycharm/2026.2.3`:
`CompletionAutoPopupHandler`, `LspCompletionCustomizer`,
`python-psi-impl/.../codeInsight/completion/PyFunctionInsertHandler`,
`CodeInsightSettings`.
