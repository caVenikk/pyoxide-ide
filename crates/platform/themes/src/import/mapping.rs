//! The mapping tables from PyCharm keys to Pyoxide tokens. The key names are
//! format identifiers of the colour scheme and UI theme files; no values are
//! stored here. `docs/theme-mapping.md` documents the same tables, and a test
//! checks that the document mentions every key.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::roles::{Metric, TextRole, UiRole};

/// Where a colour for a [`UiRole`] can come from. The sources of a role are
/// tried in order and the first that yields a colour wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Src {
    /// A key of the `ui` section of a UI theme. A two segment key such as
    /// `Panel.background` falls back to the wildcard `*.background`.
    Ui(&'static str),
    /// An option of `<colors>` in a colour scheme.
    Color(&'static str),
    /// Either of the above, for keys that both formats use.
    Key(&'static str),
    /// `FOREGROUND` of a scheme attribute.
    Fg(&'static str),
    /// `BACKGROUND` of a scheme attribute.
    Bg(&'static str),
    /// `EFFECT_COLOR` of a scheme attribute.
    Effect(&'static str),
    /// `ERROR_STRIPE_COLOR` of a scheme attribute.
    Stripe(&'static str),
    /// A key of the icon colour palette of a UI theme (`icons` >
    /// `ColorPalette`): `Green.Stroke`, `Checkbox.Background.Selected`, ...
    /// A dark theme may also write it with `.Dark` appended, and a theme
    /// may name the colour the key stands for in the icons (`#CED0D6`).
    Palette(&'static str),
}

use Src::{Bg, Color, Effect, Fg, Key, Palette, Stripe, Ui};

pub(crate) fn ui_sources(role: UiRole) -> &'static [Src] {
    use UiRole::*;
    match role {
        Surface => &[Ui("Panel.background")],
        SurfaceAlt => &[Ui("SidePanel.background"), Ui("Tree.background")],
        SurfaceRaised => &[
            Ui("Popup.background"),
            Ui("PopupMenu.background"),
            Ui("Menu.background"),
            Ui("ToolTip.background"),
        ],
        Input => &[Ui("TextField.background"), Ui("ComboBox.background")],
        Border => &[
            Ui("Borders.color"),
            Ui("ToolWindow.borderColor"),
            Ui("Popup.borderColor"),
            Ui("Menu.borderColor"),
        ],
        BorderInput => &[
            Ui("Component.borderColor"),
            Ui("TextField.borderColor"),
            Ui("Button.startBorderColor"),
        ],
        Divider => &[
            Ui("Separator.separatorColor"),
            Ui("Menu.separatorColor"),
            Ui("Popup.separatorColor"),
            Ui("MainToolbar.separatorColor"),
        ],
        Text => &[Ui("Label.foreground"), Ui("Panel.foreground")],
        TextMuted => &[Ui("Label.infoForeground"), Ui("Component.infoForeground")],
        TextDisabled => &[
            Ui("Label.disabledForeground"),
            Ui("Label.disabledText"),
            Ui("Component.disabledForeground"),
        ],
        TextLink => &[Ui("Link.activeForeground"), Ui("Link.foreground")],
        Shortcut => &[Ui("Shortcut.foreground")],
        Accent => &[
            Ui("Button.default.startBackground"),
            Ui("Component.accentColor"),
            Ui("ProgressBar.progressColor"),
            Ui("Component.focusColor"),
        ],
        OnAccent => &[Ui("Button.default.foreground")],
        Focus => &[Ui("Component.focusColor"), Ui("Button.default.focusColor")],
        Hover => &[
            Ui("ActionButton.hoverBackground"),
            Ui("List.hoverBackground"),
            Ui("Tree.hoverBackground"),
            Key("Table.hoverBackground"),
        ],
        Pressed => &[Key("ActionButton.pressedBackground")],
        SegmentedSelected => &[Ui("SegmentedButton.selectedButtonColor")],
        SegmentedSelectedBorder => &[Ui("SegmentedButton.selectedStartBorderColor")],
        ButtonBackground => &[Ui("Button.startBackground")],
        ButtonBorder => &[Ui("Button.startBorderColor")],
        ButtonFocusedBorder => &[Ui("Button.focusedBorderColor")],
        ButtonDefaultBorder => &[Ui("Button.default.startBorderColor")],
        ButtonDefaultFocusedBorder => &[Ui("Button.default.focusedBorderColor")],
        Selection => &[
            Ui("List.selectionBackground"),
            Ui("Tree.selectionBackground"),
            Ui("Table.selectionBackground"),
        ],
        SelectionLight => &[
            Ui("Plugins.lightSelectionBackground"),
            Ui("Table.lightSelectionBackground"),
        ],
        SelectionText => &[
            Ui("List.selectionForeground"),
            Ui("Tree.selectionForeground"),
        ],
        SelectionInactive => &[
            Ui("List.selectionInactiveBackground"),
            Ui("Tree.selectionInactiveBackground"),
        ],
        Danger => &[Ui("Label.errorForeground"), Ui("Component.errorFocusColor")],
        Warning => &[
            Ui("Label.warningForeground"),
            Ui("Component.warningFocusColor"),
        ],
        Success => &[Ui("Label.successForeground")],
        Info => &[Ui("Notification.informativeBorderColor")],
        DangerBg => &[
            Ui("Notification.errorBackground"),
            Ui("ValidationTooltip.errorBackground"),
            Ui("Editor.ToolTip.errorBackground"),
        ],
        WarningBg => &[
            Ui("Notification.warningBackground"),
            Ui("ValidationTooltip.warningBackground"),
            Ui("Editor.ToolTip.warningBackground"),
        ],
        SuccessBg => &[Ui("Editor.ToolTip.successBackground")],
        InfoBg => &[Ui("Notification.informativeBackground")],

        TitleBar => &[Ui("TitlePane.background")],
        Toolbar => &[Ui("MainToolbar.background"), Ui("Toolbar.background")],
        StatusBar => &[Ui("StatusBar.background")],
        StatusBarText => &[
            Ui("StatusBar.foreground"),
            Ui("StatusBar.Widget.foreground"),
        ],
        ToolWindow => &[Ui("ToolWindow.background")],
        ToolWindowHeader => &[Ui("ToolWindow.Header.background")],
        ToolWindowStripe => &[Ui("ToolWindow.Stripe.background")],
        TabBar => &[Ui("EditorTabs.background"), Ui("DefaultTabs.background")],
        TabActive => &[
            Ui("EditorTabs.underlinedTabBackground"),
            Ui("EditorTabs.selectedBackground"),
            Ui("DefaultTabs.underlinedTabBackground"),
        ],
        TabInactive => &[Ui("EditorTabs.unselectedBackground")],
        TabHover => &[
            Ui("EditorTabs.hoverBackground"),
            Ui("EditorTabs.hoverColor"),
            Ui("DefaultTabs.hoverBackground"),
        ],
        TabActiveText => &[
            Ui("EditorTabs.underlinedTabForeground"),
            Ui("EditorTabs.selectedForeground"),
            Ui("DefaultTabs.underlinedTabForeground"),
        ],
        TabInactiveText => &[Ui("EditorTabs.foreground"), Ui("TabbedPane.foreground")],
        TabUnderline => &[
            Ui("EditorTabs.underlinedBorderColor"),
            Ui("EditorTabs.underlineColor"),
            Ui("DefaultTabs.underlineColor"),
            Ui("TabbedPane.underlineColor"),
        ],
        // The Islands tab painter (`IslandsTabPainter.getColors`) draws the
        // selected tab of a strip that has no focus, editor tabs and tool
        // window header tabs alike, with these.
        TabSelectedInactive => &[Ui("EditorTabs.inactiveUnderlinedTabBackground")],
        TabUnderlineInactive => &[
            Ui("EditorTabs.inactiveUnderlinedTabBorderColor"),
            Ui("EditorTabs.inactiveUnderlineColor"),
            Ui("DefaultTabs.inactiveUnderlineColor"),
        ],
        Scrollbar => &[
            Key("ScrollBar.thumbColor"),
            Key("ScrollBar.Transparent.thumbColor"),
            Key("ScrollBar.Mac.thumbColor"),
            Key("ScrollBar.Mac.Transparent.thumbColor"),
        ],
        ScrollbarHover => &[
            Key("ScrollBar.hoverThumbColor"),
            Key("ScrollBar.Transparent.hoverThumbColor"),
            Key("ScrollBar.Mac.hoverThumbColor"),
            Key("ScrollBar.Mac.Transparent.hoverThumbColor"),
        ],
        ScrollbarTrack => &[
            Key("ScrollBar.trackColor"),
            Key("ScrollBar.Transparent.trackColor"),
            Key("ScrollBar.Mac.trackColor"),
            Key("ScrollBar.Mac.Transparent.trackColor"),
        ],
        Progress => &[Ui("ProgressBar.progressColor")],
        ProgressTrack => &[Ui("ProgressBar.trackColor")],
        ProgressIndeterminateStart => &[Ui("ProgressBar.indeterminateStartColor")],
        ProgressIndeterminateEnd => &[Ui("ProgressBar.indeterminateEndColor")],
        Notification => &[Ui("Notification.background")],
        NotificationBorder => &[Ui("Notification.borderColor")],
        NotificationText => &[Ui("Notification.foreground")],
        NotificationLink => &[Ui("Notification.linkForeground")],
        NotificationIconHover => &[Ui("Notification.iconHoverBackground")],
        NotificationMore => &[Ui("Notification.MoreButton.background")],
        NotificationMoreText => &[Ui("Notification.MoreButton.foreground")],
        NonProjectFile => &[Ui("FileColor.Yellow")],
        RunWidgetRunning => &[Ui("RunWidget.runningBackground")],
        RunWidgetStop => &[Ui("RunWidget.stopBackground")],
        RunWidgetIcon => &[Ui("RunWidget.iconColor")],
        RunWidgetRunIcon => &[Ui("RunWidget.runIconColor")],
        RunWidgetRunningIcon => &[Ui("RunWidget.runningIconColor")],
        DragAndDropBorder => &[Ui("DragAndDrop.borderColor")],
        DragAndDropRow => &[Ui("DragAndDrop.rowBackground")],
        DragAndDropArea => &[Ui("DragAndDrop.areaBackground")],
        TreeText => &[Ui("Tree.foreground")],
        TreeIndentGuide => &[Ui("Tree.hash")],
        ListButtonHover => &[Ui("List.Button.hoverBackground")],
        ListButtonSeparator => &[Ui("List.Button.separatorColor")],
        CheckboxBackground => &[Palette("Checkbox.Background.Default")],
        CheckboxBorder => &[Palette("Checkbox.Border.Default")],
        CheckboxBackgroundSelected => &[Palette("Checkbox.Background.Selected")],
        CheckboxBorderSelected => &[Palette("Checkbox.Border.Selected")],
        CheckboxCheck => &[Palette("Checkbox.Foreground.Selected")],
        CheckboxBackgroundDisabled => &[Palette("Checkbox.Background.Disabled")],
        CheckboxBorderDisabled => &[Palette("Checkbox.Border.Disabled")],
        CheckboxCheckDisabled => &[Palette("Checkbox.Foreground.Disabled")],
        IconGrayFill => &[Palette("Gray.Fill")],
        IconGraySecondaryStroke => &[Palette("Gray.SecondaryStroke")],
        IconGrayStroke => &[Palette("Gray.Stroke")],
        IconBlueFill => &[Palette("Blue.Fill")],
        IconBlueSolid => &[Palette("Blue.Solid")],
        IconBlueStroke => &[Palette("Blue.Stroke")],
        IconGreenFill => &[Palette("Green.Fill")],
        IconGreenSolid => &[Palette("Green.Solid")],
        IconGreenStroke => &[Palette("Green.Stroke")],
        IconRedFill => &[Palette("Red.Fill")],
        IconRedSolid => &[Palette("Red.Solid")],
        IconRedStroke => &[Palette("Red.Stroke")],
        IconYellowFill => &[Palette("Yellow.Fill")],
        IconYellowSolid => &[Palette("Yellow.Solid")],
        IconYellowStroke => &[Palette("Yellow.Stroke")],
        IconOrangeFill => &[Palette("Orange.Fill")],
        IconOrangeStroke => &[Palette("Orange.Stroke")],
        IconPurpleFill => &[Palette("Purple.Fill")],
        IconPurpleStroke => &[Palette("Purple.Stroke")],
        IconTealFill => &[Palette("Teal.Fill")],
        IconTealStroke => &[Palette("Teal.Stroke")],
        PopupBorder => &[Ui("Popup.borderColor"), Ui("Menu.borderColor")],

        EditorBackground => &[Bg("TEXT"), Ui("Editor.background")],
        EditorText => &[Fg("TEXT"), Ui("Editor.foreground")],
        Gutter => &[Color("EDITOR_GUTTER_BACKGROUND")],
        LineNumber => &[Color("LINE_NUMBERS_COLOR")],
        LineNumberActive => &[Color("LINE_NUMBER_ON_CARET_ROW_COLOR")],
        Caret => &[Color("CARET_COLOR")],
        CurrentLine => &[Color("CARET_ROW_COLOR")],
        EditorSelection => &[Color("SELECTION_BACKGROUND")],
        SearchMatch => &[
            Bg("TEXT_SEARCH_RESULT_ATTRIBUTES"),
            Ui("SearchMatch.startBackground"),
        ],
        SearchMatchActive => &[Bg("SEARCH_RESULT_ATTRIBUTES")],
        SearchMatchStripe => &[Stripe("TEXT_SEARCH_RESULT_ATTRIBUTES")],
        SearchField => &[Ui("Editor.SearchField.background")],
        SearchFieldBorder => &[
            Ui("Editor.SearchField.borderColor"),
            Ui("Editor.Toolbar.borderColor"),
        ],
        SearchFieldError => &[Ui("SearchField.errorForeground")],
        SearchOptionSelected => &[Ui("SearchOption.selectedBackground")],
        IndentGuide => &[Color("INDENT_GUIDE")],
        IndentGuideActive => &[Color("SELECTED_INDENT_GUIDE")],
        RightMargin => &[Color("RIGHT_MARGIN_COLOR")],
        Tearline => &[Color("TEARLINE_COLOR")],
        Whitespace => &[Color("WHITESPACES")],
        SoftWrapSign => &[Color("SOFT_WRAP_SIGN_COLOR")],
        CompletionPopup => &[Color("LOOKUP_COLOR")],
        CompletionSelection => &[Ui("CompletionPopup.selectionBackground")],
        CompletionMatch => &[Ui("CompletionPopup.matchForeground")],
        DocumentationPopup => &[Color("DOCUMENTATION_COLOR")],
        Breakpoint => &[Bg("BREAKPOINT_ATTRIBUTES")],
        ExecutionPoint => &[Bg("EXECUTIONPOINT_ATTRIBUTES")],
        ExecutionPointNotTop => &[Bg("NOT_TOP_FRAME_ATTRIBUTES")],
        DebuggerValueName => &[Ui("Debugger.Variables.valueForeground")],
        DebuggerType => &[Ui("Debugger.Variables.typeForeground")],
        DebuggerChangedValue => &[Ui("Debugger.Variables.changedValueForeground")],
        DebuggerCollectingData => &[Ui("Debugger.Variables.collectingDataForeground")],
        DebuggerEvaluating => &[Ui("Debugger.Variables.evaluatingExpressionForeground")],
        DebuggerModifyingValue => &[Ui("Debugger.Variables.modifyingValueForeground")],
        DebuggerException => &[Ui("Debugger.Variables.exceptionForeground")],
        DebuggerErrorMessage => &[Ui("Debugger.Variables.errorMessageForeground")],
        Bookmark => &[Key("Bookmark.iconBackground")],

        GutterAdded => &[Color("ADDED_LINES_COLOR")],
        GutterModified => &[Color("MODIFIED_LINES_COLOR")],
        GutterDeleted => &[Color("DELETED_LINES_COLOR")],
        GutterWhitespaceModified => &[Color("WHITESPACES_MODIFIED_LINES_COLOR")],
        GutterIgnoredAdded => &[Color("IGNORED_ADDED_LINES_BORDER_COLOR")],
        GutterIgnoredModified => &[Color("IGNORED_MODIFIED_LINES_BORDER_COLOR")],
        GutterIgnoredDeleted => &[Color("IGNORED_DELETED_LINES_BORDER_COLOR")],
        StripeAdded => &[Stripe("DIFF_INSERTED")],
        StripeModified => &[Stripe("DIFF_MODIFIED")],
        StripeDeleted => &[Stripe("DIFF_DELETED")],
        MarkerPopupBorder => &[Ui("VersionControl.MarkerPopup.borderColor")],
        MarkerPopupToolbar => &[Ui("VersionControl.MarkerPopup.Toolbar.background")],
        MarkerPopupText => &[Color("CHANGED_LINES_POPUP")],
        DiffAdded => &[Bg("DIFF_INSERTED")],
        DiffRemoved => &[Bg("DIFF_DELETED")],
        DiffChanged => &[Bg("DIFF_MODIFIED")],
        DiffConflict => &[Bg("DIFF_CONFLICT")],
        GitAdded => &[Color("FILESTATUS_ADDED"), Color("FILESTATUS_COPIED")],
        GitModified => &[Color("FILESTATUS_MODIFIED")],
        GitDeleted => &[Color("FILESTATUS_DELETED")],
        GitRenamed => &[Color("FILESTATUS_RENAMED")],
        GitUntracked => &[Color("FILESTATUS_UNKNOWN")],
        GitIgnored => &[Color("FILESTATUS_IDEA_FILESTATUS_IGNORED")],
        GitConflict => &[
            Color("FILESTATUS_IDEA_FILESTATUS_MERGED_WITH_CONFLICTS"),
            Color("FILESTATUS_changelistConflict"),
        ],
        GitMerged => &[Color("FILESTATUS_MERGED")],

        DiagnosticError => &[
            Effect("ERRORS_ATTRIBUTES"),
            Stripe("ERRORS_ATTRIBUTES"),
            Fg("ERRORS_ATTRIBUTES"),
        ],
        DiagnosticWarning => &[
            Effect("WARNING_ATTRIBUTES"),
            Stripe("WARNING_ATTRIBUTES"),
            Fg("WARNING_ATTRIBUTES"),
        ],
        DiagnosticInfo => &[
            Effect("INFO_ATTRIBUTES"),
            Stripe("INFO_ATTRIBUTES"),
            Fg("INFO_ATTRIBUTES"),
        ],
        DiagnosticHint => &[
            Effect("INFORMATION_ATTRIBUTES"),
            Stripe("INFORMATION_ATTRIBUTES"),
        ],
        DiagnosticErrorStripe => &[Stripe("ERRORS_ATTRIBUTES")],
        DiagnosticWarningStripe => &[Stripe("WARNING_ATTRIBUTES")],
        DiagnosticWeakWarningStripe => &[Stripe("INFO_ATTRIBUTES")],

        TerminalBackground => &[Color("CONSOLE_BACKGROUND_KEY")],
        TerminalForeground => &[Fg("CONSOLE_NORMAL_OUTPUT")],
        TerminalBlack => &[Fg("CONSOLE_BLACK_OUTPUT")],
        TerminalRed => &[Fg("CONSOLE_RED_OUTPUT")],
        TerminalGreen => &[Fg("CONSOLE_GREEN_OUTPUT")],
        TerminalYellow => &[Fg("CONSOLE_YELLOW_OUTPUT")],
        TerminalBlue => &[Fg("CONSOLE_BLUE_OUTPUT")],
        TerminalMagenta => &[Fg("CONSOLE_MAGENTA_OUTPUT")],
        TerminalCyan => &[Fg("CONSOLE_CYAN_OUTPUT")],
        TerminalWhite => &[Fg("CONSOLE_GRAY_OUTPUT")],
        TerminalBrightBlack => &[Fg("CONSOLE_DARKGRAY_OUTPUT")],
        TerminalBrightRed => &[Fg("CONSOLE_RED_BRIGHT_OUTPUT")],
        TerminalBrightGreen => &[Fg("CONSOLE_GREEN_BRIGHT_OUTPUT")],
        TerminalBrightYellow => &[Fg("CONSOLE_YELLOW_BRIGHT_OUTPUT")],
        TerminalBrightBlue => &[Fg("CONSOLE_BLUE_BRIGHT_OUTPUT")],
        TerminalBrightMagenta => &[Fg("CONSOLE_MAGENTA_BRIGHT_OUTPUT")],
        TerminalBrightCyan => &[Fg("CONSOLE_CYAN_BRIGHT_OUTPUT")],
        TerminalBrightWhite => &[Fg("CONSOLE_WHITE_OUTPUT")],
        TerminalDefaultBackground => &[Color("BLOCK_TERMINAL_DEFAULT_BACKGROUND")],
        TerminalDefaultForeground => &[Color("BLOCK_TERMINAL_DEFAULT_FOREGROUND")],
        TerminalAnsiBlack => &[Fg("BLOCK_TERMINAL_BLACK"), Bg("BLOCK_TERMINAL_BLACK")],
        TerminalAnsiRed => &[Fg("BLOCK_TERMINAL_RED"), Bg("BLOCK_TERMINAL_RED")],
        TerminalAnsiGreen => &[Fg("BLOCK_TERMINAL_GREEN"), Bg("BLOCK_TERMINAL_GREEN")],
        TerminalAnsiYellow => &[Fg("BLOCK_TERMINAL_YELLOW"), Bg("BLOCK_TERMINAL_YELLOW")],
        TerminalAnsiBlue => &[Fg("BLOCK_TERMINAL_BLUE"), Bg("BLOCK_TERMINAL_BLUE")],
        TerminalAnsiMagenta => &[Fg("BLOCK_TERMINAL_MAGENTA"), Bg("BLOCK_TERMINAL_MAGENTA")],
        TerminalAnsiCyan => &[Fg("BLOCK_TERMINAL_CYAN"), Bg("BLOCK_TERMINAL_CYAN")],
        TerminalAnsiWhite => &[Fg("BLOCK_TERMINAL_WHITE"), Bg("BLOCK_TERMINAL_WHITE")],
        TerminalAnsiBrightBlack => &[
            Fg("BLOCK_TERMINAL_BLACK_BRIGHT"),
            Bg("BLOCK_TERMINAL_BLACK_BRIGHT"),
        ],
        TerminalAnsiBrightRed => &[
            Fg("BLOCK_TERMINAL_RED_BRIGHT"),
            Bg("BLOCK_TERMINAL_RED_BRIGHT"),
        ],
        TerminalAnsiBrightGreen => &[
            Fg("BLOCK_TERMINAL_GREEN_BRIGHT"),
            Bg("BLOCK_TERMINAL_GREEN_BRIGHT"),
        ],
        TerminalAnsiBrightYellow => &[
            Fg("BLOCK_TERMINAL_YELLOW_BRIGHT"),
            Bg("BLOCK_TERMINAL_YELLOW_BRIGHT"),
        ],
        TerminalAnsiBrightBlue => &[
            Fg("BLOCK_TERMINAL_BLUE_BRIGHT"),
            Bg("BLOCK_TERMINAL_BLUE_BRIGHT"),
        ],
        TerminalAnsiBrightMagenta => &[
            Fg("BLOCK_TERMINAL_MAGENTA_BRIGHT"),
            Bg("BLOCK_TERMINAL_MAGENTA_BRIGHT"),
        ],
        TerminalAnsiBrightCyan => &[
            Fg("BLOCK_TERMINAL_CYAN_BRIGHT"),
            Bg("BLOCK_TERMINAL_CYAN_BRIGHT"),
        ],
        TerminalAnsiBrightWhite => &[
            Fg("BLOCK_TERMINAL_WHITE_BRIGHT"),
            Bg("BLOCK_TERMINAL_WHITE_BRIGHT"),
        ],
        TerminalAnsiBlackBackground => &[Bg("BLOCK_TERMINAL_BLACK"), Fg("BLOCK_TERMINAL_BLACK")],
        TerminalAnsiRedBackground => &[Bg("BLOCK_TERMINAL_RED"), Fg("BLOCK_TERMINAL_RED")],
        TerminalAnsiGreenBackground => &[Bg("BLOCK_TERMINAL_GREEN"), Fg("BLOCK_TERMINAL_GREEN")],
        TerminalAnsiYellowBackground => &[Bg("BLOCK_TERMINAL_YELLOW"), Fg("BLOCK_TERMINAL_YELLOW")],
        TerminalAnsiBlueBackground => &[Bg("BLOCK_TERMINAL_BLUE"), Fg("BLOCK_TERMINAL_BLUE")],
        TerminalAnsiMagentaBackground => {
            &[Bg("BLOCK_TERMINAL_MAGENTA"), Fg("BLOCK_TERMINAL_MAGENTA")]
        }
        TerminalAnsiCyanBackground => &[Bg("BLOCK_TERMINAL_CYAN"), Fg("BLOCK_TERMINAL_CYAN")],
        TerminalAnsiWhiteBackground => &[Bg("BLOCK_TERMINAL_WHITE"), Fg("BLOCK_TERMINAL_WHITE")],
        TerminalAnsiBrightBlackBackground => &[
            Bg("BLOCK_TERMINAL_BLACK_BRIGHT"),
            Fg("BLOCK_TERMINAL_BLACK_BRIGHT"),
        ],
        TerminalAnsiBrightRedBackground => &[
            Bg("BLOCK_TERMINAL_RED_BRIGHT"),
            Fg("BLOCK_TERMINAL_RED_BRIGHT"),
        ],
        TerminalAnsiBrightGreenBackground => &[
            Bg("BLOCK_TERMINAL_GREEN_BRIGHT"),
            Fg("BLOCK_TERMINAL_GREEN_BRIGHT"),
        ],
        TerminalAnsiBrightYellowBackground => &[
            Bg("BLOCK_TERMINAL_YELLOW_BRIGHT"),
            Fg("BLOCK_TERMINAL_YELLOW_BRIGHT"),
        ],
        TerminalAnsiBrightBlueBackground => &[
            Bg("BLOCK_TERMINAL_BLUE_BRIGHT"),
            Fg("BLOCK_TERMINAL_BLUE_BRIGHT"),
        ],
        TerminalAnsiBrightMagentaBackground => &[
            Bg("BLOCK_TERMINAL_MAGENTA_BRIGHT"),
            Fg("BLOCK_TERMINAL_MAGENTA_BRIGHT"),
        ],
        TerminalAnsiBrightCyanBackground => &[
            Bg("BLOCK_TERMINAL_CYAN_BRIGHT"),
            Fg("BLOCK_TERMINAL_CYAN_BRIGHT"),
        ],
        TerminalAnsiBrightWhiteBackground => &[
            Bg("BLOCK_TERMINAL_WHITE_BRIGHT"),
            Fg("BLOCK_TERMINAL_WHITE_BRIGHT"),
        ],
        TerminalSearchMatch => &[Bg("BLOCK_TERMINAL_SEARCH_ENTRY")],
        TerminalSearchMatchText => &[Fg("BLOCK_TERMINAL_SEARCH_ENTRY")],
        TerminalSearchMatchCurrent => &[Bg("BLOCK_TERMINAL_CURRENT_SEARCH_ENTRY")],
        TerminalSearchMatchCurrentText => &[Fg("BLOCK_TERMINAL_CURRENT_SEARCH_ENTRY")],
        TerminalSeparator => &[Color("BLOCK_TERMINAL_PROMPT_SEPARATOR_COLOR")],
        TerminalErrorMarker => &[Color("BLOCK_TERMINAL_ERROR_BLOCK_STROKE_COLOR")],
    }
}

/// The role is coloured by an editor colour scheme: one of its sources is
/// a scheme colour or attribute (the editor, version control markers,
/// diagnostics, the console). An editor scheme chosen apart from the theme
/// replaces these roles and every text role.
pub(crate) fn is_scheme_role(role: UiRole) -> bool {
    ui_sources(role)
        .iter()
        .any(|s| matches!(s, Color(_) | Fg(_) | Bg(_) | Effect(_) | Stripe(_)))
}

/// Scheme attribute keys that style a [`TextRole`], in order of preference.
/// A key that is not the first is reported as an approximation.
pub(crate) fn text_sources(role: TextRole) -> &'static [&'static str] {
    use TextRole::*;
    match role {
        Keyword => &["PY.KEYWORD", "DEFAULT_KEYWORD"],
        ConstantBuiltin => &[],
        String => &["DEFAULT_STRING"],
        StringEscape => &["DEFAULT_VALID_STRING_ESCAPE"],
        StringEscapeInvalid => &["DEFAULT_INVALID_STRING_ESCAPE"],
        Number => &["PY.NUMBER", "DEFAULT_NUMBER"],
        Comment => &[
            "PY.LINE_COMMENT",
            "DEFAULT_LINE_COMMENT",
            "DEFAULT_BLOCK_COMMENT",
        ],
        Docstring => &["PY.DOC_COMMENT", "DEFAULT_DOC_COMMENT"],
        DocstringTag => &["PY.DOC_COMMENT_TAG", "DEFAULT_DOC_COMMENT_TAG"],
        Todo => &["TODO_DEFAULT_ATTRIBUTES", "TODO_ATTRIBUTES"],
        FunctionDecl => &["DEFAULT_FUNCTION_DECLARATION"],
        FunctionCall => &["PY.FUNCTION_CALL", "DEFAULT_FUNCTION_CALL"],
        MethodDecl => &[],
        MethodCall => &["PY.METHOD_CALL"],
        Class => &["DEFAULT_CLASS_REFERENCE", "DEFAULT_CLASS_NAME"],
        ClassDecl => &["PY.CLASS_DEFINITION", "DEFAULT_CLASS_NAME"],
        Parameter => &["DEFAULT_PARAMETER"],
        SelfParameter => &["PY.SELF_PARAMETER"],
        KeywordArgument => &["PY.KEYWORD_ARGUMENT"],
        Variable => &["DEFAULT_LOCAL_VARIABLE", "DEFAULT_IDENTIFIER"],
        VariableReassigned => &[
            "DEFAULT_REASSIGNED_LOCAL_VARIABLE",
            "DEFAULT_REASSIGNED_PARAMETER",
        ],
        Constant => &["DEFAULT_CONSTANT"],
        Attribute => &["DEFAULT_INSTANCE_FIELD", "DEFAULT_STATIC_FIELD"],
        Builtin => &["PY.BUILTIN_NAME"],
        Decorator => &["PY.DECORATOR", "DEFAULT_METADATA"],
        Annotation => &["PY.ANNOTATION", "DEFAULT_METADATA"],
        Module => &[],
        Dunder => &[
            "PY.PREDEFINED_DEFINITION",
            "PY.PREDEFINED_USAGE",
            "DEFAULT_PREDEFINED_SYMBOL",
        ],
        FstringExpression => &[],
        FstringBrace => &["PY.FSTRING_FRAGMENT_BRACES", "PY.FSTRING_FRAGMENT_COLON"],
        Operator => &["PY.OPERATION_SIGN", "DEFAULT_OPERATION_SIGN"],
        Punctuation => &[
            "PY.COMMA",
            "PY.DOT",
            "DEFAULT_COMMA",
            "DEFAULT_DOT",
            "DEFAULT_SEMICOLON",
        ],
        Bracket => &[
            "PY.PARENTHS",
            "PY.BRACKETS",
            "PY.BRACES",
            "DEFAULT_PARENTHS",
            "DEFAULT_BRACKETS",
            "DEFAULT_BRACES",
        ],
        Tag => &["HTML_TAG_NAME", "XML_TAG_NAME", "DEFAULT_TAG"],
        TagAttribute => &[
            "HTML_ATTRIBUTE_NAME",
            "XML_ATTRIBUTE_NAME",
            "DEFAULT_ATTRIBUTE",
        ],
        PropertyKey => &["JSON.PROPERTY_KEY", "YAML_SCALAR_KEY", "PROPERTIES.KEY"],
        Link => &["HYPERLINK_ATTRIBUTES"],
        SqlTable => &[],
        SqlColumn => &[],
        SqlAlias => &[],
        SqlSchema => &[],

        DiagnosticError => &["ERRORS_ATTRIBUTES"],
        DiagnosticWarning => &["WARNING_ATTRIBUTES"],
        DiagnosticInfo => &["INFO_ATTRIBUTES"],
        DiagnosticHint => &["INFORMATION_ATTRIBUTES"],
        DiagnosticUnresolved => &["WRONG_REFERENCES_ATTRIBUTES"],
        DiagnosticUnused => &["NOT_USED_ELEMENT_ATTRIBUTES"],
        DiagnosticDeprecated => &["DEPRECATED_ATTRIBUTES"],
        Typo => &["TYPO"],
        MatchingBracket => &["MATCHED_BRACE_ATTRIBUTES"],
        UnmatchedBracket => &["UNMATCHED_BRACE_ATTRIBUTES"],
        IdentifierRead => &["IDENTIFIER_UNDER_CARET_ATTRIBUTES"],
        IdentifierWrite => &["WRITE_IDENTIFIER_UNDER_CARET_ATTRIBUTES"],
        FoldedText => &["FOLDED_TEXT_ATTRIBUTES"],
        InlayHint => &["INLAY_DEFAULT", "INLINE_PARAMETER_HINT"],
        ConsoleError => &["CONSOLE_ERROR_OUTPUT"],
        ConsoleInput => &["CONSOLE_USER_INPUT"],
        ConsoleSystem => &["CONSOLE_SYSTEM_OUTPUT"],
        DebuggerInlinedValues => &["DEBUGGER_INLINED_VALUES"],
        DebuggerInlinedValuesModified => &["DEBUGGER_INLINED_VALUES_MODIFIED"],
        DebuggerInlinedValuesExecutionLine => &["DEBUGGER_INLINED_VALUES_EXECUTION_LINE"],
        DebuggerEvaluatedExpression => &["EVALUATED_EXPRESSION_ATTRIBUTES"],
        DebuggerEvaluatedExpressionExecutionLine => {
            &["EVALUATED_EXPRESSION_EXECUTION_LINE_ATTRIBUTES"]
        }
        ReferenceLink => &["CTRL_CLICKABLE"],
    }
}

/// UI theme keys with a number for a [`Metric`] and the factor that turns the
/// value into logical pixels. `arc` values are diameters of the rounding.
pub(crate) fn metric_sources(metric: Metric) -> (&'static [&'static str], f32) {
    match metric {
        Metric::RowHeight => (&["Tree.rowHeight", "List.rowHeight"], 1.0),
        Metric::CornerRadius => (&["Component.arc", "Button.arc"], 0.5),
        Metric::ButtonRadius => (&["MainToolbar.Button.arc"], 0.5),
        Metric::IslandRadius => (&["Island.arc"], 0.5),
        Metric::FocusWidth => (&["Component.focusWidth"], 1.0),
        _ => (&[], 1.0),
    }
}

/// Every key the tables mention, to tell mapped keys from unmapped ones.
pub(crate) struct KnownKeys {
    /// `ui` keys, exactly as written.
    pub ui: HashSet<&'static str>,
    /// Property names (`background`) that a wildcard `*.background` supplies.
    pub wildcard_properties: HashSet<&'static str>,
    pub colors: HashSet<&'static str>,
    pub attributes: HashSet<&'static str>,
    /// Keys of the icon colour palette.
    pub palette: HashSet<&'static str>,
}

pub(crate) fn known_keys() -> &'static KnownKeys {
    static KEYS: OnceLock<KnownKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys = KnownKeys {
            ui: HashSet::new(),
            wildcard_properties: HashSet::new(),
            colors: HashSet::new(),
            attributes: HashSet::new(),
            palette: HashSet::new(),
        };
        for &role in UiRole::ALL {
            for src in ui_sources(role) {
                match *src {
                    Ui(key) => {
                        keys.ui.insert(key);
                    }
                    Color(key) => {
                        keys.colors.insert(key);
                    }
                    Key(key) => {
                        keys.ui.insert(key);
                        keys.colors.insert(key);
                    }
                    Fg(key) | Bg(key) | Effect(key) | Stripe(key) => {
                        keys.attributes.insert(key);
                    }
                    Palette(key) => {
                        keys.palette.insert(key);
                    }
                }
            }
        }
        for &role in TextRole::ALL {
            keys.attributes.extend(text_sources(role));
        }
        // Read into the `TEXT` background of its scheme, see
        // `Scheme::apply_background_option`.
        keys.colors.insert(super::scheme::BACKGROUND_OPTION);
        for &metric in Metric::ALL {
            keys.ui.extend(metric_sources(metric).0);
        }
        let wild: Vec<&'static str> = keys.ui.iter().copied().collect();
        for key in wild {
            if let Some(property) = wildcard_property(key) {
                keys.wildcard_properties.insert(property);
            }
        }
        keys
    })
}

/// `background` for `Panel.background`; `None` for keys with more or fewer
/// than two segments, which have no wildcard form.
pub(crate) fn wildcard_property(key: &str) -> Option<&str> {
    let (group, property) = key.split_once('.')?;
    (!group.is_empty() && !property.contains('.')).then_some(property)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_forms_exist_for_two_segment_keys_only() {
        assert_eq!(wildcard_property("Panel.background"), Some("background"));
        assert_eq!(wildcard_property("Button.default.foreground"), None);
        assert_eq!(wildcard_property("TEXT"), None);
    }

    #[test]
    fn known_keys_cover_the_tables() {
        let keys = known_keys();
        assert!(keys.ui.contains("Panel.background"));
        assert!(keys.colors.contains("CARET_ROW_COLOR"));
        assert!(keys.attributes.contains("PY.BUILTIN_NAME"));
        assert!(keys.attributes.contains("TEXT"));
        assert!(keys.colors.contains("BACKGROUND"));
        assert!(keys.wildcard_properties.contains("background"));
        assert!(keys.colors.contains("ScrollBar.thumbColor"));
        assert!(keys.ui.contains("ScrollBar.thumbColor"));
    }

    /// `docs/theme-mapping.md` must mention every key and role the tables use.
    #[test]
    fn the_mapping_document_lists_every_key_and_role() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/theme-mapping.md"
        );
        let doc = std::fs::read_to_string(path).expect("docs/theme-mapping.md exists");
        let mut missing: Vec<String> = Vec::new();
        let mut need = |text: String| {
            if !doc.contains(&text) {
                missing.push(text);
            }
        };
        let keys = known_keys();
        for key in keys
            .ui
            .iter()
            .chain(&keys.colors)
            .chain(&keys.attributes)
            .chain(&keys.palette)
        {
            need(format!("`{key}`"));
        }
        for role in UiRole::ALL {
            need(format!("`ui.{role}`"));
        }
        for role in TextRole::ALL {
            need(format!("`syntax.{role}`"));
        }
        for metric in Metric::ALL {
            need(format!("`metrics.{metric}`"));
        }
        for theme in super::super::known::KNOWN_THEMES {
            need(format!("`{}`", theme.id));
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "missing from docs/theme-mapping.md: {missing:?}"
        );
    }

    #[test]
    fn text_roles_with_sources_name_known_python_keys() {
        for key in [
            "PY.BUILTIN_NAME",
            "PY.SELF_PARAMETER",
            "PY.KEYWORD_ARGUMENT",
            "PY.DECORATOR",
            "PY.ANNOTATION",
            "DEFAULT_KEYWORD",
            "DEFAULT_STRING",
        ] {
            assert!(
                TextRole::ALL
                    .iter()
                    .any(|&r| text_sources(r).contains(&key)),
                "{key} is not mapped"
            );
        }
    }
}
