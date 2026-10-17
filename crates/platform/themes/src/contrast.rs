//! WCAG 2 contrast checks over the pairs of colours a theme puts next to each
//! other. The built-in themes must pass all of them; imported themes are
//! checked for information only.

use crate::color::{Rgba, contrast_ratio};
use crate::model::Theme;
use crate::roles::{TextRole, UiRole};

/// Minimum ratios of WCAG 2.2 level AA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Body text: 4.5:1.
    Text,
    /// Large text, deliberately dimmed text: 3:1.
    Large,
    /// Borders, focus rings, markers and underlines: 3:1.
    Ui,
}

impl Need {
    pub const fn ratio(self) -> f64 {
        match self {
            Need::Text => 4.5,
            Need::Large | Need::Ui => 3.0,
        }
    }
}

/// A checked pair and its outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct PairResult {
    pub label: String,
    pub ratio: f64,
    pub need: Need,
}

impl PairResult {
    pub fn passed(&self) -> bool {
        self.ratio >= self.need.ratio()
    }
}

/// Pairs of UI roles that must stay readable: `(foreground, background, need)`.
pub const UI_PAIRS: &[(UiRole, UiRole, Need)] = &{
    use UiRole::*;
    [
        (Text, Surface, Need::Text),
        (Text, SurfaceAlt, Need::Text),
        (Text, SurfaceRaised, Need::Text),
        (Text, ToolWindow, Need::Text),
        (Text, ToolWindowHeader, Need::Text),
        (Text, Toolbar, Need::Text),
        (Text, TitleBar, Need::Text),
        (NotificationText, Notification, Need::Text),
        (NotificationLink, Notification, Need::Text),
        (Text, CompletionPopup, Need::Text),
        (Text, DocumentationPopup, Need::Text),
        (Text, Input, Need::Text),
        (Text, Hover, Need::Text),
        (Text, Pressed, Need::Text),
        (Text, SelectionInactive, Need::Text),
        (SelectionText, Selection, Need::Text),
        (Text, CompletionSelection, Need::Text),
        (CompletionMatch, CompletionPopup, Need::Text),
        (TextMuted, Surface, Need::Text),
        (TextMuted, SurfaceAlt, Need::Text),
        (TextMuted, SurfaceRaised, Need::Text),
        (TextMuted, ToolWindow, Need::Text),
        (TextMuted, Input, Need::Text),
        (TextDisabled, Surface, Need::Large),
        (TextLink, Surface, Need::Text),
        (TextLink, SurfaceRaised, Need::Text),
        (Shortcut, EditorBackground, Need::Text),
        (OnAccent, Accent, Need::Text),
        (Accent, Surface, Need::Ui),
        (Accent, EditorBackground, Need::Ui),
        (Focus, Surface, Need::Ui),
        (Focus, Input, Need::Ui),
        (BorderInput, Input, Need::Ui),
        (BorderInput, Surface, Need::Ui),
        (Danger, Surface, Need::Text),
        (Warning, Surface, Need::Text),
        (Success, Surface, Need::Text),
        (Info, Surface, Need::Text),
        (Danger, SurfaceRaised, Need::Text),
        (Warning, SurfaceRaised, Need::Text),
        (Success, SurfaceRaised, Need::Text),
        (Info, SurfaceRaised, Need::Text),
        (Text, DangerBg, Need::Text),
        (Text, WarningBg, Need::Text),
        (Text, SuccessBg, Need::Text),
        (Text, InfoBg, Need::Text),
        (StatusBarText, StatusBar, Need::Text),
        (TabActiveText, TabActive, Need::Text),
        (TabInactiveText, TabInactive, Need::Text),
        (TabInactiveText, TabHover, Need::Text),
        (TabUnderline, TabBar, Need::Ui),
        (EditorText, EditorBackground, Need::Text),
        (EditorText, CurrentLine, Need::Text),
        (EditorText, SearchMatch, Need::Text),
        (EditorText, SearchMatchActive, Need::Text),
        (EditorText, EditorSelection, Need::Text),
        (EditorText, Breakpoint, Need::Text),
        (EditorText, ExecutionPoint, Need::Text),
        (EditorText, DiffAdded, Need::Text),
        (EditorText, DiffRemoved, Need::Text),
        (EditorText, DiffChanged, Need::Text),
        (EditorText, DiffConflict, Need::Text),
        (LineNumber, Gutter, Need::Text),
        (LineNumberActive, Gutter, Need::Text),
        (LineNumberActive, CurrentLine, Need::Text),
        (Caret, EditorBackground, Need::Ui),
        (Bookmark, Gutter, Need::Ui),
        (GutterAdded, Gutter, Need::Ui),
        (GutterModified, Gutter, Need::Ui),
        (GutterDeleted, Gutter, Need::Ui),
        (DiagnosticError, EditorBackground, Need::Ui),
        (DiagnosticWarning, EditorBackground, Need::Ui),
        (DiagnosticInfo, EditorBackground, Need::Ui),
        (DiagnosticHint, EditorBackground, Need::Ui),
        (GitAdded, SurfaceAlt, Need::Text),
        (GitModified, SurfaceAlt, Need::Text),
        (GitDeleted, SurfaceAlt, Need::Text),
        (GitRenamed, SurfaceAlt, Need::Text),
        (GitUntracked, SurfaceAlt, Need::Text),
        (GitConflict, SurfaceAlt, Need::Text),
        (GitIgnored, SurfaceAlt, Need::Large),
        (Text, NonProjectFile, Need::Text),
        (EditorText, ExecutionPointNotTop, Need::Text),
        (DebuggerValueName, ToolWindow, Need::Text),
        (DebuggerType, ToolWindow, Need::Large),
        (DebuggerChangedValue, ToolWindow, Need::Text),
        (DebuggerCollectingData, ToolWindow, Need::Large),
        (DebuggerErrorMessage, ToolWindow, Need::Text),
        (TerminalForeground, TerminalBackground, Need::Text),
        (TerminalBlack, TerminalBackground, Need::Text),
        (TerminalRed, TerminalBackground, Need::Text),
        (TerminalGreen, TerminalBackground, Need::Text),
        (TerminalYellow, TerminalBackground, Need::Text),
        (TerminalBlue, TerminalBackground, Need::Text),
        (TerminalMagenta, TerminalBackground, Need::Text),
        (TerminalCyan, TerminalBackground, Need::Text),
        (TerminalWhite, TerminalBackground, Need::Text),
        (TerminalBrightBlack, TerminalBackground, Need::Text),
        (TerminalBrightRed, TerminalBackground, Need::Text),
        (TerminalBrightGreen, TerminalBackground, Need::Text),
        (TerminalBrightYellow, TerminalBackground, Need::Text),
        (TerminalBrightBlue, TerminalBackground, Need::Text),
        (TerminalBrightMagenta, TerminalBackground, Need::Text),
        (TerminalBrightCyan, TerminalBackground, Need::Text),
        (TerminalBrightWhite, TerminalBackground, Need::Text),
        (
            TerminalDefaultForeground,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (TerminalAnsiBlack, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiRed, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiGreen, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiYellow, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiBlue, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiMagenta, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiCyan, TerminalDefaultBackground, Need::Text),
        (TerminalAnsiWhite, TerminalDefaultBackground, Need::Text),
        (
            TerminalAnsiBrightBlack,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (TerminalAnsiBrightRed, TerminalDefaultBackground, Need::Text),
        (
            TerminalAnsiBrightGreen,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (
            TerminalAnsiBrightYellow,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (
            TerminalAnsiBrightBlue,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (
            TerminalAnsiBrightMagenta,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (
            TerminalAnsiBrightCyan,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (
            TerminalAnsiBrightWhite,
            TerminalDefaultBackground,
            Need::Text,
        ),
        (TerminalSearchMatchText, TerminalSearchMatch, Need::Text),
        (
            TerminalSearchMatchCurrentText,
            TerminalSearchMatchCurrent,
            Need::Text,
        ),
        (TerminalErrorMarker, TerminalDefaultBackground, Need::Ui),
    ]
};

/// Text roles that are dimmed on purpose and only need 3:1.
const DIMMED: &[TextRole] = &[TextRole::DiagnosticUnused];

/// Text roles drawn on the terminal background instead of the editor's.
const CONSOLE: &[TextRole] = &[
    TextRole::ConsoleError,
    TextRole::ConsoleInput,
    TextRole::ConsoleSystem,
];

/// Checks every pair of [`UI_PAIRS`] and every text role of the theme.
///
/// A text role is checked as its foreground (or the editor text colour) on
/// its background (or the editor background); underline and box colours
/// are checked as UI parts against the editor background.
pub fn check_theme(theme: &Theme) -> Vec<PairResult> {
    let mut out = Vec::new();
    let editor_bg = theme.ui[UiRole::EditorBackground];
    for &(fg, bg, need) in UI_PAIRS {
        let backdrop = backdrop_of(theme, bg);
        out.push(PairResult {
            label: format!("{fg} on {bg}"),
            ratio: contrast_ratio(theme.ui[fg], theme.ui[bg], backdrop),
            need,
        });
    }
    for (role, style) in theme.text.iter() {
        let (base_bg, base_fg) = if CONSOLE.contains(&role) {
            (
                theme.ui[UiRole::TerminalBackground],
                theme.ui[UiRole::TerminalForeground],
            )
        } else {
            (editor_bg, theme.ui[UiRole::EditorText])
        };
        let fg = style.fg.unwrap_or(base_fg);
        let bg = style.bg.unwrap_or(base_bg);
        // Roles that only decorate (no foreground) are covered by the checks
        // of their underline or box colour below.
        let decorates_only =
            style.fg.is_none() && (style.underline_color.is_some() || style.border.is_some());
        if !decorates_only && (style.fg.is_some() || style.bg.is_some()) {
            let need = if DIMMED.contains(&role) {
                Need::Large
            } else {
                Need::Text
            };
            out.push(PairResult {
                label: format!("syntax {role}"),
                ratio: contrast_ratio(fg, bg, base_bg),
                need,
            });
        }
        if let Some(line) = style.underline_color.or(style.border) {
            out.push(PairResult {
                label: format!("syntax {role} (line)"),
                ratio: contrast_ratio(line, base_bg, base_bg),
                need: Need::Ui,
            });
        }
    }
    out
}

fn backdrop_of(theme: &Theme, bg: UiRole) -> Rgba {
    // Translucent backgrounds of the editor are painted over the editor
    // background, everything else over the plain surface.
    match bg {
        UiRole::CurrentLine
        | UiRole::SearchMatch
        | UiRole::SearchMatchActive
        | UiRole::EditorSelection
        | UiRole::Breakpoint
        | UiRole::ExecutionPoint
        | UiRole::ExecutionPointNotTop
        | UiRole::DiffAdded
        | UiRole::DiffRemoved
        | UiRole::DiffChanged
        | UiRole::DiffConflict
        | UiRole::Gutter => theme.ui[UiRole::EditorBackground],
        UiRole::TerminalSearchMatch | UiRole::TerminalSearchMatchCurrent => {
            theme.ui[UiRole::TerminalDefaultBackground]
        }
        _ => theme.ui[UiRole::Surface],
    }
}

/// Failing pairs only.
pub fn failures(theme: &Theme) -> Vec<PairResult> {
    check_theme(theme)
        .into_iter()
        .filter(|r| !r.passed())
        .collect()
}

/// The terminal's minimum contrast ratio by default (the reference's
/// "Minimum contrast ratio", `TerminalContrastRatio.DEFAULT_VALUE`).
pub const TERMINAL_MIN_CONTRAST: f64 = 4.5;

/// A text colour with at least `ratio` against `bg`, as close to `fg` as
/// the search finds: `fg` itself when it already has the contrast, else
/// `fg` darkened or lightened in steps of 10% of each channel's distance
/// to black or white, in the direction of its own luminance first. Alpha
/// is ignored. Adapted from the reference terminal's
/// `TerminalUiUtils.ensureContrastRatio` (IntelliJ Community, Apache-2.0,
/// see NOTICE), itself after xterm.js.
pub fn ensure_contrast(fg: Rgba, bg: Rgba, ratio: f64) -> Rgba {
    let fg = Rgba::rgb(fg.r, fg.g, fg.b);
    let bg = Rgba::rgb(bg.r, bg.g, bg.b);
    let contrast = |c: Rgba| contrast_ratio(c, bg, bg);
    if contrast(fg) >= ratio {
        return fg;
    }
    type Search = fn(Rgba, Rgba, f64) -> Rgba;
    let (first, second): (Search, Search) = if fg.luminance() < bg.luminance() {
        (reduce_luminance, increase_luminance)
    } else {
        (increase_luminance, reduce_luminance)
    };
    let a = first(fg, bg, ratio);
    let a_ratio = contrast(a);
    if a_ratio >= ratio {
        return a;
    }
    let b = second(fg, bg, ratio);
    if a_ratio > contrast(b) { a } else { b }
}

/// Darker than `fg` until the contrast with `bg` is reached (or black).
fn reduce_luminance(fg: Rgba, bg: Rgba, ratio: f64) -> Rgba {
    let mut c = fg;
    while contrast_ratio(c, bg, bg) < ratio && (c.r > 0 || c.g > 0 || c.b > 0) {
        let step = |v: u8| v - (f64::from(v) * 0.1).ceil() as u8;
        c = Rgba::rgb(step(c.r), step(c.g), step(c.b));
    }
    c
}

/// Lighter than `fg` until the contrast with `bg` is reached (or white).
fn increase_luminance(fg: Rgba, bg: Rgba, ratio: f64) -> Rgba {
    let mut c = fg;
    while contrast_ratio(c, bg, bg) < ratio && (c.r < 255 || c.g < 255 || c.b < 255) {
        let step = |v: u8| v + (f64::from(255 - v) * 0.1).ceil() as u8;
        c = Rgba::rgb(step(c.r), step(c.g), step(c.b));
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_contrast_keeps_readable_colours_and_moves_others() {
        let black = Rgba::rgb(0, 0, 0);
        let white = Rgba::rgb(255, 255, 255);
        let grey = Rgba::rgb(0x80, 0x80, 0x80);
        // Already enough: unchanged.
        assert_eq!(ensure_contrast(white, black, 4.5), white);
        // Dark grey on black: lightened until it reads.
        let dark = Rgba::rgb(0x30, 0x30, 0x30);
        let lighter = ensure_contrast(dark, black, 4.5);
        assert!(lighter.luminance() > dark.luminance());
        assert!(contrast_ratio(lighter, black, black) >= 4.5);
        // Light grey on white: darkened.
        let light = Rgba::rgb(0xd0, 0xd0, 0xd0);
        let darker = ensure_contrast(light, white, 4.5);
        assert!(darker.luminance() < light.luminance());
        assert!(contrast_ratio(darker, white, white) >= 4.5);
        // Mid grey on mid grey: the better of both directions.
        let c = ensure_contrast(grey, grey, 4.5);
        assert!(contrast_ratio(c, grey, grey) > 2.0);
        // Ratio 1 never changes anything; alpha is dropped.
        assert_eq!(ensure_contrast(grey, grey, 1.0), grey);
        assert_eq!(
            ensure_contrast(Rgba::new(255, 255, 255, 10), black, 4.5),
            white
        );
    }
}
