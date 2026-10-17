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
