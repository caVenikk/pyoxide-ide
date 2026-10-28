use std::fmt;

/// How the case of a class of words is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Case {
    /// "Do not change".
    #[default]
    Keep,
    Upper,
    Lower,
}

/// The case of built-in function names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FunctionCase {
    /// As keywords.
    #[default]
    AsKeywords,
    Keep,
    Upper,
    Lower,
}

/// Where the commas of a list stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CommaPlacement {
    /// Leading when the list in the text already has leading commas, else
    /// trailing.
    #[default]
    Auto,
    Trailing,
    Leading,
}

/// How the items of a select list are put on lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SelectItems {
    /// Items stay on one line while the list fits; a list that is too long
    /// (or whose text had line breaks between the items, see
    /// `keep_line_breaks`) puts one item per line.
    #[default]
    WrapIfLong,
    OnePerLine,
}

/// Where the elements of a clause start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClauseElements {
    /// After the clause keyword, the next ones aligned with the first.
    #[default]
    SameLine,
    /// On the next line, indented.
    NewLine,
}

/// Where `AND` and `OR` stand when a condition is broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AndOr {
    #[default]
    LineStart,
    LineEnd,
}

/// Which statements are put on one line when they fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Collapse {
    Never,
    /// Short subqueries (and CTE bodies).
    #[default]
    SubqueriesOnly,
    /// Whole queries too.
    Always,
}

/// The code style of SQL, with the defaults of the reference (the Default
/// scheme of the SQL code style pages). The option names of [`CodeStyle::set`]
/// are the keys of `[sql.code-style]` in the project file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeStyle {
    /// Leave the text alone.
    pub disable_formatting: bool,
    pub keyword_case: Case,
    pub builtin_type_case: Case,
    pub builtin_function_case: FunctionCase,
    /// Unquoted identifiers, aliases and custom types; quoted names never change.
    pub identifier_case: Case,
    pub use_tab: bool,
    pub tab_size: usize,
    pub indent: usize,
    pub continuation_indent: usize,
    /// Columns after which a line is wrapped; 0 never wraps.
    pub right_margin: usize,
    pub comma: CommaPlacement,
    pub select_items: SelectItems,
    pub clause_elements: ClauseElements,
    pub and_or: AndOr,
    /// Every top-level `AND` / `OR` of a `WHERE` condition starts a line, the
    /// conditions aligned under the first one.
    pub break_conditions: bool,
    pub collapse_short_statement: Collapse,
    /// Keep the line breaks of the text inside clauses (not the layout of the
    /// clauses themselves).
    pub keep_line_breaks: bool,
    /// Blank lines kept between statements and comments, at most.
    pub keep_blank_lines: usize,
    /// Exactly this many blank lines between statements; `None` keeps the
    /// text's (up to `keep_blank_lines`).
    pub blank_lines_between_statements: Option<usize>,
    pub spaces_around_operators: bool,
    /// `t (a, b)` instead of `t(a, b)` for column lists and tuples.
    pub space_before_parenthesis: bool,
    pub parentheses_padding: bool,
    pub space_before_comma: bool,
    pub space_after_comma: bool,
    /// Align the names of `CREATE TABLE` columns (the types and constraints
    /// follow after one space).
    pub align_ddl_columns: bool,
}

impl Default for CodeStyle {
    fn default() -> Self {
        CodeStyle {
            disable_formatting: false,
            keyword_case: Case::Keep,
            builtin_type_case: Case::Keep,
            builtin_function_case: FunctionCase::AsKeywords,
            identifier_case: Case::Keep,
            use_tab: false,
            tab_size: 4,
            indent: 4,
            continuation_indent: 8,
            right_margin: 120,
            comma: CommaPlacement::Auto,
            select_items: SelectItems::WrapIfLong,
            clause_elements: ClauseElements::SameLine,
            and_or: AndOr::LineStart,
            break_conditions: true,
            collapse_short_statement: Collapse::SubqueriesOnly,
            keep_line_breaks: true,
            keep_blank_lines: 2,
            blank_lines_between_statements: None,
            spaces_around_operators: true,
            space_before_parenthesis: true,
            parentheses_padding: false,
            space_before_comma: false,
            space_after_comma: true,
            align_ddl_columns: true,
        }
    }
}

/// A key or a value of [`CodeStyle::set`] that is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleError {
    pub key: String,
    pub message: String,
}

impl fmt::Display for StyleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.message)
    }
}

impl std::error::Error for StyleError {}

fn case(v: &str) -> Option<Case> {
    match v {
        "keep" => Some(Case::Keep),
        "upper" => Some(Case::Upper),
        "lower" => Some(Case::Lower),
        _ => None,
    }
}

fn boolean(v: &str) -> Option<bool> {
    match v {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

impl CodeStyle {
    /// Sets one option from its key and its value as written in the project
    /// file (`keyword-case = "upper"` is `("keyword-case", "upper")`). A
    /// value that does not fit leaves the option as it was.
    ///
    /// Keys: `disable-formatting`, `keyword-case`, `builtin-type-case`
    /// (`keep|upper|lower`), `builtin-function-case` (`as-keywords|keep|upper|lower`),
    /// `identifier-case`, `use-tab`, `tab-size`, `indent`, `continuation-indent`,
    /// `right-margin`, `comma` (`auto|trailing|leading`), `select-items`
    /// (`wrap-if-long|one-per-line`), `clause-elements` (`same-line|new-line`),
    /// `and-or` (`line-start|line-end`), `break-conditions`, `collapse-short-statement`
    /// (`never|subqueries-only|always`), `keep-line-breaks`, `keep-blank-lines`,
    /// `blank-lines-between-statements` (a number or `keep`),
    /// `spaces-around-operators`, `space-before-parenthesis`, `parentheses-padding`,
    /// `space-before-comma`, `space-after-comma`, `align-ddl-columns`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), StyleError> {
        let bad = |what: &str| StyleError {
            key: key.to_string(),
            message: format!("{what}: {value:?}"),
        };
        let number = |max: usize| -> Result<usize, StyleError> {
            value
                .parse::<usize>()
                .ok()
                .filter(|n| *n <= max)
                .ok_or_else(|| bad("expected a number"))
        };
        let flag = || boolean(value).ok_or_else(|| bad("expected true or false"));
        let word = |opts: &str| bad(&format!("expected one of {opts}"));
        match key {
            "disable-formatting" => self.disable_formatting = flag()?,
            "keyword-case" => {
                self.keyword_case = case(value).ok_or_else(|| word("keep, upper, lower"))?
            }
            "builtin-type-case" => {
                self.builtin_type_case = case(value).ok_or_else(|| word("keep, upper, lower"))?
            }
            "identifier-case" => {
                self.identifier_case = case(value).ok_or_else(|| word("keep, upper, lower"))?
            }
            "builtin-function-case" => {
                self.builtin_function_case = match value {
                    "as-keywords" => FunctionCase::AsKeywords,
                    "keep" => FunctionCase::Keep,
                    "upper" => FunctionCase::Upper,
                    "lower" => FunctionCase::Lower,
                    _ => return Err(word("as-keywords, keep, upper, lower")),
                }
            }
            "use-tab" => self.use_tab = flag()?,
            "tab-size" => self.tab_size = number(32)?.max(1),
            "indent" => self.indent = number(32)?,
            "continuation-indent" => self.continuation_indent = number(64)?,
            "right-margin" => self.right_margin = number(10_000)?,
            "comma" => {
                self.comma = match value {
                    "auto" => CommaPlacement::Auto,
                    "trailing" => CommaPlacement::Trailing,
                    "leading" => CommaPlacement::Leading,
                    _ => return Err(word("auto, trailing, leading")),
                }
            }
            "select-items" => {
                self.select_items = match value {
                    "wrap-if-long" => SelectItems::WrapIfLong,
                    "one-per-line" => SelectItems::OnePerLine,
                    _ => return Err(word("wrap-if-long, one-per-line")),
                }
            }
            "clause-elements" => {
                self.clause_elements = match value {
                    "same-line" => ClauseElements::SameLine,
                    "new-line" => ClauseElements::NewLine,
                    _ => return Err(word("same-line, new-line")),
                }
            }
            "and-or" => {
                self.and_or = match value {
                    "line-start" => AndOr::LineStart,
                    "line-end" => AndOr::LineEnd,
                    _ => return Err(word("line-start, line-end")),
                }
            }
            "break-conditions" => self.break_conditions = flag()?,
            "collapse-short-statement" => {
                self.collapse_short_statement = match value {
                    "never" => Collapse::Never,
                    "subqueries-only" => Collapse::SubqueriesOnly,
                    "always" => Collapse::Always,
                    _ => return Err(word("never, subqueries-only, always")),
                }
            }
            "keep-line-breaks" => self.keep_line_breaks = flag()?,
            "keep-blank-lines" => self.keep_blank_lines = number(100)?,
            "blank-lines-between-statements" => {
                self.blank_lines_between_statements = if value == "keep" {
                    None
                } else {
                    Some(number(100)?)
                }
            }
            "spaces-around-operators" => self.spaces_around_operators = flag()?,
            "space-before-parenthesis" => self.space_before_parenthesis = flag()?,
            "parentheses-padding" => self.parentheses_padding = flag()?,
            "space-before-comma" => self.space_before_comma = flag()?,
            "space-after-comma" => self.space_after_comma = flag()?,
            "align-ddl-columns" => self.align_ddl_columns = flag()?,
            _ => {
                return Err(StyleError {
                    key: key.to_string(),
                    message: "unknown option".to_string(),
                });
            }
        }
        Ok(())
    }

    /// Sets every pair, collecting the errors of the ones that do not fit.
    pub fn set_all<'a>(
        &mut self,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Vec<StyleError> {
        pairs
            .into_iter()
            .filter_map(|(k, v)| self.set(k, v).err())
            .collect()
    }
}
