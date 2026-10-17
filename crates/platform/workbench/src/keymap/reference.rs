//! Keymaps of the reference IDE (ADR 0018): its XML format (the VS Code
//! keymaps that ship, custom keymaps people import) and the conversion to a
//! derived keymap through the action table.
//!
//! ```xml
//! <keymap version="1" name="VSCode" parent="$default">
//!   <action id="EditorDuplicateLines">
//!     <keyboard-shortcut first-keystroke="shift alt down" />
//!   </action>
//!   <action id="ActivateMessagesToolWindow" />   <!-- removed -->
//! </keymap>
//! ```
//!
//! An action listed replaces all of the parent's shortcuts of that action;
//! one without shortcuts removes them. Keystrokes are Java `KeyStroke`
//! texts (`shift ctrl D`, `meta BACK_QUOTE`), mouse shortcuts name buttons
//! (`button4`), a double tap is a `keyboard-gesture-shortcut`.

use quick_xml::events::{BytesStart, Event};

use super::actions::ActionTable;
use super::derived::{Derived, Override, canonical_keys};
use super::{MACOS_NAME, VSCODE_MACOS_NAME, VSCODE_NAME, WINDOWS_NAME};

/// The VS Code keymaps of the reference, unmodified (see NOTICE).
pub const VSCODE: &str = include_str!("../../keymaps/reference/VSCode.xml");
pub const VSCODE_MACOS: &str = include_str!("../../keymaps/reference/VSCode OSX.xml");

/// A shortcut of the reference's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortcut {
    /// Keystrokes in GPUI's syntax (`ctrl-shift-d`, `ctrl-k ctrl-c`,
    /// `shift shift`, `button4`).
    Keys(String),
    /// A shortcut Pyoxide cannot bind (a mouse click with modifiers, a
    /// key it does not know), as the file writes it.
    Unsupported(String),
}

/// A keymap file of the reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceKeymap {
    pub name: String,
    /// The reference's name of the parent (`$default`, `Mac OS X 10.5+`).
    pub parent: Option<String>,
    /// The actions in the file's order; an action listed twice has the
    /// shortcuts of its last entry.
    pub actions: Vec<(String, Vec<Shortcut>)>,
}

fn attributes(start: &BytesStart<'_>) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| e.to_string())?;
        out.push((attr.key.as_ref().to_owned(), value.into_owned()));
    }
    Ok(out)
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

impl ReferenceKeymap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut reader = quick_xml::Reader::from_str(text);
        let mut keymap: Option<ReferenceKeymap> = None;
        let mut action: Option<(String, Vec<Shortcut>)> = None;
        loop {
            let event = reader
                .read_event()
                .map_err(|e| format!("{e} (at byte {})", reader.error_position()))?;
            let (start, empty) = match &event {
                Event::Start(s) => (s, false),
                Event::Empty(s) => (s, true),
                Event::End(e) => {
                    if e.name().as_ref() == "action"
                        && let (Some(k), Some(a)) = (keymap.as_mut(), action.take())
                    {
                        k.push(a);
                    }
                    continue;
                }
                Event::Eof => break,
                _ => continue,
            };
            let attrs = attributes(start)?;
            match start.name().as_ref() {
                "keymap" => {
                    keymap = Some(ReferenceKeymap {
                        name: attr(&attrs, "name")
                            .ok_or("the keymap has no name")?
                            .to_owned(),
                        parent: attr(&attrs, "parent").map(str::to_owned),
                        actions: Vec::new(),
                    });
                }
                "action" => {
                    let id = attr(&attrs, "id").ok_or("an action has no id")?.to_owned();
                    if empty {
                        if let Some(k) = keymap.as_mut() {
                            k.push((id, Vec::new()));
                        }
                    } else {
                        action = Some((id, Vec::new()));
                    }
                }
                "keyboard-shortcut" => {
                    let Some((_, shortcuts)) = action.as_mut() else {
                        continue;
                    };
                    let first = attr(&attrs, "first-keystroke").unwrap_or_default();
                    let second = attr(&attrs, "second-keystroke");
                    let text = match second {
                        Some(second) => format!("{first}, {second}"),
                        None => first.to_owned(),
                    };
                    let keys = java_keystroke(first).and_then(|f| match second {
                        Some(second) => java_keystroke(second).map(|s| format!("{f} {s}")),
                        None => Some(f),
                    });
                    shortcuts.push(keys.map_or(Shortcut::Unsupported(text), Shortcut::Keys));
                }
                "mouse-shortcut" => {
                    let Some((_, shortcuts)) = action.as_mut() else {
                        continue;
                    };
                    let text = attr(&attrs, "keystroke").unwrap_or_default().trim();
                    shortcuts.push(match text {
                        "button4" | "button5" => Shortcut::Keys(text.to_owned()),
                        _ => Shortcut::Unsupported(text.to_owned()),
                    });
                }
                "keyboard-gesture-shortcut" => {
                    let Some((_, shortcuts)) = action.as_mut() else {
                        continue;
                    };
                    let text = attr(&attrs, "keystroke").unwrap_or_default();
                    let double = attr(&attrs, "modifier") == Some("dblClick");
                    let modifier = text.split_whitespace().last().map(str::to_ascii_lowercase);
                    let keys = match modifier.as_deref() {
                        Some(m @ ("shift" | "ctrl" | "alt")) if double => Some(format!("{m} {m}")),
                        Some("control") if double => Some("ctrl ctrl".to_owned()),
                        Some("meta") if double => Some("cmd cmd".to_owned()),
                        _ => None,
                    };
                    let keys = keys.filter(|k| super::parse_double_tap(k).is_some());
                    shortcuts
                        .push(keys.map_or(Shortcut::Unsupported(text.to_owned()), Shortcut::Keys));
                }
                _ => {}
            }
        }
        keymap.ok_or_else(|| "not a keymap file".into())
    }

    fn push(&mut self, (id, shortcuts): (String, Vec<Shortcut>)) {
        match self.actions.iter_mut().find(|(a, _)| *a == id) {
            Some(entry) => entry.1 = shortcuts,
            None => self.actions.push((id, shortcuts)),
        }
    }
}

/// A Java `KeyStroke` text of the reference (`shift ctrl D`,
/// `meta BACK_QUOTE`, `alt pressed F7`) in GPUI's syntax (`ctrl-shift-d`,
/// `cmd-\``, `alt-f7`); `None` for a key Pyoxide cannot bind. Shift with a
/// symbol or digit is the shifted character of a US layout, as GPUI
/// reports it (`shift ctrl CLOSE_BRACKET` is `ctrl-}`).
pub fn java_keystroke(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (key, modifiers) = words.split_last()?;
    let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
    for m in modifiers {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            "meta" => cmd = true,
            "pressed" => {}
            _ => return None,
        }
    }
    let upper = key.to_ascii_uppercase();
    let upper = upper.strip_prefix("VK_").unwrap_or(&upper);
    let mut key = match upper {
        k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => k.to_ascii_lowercase(),
        k if k.starts_with('F') && k[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            k.to_ascii_lowercase()
        }
        k if k.starts_with("NUMPAD") && k.len() == 7 => k[6..].to_owned(),
        "ENTER" => "enter".into(),
        "ESCAPE" => "escape".into(),
        "TAB" => "tab".into(),
        "SPACE" => "space".into(),
        "BACK_SPACE" => "backspace".into(),
        "DELETE" => "delete".into(),
        "INSERT" => "insert".into(),
        "HOME" => "home".into(),
        "END" => "end".into(),
        "PAGE_UP" => "pageup".into(),
        "PAGE_DOWN" => "pagedown".into(),
        "UP" | "KP_UP" => "up".into(),
        "DOWN" | "KP_DOWN" => "down".into(),
        "LEFT" | "KP_LEFT" => "left".into(),
        "RIGHT" | "KP_RIGHT" => "right".into(),
        "CONTEXT_MENU" => "menu".into(),
        "BACK_QUOTE" => "`".into(),
        "MINUS" | "SUBTRACT" => "-".into(),
        "EQUALS" => "=".into(),
        "PLUS" | "ADD" => "+".into(),
        "MULTIPLY" | "ASTERISK" => "*".into(),
        "DIVIDE" | "SLASH" => "/".into(),
        "BACK_SLASH" => "\\".into(),
        "OPEN_BRACKET" => "[".into(),
        "CLOSE_BRACKET" => "]".into(),
        "SEMICOLON" => ";".into(),
        "QUOTE" => "'".into(),
        "COMMA" => ",".into(),
        "PERIOD" | "DECIMAL" => ".".into(),
        _ => return None,
    };
    if shift && let Some(shifted) = us_shifted(&key) {
        key = shifted.to_owned();
        shift = false;
    }
    let mut out = String::new();
    for (on, name) in [
        (ctrl, "ctrl-"),
        (alt, "alt-"),
        (shift, "shift-"),
        (cmd, "cmd-"),
    ] {
        if on {
            out.push_str(name);
        }
    }
    out.push_str(&key);
    Some(out)
}

/// The character a key gives with Shift on a US layout.
fn us_shifted(key: &str) -> Option<&'static str> {
    Some(match key {
        "`" => "~",
        "1" => "!",
        "2" => "@",
        "3" => "#",
        "4" => "$",
        "5" => "%",
        "6" => "^",
        "7" => "&",
        "8" => "*",
        "9" => "(",
        "0" => ")",
        "-" => "_",
        "=" => "+",
        "[" => "{",
        "]" => "}",
        "\\" => "|",
        ";" => ":",
        "'" => "\"",
        "," => "<",
        "." => ">",
        "/" => "?",
        _ => return None,
    })
}

/// The Pyoxide keymap a keymap of the reference names as its parent, and
/// whether it only approximates it (the Linux keymaps and the classic
/// macOS keymap, which Pyoxide does not have).
pub fn builtin_parent(reference: &str) -> Option<(&'static str, bool)> {
    Some(match reference {
        "$default" => (WINDOWS_NAME, false),
        "Mac OS X 10.5+" => (MACOS_NAME, false),
        "VSCode" => (VSCODE_NAME, false),
        "VSCode OSX" => (VSCODE_MACOS_NAME, false),
        "Default for GNOME" | "Default for KDE" | "Default for XWin" => (WINDOWS_NAME, true),
        "Mac OS X" => (MACOS_NAME, true),
        _ => return None,
    })
}

/// The shortcuts of an action Pyoxide does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmapped {
    pub action: String,
    /// As the file writes them.
    pub shortcuts: Vec<String>,
}

/// A keymap of the reference converted to a derived keymap.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub derived: Derived,
    /// Actions with shortcuts that Pyoxide does not have.
    pub unmapped: Vec<Unmapped>,
    /// Shortcuts of mapped actions that Pyoxide cannot bind: `id: text`.
    pub unsupported: Vec<String>,
}

fn shortcut_text(s: &Shortcut) -> String {
    match s {
        Shortcut::Keys(k) => display_keys(k),
        Shortcut::Unsupported(k) => k.clone(),
    }
}

/// Keystrokes as the platform shows them (`⌥⌘J`, `Ctrl+K, Ctrl+C`,
/// `Double ⇧`).
pub fn display_keys(keys: &str) -> String {
    let macos = cfg!(target_os = "macos");
    if let Some(key) = super::parse_double_tap(keys) {
        return super::double_tap_text(key, macos);
    }
    if super::parse_mouse_button(keys).is_some() {
        let n = if keys == "button4" { 4 } else { 5 };
        return format!("Button {n} Click");
    }
    keys.split(' ')
        .map(|stroke| match gpui::Keystroke::parse(stroke) {
            Ok(k) => ui::keys::format_keystroke(&k, macos),
            Err(_) => stroke.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Converts `keymap` into a derived keymap named `name` on top of the
/// Pyoxide keymap `parent`: each mapped action's shortcuts become the
/// bindings of its commands, in their contexts.
pub fn convert(
    keymap: &ReferenceKeymap,
    name: &str,
    parent: &str,
    table: &ActionTable,
) -> Converted {
    let mut overrides = Vec::new();
    let mut unmapped = Vec::new();
    let mut unsupported = Vec::new();
    // An action that uses another's shortcuts has them when the keymap
    // lists that one and not the action itself.
    let mut actions: Vec<(String, Vec<Shortcut>)> = keymap.actions.clone();
    for action in &table.actions {
        let Some(base) = &action.use_shortcut_of else {
            continue;
        };
        if keymap.actions.iter().any(|(id, _)| *id == action.id) {
            continue;
        }
        if let Some((_, shortcuts)) = keymap.actions.iter().find(|(id, _)| id == base) {
            actions.push((action.id.clone(), shortcuts.clone()));
        }
    }
    for (id, shortcuts) in &actions {
        let Some(action) = table.get(id) else {
            if !shortcuts.is_empty() {
                unmapped.push(Unmapped {
                    action: id.clone(),
                    shortcuts: shortcuts.iter().map(shortcut_text).collect(),
                });
            }
            continue;
        };
        let mut keys: Vec<String> = Vec::new();
        for shortcut in shortcuts {
            match shortcut {
                Shortcut::Keys(k) => {
                    let canonical = canonical_keys(k);
                    if !keys.iter().any(|have| canonical_keys(have) == canonical) {
                        keys.push(k.clone());
                    }
                }
                Shortcut::Unsupported(text) => unsupported.push(format!("{id}: {text}")),
            }
        }
        for target in &action.targets {
            for context in &target.contexts {
                overrides.push(Override {
                    command: target.command.clone(),
                    args: target.args.clone(),
                    context: context.clone(),
                    keys: keys.clone(),
                });
            }
        }
    }
    Converted {
        derived: Derived {
            name: name.to_owned(),
            parent: parent.to_owned(),
            overrides,
        },
        unmapped,
        unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_keystrokes_in_gpui_syntax() {
        let cases = [
            ("shift ctrl d", Some("ctrl-shift-d")),
            ("ctrl back_quote", Some("ctrl-`")),
            ("meta UP", Some("cmd-up")),
            ("alt shift F10", Some("alt-shift-f10")),
            ("control Y", Some("ctrl-y")),
            ("shift ctrl CLOSE_BRACKET", Some("ctrl-}")),
            ("shift alt 0", Some("alt-)")),
            ("ctrl add", Some("ctrl-+")),
            ("ctrl subtract", Some("ctrl--")),
            ("alt pressed F7", Some("alt-f7")),
            ("CONTEXT_MENU", Some("menu")),
            ("shift TAB", Some("shift-tab")),
            ("ctrl PAUSE", None),
            ("altGraph A", None),
            ("", None),
        ];
        for (java, gpui) in cases {
            assert_eq!(java_keystroke(java).as_deref(), gpui, "{java}");
        }
    }

    #[test]
    fn parses_actions_shortcuts_and_removals() {
        let xml = r#"<keymap version="1" name="Mine" parent="Mac OS X 10.5+">
  <action id="EditorDuplicate">
    <keyboard-shortcut first-keystroke="meta D"/>
    <keyboard-shortcut first-keystroke="ctrl K" second-keystroke="ctrl D"/>
  </action>
  <action id="Back">
    <mouse-shortcut keystroke="button4"/>
    <mouse-shortcut keystroke="ctrl button1"/>
  </action>
  <action id="SearchEverywhere">
    <keyboard-gesture-shortcut keystroke="shift SHIFT" modifier="dblClick"/>
  </action>
  <action id="EditorDeleteLine"/>
  <action id="NotInPyoxide">
    <keyboard-shortcut first-keystroke="meta alt J"/>
  </action>
  <action id="EditorDuplicate">
    <keyboard-shortcut first-keystroke="meta shift D"/>
  </action>
</keymap>"#;
        let keymap = ReferenceKeymap::parse(xml).unwrap();
        assert_eq!(keymap.name, "Mine");
        assert_eq!(keymap.parent.as_deref(), Some("Mac OS X 10.5+"));
        assert_eq!(keymap.actions.len(), 5);
        // Listed twice: the last entry counts.
        assert_eq!(
            keymap.actions[0],
            (
                "EditorDuplicate".to_owned(),
                vec![Shortcut::Keys("shift-cmd-d".into())]
            )
        );
        assert_eq!(
            keymap.actions[1].1,
            vec![
                Shortcut::Keys("button4".into()),
                Shortcut::Unsupported("ctrl button1".into())
            ]
        );
        assert_eq!(
            keymap.actions[2].1,
            vec![Shortcut::Keys("shift shift".into())]
        );
        assert_eq!(keymap.actions[3].1, vec![]);

        let converted = convert(&keymap, "Mine", MACOS_NAME, ActionTable::builtin());
        let derived = &converted.derived;
        assert_eq!(derived.parent, MACOS_NAME);
        let duplicate = derived
            .overrides
            .iter()
            .find(|o| o.command == "editor.duplicate")
            .unwrap();
        assert_eq!(duplicate.context.as_deref(), Some("Editor"));
        assert_eq!(duplicate.keys, vec!["shift-cmd-d"]);
        // Back runs in the main window and in the Settings window.
        let back: Vec<_> = derived
            .overrides
            .iter()
            .filter(|o| o.keys == ["button4"])
            .map(|o| o.command.as_str())
            .collect();
        assert_eq!(back, ["workbench.back", "settings.back"]);
        let delete = derived
            .overrides
            .iter()
            .find(|o| o.command == "editor.delete-line")
            .unwrap();
        assert!(delete.keys.is_empty());
        let mac = cfg!(target_os = "macos");
        assert_eq!(
            converted.unmapped,
            vec![Unmapped {
                action: "NotInPyoxide".into(),
                shortcuts: vec![if mac { "⌥⌘J" } else { "Alt+Win+J" }.into()]
            }]
        );
        assert_eq!(converted.unsupported, vec!["Back: ctrl button1"]);
        // The editor's Delete has the shortcuts of `$Delete` unless the
        // keymap lists it.
        let delete = |xml: &str| {
            let keymap = ReferenceKeymap::parse(xml).unwrap();
            let converted = convert(&keymap, "K", MACOS_NAME, ActionTable::builtin());
            converted
                .derived
                .overrides
                .iter()
                .filter(|o| o.command == "editor.delete")
                .map(|o| o.keys.clone())
                .collect::<Vec<_>>()
        };
        let both = r#"<keymap version="1" name="K" parent="Mac OS X 10.5+">
  <action id="$Delete"><keyboard-shortcut first-keystroke="delete"/><keyboard-shortcut first-keystroke="back_space"/></action>
</keymap>"#;
        assert_eq!(delete(both), vec![vec!["delete", "backspace"]]);
        let own = r#"<keymap version="1" name="K" parent="Mac OS X 10.5+">
  <action id="$Delete"><keyboard-shortcut first-keystroke="delete"/><keyboard-shortcut first-keystroke="back_space"/></action>
  <action id="EditorDelete"><keyboard-shortcut first-keystroke="DELETE"/></action>
</keymap>"#;
        assert_eq!(delete(own), vec![vec!["delete"]]);
        assert!(ReferenceKeymap::parse("<keymap version=\"1\">").is_err());
        assert!(ReferenceKeymap::parse("<application/>").is_err());
    }

    #[test]
    fn parents_of_the_reference() {
        assert_eq!(builtin_parent("$default"), Some((WINDOWS_NAME, false)));
        assert_eq!(builtin_parent("Mac OS X 10.5+"), Some((MACOS_NAME, false)));
        assert_eq!(
            builtin_parent("VSCode OSX"),
            Some((VSCODE_MACOS_NAME, false))
        );
        assert_eq!(
            builtin_parent("Default for KDE"),
            Some((WINDOWS_NAME, true))
        );
        assert_eq!(builtin_parent("Emacs"), None);
    }
}
