//! Commands of the main window (ADR 0007).

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Quit the application.
    Quit => "app.quit", "Quit", "Closes all windows and quits the application.";
    /// Hide the application (macOS).
    Hide => "app.hide", "Hide", "Hides the application's windows (macOS, the application menu's Hide).";
    /// Hide the other applications (macOS).
    HideOthers => "app.hide-others", "Hide Others", "Hides the windows of the other applications (macOS).";
    /// Show the other applications again (macOS).
    ShowAll => "app.show-all", "Show All", "Shows the windows of the applications hidden with Hide Others again (macOS).";
    /// Minimize the window.
    MinimizeWindow => "window.minimize", "Minimize", "Minimizes the main window.";
    /// Zoom the window.
    ZoomWindow => "window.zoom", "Zoom", "Zooms the main window to fill the screen, or back to its size (the green window button of macOS).";
    /// Close the window's project.
    CloseProject => "project.close", "Close Project",
        "Closes the window's project: modified files are saved and the window closes; when it is the last main window, a main window without a project with the Welcome tab takes its place.";
    /// Show, focus or hide the project tool window.
    ToggleProject => "workbench.toggle-project", "Project",
        "Shows and focuses the project tool window; hides it when it has the focus.";
    /// Show, focus or hide the Run tool window.
    ToggleRun => "workbench.toggle-run", "Run",
        "Shows and focuses the Run tool window; hides it when it has the focus.";
    /// Show, focus or hide the Debug tool window.
    ToggleDebug => "workbench.toggle-debug", "Debug",
        "Shows and focuses the Debug tool window; hides it when it has the focus.";
    /// Show and focus a tool window.
    ActivateToolWindow { id: String } => "workbench.activate-tool-window", "Activate Tool Window",
        "Shows a tool window by id (as `workbench.state` lists them) in its area, in place of the one there, and moves the focus into it.",
        r#"{"id": "run"}"#;
    /// Show, focus or hide a tool window.
    ToggleToolWindow { id: String } => "workbench.toggle-tool-window", "Toggle Tool Window",
        "Shows and focuses a tool window by id; hides it when it has the focus.",
        r#"{"id": "run"}"#;
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
        "Switches to a theme by id (built-in or imported) and stores it, as `settings.switch-theme` does.", r#"{"id": "islands-light"}"#;
    /// Describe the window.
    State => "workbench.state", "Workbench State",
        "Returns the state of the main window as JSON (project, tree, tabs, active editor).";
    /// Describe the sizes of the main window's areas.
    Layout => "workbench.layout", "Workbench Layout",
        "Returns the sizes of the main window's areas for its current size as JSON (window, pane, left, editor, right, row, bottom, minimum island, gap).";
    /// Open a folder as the project.
    OpenFolder { path: Option<String> } => "project.open-folder", "Open Folder…",
        "Opens a folder as the project; without a path a folder chooser is shown.",
        r#"{"path": "/path/to/project"}"#;
    /// Open a file in a tab.
    OpenFile { path: Option<String> } => "project.open-file", "Open File",
        "Opens a file in an editor tab (a path relative to the project or absolute); without a path a file chooser is shown.",
        r#"{"path": "src/app.py"}"#;
    /// Save the active file.
    Save => "editor.save", "Save", "Writes the active file to disk.";
    /// Save every modified file.
    SaveAll => "workbench.save-all", "Save All", "Writes every modified file to disk.";
    /// Save, then look at every file again.
    Synchronize => "workbench.synchronize", "Reload All from Disk",
        "Writes every modified file, then reloads the open files changed on disk and lists the project tree and the file index again.";
    /// Close a tab.
    CloseTab { index: Option<usize> } => "tabs.close", "Close Tab",
        "Closes the active editor tab, or the tab at a zero-based index; a modified file is saved first.",
        r#"{"index": 0}"#;
    /// Close the other tabs.
    CloseOtherTabs { index: Option<usize> } => "tabs.close-others", "Close Other Tabs",
        "Closes every editor tab but the active one (or the one at a zero-based index); pinned tabs stay.",
        r#"{"index": 0}"#;
    /// Close every tab.
    CloseAllTabs => "tabs.close-all", "Close All Tabs",
        "Closes every editor tab, pinned ones too; modified files are saved first.";
    /// Close the tabs without changes.
    CloseUnmodifiedTabs => "tabs.close-unmodified", "Close Unmodified Tabs",
        "Closes the editor tabs whose files have no version control changes; pinned tabs stay.";
    /// Close the tabs that are not pinned.
    CloseUnpinnedTabs => "tabs.close-unpinned", "Close All but Pinned",
        "Closes every editor tab that is not pinned.";
    /// Close the tabs to the left.
    CloseTabsToLeft { index: Option<usize> } => "tabs.close-left", "Close Tabs to the Left",
        "Closes the editor tabs to the left of the active one (or of the one at a zero-based index); pinned tabs stay.",
        r#"{"index": 2}"#;
    /// Close the tabs to the right.
    CloseTabsToRight { index: Option<usize> } => "tabs.close-right", "Close Tabs to the Right",
        "Closes the editor tabs to the right of the active one (or of the one at a zero-based index); pinned tabs stay.",
        r#"{"index": 0}"#;
    /// Pin or unpin a tab.
    TogglePinTab { index: Option<usize> } => "tabs.toggle-pin", "Pin Tab",
        "Pins the active editor tab (or the one at a zero-based index), or unpins it: pinned tabs stay on the left and stay open on Close Other Tabs, Close Tabs to the Left or Right and Close All but Pinned.",
        r#"{"index": 0}"#;
    /// Open the context menu of a tab.
    ShowTabMenu { index: Option<usize> } => "tabs.context-menu", "Show Tab Context Menu",
        "Opens the context menu of the active editor tab (or of the one at a zero-based index) below the tab, as a right click on it does.",
        r#"{"index": 0}"#;
    /// Show a file in the file manager.
    RevealFile { path: Option<String> } => "project.reveal-file", "Show in File Manager",
        "Selects a file (relative to the project or absolute; else the active editor's) in the system file manager (Finder, Explorer).",
        r#"{"path": "src/app.py"}"#;
    /// Open a terminal in a file's folder.
    OpenInTerminal { path: Option<String> } => "terminal.open-in", "Open in Terminal",
        "Opens a new tab of the Terminal tool window in the folder of a file (relative to the project or absolute; else the active editor's), or in a folder.",
        r#"{"path": "src/app.py"}"#;
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
