//! Keystrokes of the view to the keys the session encodes (xterm). Keys
//! with the platform modifier (Cmd, Windows) belong to the IDE; printable
//! text without Control or Meta goes through the input method (typed and
//! composed text, `replace_text_in_range`), so dead keys and IMEs work.

use gpui::Keystroke;
use terminal::{Key, Modifiers};

/// The session key of a keystroke, or `None` when the keystroke is the
/// IDE's or text for the input method.
///
/// `option_as_meta`: "Use Option as Meta key" (macOS; off by default, so
/// Option composes characters). Alt is always Meta elsewhere.
pub fn session_key(keystroke: &Keystroke, option_as_meta: bool) -> Option<(Key, Modifiers)> {
    let m = &keystroke.modifiers;
    if m.platform {
        return None;
    }
    let meta = m.alt && (option_as_meta || !cfg!(target_os = "macos"));
    let mods = Modifiers {
        shift: m.shift,
        alt: m.alt,
        control: m.control,
    };
    let named = match keystroke.key.as_str() {
        "enter" => Some(Key::Enter),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "escape" => Some(Key::Escape),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        "insert" => Some(Key::Insert),
        "delete" => Some(Key::Delete),
        key => key
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .filter(|n| (1..=12).contains(n))
            .map(Key::F),
    };
    if let Some(key) = named {
        // Option still modifies named keys (they compose nothing).
        return Some((key, mods));
    }
    if !m.control && !meta {
        // Printable text: the input method's.
        return None;
    }
    let base = if keystroke.key == "space" {
        ' '
    } else {
        let mut chars = keystroke.key.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        c
    };
    // With Meta alone the shifted character is sent (Alt+B: ESC B).
    let c = if !m.control && m.shift {
        keystroke
            .key_char
            .as_deref()
            .and_then(|k| k.chars().next())
            .filter(|c| !c.is_control())
            .unwrap_or_else(|| base.to_ascii_uppercase())
    } else {
        base
    };
    Some((Key::Char(c), Modifiers { alt: meta, ..mods }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Option<(Key, Modifiers)> {
        session_key(&Keystroke::parse(s).unwrap(), false)
    }

    const NONE: Modifiers = Modifiers::NONE;

    #[test]
    fn named_keys_go_to_the_session() {
        assert_eq!(key("enter"), Some((Key::Enter, NONE)));
        assert_eq!(key("escape"), Some((Key::Escape, NONE)));
        assert_eq!(
            key("shift-tab"),
            Some((
                Key::Tab,
                Modifiers {
                    shift: true,
                    ..NONE
                }
            ))
        );
        assert_eq!(key("f5"), Some((Key::F(5), NONE)));
        assert_eq!(key("f13"), None);
        assert_eq!(
            key("ctrl-up"),
            Some((
                Key::Up,
                Modifiers {
                    control: true,
                    ..NONE
                }
            ))
        );
    }

    #[test]
    fn control_combinations_and_text() {
        assert_eq!(
            key("ctrl-c"),
            Some((
                Key::Char('c'),
                Modifiers {
                    control: true,
                    ..NONE
                }
            ))
        );
        assert_eq!(
            key("ctrl-space"),
            Some((
                Key::Char(' '),
                Modifiers {
                    control: true,
                    ..NONE
                }
            ))
        );
        // Plain and shifted characters are text for the input method.
        assert_eq!(key("a"), None);
        assert_eq!(key("shift-a"), None);
        assert_eq!(key("space"), None);
        // The platform key is the IDE's.
        assert_eq!(key("cmd-c"), None);
        assert_eq!(key("cmd-enter"), None);
    }

    #[test]
    fn option_is_meta_only_when_asked_on_macos() {
        let alt_b = Keystroke::parse("alt-b").unwrap();
        let meta = Some((Key::Char('b'), Modifiers { alt: true, ..NONE }));
        assert_eq!(session_key(&alt_b, true), meta);
        if cfg!(target_os = "macos") {
            assert_eq!(session_key(&alt_b, false), None);
        } else {
            assert_eq!(session_key(&alt_b, false), meta);
        }
        // Named keys keep Option as a modifier either way.
        assert_eq!(
            session_key(&Keystroke::parse("alt-left").unwrap(), false),
            Some((Key::Left, Modifiers { alt: true, ..NONE }))
        );
    }
}
