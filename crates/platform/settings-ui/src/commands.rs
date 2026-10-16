//! Commands of the Settings window and the settings store.

ui::ui_commands! {
    OPEN_COMMANDS, scope = "Workbench";
    /// Open the Settings window.
    OpenSettings { page: Option<String>, filter: Option<String> } => "settings.open", "Settings\u{2026}",
        "Opens the Settings window (one per application; again: brought to the front, or opened anew for another project after asking about unsaved changes) on a page by id (`system`, `editor.font`), else on the last one; `filter` searches the pages.",
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
    /// Search the pages.
    Search { text: String } => "settings.search", "Search Settings",
        "Searches the pages as typing in the search field does: the tree keeps the pages found (red field: none) and selects the first; `\"\"` clears the search.",
        r#"{"text": "wrap"}"#;
    /// Focus the search field.
    Find => "settings.find", "Find",
        "Puts the focus in the search field and selects its text.";
    /// Escape: clear the search, else close.
    Escape => "settings.escape", "Clear Search or Cancel",
        "Clears the search when there is one (Escape); otherwise closes the window like Cancel.";
    /// The page selected before.
    Back => "settings.back", "Back",
        "Goes back to the page selected before; the search is cleared.";
    /// The page Back left.
    Forward => "settings.forward", "Forward",
        "Goes forward to the page Back left; the search is cleared.";
    /// The next row of the tree.
    SelectNext => "settings.select-next", "Next Page",
        "Selects the next row of the page tree (Down, in the search field too).";
    /// The previous row of the tree.
    SelectPrevious => "settings.select-previous", "Previous Page",
        "Selects the previous row of the page tree (Up, in the search field too).";
    /// Expand the selected group.
    Expand => "settings.expand", "Expand",
        "Expands the selected group of the tree, or selects its first page when it is expanded.";
    /// Collapse the selected group.
    Collapse => "settings.collapse", "Collapse",
        "Collapses the selected group of the tree, or selects the group of the selected page.";
    /// From the search field to the page.
    GoToPage => "settings.go-to-page", "Go to Page",
        "Moves the focus from the search field to the selected page (Enter there).";
    /// The recent searches.
    SearchHistory => "settings.search-history", "Search History",
        "Shows the recent searches below the search field; choosing one searches for it.";
}

ui::ui_commands! {
    QUERY_COMMANDS, scope = "Workbench";
    /// The state of the Settings window.
    State => "settings.state", "Settings Window State",
        "Returns the Settings window's state as JSON: open, title, selected page, the tree's rows, the search (text, no match, the sticky group header), Back and Forward, modified pages, errors, the selected page's fields, the window's size and split; and, open or not, `appearance`: the theme and editor color scheme in use, the theme shown (a preview), the themes and imported schemes.";
    /// Read settings.
    Get { key: Option<String> } => "settings.get", "Get Setting",
        "Returns an application setting by dotted key (`system.files.idle-seconds`) or a table of them; without a key, all of them.",
        r#"{"key": "system.files"}"#;
    /// Change a setting.
    Set { key: String, value: serde_json::Value } => "settings.set", "Set Setting",
        "Changes an application setting by dotted key; it applies at once and is saved.",
        r#"{"key": "system.files.idle-seconds", "value": 30}"#;
}

ui::ui_commands! {
    THEME_COMMANDS, scope = "Workbench";
    /// Switch the theme.
    SwitchTheme { id: Option<String> } => "settings.switch-theme", "Theme",
        "Switches to a theme by id at once, as the Theme combo box and the theme switchers do: it is stored, and Cancel in the Settings window does not undo it. Without an id, shows the Theme popup: the themes in groups with the current one selected; half a second after the selection moves the IDE previews that theme, Escape restores the starting one.",
        r#"{"id": "islands-light"}"#;
    /// The Switch popup.
    QuickSwitch => "settings.quick-switch", "Quick Switch Scheme\u{2026}",
        "Shows the Switch popup (Quick Switch Scheme): its Theme row opens the Theme popup.";
    /// Import a theme.
    ImportTheme { path: Option<String> } => "settings.import-theme", "Import Theme\u{2026}",
        "Imports a PyCharm UI theme (`.theme.json`), colour scheme (`.icls`, `.xml`), theme plugin (`.jar`, `.zip`) or a folder of them into the `themes` folder of the configuration: the first UI theme becomes the theme at once, a scheme alone is offered as an editor colour scheme, and a balloon reports what could not be mapped (an error balloon when nothing could be imported). Without a path a file chooser is shown.",
        r#"{"path": "/path/to/MyTheme.jar"}"#;
}
