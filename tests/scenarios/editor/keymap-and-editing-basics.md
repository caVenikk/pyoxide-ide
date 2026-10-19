# Keymap and editing basics

The everyday editing keys of the reference IDE, with its default keymaps:
macOS ("Mac") and Windows/Linux ("Win"). Keystrokes come from the IntelliJ
Platform Community keymap files (see NOTICE); the behaviour is described
here in Pyoxide's own words. Every binding of both keymaps is pressed as a
real keystroke in `crates/platform/workbench/tests/it/keymap.rs`; the behaviour
below is covered by `crates/platform/editor/tests/it/basics.rs` and the Python
test of `crates/python/pyoxide-ide`.

## Where the keystrokes come from
The "Reference action" column names the action of the reference keymaps
whose keystrokes a row uses. The Windows/Linux keystrokes are the action's
entries in `$default.xml`. The macOS keymap (`Mac OS X 10.5+.xml`) either
lists the action itself ("mac") or inherits it from `$default.xml`, where
Ctrl becomes Cmd ("inh"); for example Duplicate Line or Selection is
`control D` in `$default.xml` and so Cmd+D on macOS.

## Steps and expected behaviour

### Caret movement (each also with Shift, which extends the selection)
| Action | Mac | Win | Reference action |
|---|---|---|---|
| Character left / right | Left / Right, Ctrl+B / Ctrl+F | Left / Right | EditorLeft / EditorRight (mac) |
| Line up / down | Up / Down, Ctrl+P / Ctrl+N | Up / Down | EditorUp / EditorDown (mac) |
| Previous / next word | Alt+Left / Alt+Right, Ctrl+Alt+B / Ctrl+Alt+F | Ctrl+Left / Ctrl+Right | EditorPreviousWord / EditorNextWord (mac) |
| Line start / end | Cmd+Left / Cmd+Right, Home / End, Ctrl+A / Ctrl+E | Home / End | EditorLineStart / EditorLineEnd (mac) |
| Document start / end | Cmd+Home / Cmd+End | Ctrl+Home / Ctrl+End | EditorTextStart / EditorTextEnd (inh) |
| Page up / down | Page Up / Page Down | Page Up / Page Down | EditorPageUp / EditorPageDown (inh) |
| Go to line and column | Cmd+L | Ctrl+G | GotoLine (mac) |

- Line start is "smart": the first press goes to the first non-blank
  character, the next to column 1, and back.
- Up and Down keep the preferred column across shorter lines.
- Go to Line opens a small field with the current `line:column` selected;
  typing `120` or `120:5` and Enter moves the caret there and centres the
  line; Escape (or clicking elsewhere) closes the field. Text that is not a
  line number keeps it open.

### Deleting
| Action | Mac | Win | Reference action |
|---|---|---|---|
| Delete before / after the caret | Backspace (also Shift+Backspace) / Delete | same | EditorBackSpace (mac), $Delete (mac) |
| Delete to word start / end | Alt+Backspace / Alt+Delete | Ctrl+Backspace / Ctrl+Delete | EditorDeleteToWordStart / EditorDeleteToWordEnd (mac) |
| Delete line | Cmd+Backspace | Ctrl+Y | EditorDeleteLine (mac) |
| Delete to line start / end | no default shortcut (commands) | same | EditorDeleteToLineStart / EditorDeleteToLineEnd (no keystroke) |

- Backspace in the leading spaces of a line goes back to the previous
  indentation stop (one level), not one space.
- Backspace between an empty bracket or quote pair removes both.
- Delete to line start at column 1 (or to line end at the end of a line)
  joins the line with the previous (next) one.

### Enter and indentation
| Action | Mac | Win | Reference action |
|---|---|---|---|
| New line | Enter | Enter | EditorEnter (inh) |
| Start new line below | Shift+Enter | Shift+Enter | EditorStartNewLine (mac) |
| Indent / unindent | Tab / Shift+Tab | Tab / Shift+Tab | EditorIndentSelection / EditorUnindentSelection (inh) |

- Enter keeps the indentation of the line; after a line ending with `:`
  (Python), or with an unclosed bracket, the new line gets one more level.
- After a line starting with `return`, `pass`, `break`, `continue` or
  `raise` (Python) the new line gets one level less.
- Enter between `(` and `)` (or other brackets) puts the closing bracket
  on its own line below an indented empty line with the caret.
- Tab with a selection over several lines indents those lines; without
  one it fills up to the next indentation stop. Shift+Tab removes one level.

### Line operations
| Action | Mac | Win | Reference action |
|---|---|---|---|
| Duplicate line or selection | Cmd+D | Ctrl+D | EditorDuplicate (inh: Ctrl+D becomes Cmd+D) |
| Move line up / down | Alt+Shift+Up / Down | Alt+Shift+Up / Down | MoveLineUp / MoveLineDown (inh) |
| Move statement up / down | Cmd+Shift+Up / Down | Ctrl+Shift+Up / Down | MoveStatementUp / MoveStatementDown (inh) |
| Join lines | Ctrl+Shift+J | Ctrl+Shift+J | EditorJoinLines (mac) |
| Comment with line comment | Cmd+/ | Ctrl+/ | CommentByLineComment (inh) |

- Moving lines takes all selected lines and keeps them selected; the first
  line does not move up and the last does not move down.
- Moving a statement takes its indented block (and continuation lines
  closing a bracket) and swaps it with the neighbouring statement of the
  same indentation; blank lines between them stay where they are. The
  first statement of a block does not leave the block.
- Join lines removes the line break and the next line's indentation,
  leaving one space (none next to a bracket or a blank line).
- Line comments use the file type's prefix (`# ` for Python) at the
  smallest indentation of the lines; blank lines are skipped. When every
  non-blank line is commented, the prefix is removed (with or without the
  space). Without a selection the caret moves to the next line.

### Brackets and quotes
- Typing `(`, `[` or `{` adds the closing bracket when the next character
  is a space, the line end, a closing bracket or `,;:`.
- Typing a closing bracket or quote right before the same character steps
  over it.
- Quotes (Python `'` and `"`) are paired, but not after a word (except a
  string prefix such as `f`, `r`, `b`) and not inside a string.
- A third quote after an empty pair opens a triple-quoted string and puts
  the caret between `"""` and `"""`.
- Typing a bracket or quote with a selection wraps the selection.

### Selection
| Action | Mac | Win | Reference action |
|---|---|---|---|
| Select all | Cmd+A | Ctrl+A | $SelectAll (inh) |
| Extend selection | Alt+Up | Ctrl+W | EditorSelectWord (mac) |
| Shrink selection | Alt+Down | Ctrl+Shift+W | EditorUnSelectWord (mac) |
| Drop the selection | Escape | Escape | EditorEscape (inh) |

- Extend selection grows to the word, then to each enclosing syntax node
  (string content, string, arguments, call, statement, ... when the file
  type has a syntax tree), then the line text, the whole line and the file.
- Shrink selection goes back through the same steps; any other caret move
  forgets them.

### Clipboard and history
| Action | Mac | Win | Reference action |
|---|---|---|---|
| Copy | Cmd+C, Cmd+Insert | Ctrl+C, Ctrl+Insert | $Copy (inh) |
| Cut | Cmd+X, Shift+Delete | Ctrl+X, Shift+Delete | $Cut (inh) |
| Paste | Cmd+V, Shift+Insert | Ctrl+V, Shift+Insert | $Paste (inh) |
| Undo | Cmd+Z | Ctrl+Z, Alt+Backspace | $Undo (mac) |
| Redo | Cmd+Shift+Z, Alt+Shift+Backspace | Ctrl+Shift+Z, Alt+Shift+Backspace | $Redo (inh) |

- Copy or cut without a selection takes the whole caret line; pasting such
  a line puts it above the caret line, keeping the caret on its text.

### Files and tabs
| Action | Mac | Win | Reference action |
|---|---|---|---|
| Save all | Cmd+S | Ctrl+S | SaveAll (inh) |
| Close tab | Cmd+W | Ctrl+F4 | CloseContent (mac) |
| Next / previous tab | Cmd+Shift+] / [, Ctrl+Right / Left | Alt+Right / Left | NextTab / PreviousTab (mac) |
| Project tool window | Cmd+1 | Alt+1 | ActivateProjectToolWindow (mac) |
| Hide tool window | Shift+Escape | Shift+Escape | HideActiveWindow (inh) |

### Mouse
- Click places the caret; drag selects; double click selects a word;
  triple click selects the line with its line break; Shift+click extends
  the selection from its anchor.
- The wheel scrolls by lines, a trackpad by pixels; neither moves the
  caret.
- The pointer is an I-beam over the text, an arrow over the gutter and the
  scrollbar, and a left-right resize pointer over the gap between the
  project tool window and the editor, which resizes the tool window when
  dragged.

## Commands
`editor.move-*`, `editor.select-*`, `editor.page-up`, `editor.page-down`,
`editor.select-word`, `editor.shrink-selection`, `editor.escape`,
`editor.newline`, `editor.start-new-line`, `editor.tab`, `editor.unindent`,
`editor.backspace`, `editor.delete`, `editor.delete-word-left`,
`editor.delete-word-right`, `editor.delete-to-line-start`,
`editor.delete-to-line-end`, `editor.delete-line`, `editor.duplicate`,
`editor.move-line-up`, `editor.move-line-down`, `editor.move-statement-up`,
`editor.move-statement-down`, `editor.join-lines`, `editor.toggle-comment`,
`editor.go-to-line`, `workbench.go-to-line`, `go-to-line.confirm`,
`go-to-line.cancel`, `editor.copy`, `editor.cut`, `editor.paste`,
`editor.undo`, `editor.redo`, `workbench.save-all`, `tabs.close`,
`tabs.next`, `tabs.previous`, `workbench.toggle-project`,
`workbench.hide-tool-window`.

## Not covered yet
Multiple carets and column selection, Ctrl+Up/Down scrolling without moving
the caret (Win), cut to line end (Ctrl+K on Mac), Toggle Case, Move Element
Left/Right, language-aware Join Lines (string and statement joining),
block comments, folding, completion.

## Source
https://www.jetbrains.com/help/pycharm/mastering-keyboard-shortcuts.html,
https://www.jetbrains.com/help/pycharm/working-with-source-code.html,
the IntelliJ Platform Community keymap files (see NOTICE).
