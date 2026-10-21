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
        "Closes the selected tab of the Terminal tool window and ends its session; the last tab closed hides the tool window.";
    /// Rename the selected tab.
    RenameTab { name: Option<String> } => "terminal.rename-tab", "Rename Session",
        "Renames the selected tab of the Terminal tool window to `name`; an empty name is refused.",
        r#"{"name": "build"}"#;
    /// Select the next tab.
    NextTab => "terminal.next-tab", "Select Next Tab",
        "Selects the next tab of the Terminal tool window (after the last, the first).";
    /// Select the previous tab.
    PreviousTab => "terminal.previous-tab", "Select Previous Tab",
        "Selects the previous tab of the Terminal tool window (before the first, the last).";
}
