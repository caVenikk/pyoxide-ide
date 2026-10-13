//! Commands of the main window (ADR 0007).

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Quit the application.
    Quit => "app.quit", "Quit", "Closes all windows and quits the application.";
    /// Show, focus or hide the project tool window.
    ToggleProject => "workbench.toggle-project", "Project",
        "Shows and focuses the project tool window; hides it when it has the focus.";
    /// Show, focus or hide the Run tool window.
    ToggleRun => "workbench.toggle-run", "Run",
        "Shows and focuses the Run tool window; hides it when it has the focus.";
    /// Hide the active tool window.
    HideToolWindow => "workbench.hide-tool-window", "Hide Active Tool Window",
        "Hides the tool window that has the focus (or was used last) and moves the focus to the editor.";
    /// Select the active editor's file in the project tool window.
    LocateFile => "project.tree.locate", "Select Opened File",
        "Selects the active editor's file in the project tool window.";
    /// Move the focus to the editor.
    FocusEditor => "workbench.focus-editor", "Focus Editor",
        "Moves the keyboard focus to the active editor.";
    /// Switch the theme.
    SetTheme { id: String } => "workbench.set-theme", "Switch Theme",
        "Switches to a built-in theme by id.", r#"{"id": "islands-light"}"#;
    /// Describe the window.
    State => "workbench.state", "Workbench State",
        "Returns the state of the main window as JSON (project, tree, tabs, active editor).";
    /// Open a folder as the project.
    OpenFolder { path: Option<String> } => "project.open-folder", "Open Folder…",
        "Opens a folder as the project; without a path a folder chooser is shown.",
        r#"{"path": "/path/to/project"}"#;
    /// Open a file in a tab.
    OpenFile { path: String } => "project.open-file", "Open File",
        "Opens a file in an editor tab (a path relative to the project or absolute).",
        r#"{"path": "src/app.py"}"#;
    /// Save the active file.
    Save => "editor.save", "Save", "Writes the active file to disk.";
    /// Save every modified file.
    SaveAll => "workbench.save-all", "Save All", "Writes every modified file to disk.";
    /// Close a tab.
    CloseTab { index: Option<usize> } => "tabs.close", "Close Tab",
        "Closes the active editor tab, or the tab at a zero-based index; a modified file is saved first.",
        r#"{"index": 0}"#;
    /// Select a tab.
    SelectTab { index: usize } => "tabs.select", "Select Tab",
        "Makes the tab at a zero-based index active.", r#"{"index": 0}"#;
    NextTab => "tabs.next", "Select Next Tab", "Makes the tab to the right active.";
    PreviousTab => "tabs.previous", "Select Previous Tab", "Makes the tab to the left active.";
    /// The active editor's state.
    EditorState => "editor.state", "Editor State",
        "Returns the active editor's state as JSON (path, caret, selections, modified).";
    /// Highlighting of one line of the active editor.
    EditorHighlights { line: usize } => "editor.highlights", "Editor Highlights",
        "Returns the highlight spans (columns and text roles) of a zero-based line of the active editor.",
        r#"{"line": 0}"#;
    /// The active editor's text.
    EditorText => "editor.text", "Editor Text", "Returns the whole text of the active editor.";
    /// Ask for a line to go to.
    ShowGoToLine => "workbench.go-to-line", "Go to Line:Column…",
        "Opens a field for a line and column to move the active editor's caret to.";
    /// The find bar of the active editor.
    FindState => "find.state", "Find State",
        "Returns the state of the active editor's find bar as JSON (query, options, matches, current match, status).";
}

ui::ui_commands! {
    GO_TO_LINE_COMMANDS, scope = "GoToLine";
    /// Go to the line typed in the Go to Line field.
    ConfirmGoToLine => "go-to-line.confirm", "Go",
        "Moves the caret to the line and column typed in the Go to Line field and closes it.";
    /// Close the Go to Line field.
    CancelGoToLine => "go-to-line.cancel", "Cancel", "Closes the Go to Line field.";
}
