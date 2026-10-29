//! Tools > Terminal (`[terminal]`): the options of new terminal sessions
//! (ADR 0014). The start directory and the project's variables are kept
//! per project, in the personal project state.

use serde::{Deserialize, Serialize};

use crate::Section;

/// How new terminal sessions start and how they take keys (the reference's
/// `TerminalOptionsProvider`: `myShellPath`, `myShellIntegration`,
/// `closeSessionOnLogout`, `overrideIdeShortcuts`, `useOptionAsMetaKey`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TerminalSettings {
    /// "Shell path": a shell, or a command line with its options; empty
    /// for the default shell (`$SHELL` on macOS and Linux, PowerShell on
    /// Windows).
    pub shell_path: String,
    /// "Shell integration": zsh, bash, fish and PowerShell report the
    /// working directory, the prompt and the commands, and the product's
    /// scripts (a virtual environment's `activate`) are sourced before the
    /// first prompt. The reference activates no environment without it.
    pub shell_integration: bool,
    /// "Close session when it ends": the tab of a session whose shell
    /// exited closes.
    pub close_session_on_exit: bool,
    /// "Override IDE shortcuts": a focused terminal takes the keys of IDE
    /// shortcuts, except those of the allowed actions (tool windows, Search
    /// Everywhere, the terminal's own).
    pub override_ide_shortcuts: bool,
    /// "Use Option as Meta key" (macOS): Option with a key sends Escape
    /// and the key instead of the character Option types.
    pub option_as_meta: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell_path: String::new(),
            shell_integration: true,
            close_session_on_exit: true,
            override_ide_shortcuts: true,
            option_as_meta: false,
        }
    }
}

impl Section for TerminalSettings {
    const KEY: &'static str = "terminal";
}

impl TerminalSettings {
    /// The shell path, `None` when empty (the default shell).
    pub fn shell(&self) -> Option<&str> {
        Some(self.shell_path.trim()).filter(|p| !p.is_empty())
    }
}

/// The terminal's own font (Settings | Tools | Terminal | Font, the
/// reference's `terminal-font.xml`, `TerminalFontSettingsService`). A key
/// that is not set is inherited: the family and size from the editor font
/// (the reference starts from the console font, which is the editor font
/// until it is changed), the line and column spacing are 1. The characters
/// the font lacks come from the platform's fallback, not from any font
/// the machine happens to have, unless a fallback family is set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TerminalFontSettings {
    /// The font family.
    pub family: Option<String>,
    /// The size in points.
    #[serde(serialize_with = "crate::short_float::serialize")]
    pub size: Option<f32>,
    /// The line height, a multiple of the font's own.
    #[serde(serialize_with = "crate::short_float::serialize")]
    pub line_spacing: Option<f32>,
    /// The width of a cell, a multiple of the font's own.
    #[serde(serialize_with = "crate::short_float::serialize")]
    pub column_spacing: Option<f32>,
    /// The family of the characters the font lacks.
    pub fallback: Option<String>,
}

impl TerminalFontSettings {
    /// The sizes allowed (`EditorFontsConstants`: 4 to 40).
    pub const SIZES: std::ops::RangeInclusive<f32> = 4.0..=40.0;
    /// The line and column spacings allowed (`EditorFontsConstants`: 0.6 to
    /// 3).
    pub const SPACINGS: std::ops::RangeInclusive<f32> = 0.6..=3.0;
    /// The line and column spacing without a choice.
    pub const DEFAULT_SPACING: f32 = 1.0;

    /// The size without a choice (`EditorFontsConstants` default).
    pub const DEFAULT_SIZE: f32 = 13.0;
    /// The digits after the point of a size, and of a spacing
    /// (`TerminalFontSettings`).
    pub const SIZE_DIGITS: u32 = 1;
    pub const SPACING_DIGITS: u32 = 2;

    /// A size in the allowed range, to one digit.
    pub fn clamp_size(size: f32) -> f32 {
        round_to(
            size.clamp(*Self::SIZES.start(), *Self::SIZES.end()),
            Self::SIZE_DIGITS,
        )
    }

    /// A line or column spacing in the allowed range, to two digits.
    pub fn clamp_spacing(spacing: f32) -> f32 {
        round_to(
            spacing.clamp(*Self::SPACINGS.start(), *Self::SPACINGS.end()),
            Self::SPACING_DIGITS,
        )
    }

    /// The size a text field holds; a text that is no number is the
    /// default size (`TerminalFontSize.parse`).
    pub fn parse_size(text: &str) -> f32 {
        Self::clamp_size(text.trim().parse().unwrap_or(Self::DEFAULT_SIZE))
    }

    /// The spacing a text field holds; a text that is no number is 1
    /// (`TerminalLineSpacing.parse`).
    pub fn parse_spacing(text: &str) -> f32 {
        Self::clamp_spacing(text.trim().parse().unwrap_or(Self::DEFAULT_SPACING))
    }

    /// A size as a field shows it: `13.0`, `12.5`.
    pub fn format_size(size: f32) -> String {
        format_float(size, Self::SIZE_DIGITS)
    }

    /// A spacing as a field shows it: `1.0`, `1.05`, `1.1`.
    pub fn format_spacing(spacing: f32) -> String {
        format_float(spacing, Self::SPACING_DIGITS)
    }

    /// The fallback family in effect: the platform's (`None`) for an unset
    /// one, `<None>` and the reference's logical default `Monospaced`.
    pub fn fallback_family(&self) -> Option<&str> {
        self.fallback
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty() && *f != "<None>" && *f != "Monospaced")
    }
}

/// `value` to `digits` digits after the point.
fn round_to(value: f32, digits: u32) -> f32 {
    let multiplier = 10f32.powi(digits as i32);
    (value * multiplier).round() / multiplier
}

/// `value` with `digits` digits after the point, the trailing zeros left
/// out down to one digit (`TerminalSettingsFloatValueImpl.toFormattedString`).
fn format_float(value: f32, digits: u32) -> String {
    let mut text = format!("{:.*}", digits as usize, value);
    while text.ends_with('0') && text.split('.').nth(1).is_some_and(|f| f.len() > 1) {
        text.pop();
    }
    text
}

impl Section for TerminalFontSettings {
    const KEY: &'static str = "terminal.font";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn integration_is_on_by_default() {
        let mut store = Store::new();
        store.register::<TerminalSettings>();
        let settings = store.section::<TerminalSettings>();
        assert!(settings.shell_integration);
        assert_eq!(settings.shell(), None);
        assert_eq!(
            store.get("terminal.shell-integration"),
            Some(serde_json::json!(true))
        );
        // The reference's defaults: tabs close with their session, a
        // terminal overrides IDE shortcuts, Option types characters.
        assert!(settings.close_session_on_exit);
        assert!(settings.override_ide_shortcuts);
        assert!(!settings.option_as_meta);
        let changed = TerminalSettings {
            shell_path: " /bin/zsh -l ".into(),
            shell_integration: false,
            ..TerminalSettings::default()
        };
        assert!(store.set_section(&changed));
        assert_eq!(store.section::<TerminalSettings>(), changed);
        assert_eq!(changed.shell(), Some("/bin/zsh -l"));
        assert_eq!(
            store.get("terminal.shell-path"),
            Some(serde_json::json!(" /bin/zsh -l "))
        );
    }

    #[test]
    fn the_font_is_inherited_until_chosen() {
        let mut store = Store::new();
        store.register::<TerminalFontSettings>();
        assert_eq!(
            store.section::<TerminalFontSettings>(),
            TerminalFontSettings::default()
        );
        let chosen = TerminalFontSettings {
            family: Some("JetBrainsMono Nerd Font".into()),
            size: Some(14.0),
            fallback: Some("Monospaced".into()),
            ..TerminalFontSettings::default()
        };
        assert!(store.set_section(&chosen));
        assert_eq!(
            store.get("terminal.font.family"),
            Some(serde_json::json!("JetBrainsMono Nerd Font"))
        );
        assert_eq!(chosen.fallback_family(), None);
        let own = TerminalFontSettings {
            fallback: Some(" Symbols Nerd Font ".into()),
            ..TerminalFontSettings::default()
        };
        assert_eq!(own.fallback_family(), Some("Symbols Nerd Font"));
        assert_eq!(TerminalFontSettings::clamp_size(100.0), 40.0);
        assert_eq!(TerminalFontSettings::clamp_spacing(0.1), 0.6);
    }

    #[test]
    fn numbers_are_parsed_and_shown_as_the_reference_does() {
        use TerminalFontSettings as F;
        assert_eq!(F::parse_size(" 12.55 "), 12.6);
        assert_eq!(F::parse_size("big"), 13.0);
        assert_eq!(F::parse_size("2"), 4.0);
        assert_eq!(F::parse_spacing("1.234"), 1.23);
        assert_eq!(F::parse_spacing(""), 1.0);
        assert_eq!(F::parse_spacing("9"), 3.0);
        assert_eq!(F::format_size(13.0), "13.0");
        assert_eq!(F::format_size(12.5), "12.5");
        assert_eq!(F::format_spacing(1.0), "1.0");
        assert_eq!(F::format_spacing(1.1), "1.1");
        assert_eq!(F::format_spacing(1.05), "1.05");
        // The file holds the numbers as they were typed.
        let settings = F {
            size: Some(12.6),
            column_spacing: Some(1.05),
            ..F::default()
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["size"], serde_json::json!(12.6));
        assert_eq!(json["column-spacing"], serde_json::json!(1.05));
    }
}
