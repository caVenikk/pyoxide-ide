//! Commands of code intelligence (ADR 0010). Titles follow the reference's
//! action texts (IntelliJ Platform Community, `ActionsBundle.properties`,
//! see NOTICE); its "Basic" of the Code Completion menu is "Basic
//! Completion" here. No default key bindings yet: each comes with its
//! feature and its keystroke check. Next and previous problem are editor
//! commands (`editor::commands`): the problems are the editor's marks.

ui::ui_commands! {
    COMMANDS, scope = "Editor";
    Complete => "code.complete", "Basic Completion",
        "Shows the completion popup for the identifier at the caret.";
    QuickDoc => "code.quick-doc", "Quick Documentation",
        "Shows the documentation of the symbol at the caret in a popup.";
    ParameterInfo => "code.parameter-info", "Parameter Info",
        "Shows the parameters of the call at the caret.";
    GoToDeclaration => "code.go-to-declaration", "Go to Declaration or Usages",
        "Goes to the declaration of the symbol at the caret; on the declaration, shows its usages.";
    ShowUsages => "code.show-usages", "Show Usages",
        "Shows the usages of the symbol at the caret in a popup.";
    ShowIntentions => "code.show-intentions", "Show Context Actions",
        "Shows the quick fixes and intention actions at the caret.";
    Reformat => "code.reformat", "Reformat Code",
        "Reformats the file, or the selected lines.";
    OptimizeImports => "code.optimize-imports", "Optimize Imports",
        "Sorts the imports of the file.";
    ErrorDescription => "code.error-description", "Error Description",
        "Shows the description of the problem at the caret.";
    ApplyFirstFix => "code.apply-first-fix", "Apply First Quick Fix",
        "Applies the first quick fix of the problem at the caret.";
}

ui::ui_commands! {
    DOCUMENTATION_COMMANDS, scope = "Documentation";
    CloseDocumentation => "documentation.close", "Close Documentation",
        "Closes the documentation popup.";
}

ui::ui_commands! {
    PARAMETER_INFO_COMMANDS, scope = "ParameterInfo";
    CloseParameterInfo => "parameter-info.close", "Close Parameter Info",
        "Closes the parameter info popup.";
}

ui::ui_commands! {
    COMPLETION_COMMANDS, scope = "Completion";
    SelectNext => "completion.select-next", "Down",
        "Selects the next item of the completion popup.";
    SelectPrevious => "completion.select-previous", "Up",
        "Selects the previous item of the completion popup.";
    PageDown => "completion.page-down", "Page Down",
        "Selects the item a page further down in the completion popup.";
    PageUp => "completion.page-up", "Page Up",
        "Selects the item a page further up in the completion popup.";
    First => "completion.first", "First Item",
        "Selects the first item of the completion popup.";
    Last => "completion.last", "Last Item",
        "Selects the last item of the completion popup.";
    Accept => "completion.accept", "Choose Lookup Item",
        "Inserts the selected completion item at the caret.";
    Replace => "completion.replace", "Choose Lookup Item Replace",
        "Inserts the selected completion item in place of the identifier at the caret.";
    Cancel => "completion.cancel", "Close Completion",
        "Closes the completion popup.";
}

ui::ui_commands! {
    SERVICE_COMMANDS, scope = "Workbench";
    State => "code.state", "Code Intelligence State",
        "Returns the code intelligence state of the window as JSON: the popups, the requests in flight and the problems of the active file.";
    Servers => "code.servers", "Language Services",
        "Returns the language servers of the project as JSON: name, version, state, roots, features and documents.";
    RestartServers { server: Option<String> } => "code.restart-servers", "Restart Language Servers",
        "Restarts one language server of the project, or all of them.",
        r#"{"server": "ruff"}"#;
    StopServer { server: String } => "code.stop-server", "Stop Language Server",
        "Stops a language server; it starts again when a file needs it.",
        r#"{"server": "ruff"}"#;
}
