//! xterm encoding of keys, pastes, mouse and focus events
//! (https://invisible-island.net/xterm/ctlseqs/ctlseqs.html): PC-style
//! function keys with modifier parameters, application cursor keys, Alt as
//! an ESC prefix, SGR and legacy mouse reports, bracketed paste, focus
//! reports.

use crate::snapshot::{Modes, MouseMode};

/// A key the view passes on to the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character key, as typed (already shifted: `A`, `!`).
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F12.
    F(u8),
}

/// Modifier keys held with a key or a mouse event.
///
/// `alt` is sent as Meta (an ESC prefix or the modifier parameter). On macOS
/// the view clears it when "Use Option as Meta key" is off and passes the
/// composed character instead. The platform key (Cmd, Windows) is never
/// encoded: keys with it belong to the IDE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        alt: false,
        control: false,
    };

    fn any(self) -> bool {
        self.shift || self.alt || self.control
    }

    /// xterm's modifier parameter: 1 + Shift 1 + Alt 2 + Control 4.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.control as u8
    }
}

/// The bytes a key sends, or `None` when it sends nothing.
pub fn encode_key(key: Key, mods: Modifiers, modes: &Modes) -> Option<Vec<u8>> {
    let csi_mod = |final_byte: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{}", mods.param(), final_byte as char).into_bytes()
        } else if modes.app_cursor {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[{code};{}~", mods.param()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let alt_prefixed = |bytes: &[u8]| -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if mods.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    Some(match key {
        Key::Up => csi_mod(b'A'),
        Key::Down => csi_mod(b'B'),
        Key::Right => csi_mod(b'C'),
        Key::Left => csi_mod(b'D'),
        Key::Home => csi_mod(b'H'),
        Key::End => csi_mod(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let final_byte = b"PQRS"[n as usize - 1];
            if mods.any() {
                format!("\x1b[1;{}{}", mods.param(), final_byte as char).into_bytes()
            } else {
                vec![0x1b, b'O', final_byte]
            }
        }
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        Key::F(_) => return None,
        Key::Enter => {
            if modes.line_feed_new_line {
                alt_prefixed(b"\r\n")
            } else {
                alt_prefixed(b"\r")
            }
        }
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt_prefixed(b"\t"),
        Key::Backspace if mods.control => alt_prefixed(&[0x08]),
        Key::Backspace => alt_prefixed(&[0x7f]),
        Key::Escape => alt_prefixed(&[0x1b]),
        Key::Char(c) => {
            if mods.control
                && let Some(code) = control_code(c)
            {
                return Some(alt_prefixed(&[code]));
            }
            let mut buf = [0; 4];
            alt_prefixed(c.encode_utf8(&mut buf).as_bytes())
        }
    })
}

/// The C0 control code of Ctrl + `c` as xterm sends it.
fn control_code(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '~' | '6' => 0x1e,
        '_' | '/' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// The bytes a paste sends: line breaks become CR (what Enter sends); in
/// bracketed paste mode the text is wrapped in `ESC [200~` and `ESC [201~`
/// and loses its ESC characters, so it cannot end the paste early.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut out = Vec::with_capacity(text.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend(text.bytes().filter(|&b| b != 0x1b));
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.into_bytes()
    }
}

/// A focus report (mode 1004), or `None` when the program did not ask.
pub fn encode_focus(focused: bool, modes: &Modes) -> Option<Vec<u8>> {
    modes.focus_reporting.then(|| {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
            MouseButton::WheelLeft => 66,
            MouseButton::WheelRight => 67,
        }
    }

    fn is_wheel(self) -> bool {
        self.code() >= 64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press(MouseButton),
    Release(MouseButton),
    /// The pointer moved, with the button held (if any).
    Move(Option<MouseButton>),
}

/// A mouse report at the cell `column`, `line` (0-based, viewport), or
/// `None` when the program does not want this event.
///
/// The view does not report while Shift is held: Shift selects text in the
/// reference (and in xterm).
pub fn encode_mouse(
    action: MouseAction,
    column: usize,
    line: usize,
    mods: Modifiers,
    modes: &Modes,
) -> Option<Vec<u8>> {
    let (mut code, release) = match action {
        MouseAction::Press(button) => (button.code(), false),
        MouseAction::Release(button) if button.is_wheel() => return None,
        MouseAction::Release(button) => (button.code(), true),
        MouseAction::Move(button) => {
            let wanted = match modes.mouse {
                MouseMode::Motion => true,
                MouseMode::Drag => button.is_some(),
                MouseMode::Click | MouseMode::None => false,
            };
            if !wanted {
                return None;
            }
            // 3 is "no button" in motion reports.
            (32 + button.map_or(3, MouseButton::code), false)
        }
    };
    if modes.mouse == MouseMode::None {
        return None;
    }
    code += 4 * mods.shift as u8 + 8 * mods.alt as u8 + 16 * mods.control as u8;
    if modes.sgr_mouse {
        let final_byte = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{};{}{final_byte}", column + 1, line + 1).into_bytes());
    }
    // Legacy encoding: the release has no button, positions up to 223.
    if release {
        code = (code & !0b11) | 3;
    }
    if column > 222 || line > 222 {
        return None;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        32 + 1 + column as u8,
        32 + 1 + line as u8,
    ])
}

/// The cursor keys the wheel sends on the alternate screen when the program
/// does not report the mouse (mode 1007): `lines` up when positive.
pub fn encode_alternate_scroll(lines: i32, modes: &Modes) -> Option<Vec<u8>> {
    if !modes.alt_screen || !modes.alternate_scroll || modes.mouse != MouseMode::None || lines == 0
    {
        return None;
    }
    let final_byte = if lines > 0 { b'A' } else { b'B' };
    let prefix = if modes.app_cursor { b'O' } else { b'[' };
    Some(
        (0..lines.unsigned_abs())
            .flat_map(|_| [0x1b, prefix, final_byte])
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key, mods: Modifiers) -> Vec<u8> {
        encode_key(k, mods, &Modes::default()).unwrap()
    }

    const SHIFT: Modifiers = Modifiers {
        shift: true,
        alt: false,
        control: false,
    };
    const ALT: Modifiers = Modifiers {
        shift: false,
        alt: true,
        control: false,
    };
    const CTRL: Modifiers = Modifiers {
        shift: false,
        alt: false,
        control: true,
    };

    #[test]
    fn cursor_keys_normal_application_and_modified() {
        assert_eq!(key(Key::Up, Modifiers::NONE), b"\x1b[A");
        let app = Modes {
            app_cursor: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_key(Key::Up, Modifiers::NONE, &app).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(Key::Home, Modifiers::NONE, &app).unwrap(),
            b"\x1bOH"
        );
        assert_eq!(key(Key::Left, CTRL), b"\x1b[1;5D");
        assert_eq!(key(Key::Right, ALT), b"\x1b[1;3C");
        let ctrl_shift = Modifiers {
            shift: true,
            control: true,
            alt: false,
        };
        assert_eq!(
            encode_key(Key::End, ctrl_shift, &app).unwrap(),
            b"\x1b[1;6F"
        );
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(key(Key::Delete, Modifiers::NONE), b"\x1b[3~");
        assert_eq!(key(Key::PageUp, SHIFT), b"\x1b[5;2~");
        assert_eq!(key(Key::F(1), Modifiers::NONE), b"\x1bOP");
        assert_eq!(key(Key::F(4), CTRL), b"\x1b[1;5S");
        assert_eq!(key(Key::F(5), Modifiers::NONE), b"\x1b[15~");
        assert_eq!(key(Key::F(12), ALT), b"\x1b[24;3~");
        assert_eq!(
            encode_key(Key::F(13), Modifiers::NONE, &Modes::default()),
            None
        );
    }

    #[test]
    fn enter_tab_backspace_escape() {
        assert_eq!(key(Key::Enter, Modifiers::NONE), b"\r");
        assert_eq!(key(Key::Enter, ALT), b"\x1b\r");
        let lnm = Modes {
            line_feed_new_line: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_key(Key::Enter, Modifiers::NONE, &lnm).unwrap(),
            b"\r\n"
        );
        assert_eq!(key(Key::Tab, Modifiers::NONE), b"\t");
        assert_eq!(key(Key::Tab, SHIFT), b"\x1b[Z");
        assert_eq!(key(Key::Backspace, Modifiers::NONE), b"\x7f");
        assert_eq!(key(Key::Backspace, CTRL), b"\x08");
        assert_eq!(key(Key::Backspace, ALT), b"\x1b\x7f");
        assert_eq!(key(Key::Escape, Modifiers::NONE), b"\x1b");
    }

    #[test]
    fn characters_control_and_meta() {
        assert_eq!(key(Key::Char('a'), Modifiers::NONE), b"a");
        assert_eq!(key(Key::Char('ж'), Modifiers::NONE), "ж".as_bytes());
        assert_eq!(key(Key::Char('c'), CTRL), b"\x03");
        assert_eq!(key(Key::Char('C'), CTRL), b"\x03");
        assert_eq!(key(Key::Char(' '), CTRL), b"\x00");
        assert_eq!(key(Key::Char('['), CTRL), b"\x1b");
        assert_eq!(key(Key::Char('/'), CTRL), b"\x1f");
        assert_eq!(key(Key::Char('b'), ALT), b"\x1bb");
        let ctrl_alt = Modifiers {
            control: true,
            alt: true,
            shift: false,
        };
        assert_eq!(key(Key::Char('x'), ctrl_alt), b"\x1b\x18");
        // No control code: the character itself.
        assert_eq!(key(Key::Char('1'), CTRL), b"1");
    }

    #[test]
    fn paste_plain_and_bracketed() {
        assert_eq!(encode_paste("one\ntwo\r\nthree", false), b"one\rtwo\rthree");
        assert_eq!(
            encode_paste("one\ntwo\x1b[201~", true),
            b"\x1b[200~one\rtwo[201~\x1b[201~"
        );
    }

    #[test]
    fn focus_reports_only_when_asked() {
        assert_eq!(encode_focus(true, &Modes::default()), None);
        let modes = Modes {
            focus_reporting: true,
            ..Modes::default()
        };
        assert_eq!(encode_focus(true, &modes).unwrap(), b"\x1b[I");
        assert_eq!(encode_focus(false, &modes).unwrap(), b"\x1b[O");
    }

    #[test]
    fn mouse_reports() {
        let off = Modes::default();
        let press = MouseAction::Press(MouseButton::Left);
        assert_eq!(encode_mouse(press, 0, 0, Modifiers::NONE, &off), None);
        let click = Modes {
            mouse: MouseMode::Click,
            ..Modes::default()
        };
        assert_eq!(
            encode_mouse(press, 4, 9, Modifiers::NONE, &click).unwrap(),
            vec![0x1b, b'[', b'M', 32, 32 + 5, 32 + 10]
        );
        assert_eq!(
            encode_mouse(
                MouseAction::Release(MouseButton::Right),
                0,
                0,
                Modifiers::NONE,
                &click
            )
            .unwrap(),
            vec![0x1b, b'[', b'M', 32 + 3, 33, 33]
        );
        assert_eq!(encode_mouse(press, 300, 0, Modifiers::NONE, &click), None);
        let drag = MouseAction::Move(Some(MouseButton::Left));
        assert_eq!(encode_mouse(drag, 0, 0, Modifiers::NONE, &click), None);

        let sgr = Modes {
            mouse: MouseMode::Drag,
            sgr_mouse: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_mouse(press, 4, 9, CTRL, &sgr).unwrap(),
            b"\x1b[<16;5;10M"
        );
        assert_eq!(
            encode_mouse(
                MouseAction::Release(MouseButton::Left),
                300,
                2,
                Modifiers::NONE,
                &sgr
            )
            .unwrap(),
            b"\x1b[<0;301;3m"
        );
        assert_eq!(
            encode_mouse(drag, 1, 1, Modifiers::NONE, &sgr).unwrap(),
            b"\x1b[<32;2;2M"
        );
        assert_eq!(
            encode_mouse(MouseAction::Move(None), 1, 1, Modifiers::NONE, &sgr),
            None
        );
        let motion = Modes {
            mouse: MouseMode::Motion,
            sgr_mouse: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_mouse(MouseAction::Move(None), 1, 1, Modifiers::NONE, &motion).unwrap(),
            b"\x1b[<35;2;2M"
        );
        assert_eq!(
            encode_mouse(
                MouseAction::Press(MouseButton::WheelDown),
                0,
                0,
                Modifiers::NONE,
                &sgr
            )
            .unwrap(),
            b"\x1b[<65;1;1M"
        );
        assert_eq!(
            encode_mouse(
                MouseAction::Release(MouseButton::WheelDown),
                0,
                0,
                Modifiers::NONE,
                &sgr
            ),
            None
        );
    }

    #[test]
    fn wheel_on_the_alternate_screen() {
        let alt = Modes {
            alt_screen: true,
            alternate_scroll: true,
            ..Modes::default()
        };
        assert_eq!(encode_alternate_scroll(2, &alt).unwrap(), b"\x1b[A\x1b[A");
        assert_eq!(encode_alternate_scroll(-1, &alt).unwrap(), b"\x1b[B");
        assert_eq!(encode_alternate_scroll(1, &Modes::default()), None);
        let reporting = Modes {
            mouse: MouseMode::Click,
            ..alt
        };
        assert_eq!(encode_alternate_scroll(1, &reporting), None);
    }
}
