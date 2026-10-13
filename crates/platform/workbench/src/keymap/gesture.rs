//! Double-tapping a modifier key, as the reference IDE's double Shift for
//! Search Everywhere: the key goes down, up, down and up with at most
//! [`STEP`] between the steps, with no other key or modifier in between,
//! and the command runs on the second release. A tap within
//! [`TYPING_GUARD`] after another key (Shift while typing capitals) does
//! not count; a sequence older than [`RESET`] starts over.
//!
//! Keymaps write it as the key twice: `"shift shift"`.

use std::time::{Duration, Instant};

use gpui::Modifiers;

/// Longest time between two steps of a double tap.
pub const STEP: Duration = Duration::from_millis(300);
/// A sequence this old starts over.
pub const RESET: Duration = Duration::from_millis(500);
/// A modifier pressed this soon after another key is typing, not a tap.
pub const TYPING_GUARD: Duration = Duration::from_millis(100);

/// A modifier key that can be double-tapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    Shift,
    Control,
    Alt,
    /// Cmd on macOS, the Windows key elsewhere.
    Platform,
}

const ALL: [ModifierKey; 4] = [
    ModifierKey::Shift,
    ModifierKey::Control,
    ModifierKey::Alt,
    ModifierKey::Platform,
];

impl ModifierKey {
    fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "shift" => Self::Shift,
            "ctrl" | "control" => Self::Control,
            "alt" | "option" => Self::Alt,
            "cmd" | "platform" | "super" | "win" => Self::Platform,
            _ => return None,
        })
    }

    fn is_down(self, m: &Modifiers) -> bool {
        match self {
            Self::Shift => m.shift,
            Self::Control => m.control,
            Self::Alt => m.alt,
            Self::Platform => m.platform,
        }
    }

    /// The key's name as shortcuts show it.
    pub fn label(self, macos: bool) -> &'static str {
        match (self, macos) {
            (Self::Shift, true) => "⇧",
            (Self::Shift, false) => "Shift",
            (Self::Control, true) => "⌃",
            (Self::Control, false) => "Ctrl",
            (Self::Alt, true) => "⌥",
            (Self::Alt, false) => "Alt",
            (Self::Platform, true) => "⌘",
            (Self::Platform, false) => "Win",
        }
    }
}

/// The modifier of a double-tap keystroke text (`"shift shift"`).
pub fn parse_double_tap(keys: &str) -> Option<ModifierKey> {
    let mut parts = keys.split_whitespace();
    let (first, second) = (parts.next()?, parts.next()?);
    if parts.next().is_some() || first != second {
        return None;
    }
    ModifierKey::parse(first)
}

/// How a double tap reads in hints: "Double ⇧", "Double Shift".
pub fn double_tap_text(key: ModifierKey, macos: bool) -> String {
    format!("Double {}", key.label(macos))
}

/// The state of a double tap in progress.
#[derive(Debug, Default)]
pub struct DoubleTap {
    /// The modifiers held after the last change.
    held: Modifiers,
    key: Option<ModifierKey>,
    first_pressed: bool,
    first_released: bool,
    second_pressed: bool,
    last_step: Option<Instant>,
    /// When another key was last pressed (none after Escape and Tab).
    other_key: Option<Instant>,
}

impl DoubleTap {
    /// The modifiers changed at `now`; returns the key double-tapped when
    /// this was its second release.
    pub fn modifiers_changed(&mut self, modifiers: Modifiers, now: Instant) -> Option<ModifierKey> {
        let before = std::mem::replace(&mut self.held, modifiers);
        let changed: Vec<ModifierKey> = ALL
            .into_iter()
            .filter(|k| k.is_down(&before) != k.is_down(&modifiers))
            .collect();
        let [key] = changed[..] else {
            self.reset();
            return None;
        };
        let others =
            ALL.into_iter().any(|k| k != key && k.is_down(&modifiers)) || modifiers.function;
        if others {
            self.reset();
            return None;
        }
        if self.key != Some(key) {
            self.reset();
            self.key = Some(key);
        }
        if self
            .other_key
            .is_some_and(|t| now.saturating_duration_since(t) < TYPING_GUARD)
        {
            self.reset();
            return None;
        }
        let since = |s: &Self| s.last_step.map(|t| now.saturating_duration_since(t));
        if self.first_pressed && since(self).is_some_and(|d| d > RESET) {
            self.reset();
            self.key = Some(key);
        }
        if self.first_pressed && since(self).is_some_and(|d| d > STEP) {
            self.reset();
            return None;
        }
        let down = key.is_down(&modifiers);
        if down && !self.first_pressed {
            self.first_pressed = true;
        } else if down && self.first_released && !self.second_pressed {
            self.second_pressed = true;
        } else if !down && self.first_pressed && !self.first_released {
            self.first_released = true;
        } else if !down && self.second_pressed {
            self.reset();
            return Some(key);
        } else {
            self.reset();
            return None;
        }
        self.last_step = Some(now);
        None
    }

    /// Another key went down at `now`: a tap in progress is over. After
    /// Escape and Tab (`guard` false) the next tap counts at once.
    pub fn key_pressed(&mut self, now: Instant, guard: bool) {
        self.other_key = guard.then_some(now);
        self.reset();
    }

    fn reset(&mut self) {
        self.key = None;
        self.first_pressed = false;
        self.first_released = false;
        self.second_pressed = false;
        self.last_step = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shift(on: bool) -> Modifiers {
        Modifiers {
            shift: on,
            ..Modifiers::none()
        }
    }

    /// Feeds Shift down/up steps `ms` apart; the result of each.
    fn taps(
        tap: &mut DoubleTap,
        start: Instant,
        steps: &[(u64, bool)],
    ) -> Vec<Option<ModifierKey>> {
        steps
            .iter()
            .map(|&(ms, down)| {
                tap.modifiers_changed(shift(down), start + Duration::from_millis(ms))
            })
            .collect()
    }

    #[test]
    fn keystroke_texts() {
        assert_eq!(parse_double_tap("shift shift"), Some(ModifierKey::Shift));
        assert_eq!(parse_double_tap("ctrl ctrl"), Some(ModifierKey::Control));
        assert_eq!(parse_double_tap("shift"), None);
        assert_eq!(parse_double_tap("shift ctrl"), None);
        assert_eq!(parse_double_tap("cmd-k cmd-k"), None);
        assert_eq!(double_tap_text(ModifierKey::Shift, true), "Double ⇧");
        assert_eq!(double_tap_text(ModifierKey::Shift, false), "Double Shift");
    }

    #[test]
    fn a_double_tap_fires_on_the_second_release() {
        let t = Instant::now();
        let mut tap = DoubleTap::default();
        let out = taps(
            &mut tap,
            t,
            &[(0, true), (80, false), (160, true), (240, false)],
        );
        assert_eq!(out, [None, None, None, Some(ModifierKey::Shift)]);
    }

    #[test]
    fn slow_steps_do_not_count() {
        let t = Instant::now();
        let mut tap = DoubleTap::default();
        // The second press held 400 ms.
        let out = taps(
            &mut tap,
            t,
            &[(0, true), (80, false), (160, true), (560, false)],
        );
        assert!(out.iter().all(Option::is_none));
        // A tap 300-500 ms late is dropped, not a new start.
        let mut tap = DoubleTap::default();
        let out = taps(
            &mut tap,
            t,
            &[
                (0, true),
                (80, false),
                (480, true),
                (560, false),
                (640, true),
                (720, false),
            ],
        );
        assert!(out.iter().all(Option::is_none), "{out:?}");
        // More than 500 ms later a new sequence starts.
        let mut tap = DoubleTap::default();
        let out = taps(
            &mut tap,
            t,
            &[
                (0, true),
                (80, false),
                (1000, true),
                (1080, false),
                (1160, true),
                (1240, false),
            ],
        );
        assert_eq!(out[5], Some(ModifierKey::Shift));
    }

    #[test]
    fn other_keys_and_modifiers_break_it() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        // Shift+A: a capital, not a tap.
        let mut tap = DoubleTap::default();
        tap.modifiers_changed(shift(true), ms(0));
        tap.key_pressed(ms(40), true);
        tap.modifiers_changed(shift(false), ms(80));
        tap.modifiers_changed(shift(true), ms(160));
        assert_eq!(tap.modifiers_changed(shift(false), ms(240)), None);
        // A tap right after typing is ignored.
        let mut tap = DoubleTap::default();
        tap.key_pressed(ms(0), true);
        let out = taps(
            &mut tap,
            t,
            &[(50, true), (120, false), (200, true), (280, false)],
        );
        assert!(out.iter().all(Option::is_none), "{out:?}");
        // After Escape it counts at once.
        let mut tap = DoubleTap::default();
        tap.key_pressed(ms(0), false);
        let out = taps(
            &mut tap,
            t,
            &[(20, true), (100, false), (180, true), (260, false)],
        );
        assert_eq!(out[3], Some(ModifierKey::Shift));
        // Ctrl held: no tap.
        let mut tap = DoubleTap::default();
        let both = Modifiers {
            shift: true,
            control: true,
            ..Modifiers::none()
        };
        tap.modifiers_changed(both, ms(0));
        tap.modifiers_changed(Modifiers::none(), ms(50));
        tap.modifiers_changed(shift(true), ms(100));
        assert_eq!(tap.modifiers_changed(shift(false), ms(150)), None);
    }
}
