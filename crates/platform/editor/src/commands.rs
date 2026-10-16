//! Commands of the code editor (ADR 0007). They act on the focused editor;
//! the workbench forwards them to the active editor when another component
//! has the focus.

/// A zero-based line and column (in characters) as command arguments.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    gpui::private::serde::Deserialize,
    gpui::private::schemars::JsonSchema,
)]
#[serde(crate = "gpui::private::serde")]
#[schemars(crate = "gpui::private::schemars")]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl From<Position> for text::Point {
    fn from(p: Position) -> Self {
        text::Point::new(p.line, p.column)
    }
}

ui::ui_commands! {
    COMMANDS, scope = "Editor";
    MoveLeft => "editor.move-left", "Left", "Moves the caret one character left.";
    MoveRight => "editor.move-right", "Right", "Moves the caret one character right.";
    MoveUp => "editor.move-up", "Up", "Moves the caret one line up.";
    MoveDown => "editor.move-down", "Down", "Moves the caret one line down.";
    MoveWordLeft => "editor.move-word-left", "Previous Word",
        "Moves the caret to the start of the previous word.";
    MoveWordRight => "editor.move-word-right", "Next Word",
        "Moves the caret to the next word boundary.";
    MoveLineStart => "editor.move-line-start", "Line Start",
        "Moves the caret to the first non-blank character of the line, or to its start.";
    MoveLineEnd => "editor.move-line-end", "Line End", "Moves the caret to the end of the line.";
    MoveDocumentStart => "editor.move-document-start", "Document Start",
        "Moves the caret to the start of the file.";
    MoveDocumentEnd => "editor.move-document-end", "Document End",
        "Moves the caret to the end of the file.";
    PageUp => "editor.page-up", "Page Up", "Moves the caret and the view one screen up.";
    PageDown => "editor.page-down", "Page Down", "Moves the caret and the view one screen down.";
    SelectLeft => "editor.select-left", "Left with Selection",
        "Extends the selection one character left.";
    SelectRight => "editor.select-right", "Right with Selection",
        "Extends the selection one character right.";
    SelectUp => "editor.select-up", "Up with Selection", "Extends the selection one line up.";
    SelectDown => "editor.select-down", "Down with Selection",
        "Extends the selection one line down.";
    SelectWordLeft => "editor.select-word-left", "Previous Word with Selection",
        "Extends the selection to the start of the previous word.";
    SelectWordRight => "editor.select-word-right", "Next Word with Selection",
        "Extends the selection to the next word boundary.";
    SelectLineStart => "editor.select-line-start", "Line Start with Selection",
        "Extends the selection to the start of the line.";
    SelectLineEnd => "editor.select-line-end", "Line End with Selection",
        "Extends the selection to the end of the line.";
    SelectDocumentStart => "editor.select-document-start", "Document Start with Selection",
        "Extends the selection to the start of the file.";
    SelectDocumentEnd => "editor.select-document-end", "Document End with Selection",
        "Extends the selection to the end of the file.";
    SelectPageUp => "editor.select-page-up", "Page Up with Selection",
        "Extends the selection one screen up.";
    SelectPageDown => "editor.select-page-down", "Page Down with Selection",
        "Extends the selection one screen down.";
    SelectAll => "editor.select-all", "Select All", "Selects the whole file.";
    SelectWord => "editor.select-word", "Extend Selection",
        "Selects the word at the caret, then the line, then the whole file.";
    SetSelection { anchor: Position, head: Position } => "editor.set-selection", "Set Selection",
        "Selects from anchor to head (zero-based line and column); equal points place the caret.",
        r#"{"anchor": {"line": 0, "column": 0}, "head": {"line": 0, "column": 3}}"#;
    Type { text: String } => "editor.type", "Type Text",
        "Inserts text at every caret, replacing selections.", r#"{"text": "print()"}"#;
    Newline => "editor.newline", "Enter",
        "Breaks the line at the caret and keeps the indentation.";
    StartNewLine => "editor.start-new-line", "Start New Line",
        "Starts a new line after the caret line.";
    Tab => "editor.tab", "Tab", "Inserts indentation, or indents the selected lines.";
    Unindent => "editor.unindent", "Unindent Line or Selection",
        "Removes one level of indentation from the caret or selected lines.";
    Backspace => "editor.backspace", "Backspace",
        "Deletes the selection or the character before the caret.";
    Delete => "editor.delete", "Delete", "Deletes the selection or the character after the caret.";
    DeleteWordLeft => "editor.delete-word-left", "Delete to Word Start",
        "Deletes from the caret to the start of the word.";
    DeleteWordRight => "editor.delete-word-right", "Delete to Word End",
        "Deletes from the caret to the next word boundary.";
    DeleteLine => "editor.delete-line", "Delete Line", "Deletes the caret line or selected lines.";
    DuplicateLine => "editor.duplicate", "Duplicate Line or Selection",
        "Duplicates the selection, or the caret line when nothing is selected.";
    Undo => "editor.undo", "Undo", "Undoes the last group of edits.";
    Redo => "editor.redo", "Redo", "Redoes the last undone group of edits.";
    Copy => "editor.copy", "Copy", "Copies the selection, or the caret line when nothing is selected.";
    Cut => "editor.cut", "Cut", "Cuts the selection, or the caret line when nothing is selected.";
    Paste => "editor.paste", "Paste", "Pastes the clipboard at the carets.";
    ShrinkSelection => "editor.shrink-selection", "Shrink Selection",
        "Goes back to the selection before the last Extend Selection.";
    Escape => "editor.escape", "Escape", "Drops the selection and extra carets.";
    DeleteToLineStart => "editor.delete-to-line-start", "Delete to Line Start",
        "Deletes from the start of the line to the caret.";
    DeleteToLineEnd => "editor.delete-to-line-end", "Delete to Line End",
        "Deletes from the caret to the end of the line.";
    MoveLineUp => "editor.move-line-up", "Move Line Up",
        "Moves the caret line or the selected lines up by one line.";
    MoveLineDown => "editor.move-line-down", "Move Line Down",
        "Moves the caret line or the selected lines down by one line.";
    MoveStatementUp => "editor.move-statement-up", "Move Statement Up",
        "Moves the statement at the caret (with its indented block) above the previous one.";
    MoveStatementDown => "editor.move-statement-down", "Move Statement Down",
        "Moves the statement at the caret (with its indented block) below the next one.";
    JoinLines => "editor.join-lines", "Join Lines",
        "Joins the next line to the caret line, or the selected lines.";
    ToggleComment => "editor.toggle-comment", "Comment with Line Comment",
        "Comments or uncomments the caret line or the selected lines.";
    GoToLine { line: usize, column: Option<usize> } => "editor.go-to-line", "Go to Line:Column",
        "Moves the caret to a one-based line and optional column.", r#"{"line": 10, "column": 1}"#;
    ScrollToLine { line: usize } => "editor.scroll-to", "Scroll to Line",
        "Scrolls so that a zero-based line is at the top of the view.", r#"{"line": 100}"#;
    // The problems are the editor's marks, so moving between them is the
    // editor's command (ids of code intelligence, ADR 0010).
    NextProblem => "code.next-problem", "Next Highlighted Error",
        "Moves the caret to the next problem in the file, errors first.";
    PreviousProblem => "code.previous-problem", "Previous Highlighted Error",
        "Moves the caret to the previous problem in the file, errors first.";
    ToggleSoftWraps => "editor.toggle-soft-wraps", "Soft-Wrap",
        "Turns soft wraps of the editor on or off: lines wider than the view go on in the next rows.";
    // For screenshots and checks of the decorations API (no menu item, no
    // key): what the debugger and version control draw, on fixed lines.
    SampleDecorations => "editor.sample-decorations", "Show Sample Decorations",
        "Test only: draws a breakpoint, the execution point, inline values and change markers on fixed lines.";
}
