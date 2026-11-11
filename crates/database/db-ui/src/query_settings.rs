//! Tools > Database > Query Execution: the options of running statements in
//! a console that are kept for the application (`[database.query-execution]`
//! of the settings file).

use serde::{Deserialize, Serialize};
use settings::Section;

/// One option of a combo box: its stored id and the text of the list.
pub(crate) trait Choice: Copy + PartialEq + 'static {
    const ALL: &'static [Self];
    fn id(self) -> &'static str;
    fn label(self) -> &'static str;

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.id() == id)
    }
}

macro_rules! choices {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $id:literal, $label:literal;)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        pub enum $name {
            $($variant,)+
        }

        impl Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$variant,)+];

            fn id(self) -> &'static str {
                match self {
                    $(Self::$variant => $id,)+
                }
            }

            fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }
        }
    };
}

choices! {
    /// What Execute runs when the caret is inside a statement.
    CaretInside {
        Ask => "ask", "Ask what to execute";
        Smallest => "smallest", "Smallest statement";
        Largest => "largest", "Largest statement";
    }
}

choices! {
    /// What Execute runs when the caret is between statements.
    CaretOutside {
        Nothing => "nothing", "Nothing";
        Closest => "closest", "Closest statement";
        Ask => "ask", "Ask what to execute";
    }
}

choices! {
    /// How Execute runs a selection.
    ForSelection {
        Separate => "separate", "Exactly as separate statements";
        One => "one", "Exactly as one statement";
    }
}

choices! {
    /// How a script is split into statements in the Generic and ANSI SQL
    /// dialects.
    SplitScript {
        AnsiOrSeparator => "ansi-or-separator", "Into valid ANSI SQL statements or by separator";
    }
}

/// What one of the Execute actions (Execute, Execute (2), Execute (3))
/// runs and where its result goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ExecuteOptions {
    pub caret_inside: CaretInside,
    pub caret_outside: CaretOutside,
    pub for_selection: ForSelection,
    /// Every run of the action opens its result in a tab of its own
    /// (off: the first tab that is not kept is reused).
    pub new_tab: bool,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            caret_inside: CaretInside::Ask,
            caret_outside: CaretOutside::Nothing,
            for_selection: ForSelection::Separate,
            new_tab: false,
        }
    }
}

/// The Execute actions of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Execute,
    Execute2,
    Execute3,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Execute, Variant::Execute2, Variant::Execute3];

    pub fn title(self) -> &'static str {
        match self {
            Variant::Execute => "Execute",
            Variant::Execute2 => "Execute (2)",
            Variant::Execute3 => "Execute (3)",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Variant::Execute => 0,
            Variant::Execute2 => 1,
            Variant::Execute3 => 2,
        }
    }
}

/// What running a statement checks and shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct QueryExecutionSettings {
    /// Execute, Execute (2) and Execute (3).
    pub execute: ExecuteOptions,
    pub execute_2: ExecuteOptions,
    pub execute_3: ExecuteOptions,
    /// Not used yet: the console always splits by the lexer.
    pub split_script: SplitScript,
    /// Ask for the values of the parameters of a statement before it runs
    /// (off: only when a value is missing).
    pub review_parameters: bool,
    /// Show how long a statement took after it, in the editor.
    pub show_execution_time: bool,
    /// Warn before a `DELETE` or `UPDATE` without `WHERE` runs.
    pub warn_unsafe_queries: bool,
}

impl Default for QueryExecutionSettings {
    fn default() -> Self {
        Self {
            execute: ExecuteOptions::default(),
            execute_2: ExecuteOptions::default(),
            execute_3: ExecuteOptions::default(),
            split_script: SplitScript::AnsiOrSeparator,
            review_parameters: true,
            show_execution_time: true,
            warn_unsafe_queries: true,
        }
    }
}

impl QueryExecutionSettings {
    pub fn options(&self, variant: Variant) -> &ExecuteOptions {
        match variant {
            Variant::Execute => &self.execute,
            Variant::Execute2 => &self.execute_2,
            Variant::Execute3 => &self.execute_3,
        }
    }

    pub fn options_mut(&mut self, variant: Variant) -> &mut ExecuteOptions {
        match variant {
            Variant::Execute => &mut self.execute,
            Variant::Execute2 => &mut self.execute_2,
            Variant::Execute3 => &mut self.execute_3,
        }
    }
}

impl Section for QueryExecutionSettings {
    const KEY: &'static str = "database.query-execution";
}

/// The settings as stored.
pub(crate) fn get(cx: &gpui::App) -> QueryExecutionSettings {
    settings_ui::store::get(cx)
}

pub(crate) fn set(cx: &mut gpui::App, value: &QueryExecutionSettings) {
    settings_ui::store::set(cx, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for c in CaretInside::ALL {
            assert_eq!(CaretInside::from_id(c.id()), Some(*c));
        }
        for c in CaretOutside::ALL {
            assert_eq!(CaretOutside::from_id(c.id()), Some(*c));
        }
        for c in ForSelection::ALL {
            assert_eq!(ForSelection::from_id(c.id()), Some(*c));
        }
        assert_eq!(CaretInside::from_id("nope"), None);
    }

    #[test]
    fn ids_are_the_serialized_names() {
        let value = serde_json::to_value(ExecuteOptions {
            caret_inside: CaretInside::Largest,
            caret_outside: CaretOutside::Closest,
            for_selection: ForSelection::One,
            new_tab: true,
        })
        .unwrap();
        assert_eq!(value["caret-inside"], CaretInside::Largest.id());
        assert_eq!(value["caret-outside"], CaretOutside::Closest.id());
        assert_eq!(value["for-selection"], ForSelection::One.id());
        assert_eq!(value["new-tab"], true);
    }

    #[test]
    fn defaults_are_the_references() {
        let s = QueryExecutionSettings::default();
        assert_eq!(s.execute.caret_inside.label(), "Ask what to execute");
        assert_eq!(s.execute.caret_outside.label(), "Nothing");
        assert_eq!(
            s.execute.for_selection.label(),
            "Exactly as separate statements"
        );
        assert!(!s.execute.new_tab);
        assert_eq!(
            s.split_script.label(),
            "Into valid ANSI SQL statements or by separator"
        );
    }

    #[test]
    fn an_old_file_without_the_new_keys_reads_with_defaults() {
        let s: QueryExecutionSettings =
            serde_json::from_str(r#"{"review-parameters": false}"#).unwrap();
        assert!(!s.review_parameters);
        assert_eq!(s.execute, ExecuteOptions::default());
    }

    #[test]
    fn the_variants_are_stored_apart() {
        let mut s = QueryExecutionSettings::default();
        s.options_mut(Variant::Execute2).new_tab = true;
        let text = serde_json::to_string(&s).unwrap();
        let back: QueryExecutionSettings = serde_json::from_str(&text).unwrap();
        assert!(back.options(Variant::Execute2).new_tab);
        assert!(!back.options(Variant::Execute).new_tab);
        assert!(!back.options(Variant::Execute3).new_tab);
    }
}
