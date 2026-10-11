//! Commands of the main window (ADR 0007).

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Quit the application.
    Quit => "app.quit", "Quit", "Closes all windows and quits the application.";
    /// Show or hide the project tool window.
    ToggleProject => "workbench.toggle-project", "Project",
        "Shows or hides the project tool window.";
    /// Switch the theme.
    SetTheme { id: String } => "workbench.set-theme", "Switch Theme",
        "Switches to a built-in theme by id.", r#"{"id": "pyoxide-light"}"#;
    /// Describe the window.
    State => "workbench.state", "Workbench State",
        "Returns the state of the main window as JSON (project, tool windows, theme).";
}
