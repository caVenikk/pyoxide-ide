//! Commands of the Settings window and the settings store.

ui::ui_commands! {
    OPEN_COMMANDS, scope = "Workbench";
    /// Open the Settings window.
    OpenSettings { page: Option<String>, filter: Option<String> } => "settings.open", "Settings\u{2026}",
        "Opens the Settings window (one per application; again: brought to the front) on a page by id (`system`, `editor.font`), else on the last one; `filter` searches for it.",
        r#"{"page": "system"}"#;
}

ui::ui_commands! {
    COMMANDS, scope = "SettingsWindow";
    /// Select a page.
    SelectPage { page: String } => "settings.select", "Select Page",
        "Selects a page of the Settings window by id.",
        r#"{"page": "system"}"#;
    /// Change a field of the selected page.
    EditField { field: String, value: serde_json::Value } => "settings.edit", "Change Field",
        "Changes a field of the selected page (not applied yet), as typing or clicking it would.",
        r#"{"field": "idle-seconds", "value": 30}"#;
    /// Apply the changes.
    Apply => "settings.apply", "Apply",
        "Applies the changes of every modified page and keeps the window open; a page that fails stays selected with its error.";
    /// Apply and close.
    Confirm => "settings.ok", "OK",
        "Applies the changes and closes the Settings window.";
    /// Close without applying.
    Cancel => "settings.cancel", "Cancel",
        "Closes the Settings window; changes not applied are dropped.";
    /// Revert the selected page.
    Revert => "settings.revert", "Revert Changes",
        "Puts the selected page back to the stored settings.";
}

ui::ui_commands! {
    QUERY_COMMANDS, scope = "Workbench";
    /// The state of the Settings window.
    State => "settings.state", "Settings Window State",
        "Returns the Settings window's state as JSON: open, title, selected page, pages, modified pages, errors, the selected page's fields.";
    /// Read settings.
    Get { key: Option<String> } => "settings.get", "Get Setting",
        "Returns an application setting by dotted key (`system.files.idle-seconds`) or a table of them; without a key, all of them.",
        r#"{"key": "system.files"}"#;
    /// Change a setting.
    Set { key: String, value: serde_json::Value } => "settings.set", "Set Setting",
        "Changes an application setting by dotted key; it applies at once and is saved.",
        r#"{"key": "system.files.idle-seconds", "value": 30}"#;
}
