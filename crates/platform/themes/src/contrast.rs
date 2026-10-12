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
        (Text, Notification, Need::Text),
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
    ]
};

/// Text roles that are dimmed on purpose and only need 3:1.
const DIMMED: &[TextRole] = &[TextRole::DiagnosticUnused];

/// Text roles drawn on the terminal background instead of the editor's.
const CONSOLE: &[TextRole] = &[TextRole::ConsoleError, TextRole::ConsoleInput];

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
        | UiRole::DiffAdded
        | UiRole::DiffRemoved
        | UiRole::DiffChanged
        | UiRole::DiffConflict
        | UiRole::Gutter => theme.ui[UiRole::EditorBackground],
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
