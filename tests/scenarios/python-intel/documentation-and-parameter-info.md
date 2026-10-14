# Quick documentation and parameter info

## Steps
1. Put the caret on a call of a documented function and press F1 (macOS;
   also Ctrl+J) or Ctrl+Q (Windows/Linux).
2. Hover a name with the mouse for half a second; move into the popup
   and scroll.
3. Inside the parentheses of a call press Cmd+P (Ctrl+P); move the caret
   across the arguments.
4. Type `(` after a function name and wait one second.

## Expected
- Step 1: a documentation popup at the caret with the signature in a
  highlighted code block and the docstring rendered from Markdown; Escape
  closes it.
- Step 2: the same popup appears after 500 ms of hovering; it stays open
  while the mouse moves into it; it can be scrolled.
- Step 3: the parameter info popup above the call lists the signatures,
  one per overload; the parameter at the caret is bold and follows the
  caret.
- Step 4: parameter info opens automatically after 1000 ms.

## Commands
`code.quick-doc`, `code.parameter-info`, `code.state` (`popup.kind`,
`popup.markdown`, `signature.active_parameter`).

## Source
https://www.jetbrains.com/help/pycharm/viewing-reference-information.html;
IntelliJ Platform Community (Apache-2.0) at `pycharm/2026.2.3`:
`CodeInsightSettings` (`PARAMETER_INFO_DELAY`),
`EditorSettingsExternalizable` (hover delay),
`LspDocumentationTargetProvider`.
