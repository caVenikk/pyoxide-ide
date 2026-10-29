//! The breakpoint commands (ADR 0001, 0015): the reference's actions of
//! the editor and the Run menu (`ToggleLineBreakpoint`,
//! `ToggleTemporaryLineBreakpoint`, `ToggleBreakpointEnabled`,
//! `EditBreakpoint`, `ViewBreakpoints`, `XDebugger.MuteBreakpoints`), the
//! properties popup's and the Breakpoints dialog's. Files are relative to
//! the project root or absolute; lines count from 1. Without a file and a
//! line a command acts at the caret of the active editor.

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Toggle Line Breakpoint.
    ToggleBreakpoint {
        file: Option<String>,
        line: Option<u32>,
        temporary: Option<bool>,
    } => "debug.toggle-breakpoint", "Toggle Line Breakpoint",
        "Adds a line breakpoint on a line a breakpoint can be put on, or removes the one there; at the caret without a file and a line.",
        r#"{"file": "main.py", "line": 7}"#;
    /// Toggle Temporary Line Breakpoint.
    ToggleTemporaryBreakpoint {
        file: Option<String>,
        line: Option<u32>,
    } => "debug.toggle-temporary-breakpoint", "Toggle Temporary Line Breakpoint",
        "Adds a line breakpoint that is removed once hit, or removes the breakpoint of the line; at the caret without a file and a line.",
        r#"{"file": "main.py", "line": 7}"#;
    /// Toggle Breakpoint Enabled.
    ToggleBreakpointEnabled {
        file: Option<String>,
        line: Option<u32>,
    } => "debug.toggle-breakpoint-enabled", "Toggle Breakpoint Enabled",
        "Enables or disables the breakpoint of a line; at the caret without a file and a line.",
        r#"{"file": "main.py", "line": 7}"#;
    /// Edit Breakpoint.
    EditBreakpoint {
        file: Option<String>,
        line: Option<u32>,
        enabled: Option<bool>,
        suspend: Option<String>,
        condition: Option<String>,
        log_message: Option<bool>,
        log_stack: Option<bool>,
        log_expression: Option<String>,
    } => "debug.edit-breakpoint", "Edit Breakpoint",
        "Changes the breakpoint of a line: enabled, suspend (all, thread, none), condition (empty removes it), logging. Without changes it opens the breakpoint's properties popup (the Breakpoints dialog when the line has none).",
        r#"{"file": "main.py", "line": 7, "condition": "i == 3", "suspend": "thread"}"#;
    /// View Breakpoints….
    ViewBreakpoints => "debug.view-breakpoints", "View Breakpoints\u{2026}",
        "Opens the Breakpoints dialog: the line and exception breakpoints by type, the selected one's properties and its source.";
    /// Mute Breakpoints.
    MuteBreakpoints { muted: Option<bool> } => "debug.mute-breakpoints", "Mute Breakpoints",
        "Mutes or unmutes every breakpoint (toggles without an argument): muted breakpoints do not stop the program.",
        r#"{"muted": true}"#;
    /// Add an exception breakpoint.
    AddExceptionBreakpoint { class: Option<String> } => "debug.add-exception-breakpoint", "Add Exception Breakpoint",
        "Adds an exception breakpoint for an exception class; without a class the class chooser opens (in the Breakpoints dialog).",
        r#"{"class": "KeyError"}"#;
    /// The debugger's state.
    State => "debug.state", "Debugger State",
        "Returns the debugger as JSON: breakpoints with their state and icons, exception breakpoints, mute, sessions, the properties popup, the gutter tooltip and the Breakpoints dialog.";
}

ui::ui_commands! {
    POPUP_COMMANDS, scope = "BreakpointPopup";
    /// Done.
    PopupDone => "breakpoint-popup.done", "Done",
        "Closes the breakpoint's properties popup, keeping its changes.";
    /// More.
    PopupMore => "breakpoint-popup.more", "More",
        "Closes the properties popup and opens the breakpoint in the Breakpoints dialog.";
    /// A check box or radio button of the popup.
    PopupToggle { option: String } => "breakpoint-popup.toggle", "Toggle Option",
        "Clicks a check box or radio button of the properties popup: enabled, suspend, all, thread, make-default.",
        r#"{"option": "suspend"}"#;
    /// The condition's history.
    PopupHistory => "breakpoint-popup.history", "Show History",
        "Opens the list of conditions entered before under the condition field.";
    /// A condition of the history.
    PopupSetCondition { text: String } => "breakpoint-popup.set-condition", "Set Condition",
        "Puts a condition into the popup's condition field (a row of its history).",
        r#"{"text": "i == 3"}"#;
}

ui::ui_commands! {
    DIALOG_COMMANDS, scope = "BreakpointsDialog";
    /// Selects a row of the tree.
    DialogSelect {
        index: Option<usize>,
        file: Option<String>,
        line: Option<u32>,
        class: Option<String>,
    } => "breakpoints-dialog.select", "Select",
        "Selects a row of the Breakpoints dialog's tree by its index, or a breakpoint by its file and line or its exception class.",
        r#"{"file": "main.py", "line": 7}"#;
    DialogPrevious => "breakpoints-dialog.previous", "Up", "Selects the row above.";
    DialogNext => "breakpoints-dialog.next", "Down", "Selects the row below.";
    DialogToggleEnabled => "breakpoints-dialog.toggle-enabled", "Toggle Enabled",
        "Enables or disables the selected breakpoints (the check box of the row; all of a group).";
    DialogRemove => "breakpoints-dialog.remove", "Remove",
        "Removes the selected breakpoint (a default exception breakpoint stays).";
    DialogGoToSource => "breakpoints-dialog.go-to-source", "Jump to Source",
        "Opens the selected line breakpoint's file at its line and closes the dialog.";
    DialogAdd => "breakpoints-dialog.add", "Add",
        "Opens the list of breakpoint types that can be added here (Python Exception Breakpoint).";
    DialogChooseClass => "breakpoints-dialog.choose-class", "Select Exception Class",
        "Opens the class chooser of a new exception breakpoint (a dialog with a name field and the matching classes).";
    /// Adds an exception breakpoint and selects it.
    DialogAddException { class: String } => "breakpoints-dialog.add-exception", "Add Exception Breakpoint",
        "Adds an exception breakpoint for a class (a row of the class chooser) and selects it.",
        r#"{"class": "KeyError"}"#;
    /// Changes the selected breakpoint.
    DialogSet {
        enabled: Option<bool>,
        suspend: Option<String>,
        condition: Option<String>,
        condition_enabled: Option<bool>,
        option: Option<String>,
        value: Option<bool>,
    } => "breakpoints-dialog.set", "Set Property",
        "Changes the selected breakpoint: enabled, suspend (all, thread, none), condition and its check box, an exception option (on termination: notifyOnTerminate) with value.",
        r#"{"condition": "i == 3"}"#;
    DialogDone => "breakpoints-dialog.done", "Done", "Closes the Breakpoints dialog.";
}

ui::ui_commands! {
    CHOOSER_COMMANDS, scope = "ExceptionClassChooser";
    ChooserOk => "exception-class-chooser.ok", "OK",
        "Adds an exception breakpoint for the class selected in the class chooser (or the dotted class name typed) and closes the chooser.";
    ChooserCancel => "exception-class-chooser.cancel", "Cancel",
        "Closes the class chooser without adding a breakpoint.";
    ChooserPrevious => "exception-class-chooser.previous", "Up", "Selects the class above.";
    ChooserNext => "exception-class-chooser.next", "Down", "Selects the class below.";
    /// Replaces the text of the name field.
    ChooserSetQuery { text: String } => "exception-class-chooser.set-query", "Set Class Name",
        "Replaces the text of the class chooser's name field and filters the classes by it.",
        r#"{"text": "ValueError"}"#;
}

/// Registers the commands.
pub fn register(cx: &mut gpui::App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
    ui::command::register(cx, DIALOG_COMMANDS);
    ui::command::register(cx, CHOOSER_COMMANDS);
}
