//! The commands of debug sessions (ADR 0001, 0015): the Debug executor
//! (`Debug`, `DebugClass`, the run widget's and the dropdown's Debug), the
//! session's actions of the Run menu and the Debug tool window's toolbar
//! (`XDebugger.Resume`, `Pause`, `StepOver`, `StepInto`,
//! `StepIntoMyCode`, `StepOut`, `RunToCursor`, `ForceRunToCursor`,
//! `ShowExecutionPoint`) and the tool window's tabs. Titles are the
//! reference's action texts (`ActionsBundle`, `XDebuggerBundle`).
//! Commands on a session act on the selected tab's session, else on the
//! last one started that has not ended.

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Debug the selected configuration.
    Debug => "debug.debug", "Debug",
        "Debugs the selected run configuration; with Current File selected, the file in the editor.";
    /// Debug the file in the editor.
    DebugCurrentFile { file: Option<String> } => "debug.current-file", "Debug Context Configuration",
        "Debugs the file in the editor (or `file`, relative to the project or absolute) with its context configuration: an existing one that runs it, else a new temporary one, which becomes the selected one.",
        r#"{"file": "main.py"}"#;
    /// Debug the file in the editor without a configuration.
    DebugFile => "debug.file", "Debug Current File",
        "Debugs the file in the editor with its context configuration, without selecting it (the dropdown's Current File).";
    /// Debug a run configuration.
    DebugConfiguration { name: Option<String>, r#type: Option<String> }
        => "debug.configuration.debug", "Debug Configuration",
        "Debugs a run configuration by name (and type id when two types have the name), or the selected one.",
        r#"{"name": "main"}"#;
    /// Resume Program.
    Resume { index: Option<usize> } => "debug.resume", "Resume Program",
        "Resumes the suspended program of the debug session (or of the Debug tool window's tab at a zero-based index).",
        r#"{"index": 0}"#;
    /// Pause Program.
    Pause { index: Option<usize> } => "debug.pause", "Pause Program",
        "Suspends the running program of the debug session (or of the tab at a zero-based index) where it is.",
        r#"{"index": 0}"#;
    /// Step Over.
    StepOver => "debug.step-over", "Step Over",
        "Runs the current line and stops at the next one of the same function or its caller.";
    /// Step Into.
    StepInto => "debug.step-into", "Step Into",
        "Steps into the function called on the current line, library code included.";
    /// Step Into My Code.
    StepIntoMyCode => "debug.step-into-my-code", "Step Into My Code",
        "Steps into the function called on the current line, skipping library code.";
    /// Step Out.
    StepOut => "debug.step-out", "Step Out",
        "Runs to the end of the current function and stops in its caller.";
    /// Run to Cursor.
    RunToCursor { file: Option<String>, line: Option<u32> } => "debug.run-to-cursor", "Run to Cursor",
        "Resumes the program until it reaches a line (the caret's in the editor without a file and a line), stopping at breakpoints on the way.",
        r#"{"file": "main.py", "line": 9}"#;
    /// Force Run to Cursor.
    ForceRunToCursor { file: Option<String>, line: Option<u32> } => "debug.force-run-to-cursor", "Force Run to Cursor",
        "Resumes the program until it reaches a line (the caret's in the editor without a file and a line), ignoring breakpoints on the way.",
        r#"{"file": "main.py", "line": 9}"#;
    /// Show Execution Point.
    ShowExecutionPoint => "debug.show-execution-point", "Show Execution Point",
        "Opens the file and line where the program is suspended.";
    /// Stop the session.
    StopSession { index: Option<usize> } => "debug.stop", "Stop",
        "Stops the program of the selected debug session (or of the tab at a zero-based index) and ends its debugger.",
        r#"{"index": 0}"#;
    /// Rerun the session.
    RerunSession { index: Option<usize> } => "debug.rerun", "Rerun",
        "Stops the selected debug session (or the one at a zero-based index) and debugs its configuration again in the same tab.",
        r#"{"index": 0}"#;
    /// Close a session's tab.
    CloseSession { index: Option<usize> } => "debug.close-tab", "Close Tab",
        "Closes the selected debug session's tab (or the one at a zero-based index); a running session is stopped.",
        r#"{"index": 0}"#;
    /// Select a session's tab.
    SelectSession { index: usize } => "debug.select-tab", "Select Tab",
        "Selects the Debug tool window's tab at a zero-based index.", r#"{"index": 0}"#;
    /// Select the Threads & Variables or the Console tab.
    SelectContent { content: String } => "debug.select-content", "Select Debugger Tab",
        "Selects the session's Threads & Variables (`threads-variables`) or Console (`console`) tab.",
        r#"{"content": "console"}"#;
    /// The toolbar's More menu.
    MoreActions => "debug.more-actions", "More",
        "Opens the Debug tool window toolbar's More menu: Run to Cursor, Force Run to Cursor, Show Execution Point, Debugger Settings.";
    /// Type into the session's console.
    TypeInput { text: String } => "debug.console.type", "Type into Console",
        "Types text into the selected debug session's Console as the person would; a line break sends the line to the program.",
        r#"{"text": "42\n"}"#;
    /// The list of threads.
    ShowThreads => "debug.show-threads", "Threads",
        "Opens the list of the suspended session's threads below the thread combo of Threads & Variables.";
    /// Select a thread.
    SelectThread { id: i64 } => "debug.select-thread", "Select Thread",
        "Shows the frames of a thread of the suspended session (by its id, as debug.state lists them).",
        r#"{"id": 1}"#;
    /// Select a frame.
    SelectFrame { index: usize } => "debug.select-frame", "Select Frame",
        "Selects a frame of the selected thread by its zero-based index in the frames list: the editor shows its line, the variables view its values.",
        r#"{"index": 1}"#;
    /// The frame above the selected one.
    PreviousFrame => "debug.previous-frame", "Previous Frame",
        "Selects the frame above the selected one in the frames list (from anywhere in the window, the reference's Previous Occurrence while debugging).";
    /// The frame below the selected one.
    NextFrame => "debug.next-frame", "Next Frame",
        "Selects the frame below the selected one in the frames list (its caller).";
    /// Show the selected frame's values after the lines in the editor.
    ToggleInlineValues { on: Option<bool> } => "debug.toggle-inline-values", "Show Variable Values in Editor",
        "Shows the values of the selected frame's variables after the lines that declare them while the program is suspended, or stops showing them (with `on` set, as given).",
        r#"{"on": false}"#;
    /// Close the hint below the frames.
    CloseFramesHint => "debug.close-frames-hint", "Close Frames Hint",
        "Closes the hint about switching frames below the frames list.";
    /// The row above in the variables view.
    VariablesUp => "debug.variables.up", "Up",
        "Selects the row above the selected one in the variables view.";
    /// The row below in the variables view.
    VariablesDown => "debug.variables.down", "Down",
        "Selects the row below the selected one in the variables view.";
    /// Collapse, or go to the parent.
    VariablesLeft => "debug.variables.left", "Left",
        "Collapses the selected node of the variables view, or selects its parent.";
    /// Expand, or go to the first child.
    VariablesRight => "debug.variables.right", "Right",
        "Expands the selected node of the variables view, or selects its first child.";
    /// Select a node of the variables view.
    SelectVariable { path: Vec<String> } => "debug.variables.select", "Select Variable",
        "Selects a node of the variables view by its path of names as shown (a group by its title).",
        r#"{"path": ["items", "0"]}"#;
    /// Expand or collapse a node of the variables view.
    ExpandVariable { path: Vec<String>, expanded: Option<bool> } => "debug.variables.expand", "Expand Variable",
        "Expands (or with `expanded: false` collapses, without it toggles) a node of the variables view by its path of names; an ellipsis row shows the rest of its collection.",
        r#"{"path": ["items"], "expanded": true}"#;
    /// Evaluate the expression of the evaluate field.
    EvaluateField { expression: Option<String> } => "debug.variables.evaluate", "Evaluate",
        "Evaluates the expression of the evaluate field above the variables (or `expression`, put into the field first) in the selected frame: the result is the first node of the variables view, `result`.",
        r#"{"expression": "len(items)"}"#;
    /// Add the evaluate field's expression to the watches.
    WatchField { expression: Option<String> } => "debug.variables.watch-expression", "Add Expression to Watches",
        "Adds the expression of the evaluate field (or `expression`, put into the field first) to the watches.",
        r#"{"expression": "items[0]"}"#;
    /// The evaluate field's history.
    ShowEvaluateHistory => "debug.variables.history", "Show History",
        "Opens the expressions evaluated or watched before, below the evaluate field; choosing one puts it into the field.";
    /// The history of the editor of a row.
    ShowEditorHistory => "debug.variables.editor-history", "Show History",
        "Opens the expressions typed before into the editor of the value or the watch edited in its row, below the editor; choosing one puts it into the editor.";
    /// Choose an expression of the open history.
    ChooseHistory { index: usize } => "debug.variables.choose-history", "Choose from History",
        "Chooses the expression at a zero-based index of the open history list of the evaluate field or of a row's editor: it goes into that field.",
        r#"{"index": 0}"#;
    /// Put an expression into the evaluate field.
    SetFieldExpression { text: String } => "debug.variables.set-expression", "Set Expression",
        "Puts an expression into the evaluate field and gives the field the focus.",
        r#"{"text": "len(items)"}"#;
    /// Evaluate Expression of a value.
    EvaluateValue { expression: Option<String> } => "debug.evaluate-expression", "Evaluate Expression\u{2026}",
        "Puts an expression (the selected value's without one) into the evaluate field and gives the field the focus.",
        r#"{"expression": "items"}"#;
    /// New Watch.
    NewWatch { expression: Option<String> } => "debug.new-watch", "New Watch\u{2026}",
        "Adds a watch after the selected one (at the end without one): `expression`, else an empty one edited in place.",
        r#"{"expression": "len(items)"}"#;
    /// Remove Watch.
    RemoveWatch { expression: Option<String>, index: Option<usize> } => "debug.remove-watch", "Remove Watch",
        "Removes the selected watch, or the one with an expression or at a zero-based index.",
        r#"{"expression": "len(items)"}"#;
    /// Edit a watch.
    EditWatch { expression: Option<String> } => "debug.edit-watch", "Edit\u{2026}",
        "Edits the selected watch in place, or sets its expression to `expression` (an empty one removes it).",
        r#"{"expression": "len(items) + 1"}"#;
    /// Remove All Watches.
    RemoveAllWatches => "debug.remove-all-watches", "Remove All Watches",
        "Removes every watch of the session's configuration type and the evaluate field's result.";
    /// Pause Watch / Resume Watch.
    PauseWatch { index: Option<usize> } => "debug.pause-watch", "Pause Watch",
        "Pauses the selected watch (or the one at a zero-based index): it keeps its value and is not evaluated at the next stops; a paused watch is resumed.",
        r#"{"index": 0}"#;
    /// Evaluate a paused watch once.
    EvaluatePausedWatch { index: usize } => "debug.evaluate-paused-watch", "Evaluate",
        "Evaluates a paused watch (by its zero-based index) once in the frame shown; it stays paused.",
        r#"{"index": 0}"#;
    /// Add the selected value to the watches.
    AddToWatches => "debug.add-to-watches", "Add to Watches",
        "Adds the selected value's expression to the watches.";
    /// Set Value.
    SetValue { value: Option<String> } => "debug.variables.set-value", "Set Value\u{2026}",
        "Edits the selected value in place (a watch's expression on a watch), or sets it to the value of `value`.",
        r#"{"value": "5"}"#;
    /// Apply the value or the watch edited in place.
    CommitEdit => "debug.variables.commit-edit", "Apply",
        "Applies the value or the watch expression edited in place in the variables view.";
    /// Close the editor in a row without a change.
    CancelEdit => "debug.variables.cancel-edit", "Cancel",
        "Closes the editor of a value or a watch in the variables view without a change.";
    /// Copy Value.
    CopyValue => "debug.variables.copy-value", "Copy Value",
        "Copies the selected value (the whole of it when the debugger can give it) to the clipboard.";
    /// Copy Name.
    CopyName => "debug.variables.copy-name", "Copy Name",
        "Copies the selected value's name to the clipboard.";
    /// Jump To Source.
    JumpToValueSource => "debug.variables.jump-to-source", "Jump To Source",
        "Opens the selected value's declaration in the frame's file.";
    /// Jump To Type Source.
    JumpToTypeSource => "debug.variables.jump-to-type-source", "Jump To Type Source",
        "Opens the declaration of the selected value's type, where the running program finds it.";
    /// View as.
    ViewAs { renderer: String } => "debug.variables.view-as", "View as",
        "Shows the selected value as a renderer of its type shows it (an integer: `Binary`, `Decimal`, `Hex`).",
        r#"{"renderer": "Hex"}"#;
    /// The variables view's context menu.
    VariablesMenu => "debug.variables.context-menu", "Show Context Menu",
        "Opens the context menu of the selected node of the variables view.";
}
