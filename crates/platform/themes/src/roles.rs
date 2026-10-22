//! The vocabulary of a theme: UI colour roles, text roles (syntax tokens and
//! editor decorations) and layout metrics. Every role has a stable kebab-case
//! name used in theme files and in `docs/theme-mapping.md`.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! roles {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $variant:ident => ($text:literal, $group:literal, $doc:literal), )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis enum $name {
            $( $variant, )*
        }

        impl $name {
            /// Every role in declaration order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant, )* ];
            /// Number of roles.
            pub const COUNT: usize = Self::ALL.len();

            /// Name used in theme files.
            pub const fn name(self) -> &'static str {
                match self { $( $name::$variant => $text, )* }
            }

            /// Group heading used in the documentation.
            pub const fn group(self) -> &'static str {
                match self { $( $name::$variant => $group, )* }
            }

            /// One-line description of what the role colours.
            pub const fn description(self) -> &'static str {
                match self { $( $name::$variant => $doc, )* }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                match name { $( $text => Some($name::$variant), )* _ => None }
            }

            pub const fn index(self) -> usize {
                self as usize
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.name())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::from_name(&text).ok_or_else(|| {
                    D::Error::custom(format!(
                        concat!("unknown ", stringify!($name), " `{}`"),
                        text
                    ))
                })
            }
        }
    };
}

roles! {
    /// A flat colour of the user interface or of the editor chrome.
    pub enum UiRole {
        Surface => ("surface", "General", "Main window and panel background"),
        SurfaceAlt => ("surface-alt", "General", "Secondary panels such as the project tree and side panels"),
        SurfaceRaised => ("surface-raised", "General", "Popups, menus, dialogs and tooltips"),
        Input => ("input", "General", "Background of text fields and combo boxes"),
        Border => ("border", "General", "Borders between panels"),
        BorderInput => ("border-input", "General", "Outline of inputs and buttons"),
        Divider => ("divider", "General", "Thin separators inside a panel"),
        PopupBorder => ("popup-border", "General", "Border of popups and menus, and the separators in menus"),
        Text => ("text", "General", "Primary text"),
        TextMuted => ("text-muted", "General", "Secondary text and hints"),
        TextDisabled => ("text-disabled", "General", "Text of disabled controls"),
        TextLink => ("text-link", "General", "Links in the user interface"),
        Shortcut => ("shortcut", "General", "Keyboard shortcuts shown after an action, as in the empty editor area"),
        Accent => ("accent", "General", "Primary action and focus accent"),
        OnAccent => ("on-accent", "General", "Text and icons on the accent colour"),
        Focus => ("focus", "General", "Keyboard focus ring"),
        Hover => ("hover", "General", "Background of a hovered row or button"),
        Pressed => ("pressed", "General", "Background of a pressed button"),
        SegmentedSelected => ("segmented-selected", "General", "Background of the selected choice of a segmented button"),
        SegmentedSelectedBorder => ("segmented-selected-border", "General", "Outline of the selected choice of a segmented button"),
        Selection => ("selection", "General", "Background of the selected row in lists and trees"),
        SelectionText => ("selection-text", "General", "Text on the selected row"),
        SelectionInactive => ("selection-inactive", "General", "Selected row in a list that has no focus"),
        SelectionLight => ("selection-light", "General", "Selected row of a list of rich rows (the projects of the project widget's popup and of the Welcome tab)"),
        Danger => ("danger", "General", "Errors and destructive actions"),
        Warning => ("warning", "General", "Warnings"),
        Success => ("success", "General", "Success and completed states"),
        Info => ("info", "General", "Informational accents"),
        DangerBg => ("danger-bg", "General", "Background of error notices"),
        WarningBg => ("warning-bg", "General", "Background of warning notices"),
        SuccessBg => ("success-bg", "General", "Background of success notices"),
        InfoBg => ("info-bg", "General", "Background of informational notices"),

        TitleBar => ("title-bar", "Chrome", "Window title bar"),
        Toolbar => ("toolbar", "Chrome", "Main toolbar"),
        StatusBar => ("status-bar", "Chrome", "Status bar background"),
        StatusBarText => ("status-bar-text", "Chrome", "Status bar text"),
        ToolWindow => ("tool-window", "Chrome", "Tool window content background"),
        ToolWindowHeader => ("tool-window-header", "Chrome", "Tool window title strip"),
        ToolWindowStripe => ("tool-window-stripe", "Chrome", "Strip with the tool window buttons"),
        TabBar => ("tab-bar", "Chrome", "Strip behind the editor tabs"),
        TabActive => ("tab-active", "Chrome", "Selected editor tab"),
        TabInactive => ("tab-inactive", "Chrome", "Editor tab that is not selected"),
        TabHover => ("tab-hover", "Chrome", "Hovered editor tab"),
        TabActiveText => ("tab-active-text", "Chrome", "Title of the selected tab"),
        TabInactiveText => ("tab-inactive-text", "Chrome", "Title of other tabs"),
        TabUnderline => ("tab-underline", "Chrome", "Indicator under the selected tab"),
        TabSelectedInactive => ("tab-selected-inactive", "Chrome", "Selected tab of a tab strip without the focus: the editor tabs, a tool window's header tabs"),
        TabUnderlineInactive => ("tab-underline-inactive", "Chrome", "Outline of the selected tab of a tab strip without the focus"),
        Scrollbar => ("scrollbar", "Chrome", "Scrollbar thumb"),
        ScrollbarHover => ("scrollbar-hover", "Chrome", "Hovered scrollbar thumb"),
        ScrollbarTrack => ("scrollbar-track", "Chrome", "Scrollbar track"),
        Progress => ("progress", "Chrome", "Progress bar fill"),
        ProgressTrack => ("progress-track", "Chrome", "Progress bar track"),
        Notification => ("notification", "Chrome", "Background of notification balloons"),
        NotificationBorder => ("notification-border", "Chrome", "Border of notification balloons"),
        NotificationText => ("notification-text", "Chrome", "Title and content of notification balloons"),
        NotificationLink => ("notification-link", "Chrome", "Action links of notification balloons"),
        NotificationIconHover => ("notification-icon-hover", "Chrome", "Background of the hovered close button of a notification balloon"),
        NotificationMore => ("notification-more", "Chrome", "Strip below the balloons that counts the collapsed notifications"),
        NotificationMoreText => ("notification-more-text", "Chrome", "Text of that strip"),
        NonProjectFile => ("non-project-file", "Chrome", "Row background of files outside the project content, such as excluded folders"),
        RunWidgetRunning => ("run-widget-running", "Chrome", "Background of the run widget's Run button while its process runs"),
        RunWidgetStop => ("run-widget-stop", "Chrome", "Background of the run widget's Stop button"),
        RunWidgetIcon => ("run-widget-icon", "Chrome", "Icons of the run widget's buttons other than Run and Debug, such as More Actions"),
        RunWidgetRunIcon => ("run-widget-run-icon", "Chrome", "Run and Debug icons of the run widget"),
        RunWidgetRunningIcon => ("run-widget-running-icon", "Chrome", "Icons on the run widget's coloured buttons (Rerun while a process runs, Stop)"),
        DragAndDropBorder => ("drag-and-drop-border", "Chrome", "Border of the row that takes dragged files or folders"),
        DragAndDropRow => ("drag-and-drop-row", "Chrome", "Background of the row that takes dragged files or folders"),
        TreeText => ("tree-text", "Chrome", "Text of tree rows, such as the pages of the Settings window and the Keymap and Run/Debug Configurations trees"),
        TreeIndentGuide => ("tree-indent-guide", "Chrome", "Indent guides of trees: the vertical lines below an open folder"),
        ListButtonHover => ("list-button-hover", "Chrome", "Background of a hovered button at the end of a popup row, such as Run in the run widget's dropdown"),
        ListButtonSeparator => ("list-button-separator", "Chrome", "Line before the buttons at the end of a popup row"),

        CheckboxBackground => ("checkbox-background", "Controls", "Fill of a check box that is off"),
        CheckboxBorder => ("checkbox-border", "Controls", "Outline of a check box that is off"),
        CheckboxBackgroundSelected => ("checkbox-background-selected", "Controls", "Fill of a check box that is on"),
        CheckboxBorderSelected => ("checkbox-border-selected", "Controls", "Outline of a check box that is on"),
        CheckboxCheck => ("checkbox-check", "Controls", "Check mark of a check box that is on"),
        CheckboxBackgroundDisabled => ("checkbox-background-disabled", "Controls", "Fill of a disabled check box"),
        CheckboxBorderDisabled => ("checkbox-border-disabled", "Controls", "Outline of a disabled check box"),
        CheckboxCheckDisabled => ("checkbox-check-disabled", "Controls", "Check mark of a disabled check box"),

        IconGrayFill => ("icon-gray-fill", "Icons", "Icon colour Gray.Fill: the inside of grey icons"),
        IconGraySecondaryStroke => ("icon-gray-secondary-stroke", "Icons", "Icon colour Gray.SecondaryStroke: secondary lines of grey icons, such as the circle of the help icon"),
        IconGrayStroke => ("icon-gray-stroke", "Icons", "Icon colour Gray.Stroke: the lines of grey icons, such as folders and most actions"),
        IconBlueFill => ("icon-blue-fill", "Icons", "Icon colour Blue.Fill: the inside of blue icons, such as source roots"),
        IconBlueSolid => ("icon-blue-solid", "Icons", "Icon colour Blue.Solid: solid blue shapes (light themes)"),
        IconBlueStroke => ("icon-blue-stroke", "Icons", "Icon colour Blue.Stroke: the lines of blue icons, such as source roots and incoming commits"),
        IconGreenFill => ("icon-green-fill", "Icons", "Icon colour Green.Fill: the inside of green icons, such as Run"),
        IconGreenSolid => ("icon-green-solid", "Icons", "Icon colour Green.Solid: solid green shapes (light themes)"),
        IconGreenStroke => ("icon-green-stroke", "Icons", "Icon colour Green.Stroke: the lines of green icons, such as Run, Debug, test roots and outgoing commits"),
        IconRedFill => ("icon-red-fill", "Icons", "Icon colour Red.Fill: the inside of red icons"),
        IconRedSolid => ("icon-red-solid", "Icons", "Icon colour Red.Solid: solid red shapes (light themes)"),
        IconRedStroke => ("icon-red-stroke", "Icons", "Icon colour Red.Stroke: the lines of red icons, such as Stop"),
        IconYellowFill => ("icon-yellow-fill", "Icons", "Icon colour Yellow.Fill: the inside of yellow icons"),
        IconYellowSolid => ("icon-yellow-solid", "Icons", "Icon colour Yellow.Solid: solid yellow shapes, such as the intention bulb"),
        IconYellowStroke => ("icon-yellow-stroke", "Icons", "Icon colour Yellow.Stroke: the lines of yellow icons"),
        IconOrangeFill => ("icon-orange-fill", "Icons", "Icon colour Orange.Fill: the inside of orange icons, such as excluded folders"),
        IconOrangeStroke => ("icon-orange-stroke", "Icons", "Icon colour Orange.Stroke: the lines of orange icons, such as excluded folders"),
        IconPurpleFill => ("icon-purple-fill", "Icons", "Icon colour Purple.Fill: the inside of purple icons"),
        IconPurpleStroke => ("icon-purple-stroke", "Icons", "Icon colour Purple.Stroke: the lines of purple icons"),
        IconTealFill => ("icon-teal-fill", "Icons", "Icon colour Teal.Fill: the inside of teal icons"),
        IconTealStroke => ("icon-teal-stroke", "Icons", "Icon colour Teal.Stroke: the lines of teal icons"),

        EditorBackground => ("editor-background", "Editor", "Editor background"),
        EditorText => ("editor-text", "Editor", "Default editor text"),
        Gutter => ("gutter", "Editor", "Gutter background"),
        LineNumber => ("line-number", "Editor", "Line numbers"),
        LineNumberActive => ("line-number-active", "Editor", "Line number of the caret row"),
        Caret => ("caret", "Editor", "Text caret"),
        CurrentLine => ("current-line", "Editor", "Background of the caret row"),
        EditorSelection => ("editor-selection", "Editor", "Selected text background"),
        SearchMatch => ("search-match", "Editor", "Background of search matches"),
        SearchMatchActive => ("search-match-active", "Editor", "Background of the current search match"),
        SearchMatchStripe => ("search-match-stripe", "Editor", "Marks of search matches on the scrollbar"),
        SearchField => ("search-field", "Editor", "Background of the find bar at the top of the editor"),
        SearchFieldBorder => ("search-field-border", "Editor", "Line between the find bar and the text"),
        SearchFieldError => ("search-field-error", "Editor", "Text of a search field whose search finds nothing"),
        SearchOptionSelected => ("search-option-selected", "Editor", "Background of a search option that is on (Match Case, Words, Regex)"),
        IndentGuide => ("indent-guide", "Editor", "Indent guides"),
        IndentGuideActive => ("indent-guide-active", "Editor", "Indent guide of the caret block"),
        RightMargin => ("right-margin", "Editor", "Right margin line"),
        Tearline => ("tearline", "Editor", "Line under the titles of a diff's sides"),
        Whitespace => ("whitespace", "Editor", "Visible whitespace markers"),
        SoftWrapSign => ("soft-wrap-sign", "Editor", "Signs at the soft wraps of a line"),
        CompletionPopup => ("completion-popup", "Editor", "Completion popup background"),
        CompletionSelection => ("completion-selection", "Editor", "Selected completion item"),
        CompletionMatch => ("completion-match", "Editor", "Matched characters in completion items"),
        DocumentationPopup => ("documentation-popup", "Editor", "Documentation popup background"),
        Breakpoint => ("breakpoint", "Editor", "Line with a breakpoint"),
        ExecutionPoint => ("execution-point", "Editor", "Line the debugger stopped on"),
        ExecutionPointNotTop => ("execution-point-not-top", "Editor", "Line of a frame the debugger shows other than the one it stopped in"),
        Bookmark => ("bookmark", "Editor", "Bookmark marker"),

        GutterAdded => ("gutter-added", "Version control", "Gutter marker of added lines"),
        GutterModified => ("gutter-modified", "Version control", "Gutter marker of modified lines"),
        GutterDeleted => ("gutter-deleted", "Version control", "Gutter marker of deleted lines"),
        GutterWhitespaceModified => ("gutter-whitespace-modified", "Version control", "Gutter marker of lines changed only in whitespace"),
        GutterIgnoredAdded => ("gutter-ignored-added", "Version control", "Outline marker of added lines that are left out (another changelist)"),
        GutterIgnoredModified => ("gutter-ignored-modified", "Version control", "Outline marker of modified lines that are left out"),
        GutterIgnoredDeleted => ("gutter-ignored-deleted", "Version control", "Outline marker of deleted lines that are left out"),
        StripeAdded => ("stripe-added", "Version control", "Error stripe mark of added lines"),
        StripeModified => ("stripe-modified", "Version control", "Error stripe mark of modified lines"),
        StripeDeleted => ("stripe-deleted", "Version control", "Error stripe mark of deleted lines"),
        MarkerPopupBorder => ("marker-popup-border", "Version control", "Border of the popup of a change marker"),
        MarkerPopupToolbar => ("marker-popup-toolbar", "Version control", "Toolbar background of the popup of a change marker"),
        MarkerPopupText => ("marker-popup-text", "Version control", "Background of the old text in the popup of a change marker"),
        DiffAdded => ("diff-added", "Version control", "Added lines in the diff viewer"),
        DiffRemoved => ("diff-removed", "Version control", "Removed lines in the diff viewer"),
        DiffChanged => ("diff-changed", "Version control", "Changed lines in the diff viewer"),
        DiffConflict => ("diff-conflict", "Version control", "Conflicting lines in the diff viewer"),
        GitAdded => ("git-added", "Version control", "File name colour: added"),
        GitModified => ("git-modified", "Version control", "File name colour: modified"),
        GitDeleted => ("git-deleted", "Version control", "File name colour: deleted"),
        GitRenamed => ("git-renamed", "Version control", "File name colour: renamed"),
        GitUntracked => ("git-untracked", "Version control", "File name colour: untracked"),
        GitIgnored => ("git-ignored", "Version control", "File name colour: ignored"),
        GitConflict => ("git-conflict", "Version control", "File name colour: merge conflict"),
        GitMerged => ("git-merged", "Version control", "File name colour: changed by a merge"),

        DiagnosticError => ("diagnostic-error", "Diagnostics", "Error marker, underline and stripe"),
        DiagnosticWarning => ("diagnostic-warning", "Diagnostics", "Warning marker, underline and stripe"),
        DiagnosticInfo => ("diagnostic-info", "Diagnostics", "Weak warning and information marker"),
        DiagnosticHint => ("diagnostic-hint", "Diagnostics", "Hint marker"),
        DiagnosticErrorStripe => ("diagnostic-error-stripe", "Diagnostics", "Error mark on the error stripe and error icon"),
        DiagnosticWarningStripe => ("diagnostic-warning-stripe", "Diagnostics", "Warning mark on the error stripe and warning icon"),
        DiagnosticWeakWarningStripe => ("diagnostic-weak-warning-stripe", "Diagnostics", "Weak warning mark on the error stripe and weak warning icon"),

        TerminalBackground => ("terminal-background", "Terminal", "Run console background"),
        TerminalForeground => ("terminal-foreground", "Terminal", "Run console default text"),
        TerminalBlack => ("terminal-black", "Terminal", "ANSI colour 0 in the Run console"),
        TerminalRed => ("terminal-red", "Terminal", "ANSI colour 1 in the Run console"),
        TerminalGreen => ("terminal-green", "Terminal", "ANSI colour 2 in the Run console"),
        TerminalYellow => ("terminal-yellow", "Terminal", "ANSI colour 3 in the Run console"),
        TerminalBlue => ("terminal-blue", "Terminal", "ANSI colour 4 in the Run console"),
        TerminalMagenta => ("terminal-magenta", "Terminal", "ANSI colour 5 in the Run console"),
        TerminalCyan => ("terminal-cyan", "Terminal", "ANSI colour 6 in the Run console"),
        TerminalWhite => ("terminal-white", "Terminal", "ANSI colour 7 in the Run console"),
        TerminalBrightBlack => ("terminal-bright-black", "Terminal", "ANSI colour 8 in the Run console"),
        TerminalBrightRed => ("terminal-bright-red", "Terminal", "ANSI colour 9 in the Run console"),
        TerminalBrightGreen => ("terminal-bright-green", "Terminal", "ANSI colour 10 in the Run console"),
        TerminalBrightYellow => ("terminal-bright-yellow", "Terminal", "ANSI colour 11 in the Run console"),
        TerminalBrightBlue => ("terminal-bright-blue", "Terminal", "ANSI colour 12 in the Run console"),
        TerminalBrightMagenta => ("terminal-bright-magenta", "Terminal", "ANSI colour 13 in the Run console"),
        TerminalBrightCyan => ("terminal-bright-cyan", "Terminal", "ANSI colour 14 in the Run console"),
        TerminalBrightWhite => ("terminal-bright-white", "Terminal", "ANSI colour 15 in the Run console"),
        TerminalDefaultBackground => ("terminal-default-background", "Terminal", "Terminal tool window background"),
        TerminalDefaultForeground => ("terminal-default-foreground", "Terminal", "Terminal tool window default text"),
        TerminalAnsiBlack => ("terminal-ansi-black", "Terminal", "Text of ANSI colour 0 in the terminal"),
        TerminalAnsiRed => ("terminal-ansi-red", "Terminal", "Text of ANSI colour 1 in the terminal"),
        TerminalAnsiGreen => ("terminal-ansi-green", "Terminal", "Text of ANSI colour 2 in the terminal"),
        TerminalAnsiYellow => ("terminal-ansi-yellow", "Terminal", "Text of ANSI colour 3 in the terminal"),
        TerminalAnsiBlue => ("terminal-ansi-blue", "Terminal", "Text of ANSI colour 4 in the terminal"),
        TerminalAnsiMagenta => ("terminal-ansi-magenta", "Terminal", "Text of ANSI colour 5 in the terminal"),
        TerminalAnsiCyan => ("terminal-ansi-cyan", "Terminal", "Text of ANSI colour 6 in the terminal"),
        TerminalAnsiWhite => ("terminal-ansi-white", "Terminal", "Text of ANSI colour 7 in the terminal"),
        TerminalAnsiBrightBlack => ("terminal-ansi-bright-black", "Terminal", "Text of ANSI colour 8 in the terminal"),
        TerminalAnsiBrightRed => ("terminal-ansi-bright-red", "Terminal", "Text of ANSI colour 9 in the terminal"),
        TerminalAnsiBrightGreen => ("terminal-ansi-bright-green", "Terminal", "Text of ANSI colour 10 in the terminal"),
        TerminalAnsiBrightYellow => ("terminal-ansi-bright-yellow", "Terminal", "Text of ANSI colour 11 in the terminal"),
        TerminalAnsiBrightBlue => ("terminal-ansi-bright-blue", "Terminal", "Text of ANSI colour 12 in the terminal"),
        TerminalAnsiBrightMagenta => ("terminal-ansi-bright-magenta", "Terminal", "Text of ANSI colour 13 in the terminal"),
        TerminalAnsiBrightCyan => ("terminal-ansi-bright-cyan", "Terminal", "Text of ANSI colour 14 in the terminal"),
        TerminalAnsiBrightWhite => ("terminal-ansi-bright-white", "Terminal", "Text of ANSI colour 15 in the terminal"),
        TerminalAnsiBlackBackground => ("terminal-ansi-black-background", "Terminal", "Cell background of ANSI colour 0 in the terminal"),
        TerminalAnsiRedBackground => ("terminal-ansi-red-background", "Terminal", "Cell background of ANSI colour 1 in the terminal"),
        TerminalAnsiGreenBackground => ("terminal-ansi-green-background", "Terminal", "Cell background of ANSI colour 2 in the terminal"),
        TerminalAnsiYellowBackground => ("terminal-ansi-yellow-background", "Terminal", "Cell background of ANSI colour 3 in the terminal"),
        TerminalAnsiBlueBackground => ("terminal-ansi-blue-background", "Terminal", "Cell background of ANSI colour 4 in the terminal"),
        TerminalAnsiMagentaBackground => ("terminal-ansi-magenta-background", "Terminal", "Cell background of ANSI colour 5 in the terminal"),
        TerminalAnsiCyanBackground => ("terminal-ansi-cyan-background", "Terminal", "Cell background of ANSI colour 6 in the terminal"),
        TerminalAnsiWhiteBackground => ("terminal-ansi-white-background", "Terminal", "Cell background of ANSI colour 7 in the terminal"),
        TerminalAnsiBrightBlackBackground => ("terminal-ansi-bright-black-background", "Terminal", "Cell background of ANSI colour 8 in the terminal"),
        TerminalAnsiBrightRedBackground => ("terminal-ansi-bright-red-background", "Terminal", "Cell background of ANSI colour 9 in the terminal"),
        TerminalAnsiBrightGreenBackground => ("terminal-ansi-bright-green-background", "Terminal", "Cell background of ANSI colour 10 in the terminal"),
        TerminalAnsiBrightYellowBackground => ("terminal-ansi-bright-yellow-background", "Terminal", "Cell background of ANSI colour 11 in the terminal"),
        TerminalAnsiBrightBlueBackground => ("terminal-ansi-bright-blue-background", "Terminal", "Cell background of ANSI colour 12 in the terminal"),
        TerminalAnsiBrightMagentaBackground => ("terminal-ansi-bright-magenta-background", "Terminal", "Cell background of ANSI colour 13 in the terminal"),
        TerminalAnsiBrightCyanBackground => ("terminal-ansi-bright-cyan-background", "Terminal", "Cell background of ANSI colour 14 in the terminal"),
        TerminalAnsiBrightWhiteBackground => ("terminal-ansi-bright-white-background", "Terminal", "Cell background of ANSI colour 15 in the terminal"),
        TerminalSearchMatch => ("terminal-search-match", "Terminal", "Background of search matches in the terminal"),
        TerminalSearchMatchText => ("terminal-search-match-text", "Terminal", "Text of search matches in the terminal"),
        TerminalSearchMatchCurrent => ("terminal-search-match-current", "Terminal", "Background of the current search match in the terminal"),
        TerminalSearchMatchCurrentText => ("terminal-search-match-current-text", "Terminal", "Text of the current search match in the terminal"),
        TerminalSeparator => ("terminal-separator", "Terminal", "Line between the commands of the terminal"),
        TerminalErrorMarker => ("terminal-error-marker", "Terminal", "Marker beside a terminal command that failed"),

        DebuggerValueName => ("debugger-value-name", "Debugger", "Names in the variables view of the debugger"),
        DebuggerType => ("debugger-type", "Debugger", "Types in the variables view of the debugger: {list: 4}"),
        DebuggerChangedValue => ("debugger-changed-value", "Debugger", "Names of values that changed since the debugger last stopped"),
        DebuggerCollectingData => ("debugger-collecting-data", "Debugger", "Collecting data while the debugger loads values"),
        DebuggerEvaluating => ("debugger-evaluating", "Debugger", "Evaluating while the debugger evaluates an expression"),
        DebuggerModifyingValue => ("debugger-modifying-value", "Debugger", "Modifying value while the debugger sets a value"),
        DebuggerException => ("debugger-exception", "Debugger", "Exceptions in the variables view of the debugger"),
        DebuggerErrorMessage => ("debugger-error-message", "Debugger", "Error messages in the variables view of the debugger"),
    }
}

roles! {
    /// Something drawn with text attributes: a syntax token or an editor
    /// decoration.
    pub enum TextRole {
        Keyword => ("keyword", "Syntax", "Keywords: def, class, if, return, import, ..."),
        ConstantBuiltin => ("constant-builtin", "Syntax", "True, False, None and Ellipsis"),
        String => ("string", "Syntax", "String and bytes literals"),
        StringEscape => ("string-escape", "Syntax", "Valid escape sequence in a string"),
        StringEscapeInvalid => ("string-escape-invalid", "Syntax", "Invalid escape sequence in a string"),
        Number => ("number", "Syntax", "Numeric literals"),
        Comment => ("comment", "Syntax", "Line and block comments"),
        Docstring => ("docstring", "Syntax", "Docstrings and documentation comments"),
        DocstringTag => ("docstring-tag", "Syntax", "Tags inside docstrings such as :param or @param"),
        Todo => ("todo", "Syntax", "TODO and FIXME markers in comments"),
        FunctionDecl => ("function-decl", "Syntax", "Name in a function definition"),
        FunctionCall => ("function-call", "Syntax", "Name of a called function"),
        MethodDecl => ("method-decl", "Syntax", "Name in a method definition"),
        MethodCall => ("method-call", "Syntax", "Name of a called method"),
        Class => ("class", "Syntax", "Reference to a class or type"),
        ClassDecl => ("class-decl", "Syntax", "Name in a class definition"),
        Parameter => ("parameter", "Syntax", "Function parameter"),
        SelfParameter => ("self", "Syntax", "self and cls"),
        KeywordArgument => ("keyword-argument", "Syntax", "Name of a keyword argument in a call"),
        Variable => ("variable", "Syntax", "Variables and plain identifiers"),
        VariableReassigned => ("variable-reassigned", "Syntax", "Variable that is assigned more than once"),
        Constant => ("constant", "Syntax", "Module level constants (UPPER_CASE)"),
        Attribute => ("attribute", "Syntax", "Instance and class attributes"),
        Builtin => ("builtin", "Syntax", "Built-in functions and types: len, print, int"),
        Decorator => ("decorator", "Syntax", "Decorators"),
        Annotation => ("annotation", "Syntax", "Type annotations"),
        Module => ("module", "Syntax", "Module and package names"),
        Dunder => ("dunder", "Syntax", "Special names such as __init__ and __name__"),
        FstringExpression => ("fstring-expression", "Syntax", "Expression inside an f-string"),
        FstringBrace => ("fstring-brace", "Syntax", "Braces and format spec colon of an f-string"),
        Operator => ("operator", "Syntax", "Operators"),
        Punctuation => ("punctuation", "Syntax", "Commas, dots, colons and semicolons"),
        Bracket => ("bracket", "Syntax", "Parentheses, brackets and braces"),
        Tag => ("tag", "Syntax", "Markup tags"),
        TagAttribute => ("tag-attribute", "Syntax", "Markup attributes"),
        PropertyKey => ("property-key", "Syntax", "Keys in JSON, YAML, TOML and similar"),
        Link => ("link", "Syntax", "Hyperlinks in the editor"),

        DiagnosticError => ("diagnostic-error", "Editor decoration", "Code with an error"),
        DiagnosticWarning => ("diagnostic-warning", "Editor decoration", "Code with a warning"),
        DiagnosticInfo => ("diagnostic-info", "Editor decoration", "Code with a weak warning"),
        DiagnosticHint => ("diagnostic-hint", "Editor decoration", "Code with a hint"),
        DiagnosticUnresolved => ("diagnostic-unresolved", "Editor decoration", "Reference that cannot be resolved"),
        DiagnosticUnused => ("diagnostic-unused", "Editor decoration", "Unused symbol"),
        DiagnosticDeprecated => ("diagnostic-deprecated", "Editor decoration", "Deprecated symbol"),
        Typo => ("typo", "Editor decoration", "Spelling mistake"),
        MatchingBracket => ("matching-bracket", "Editor decoration", "Bracket matching the one at the caret"),
        UnmatchedBracket => ("unmatched-bracket", "Editor decoration", "Bracket without a pair"),
        IdentifierRead => ("identifier-read", "Editor decoration", "Other reads of the symbol at the caret"),
        IdentifierWrite => ("identifier-write", "Editor decoration", "Other writes of the symbol at the caret"),
        FoldedText => ("folded-text", "Editor decoration", "Placeholder of a collapsed region"),
        InlayHint => ("inlay-hint", "Editor decoration", "Inline type and parameter hints"),
        ConsoleError => ("console-error", "Editor decoration", "Error output in the run console"),
        ConsoleInput => ("console-input", "Editor decoration", "User input in the run console"),
        ConsoleSystem => ("console-system", "Editor decoration", "The IDE's own lines in the run console (command line, exit code)"),
        DebuggerInlinedValues => ("debugger-inlined-values", "Editor decoration", "Values of variables shown after the lines while debugging"),
        DebuggerInlinedValuesModified => ("debugger-inlined-values-modified", "Editor decoration", "Values shown after the lines that changed since the last stop"),
        DebuggerInlinedValuesExecutionLine => ("debugger-inlined-values-execution-line", "Editor decoration", "Values shown after the line the program is suspended on"),
        DebuggerEvaluatedExpression => ("debugger-evaluated-expression", "Editor decoration", "The expression whose value the debugger shows under the mouse"),
        DebuggerEvaluatedExpressionExecutionLine => ("debugger-evaluated-expression-execution-line", "Editor decoration", "The expression whose value the debugger shows, on the line the program is suspended on"),
        ReferenceLink => ("reference-link", "Editor decoration", "A value of the debugger after a line under the mouse, drawn as a link"),
    }
}

roles! {
    /// A layout number in logical pixels.
    pub enum Metric {
        RowHeight => ("row-height", "Rows", "Height of a list, tree or table row"),
        RowHeightCompact => ("row-height-compact", "Rows", "Row height in compact mode"),
        TreeIndent => ("tree-indent", "Rows", "Indentation of one tree level"),
        TabHeight => ("tab-height", "Bars", "Height of an editor tab"),
        TabSelectionHeight => ("tab-selection-height", "Bars", "Height of the rounded background of the selected or hovered tab"),
        ToolbarHeight => ("toolbar-height", "Bars", "Height of the main toolbar"),
        ToolbarButtonSize => ("toolbar-button-size", "Bars", "Size of an icon button in the main toolbar"),
        StatusBarHeight => ("status-bar-height", "Bars", "Height of the status bar"),
        ToolWindowHeaderHeight => ("tool-window-header-height", "Bars", "Height of a tool window title strip"),
        StripeWidth => ("stripe-width", "Bars", "Width of a tool window stripe"),
        StripeButtonSize => ("stripe-button-size", "Bars", "Size of a tool window stripe button"),
        GutterWidth => ("gutter-width", "Editor", "Minimum width of the editor gutter"),
        GutterPadding => ("gutter-padding", "Editor", "Horizontal padding inside the gutter"),
        IndentGuideWidth => ("indent-guide-width", "Editor", "Thickness of indent guides"),
        CaretWidth => ("caret-width", "Editor", "Thickness of the text caret"),
        PanelPadding => ("panel-padding", "Spacing", "Padding inside panels and tool windows"),
        PaddingSmall => ("padding-small", "Spacing", "Small gap between related elements"),
        Padding => ("padding", "Spacing", "Standard gap between elements"),
        PaddingLarge => ("padding-large", "Spacing", "Gap between groups of elements"),
        IslandGap => ("island-gap", "Spacing", "Gap between the island panels of the main window and around them"),
        CornerRadius => ("corner-radius", "Shape", "Radius of buttons, inputs and popups"),
        CornerRadiusSmall => ("corner-radius-small", "Shape", "Radius of small elements such as tags"),
        ButtonRadius => ("button-radius", "Shape", "Radius of the hover and selection backgrounds of toolbar, stripe and tab buttons"),
        IslandRadius => ("island-radius", "Shape", "Radius of the island panels of the main window"),
        BorderWidth => ("border-width", "Shape", "Thickness of borders"),
        FocusWidth => ("focus-width", "Shape", "Thickness of the focus ring"),
        ScrollbarWidth => ("scrollbar-width", "Shape", "Width of scrollbars"),
        IconSize => ("icon-size", "Shape", "Size of toolbar and tree icons"),
    }
}

/// How a [`TextRole`] is treated when a theme does not define it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRoleKind {
    /// Part of the highlighted code; falls back to a related token.
    Syntax,
    /// Draws on top of the code; a missing value cannot be derived.
    Decoration,
}

impl TextRole {
    pub const fn kind(self) -> TextRoleKind {
        if self.index() < TextRole::DiagnosticError.index() {
            TextRoleKind::Syntax
        } else {
            TextRoleKind::Decoration
        }
    }

    /// The role whose style is used when a source defines this one only
    /// implicitly (for example a method call looks like a function call).
    pub const fn fallback(self) -> Option<TextRole> {
        use TextRole::*;
        match self {
            ConstantBuiltin | Builtin => Some(Keyword),
            StringEscape => Some(String),
            Docstring | Todo => Some(Comment),
            DocstringTag => Some(Docstring),
            FunctionDecl | MethodCall => Some(FunctionCall),
            MethodDecl => Some(FunctionDecl),
            ClassDecl => Some(Class),
            Parameter | VariableReassigned | Constant | Attribute | Module | FstringExpression
            | Operator | Punctuation | Bracket | TagAttribute | PropertyKey => Some(Variable),
            SelfParameter | KeywordArgument => Some(Parameter),
            Decorator => Some(Annotation),
            Dunder => Some(Builtin),
            FstringBrace => Some(Punctuation),
            DiagnosticUnresolved => Some(DiagnosticError),
            ReferenceLink => Some(Link),
            _ => None,
        }
    }
}

impl UiRole {
    /// The role whose colour stands in when a source does not define this one.
    pub const fn fallback(self) -> Option<UiRole> {
        use UiRole::*;
        match self {
            SurfaceAlt | SurfaceRaised | Input | TitleBar | Toolbar | StatusBar | ToolWindow
            | TabBar | Notification | EditorBackground => Some(Surface),
            ToolWindowHeader | ToolWindowStripe => Some(ToolWindow),
            TabInactive => Some(TabBar),
            TabActive => Some(EditorBackground),
            TabSelectedInactive => Some(TabActive),
            TabUnderlineInactive => Some(BorderInput),
            TabActiveText | TabInactiveText => Some(Text),
            StatusBarText | SelectionText => Some(Text),
            SelectionInactive | SelectionLight => Some(Selection),
            BorderInput | Divider | PopupBorder => Some(Border),
            SegmentedSelected => Some(Pressed),
            SegmentedSelectedBorder => Some(BorderInput),
            TextDisabled | Shortcut => Some(TextMuted),
            TextLink | Focus | Progress | TabUnderline | CompletionMatch => Some(Accent),
            ScrollbarHover => Some(Scrollbar),
            CompletionPopup | DocumentationPopup => Some(SurfaceRaised),
            CompletionSelection => Some(Selection),
            Gutter | TerminalBackground => Some(EditorBackground),
            LineNumberActive => Some(LineNumber),
            Caret | TerminalForeground => Some(EditorText),
            IndentGuideActive | Tearline => Some(IndentGuide),
            SoftWrapSign => Some(Whitespace),
            SearchMatchActive | SearchMatchStripe => Some(SearchMatch),
            SearchField => Some(EditorBackground),
            SearchFieldBorder => Some(Border),
            SearchFieldError => Some(Danger),
            SearchOptionSelected => Some(Pressed),
            NonProjectFile => Some(SurfaceAlt),
            ExecutionPointNotTop => Some(CurrentLine),
            DebuggerValueName => Some(Text),
            DebuggerType | DebuggerCollectingData | DebuggerEvaluating => Some(TextMuted),
            DebuggerChangedValue | DebuggerModifyingValue => Some(Accent),
            DebuggerException | DebuggerErrorMessage => Some(Danger),
            RunWidgetRunning => Some(Success),
            RunWidgetStop => Some(Danger),
            DragAndDropBorder => Some(Accent),
            DragAndDropRow => Some(Selection),
            TreeText => Some(Text),
            TreeIndentGuide => Some(Divider),
            ListButtonHover => Some(Hover),
            ListButtonSeparator => Some(Pressed),
            NotificationLink => Some(TextLink),
            NotificationIconHover => Some(Hover),
            NotificationMore => Some(Notification),
            NotificationMoreText => Some(TextMuted),
            GutterWhitespaceModified | GutterIgnoredModified | StripeModified => {
                Some(GutterModified)
            }
            GutterIgnoredAdded | StripeAdded => Some(GutterAdded),
            GutterIgnoredDeleted | StripeDeleted => Some(GutterDeleted),
            MarkerPopupToolbar => Some(Surface),
            MarkerPopupText => Some(CurrentLine),
            GitMerged => Some(GitModified),
            DiagnosticErrorStripe => Some(DiagnosticError),
            DiagnosticWarningStripe => Some(DiagnosticWarning),
            DiagnosticWeakWarningStripe => Some(DiagnosticInfo),
            TerminalDefaultBackground => Some(TerminalBackground),
            TerminalDefaultForeground => Some(TerminalForeground),
            TerminalAnsiBlack => Some(TerminalBlack),
            TerminalAnsiRed => Some(TerminalRed),
            TerminalAnsiGreen => Some(TerminalGreen),
            TerminalAnsiYellow => Some(TerminalYellow),
            TerminalAnsiBlue => Some(TerminalBlue),
            TerminalAnsiMagenta => Some(TerminalMagenta),
            TerminalAnsiCyan => Some(TerminalCyan),
            TerminalAnsiWhite => Some(TerminalWhite),
            TerminalAnsiBrightBlack => Some(TerminalBrightBlack),
            TerminalAnsiBrightRed => Some(TerminalBrightRed),
            TerminalAnsiBrightGreen => Some(TerminalBrightGreen),
            TerminalAnsiBrightYellow => Some(TerminalBrightYellow),
            TerminalAnsiBrightBlue => Some(TerminalBrightBlue),
            TerminalAnsiBrightMagenta => Some(TerminalBrightMagenta),
            TerminalAnsiBrightCyan => Some(TerminalBrightCyan),
            TerminalAnsiBrightWhite => Some(TerminalBrightWhite),
            TerminalAnsiBlackBackground => Some(TerminalAnsiBlack),
            TerminalAnsiRedBackground => Some(TerminalAnsiRed),
            TerminalAnsiGreenBackground => Some(TerminalAnsiGreen),
            TerminalAnsiYellowBackground => Some(TerminalAnsiYellow),
            TerminalAnsiBlueBackground => Some(TerminalAnsiBlue),
            TerminalAnsiMagentaBackground => Some(TerminalAnsiMagenta),
            TerminalAnsiCyanBackground => Some(TerminalAnsiCyan),
            TerminalAnsiWhiteBackground => Some(TerminalAnsiWhite),
            TerminalAnsiBrightBlackBackground => Some(TerminalAnsiBrightBlack),
            TerminalAnsiBrightRedBackground => Some(TerminalAnsiBrightRed),
            TerminalAnsiBrightGreenBackground => Some(TerminalAnsiBrightGreen),
            TerminalAnsiBrightYellowBackground => Some(TerminalAnsiBrightYellow),
            TerminalAnsiBrightBlueBackground => Some(TerminalAnsiBrightBlue),
            TerminalAnsiBrightMagentaBackground => Some(TerminalAnsiBrightMagenta),
            TerminalAnsiBrightCyanBackground => Some(TerminalAnsiBrightCyan),
            TerminalAnsiBrightWhiteBackground => Some(TerminalAnsiBrightWhite),
            TerminalSearchMatch => Some(SearchMatch),
            TerminalSearchMatchText | TerminalSearchMatchCurrentText => {
                Some(TerminalDefaultForeground)
            }
            TerminalSearchMatchCurrent => Some(SearchMatchActive),
            TerminalSeparator => Some(Divider),
            TerminalErrorMarker => Some(Danger),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_kebab_case_and_round_trip() {
        fn check<T: Copy + PartialEq + std::fmt::Debug>(
            all: &[T],
            name: fn(T) -> &'static str,
            from: fn(&str) -> Option<T>,
        ) {
            let mut seen = HashSet::new();
            for &role in all {
                let n = name(role);
                assert!(seen.insert(n), "duplicate name {n}");
                assert!(
                    n.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                    "{n}"
                );
                assert_eq!(from(n), Some(role));
            }
        }
        check(UiRole::ALL, UiRole::name, UiRole::from_name);
        check(TextRole::ALL, TextRole::name, TextRole::from_name);
        check(Metric::ALL, Metric::name, Metric::from_name);
    }

    #[test]
    fn index_matches_position() {
        for (i, role) in UiRole::ALL.iter().enumerate() {
            assert_eq!(role.index(), i);
        }
        for (i, role) in TextRole::ALL.iter().enumerate() {
            assert_eq!(role.index(), i);
        }
    }

    #[test]
    fn fallback_chains_terminate() {
        for &role in TextRole::ALL {
            let mut current = role;
            let mut steps = 0;
            while let Some(next) = current.fallback() {
                current = next;
                steps += 1;
                assert!(steps < 10, "cycle at {role}");
            }
        }
        for &role in UiRole::ALL {
            let mut current = role;
            let mut steps = 0;
            while let Some(next) = current.fallback() {
                current = next;
                steps += 1;
                assert!(steps < 10, "cycle at {role}");
            }
        }
    }

    #[test]
    fn kinds_split_at_the_first_decoration() {
        assert_eq!(TextRole::Link.kind(), TextRoleKind::Syntax);
        assert_eq!(TextRole::DiagnosticError.kind(), TextRoleKind::Decoration);
        assert_eq!(TextRole::ConsoleInput.kind(), TextRoleKind::Decoration);
    }

    #[test]
    fn serde_rejects_unknown_names() {
        let err = serde_json::from_str::<UiRole>("\"no-such-role\"").unwrap_err();
        assert!(err.to_string().contains("no-such-role"));
        assert_eq!(
            serde_json::from_str::<UiRole>("\"surface\"").unwrap(),
            UiRole::Surface
        );
    }
}
