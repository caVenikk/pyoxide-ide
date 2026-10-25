# Quick documentation and parameter info

## Steps
1. Put the caret on a call of a documented function and press F1 (macOS;
   also Ctrl+J) or Ctrl+Q (Windows/Linux).
2. Hover a name with the mouse for half a second; move into the popup
   and scroll.
3. Inside the parentheses of a call press Cmd+P (Ctrl+P); move the caret
   across the arguments.
4. Type `(` after a function name and wait one second.
5. Show the documentation of an imported module (F1 on `contextlib`) and
   click the popup's more button (the three dots at its bottom right).
6. In that menu turn off Show on Mouse Move and hover a name; turn off
   Show Documentation Popup First and press F1; choose Adjust Font
   Size... and press Right; choose Open in Documentation Tool Window.

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
- Step 5: the menu lists Open in Documentation Tool Window (F1), Show
  Documentation Popup First and Show on Mouse Move (both checked by
  default), Adjust Font Size..., a separator, then Back and Forward with
  the window's Back and Forward shortcuts, disabled.
- Step 6: the hover popup shows problems only (no documentation); F1
  shows the documentation in the Documentation tool window on the right
  instead of a popup; a slider popup at the mouse makes the documentation
  text larger at once; the popup's documentation moves into the tool
  window and the popup closes. The options are kept after a restart.

## Commands
`code.quick-doc`, `code.parameter-info`, `code.state` (`popup.kind`,
`popup.markdown`, `popup.menu`, `popup.font_size`, `tool_window`,
`signature.active_parameter`), `documentation.open-in-tool-window`,
`documentation.toggle-show-popup-first`,
`documentation.toggle-show-on-mouse-move`,
`documentation.adjust-font-size`, `documentation.back`,
`documentation.forward`.

## Source
https://www.jetbrains.com/help/pycharm/viewing-reference-information.html;
IntelliJ Platform Community (Apache-2.0) at `pycharm/2026.2.3`:
`CodeInsightSettings` (`PARAMETER_INFO_DELAY`),
`EditorSettingsExternalizable` (hover delay),
`LspDocumentationTargetProvider`, `DocumentationPopupUI`,
`DocumentationManager.skipPopup`, `ToggleShowDocsOnHoverAction`,
`FontSizePopup`, `DocumentationToolWindowManager`.
