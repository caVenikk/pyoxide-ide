//! The context menus of the object tree's rows, per kind of node, in the
//! reference's order with its separators, shortcuts and submenus (refs
//! DBCTX01-70, the maintainer's local screenshots). A row whose function
//! is not there yet is left out, or shown disabled where the reference
//! shows it disabled (Compare Schema Structure of a data source, the SQL
//! Generator, Move To Top Level outside a folder, the Introspection Level
//! of an object, what a group cannot do); the PR lists the rest as
//! follow-ups. Separators of left-out sections collapse.
//!
//! Every row runs a command of the tool window on the selected row (the
//! right click selects it), so the keyboard and the control channel reach
//! the same functions.

use db_core::schema::Level;
use gpui::Keystroke;
use ui::icons::IconName;
use ui::menu::MenuItem;

use super::actions::{
    CopyReference, Deactivate, Delete, Duplicate, ForceRefresh, ForgetCachedSchemas, GenerateDdl,
    HideSchema, ManageShownSchemas, MoveDown, MoveUp, QuickDocumentation, Rename, SetColor,
    SetIntrospectionLevel, Truncate,
};
use super::{AddDataSource, ImportDataSources, Properties, Refresh};
use crate::dialog::COLORS;
use crate::{BACKENDS, backend_title, icons};

/// The rows' texts: the reference's action names.
pub const PROPERTIES: &str = "Properties";
pub const NEW: &str = "New";
pub const COPY_PASTE: &str = "Copy/Paste";
pub const REFRESH: &str = "Refresh";
pub const DEACTIVATE: &str = "Deactivate";
pub const RENAME: &str = "Rename\u{2026}";
pub const MOVE: &str = "Move";
pub const SET_COLOR: &str = "Set Color";
pub const QUICK_DOC: &str = "Quick Documentation";
pub const COMPARE: &str = "Compare Schema Structure";
pub const INTROSPECTION_LEVEL: &str = "Introspection Level";
pub const SQL_SCRIPTS: &str = "SQL Scripts";
pub const OBJECT_ACTIONS: &str = "Object Actions";
pub const TOOLS: &str = "Tools";
pub const DIAGNOSTICS: &str = "Diagnostics";
pub const REMOVE_DATA_SOURCE: &str = "Remove Data Source\u{2026}";
pub const DROP: &str = "Drop\u{2026}";
const DATA_SOURCE: &str = "Data Source";
const FROM_FILE: &str = "Data Source from File/Folder";
const FROM_URL: &str = "Data Source from URL";
const IMPORT: &str = "Import Data Sources\u{2026}";
const COPY_REFERENCE: &str = "Copy Reference";
const DUPLICATE: &str = "Duplicate";
const UP: &str = "Up";
const DOWN: &str = "Down";
const TO_TOP_LEVEL: &str = "To Top Level";
const SQL_GENERATOR: &str = "SQL Generator\u{2026}";
const GENERATE_DDL: &str = "Generate DDL to Clipboard";
const MANAGE_SCHEMAS: &str = "Manage Shown Schemas\u{2026}";
const HIDE_SCHEMAS: &str = "Hide Schemas";
const TRUNCATE: &str = "Truncate\u{2026}";
const FORCE_REFRESH: &str = "Force Refresh";
const FORGET_SCHEMAS: &str = "Forget All Cached Schemas";
const AUTO_SELECT: &str = "Auto select";
const DEFAULT_AUTO_SELECT: &str = "Default (Auto select)";
const LEVELS: [(Level, &str, &str); 3] = [
    (Level::Names, "names", "Level 1: Names only"),
    (Level::Structure, "structure", "Level 2: Skip sources"),
    (Level::Full, "full", "Level 3: Everything"),
];

/// What a menu is for: the kind of the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Source,
    /// The database row of a PostgreSQL data source.
    Database,
    Schema,
    /// A group (`tables`, `columns`, ...).
    Group,
    /// A schema object or a part of one; `table` for a table (it can be
    /// truncated).
    Object {
        table: bool,
    },
}

/// What the menu of a row needs to know.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuInput {
    pub target: Target,
    /// The data source has a connection (Deactivate).
    pub connected: bool,
    /// The data source's place among the project's (Move Up and Down).
    pub index: usize,
    pub count: usize,
    /// The data source's colour.
    pub color: Option<String>,
    /// The data source's introspection level (`None`: chosen
    /// automatically).
    pub level: Option<Level>,
    /// The schema's own level (`None`: the data source's).
    pub schema_level: Option<Level>,
    /// The node can be renamed or dropped with a statement of its dialect.
    pub rename: bool,
    pub drop: bool,
}

/// The text of a keystroke (`cmd-d`, `ctrl-d`) for a row without a
/// command, whose shortcut the keymap cannot give.
fn keys(mac: &str, other: &str) -> String {
    let (text, macos) = if cfg!(target_os = "macos") {
        (mac, true)
    } else {
        (other, false)
    };
    Keystroke::parse(text)
        .map(|k| ui::keys::format_keystroke(&k, macos))
        .unwrap_or_default()
}

/// The New submenu: data sources (the database objects are not there yet).
fn new_items() -> Vec<MenuItem> {
    let drivers = BACKENDS
        .iter()
        .map(|b| {
            MenuItem::action(
                backend_title(*b),
                AddDataSource {
                    backend: b.id().to_owned(),
                },
            )
            .icon(icons::backend(*b))
        })
        .collect();
    vec![
        MenuItem::submenu(DATA_SOURCE, drivers),
        MenuItem::action(
            FROM_FILE,
            AddDataSource {
                backend: "sqlite".into(),
            },
        )
        .icon(IconName::FOLDER),
        MenuItem::action(
            FROM_URL,
            AddDataSource {
                backend: "url".into(),
            },
        )
        .icon(icons::URL),
        MenuItem::separator(),
        MenuItem::action(IMPORT, ImportDataSources).icon(icons::IMPORT),
    ]
}

fn copy_paste(input: &MenuInput) -> MenuItem {
    let mut items = vec![MenuItem::action(COPY_REFERENCE, CopyReference).mnemonic(3)];
    if input.target == Target::Source {
        items.push(MenuItem::action(DUPLICATE, Duplicate).icon(IconName::COPY));
    }
    MenuItem::submenu(COPY_PASTE, items)
}

fn move_items(input: &MenuInput) -> MenuItem {
    MenuItem::submenu(
        MOVE,
        vec![
            MenuItem::action(UP, MoveUp).enabled(input.index > 0),
            MenuItem::action(DOWN, MoveDown).enabled(input.index + 1 < input.count),
            MenuItem::separator(),
            MenuItem::unavailable(TO_TOP_LEVEL),
        ],
    )
}

fn color_items(input: &MenuInput) -> MenuItem {
    let current = input
        .color
        .as_deref()
        .map(crate::dialog::color_id)
        .unwrap_or("none");
    let items = COLORS
        .iter()
        .map(|(id, label)| {
            let label = match (*id, *id == current) {
                ("none", true) => "No Color (Current)".to_owned(),
                ("none", false) => "No Color".to_owned(),
                (_, true) => format!("{label} (Current)"),
                (_, false) => (*label).to_owned(),
            };
            MenuItem::action(
                label,
                SetColor {
                    color: (*id).to_owned(),
                },
            )
            .icon(icons::color(id))
        })
        .collect();
    MenuItem::submenu(SET_COLOR, items)
}

/// The levels with a check at the current one; `enabled` for a data
/// source or a schema, disabled for what follows its schema.
fn level_items(input: &MenuInput) -> MenuItem {
    let (auto, current, enabled) = match input.target {
        Target::Source | Target::Database => (AUTO_SELECT, input.level, true),
        Target::Schema => (DEFAULT_AUTO_SELECT, input.schema_level, true),
        Target::Group | Target::Object { .. } => (DEFAULT_AUTO_SELECT, None, false),
    };
    let row = |label: &str, id: &str, checked: bool| {
        let item = MenuItem::action(
            label.to_owned(),
            SetIntrospectionLevel {
                level: id.to_owned(),
            },
        )
        .enabled(enabled);
        if checked {
            item.icon(IconName::CHECKED)
        } else {
            item
        }
    };
    let mut items = vec![row(auto, "auto", current.is_none()), MenuItem::separator()];
    for (level, id, label) in LEVELS {
        items.push(row(label, id, current == Some(level)));
    }
    MenuItem::submenu(INTROSPECTION_LEVEL, items)
}

fn sql_scripts(input: &MenuInput) -> MenuItem {
    let mut items = Vec::new();
    if input.target == Target::Source {
        // Shown disabled for a data source, as the reference does.
        items.push(
            MenuItem::unavailable(SQL_GENERATOR)
                .icon(icons::SQL_GENERATOR)
                .shortcut(keys("alt-cmd-g", "ctrl-alt-g")),
        );
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::action(GENERATE_DDL, GenerateDdl).icon(IconName::EXTERNAL_LINK));
    MenuItem::submenu(SQL_SCRIPTS, items)
}

fn diagnostics() -> MenuItem {
    MenuItem::submenu(
        DIAGNOSTICS,
        vec![
            MenuItem::action(FORCE_REFRESH, ForceRefresh).icon(icons::FORCE_REFRESH),
            MenuItem::action(FORGET_SCHEMAS, ForgetCachedSchemas),
        ],
    )
}

/// The rows of the menu of a row.
pub fn items(input: &MenuInput) -> Vec<MenuItem> {
    let sep = MenuItem::separator;
    let refresh = || MenuItem::action(REFRESH, Refresh).icon(IconName::REFRESH);
    let new = || MenuItem::submenu(NEW, new_items()).icon(IconName::ADD);
    let rename = || MenuItem::action(RENAME, Rename).mnemonic(0);
    let quick_doc = || MenuItem::action(QUICK_DOC, QuickDocumentation).mnemonic(6);
    let compare_disabled = || {
        MenuItem::unavailable(COMPARE)
            .icon(icons::COMPARE)
            .shortcut(keys("cmd-d", "ctrl-d"))
    };
    let mut out = Vec::new();
    match input.target {
        Target::Source => {
            out.push(MenuItem::action(PROPERTIES, Properties).icon(icons::PROPERTIES));
            out.push(sep());
            out.push(new());
            out.push(copy_paste(input));
            out.push(refresh());
            out.push(
                MenuItem::action(DEACTIVATE, Deactivate)
                    .icon(icons::DISCONNECT)
                    .enabled(input.connected),
            );
            out.push(sep());
            out.push(rename());
            out.push(move_items(input));
            out.push(color_items(input));
            out.push(sep());
            out.push(quick_doc());
            out.push(compare_disabled());
            out.push(sep());
            out.push(level_items(input));
            out.push(sep());
            out.push(sql_scripts(input));
            out.push(MenuItem::submenu(
                TOOLS,
                vec![MenuItem::action(MANAGE_SCHEMAS, ManageShownSchemas)],
            ));
            out.push(sep());
            out.push(diagnostics());
            out.push(sep());
            out.push(MenuItem::action(REMOVE_DATA_SOURCE, Delete));
        }
        Target::Database => {
            out.push(new());
            out.push(copy_paste(input));
            out.push(refresh());
            out.push(sep());
            out.push(quick_doc());
            out.push(sep());
            out.push(level_items(input));
            out.push(sep());
            out.push(sql_scripts(input));
            out.push(MenuItem::submenu(
                TOOLS,
                vec![MenuItem::action(MANAGE_SCHEMAS, ManageShownSchemas)],
            ));
            out.push(sep());
            out.push(diagnostics());
        }
        Target::Schema => {
            out.push(new());
            out.push(copy_paste(input));
            out.push(refresh());
            out.push(sep());
            if input.rename {
                out.push(rename());
            }
            out.push(sep());
            out.push(quick_doc());
            out.push(sep());
            out.push(level_items(input));
            out.push(sep());
            out.push(sql_scripts(input));
            out.push(MenuItem::submenu(
                TOOLS,
                vec![
                    MenuItem::action(MANAGE_SCHEMAS, ManageShownSchemas),
                    MenuItem::separator(),
                    MenuItem::action(HIDE_SCHEMAS, HideSchema),
                ],
            ));
            out.push(sep());
            if input.drop {
                out.push(MenuItem::action(DROP, Delete));
            }
        }
        Target::Group => {
            out.push(new());
            out.push(MenuItem::submenu(COPY_PASTE, Vec::new()).enabled(false));
            out.push(refresh());
            out.push(sep());
            out.push(
                MenuItem::unavailable(RENAME)
                    .mnemonic(0)
                    .shortcut(keys("shift-f6", "shift-f6")),
            );
            out.push(sep());
            out.push(
                MenuItem::unavailable(QUICK_DOC)
                    .mnemonic(6)
                    .shortcut(keys("f1", "ctrl-q")),
            );
            out.push(compare_disabled());
            out.push(sep());
            out.push(level_items(input));
            out.push(sep());
            out.push(sql_scripts(input));
            out.push(sep());
            if input.drop {
                out.push(MenuItem::action(DROP, Delete));
            }
        }
        Target::Object { table } => {
            out.push(new());
            out.push(copy_paste(input));
            out.push(refresh());
            out.push(sep());
            if input.rename {
                out.push(rename());
            }
            out.push(sep());
            out.push(quick_doc());
            out.push(sep());
            out.push(level_items(input));
            out.push(sep());
            out.push(sql_scripts(input));
            if table {
                out.push(MenuItem::submenu(
                    OBJECT_ACTIONS,
                    vec![
                        MenuItem::action(TRUNCATE, Truncate)
                            .icon(IconName::DELETE)
                            .mnemonic(0),
                    ],
                ));
            }
            out.push(sep());
            if input.drop {
                out.push(MenuItem::action(DROP, Delete));
            }
        }
    }
    tidy(out)
}

/// Drops separators at the ends and repeated ones (left by sections
/// whose rows are all left out).
fn tidy(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        let separator = matches!(item, MenuItem::Separator);
        if separator && out.last().is_none_or(|l| matches!(l, MenuItem::Separator)) {
            continue;
        }
        out.push(item);
    }
    while out.last().is_some_and(|l| matches!(l, MenuItem::Separator)) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                MenuItem::Separator => "|".to_owned(),
                other => other.label().map(|l| l.to_string()).unwrap_or_default(),
            })
            .collect()
    }

    fn input(target: Target) -> MenuInput {
        MenuInput {
            target,
            connected: true,
            index: 0,
            count: 1,
            color: None,
            level: None,
            schema_level: None,
            rename: true,
            drop: true,
        }
    }

    #[test]
    fn data_source_menu_in_the_reference_order() {
        let items = items(&input(Target::Source));
        assert_eq!(
            labels(&items),
            [
                "Properties",
                "|",
                "New",
                "Copy/Paste",
                "Refresh",
                "Deactivate",
                "|",
                "Rename\u{2026}",
                "Move",
                "Set Color",
                "|",
                "Quick Documentation",
                "Compare Schema Structure",
                "|",
                "Introspection Level",
                "|",
                "SQL Scripts",
                "Tools",
                "|",
                "Diagnostics",
                "|",
                "Remove Data Source\u{2026}",
            ]
        );
    }

    #[test]
    fn table_menu_in_the_reference_order() {
        let items = items(&input(Target::Object { table: true }));
        assert_eq!(
            labels(&items),
            [
                "New",
                "Copy/Paste",
                "Refresh",
                "|",
                "Rename\u{2026}",
                "|",
                "Quick Documentation",
                "|",
                "Introspection Level",
                "|",
                "SQL Scripts",
                "Object Actions",
                "|",
                "Drop\u{2026}",
            ]
        );
        // Without a rename statement the section goes, and its separator.
        let mut no_rename = input(Target::Object { table: false });
        no_rename.rename = false;
        let got = labels(&items_of(&no_rename));
        assert_eq!(
            &got[..5],
            ["New", "Copy/Paste", "Refresh", "|", "Quick Documentation"]
        );
    }

    fn items_of(input: &MenuInput) -> Vec<MenuItem> {
        items(input)
    }
}
