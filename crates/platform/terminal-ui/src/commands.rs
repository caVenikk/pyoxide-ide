//! The commands of a focused terminal (key context `Terminal`), named as
//! the reference names its actions (`Terminal.CopySelectedText`,
//! `Terminal.Paste`, `Terminal.SelectAll`, `Terminal.ClearBuffer`,
//! `Terminal.LineUp`, `Terminal.LineDown`, `Terminal.PageUp`,
//! `Terminal.PageDown`).

ui::ui_commands! {
    COMMANDS, scope = "Terminal";
    /// Copy the selected text.
    Copy => "terminal.copy", "Copy",
        "Copies the selected text of the terminal; without a selection the key goes to the shell.";
    /// Paste the clipboard.
    Paste => "terminal.paste", "Paste",
        "Pastes the clipboard into the terminal, bracketed when the program asked for it.";
    /// Select everything.
    SelectAll => "terminal.select-all", "Select All",
        "Selects all output of the terminal, the scrollback included.";
    /// Clear the terminal.
    Clear => "terminal.clear", "Clear Terminal",
        "Clears the scrollback and the screen above the prompt line, which moves to the top.";
    /// Scroll one line up.
    LineUp => "terminal.line-up", "Line Up", "Scrolls the terminal one line up.";
    /// Scroll one line down.
    LineDown => "terminal.line-down", "Line Down", "Scrolls the terminal one line down.";
    /// Scroll one page up.
    PageUp => "terminal.page-up", "Page Up", "Scrolls the terminal one page up.";
    /// Scroll one page down.
    PageDown => "terminal.page-down", "Page Down", "Scrolls the terminal one page down.";
}

ui::ui_commands! {
    TAB_COMMANDS, scope = "Terminal";
    /// Open a new tab.
    NewTab => "terminal.new-tab", "New Tab",
        "Starts a new terminal session in a new tab of the Terminal tool window and selects it.";
    /// Close the selected tab.
    CloseTab => "terminal.close-tab", "Close Tab",
        "Closes the selected tab of the Terminal tool window and ends its session, after a confirmation when a command runs in the shell; the last tab closed hides the tool window.";
    /// Rename the selected tab.
    RenameTab { name: Option<String> } => "terminal.rename-tab", "Rename Session",
        "Renames the selected tab of the Terminal tool window: without `name` a balloon above the tab asks for the name (\"Session name:\", the name selected; Enter renames, an empty name is refused with an error outline, Escape or a click elsewhere cancels); with `name` the tab is renamed at once (an empty name is refused). The new name stays while programs set their titles.",
        r#"{"name": "build"}"#;
    /// Rename with the name in the balloon.
    RenameConfirm => "terminal.rename-confirm", "Rename",
        "Renames the tab to the name in the open rename balloon (Enter); an empty name is refused with an error outline.";
    /// Close the rename balloon.
    RenameCancel => "terminal.rename-cancel", "Cancel Rename",
        "Closes the open rename balloon without renaming (Escape).";
    /// The tool window's options menu.
    ShowOptions => "terminal.options-menu", "Options",
        "Opens the menu of the header's ⋮ button: Terminal Engine, Settings, Close All, Group Tabs (once the tool window had several tabs), View Mode, Move to, Resize and Remove from Sidebar.";
    /// Close every tab.
    CloseAllTabs => "terminal.close-all-tabs", "Close All",
        "Closes every tab of the Terminal tool window, each after a confirmation when a command runs in its shell; the tool window hides.";
    /// Group the tabs in a list.
    GroupTabs => "terminal.group-tabs", "Group Tabs",
        "Toggles Group Tabs: the header shows the selected tab's name with an arrow that opens the list of tabs instead of a tab per session.";
    /// Remove the tool window's stripe button.
    RemoveFromSidebar => "terminal.remove-from-sidebar", "Remove from Sidebar",
        "Hides the Terminal tool window and removes its button from the sidebar; activating the tool window brings the button back.";
    /// Choose the terminal engine.
    SelectEngine { engine: Option<String> } => "terminal.select-engine", "Terminal Engine",
        "Chooses the terminal engine. Only the reworked engine (`reworked`, \"Reworked\") exists, and it is in use.",
        r#"{"engine": "reworked"}"#;
    /// The Terminal settings.
    ShowSettings => "terminal.settings", "Settings",
        "Opens the Settings window (the reference opens its Tools > Terminal page).";
    /// Select the next tab.
    NextTab => "terminal.next-tab", "Select Next Tab",
        "Selects the next tab of the Terminal tool window (after the last, the first).";
    /// Select the previous tab.
    PreviousTab => "terminal.previous-tab", "Select Previous Tab",
        "Selects the previous tab of the Terminal tool window (before the first, the last).";
    /// Select a tab by its position.
    SelectTab { index: Option<usize> } => "terminal.select-tab", "Select Tab",
        "Selects the tab at `index` (counted from 0) of the Terminal tool window.",
        r#"{"index": 1}"#;
    /// Show the list of tabs.
    ShowTabList => "terminal.show-tab-list", "Show List of Tabs",
        "Opens the list of the tabs of the Terminal tool window under the selected tab (with a search by typing); choosing a row selects that tab. Needs more than one tab.";
}
