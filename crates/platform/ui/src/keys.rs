//! Keystrokes as shown in tooltips and hints: `⌥⇧⌘G` on macOS,
//! `Ctrl+Shift+F3` elsewhere.

use gpui::{Action, FocusHandle, KeyBinding, Keystroke, Window};

/// Text of one keystroke for the platform the IDE runs on.
pub fn keystroke_text(keystroke: &Keystroke) -> String {
    format_keystroke(keystroke, cfg!(target_os = "macos"))
}

/// Text of a keystroke in the macOS or the Windows and Linux style.
pub fn format_keystroke(keystroke: &Keystroke, macos: bool) -> String {
    let m = &keystroke.modifiers;
    let key = key_name(&keystroke.key, macos);
    if macos {
        let mut text = String::new();
        for (on, symbol) in [
            (m.control, '⌃'),
            (m.alt, '⌥'),
            (m.shift, '⇧'),
            (m.platform, '⌘'),
        ] {
            if on {
                text.push(symbol);
            }
        }
        text + &key
    } else {
        let mut parts: Vec<String> = Vec::new();
        for (on, name) in [
            (m.control, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.platform, "Win"),
        ] {
            if on {
                parts.push(name.to_owned());
            }
        }
        parts.push(key);
        parts.join("+")
    }
}

/// macOS shows the symbols of the reference's `MacKeymapUtil` (Return ⏎,
/// Delete ⌦, Home ↖, End ↘, Page Up ⇞, Page Down ⇟, ...).
fn key_name(key: &str, macos: bool) -> String {
    let named = match key {
        "enter" if macos => "⏎",
        "escape" if macos => "⎋",
        "backspace" if macos => "⌫",
        "tab" if macos => "⇥",
        "up" if macos => "↑",
        "down" if macos => "↓",
        "left" if macos => "←",
        "right" if macos => "→",
        "delete" if macos => "⌦",
        "home" if macos => "↖",
        "end" if macos => "↘",
        "pageup" if macos => "⇞",
        "pagedown" if macos => "⇟",
        "enter" => "Enter",
        "escape" => "Esc",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "space" => "Space",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page Up",
        "pagedown" => "Page Down",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The keystrokes of a binding as text (sequences separated by spaces).
pub fn binding_text(binding: &KeyBinding) -> String {
    let text: Vec<String> = binding
        .keystrokes()
        .iter()
        .map(|k| {
            let k = Keystroke {
                modifiers: *k.modifiers(),
                key: k.key().to_owned(),
                key_char: None,
            };
            keystroke_text(&k)
        })
        .collect();
    text.join(" ")
}

/// The shortcut of `action` that wins where `focus` is (as if it had the
/// focus), as text; `None` when the action has no binding there.
pub fn shortcut_in(action: &dyn Action, focus: &FocusHandle, window: &Window) -> Option<String> {
    window
        .highest_precedence_binding_for_action_in(action, focus)
        .map(|b| binding_text(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(keys: &str, macos: bool) -> String {
        format_keystroke(&Keystroke::parse(keys).unwrap(), macos)
    }

    #[test]
    fn formats_both_styles() {
        assert_eq!(text("cmd-g", true), "⌘G");
        assert_eq!(text("cmd-shift-g", true), "⇧⌘G");
        assert_eq!(text("ctrl-alt-x", true), "⌃⌥X");
        assert_eq!(text("shift-enter", true), "⇧⏎");
        assert_eq!(text("delete", true), "⌦");
        assert_eq!(text("cmd-home", true), "⌘↖");
        assert_eq!(text("end", true), "↘");
        assert_eq!(text("pageup", true), "⇞");
        assert_eq!(text("alt-pagedown", true), "⌥⇟");
        assert_eq!(text("space", true), "Space");
        assert_eq!(text("delete", false), "Delete");
        assert_eq!(text("pageup", false), "Page Up");
        assert_eq!(text("f3", false), "F3");
        assert_eq!(text("shift-f3", false), "Shift+F3");
        assert_eq!(text("ctrl-alt-e", false), "Ctrl+Alt+E");
        assert_eq!(text("escape", false), "Esc");
    }
}
