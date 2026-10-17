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
    DebugCurrentFile => "debug.current-file", "Debug Context Configuration",
        "Debugs the file in the editor with its context configuration: an existing one that runs it, else a new temporary one, which becomes the selected one.";
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
}
