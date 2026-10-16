//! The run widget's configuration dropdown, in the reference's structure
//! (`RunToolbarPopup.createRunConfigurationsActionGroup`):
//!
//! 1. "Pinned" and the pinned configurations, when there are any.
//! 2. "Recent Configurations" (the start history without the pinned ones,
//!    at most [`RECENT_COUNT`]) and the "All Configurations  N" toggle,
//!    when something is pinned or was started and the project has at
//!    least [`RECENT_COUNT`] configurations.
//! 3. Every configuration by type in list order, with a separator after
//!    each type and folders as submenus; behind the toggle when 2. is
//!    shown (speed search still finds them), and the configurations in
//!    folders once more, found by speed search only.
//! 4. Current File, a separator, Edit Configurations….
//!
//! A configuration's row selects it on a click and opens its submenu while
//! it is selected (Run, Debug, "Configuration", Edit Configurations…, Save
//! Configuration for a temporary one, Delete, Pin or Unpin); its buttons
//! run it, stop it while it runs, and debug it (not yet).

use run_config::{ConfigKey, RECENT_COUNT, RunConfiguration, RunConfigurations, Types};
use ui::icons::IconName;
use ui::menu::{InlineButton, MenuItem};

use super::commands::*;
use super::widget::TEMPORARY_ICON_ALPHA;

/// A row of the dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A section title ("Pinned", "Recent Configurations").
    Header(&'static str),
    /// A configuration; `folder` is shown after its name; a `hidden` row
    /// shows only while the speed search filters.
    Configuration {
        key: ConfigKey,
        folder: Option<String>,
        hidden: bool,
    },
    /// A folder's configurations, as a submenu.
    Folder {
        name: String,
        keys: Vec<ConfigKey>,
        hidden: bool,
    },
    /// "All Configurations  N": shows or hides the full list.
    Toggle {
        count: usize,
        expanded: bool,
    },
    Separator,
    CurrentFile,
    EditConfigurations,
}

pub const PINNED: &str = "Pinned";
pub const RECENT: &str = "Recent Configurations";
/// The title over a configuration's own actions in its submenu
/// (`choose.run.popup.separator`).
pub const CONFIGURATION: &str = "Configuration";
/// The secondary text of a configuration Pyoxide cannot run yet.
pub const NOT_SUPPORTED: &str = "not supported yet";
/// Width of the dropdown (`MINIMAL_POPUP_WIDTH`, `MAXIMAL_POPUP_WIDTH`).
pub const MIN_WIDTH: f32 = 310.0;
pub const MAX_WIDTH: f32 = 500.0;

/// The toggle's text (`run.toolbar.widget.all.configurations`).
pub fn toggle_text(count: usize) -> String {
    format!("All Configurations  {count}")
}

/// The rows of the dropdown; `expanded` shows the full list behind the
/// toggle.
pub fn rows(list: &RunConfigurations, types: &Types, expanded: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let folder_of = |k: &ConfigKey| list.get(k).and_then(|c| c.folder.clone());
    let pinned: Vec<&ConfigKey> = list.pinned.iter().filter(|k| list.contains(k)).collect();
    let recent: Vec<&ConfigKey> = list
        .history
        .iter()
        .filter(|k| !list.pinned.contains(k) && list.contains(k))
        .take(RECENT_COUNT)
        .collect();
    let mut shown: Vec<ConfigKey> = Vec::new();
    if !pinned.is_empty() {
        rows.push(Row::Header(PINNED));
        for k in &pinned {
            rows.push(Row::Configuration {
                key: (*k).clone(),
                folder: folder_of(k),
                hidden: false,
            });
            shown.push((*k).clone());
        }
        rows.push(Row::Separator);
    }
    let count = list.len();
    let with_toggle = (!recent.is_empty() || !pinned.is_empty()) && count >= RECENT_COUNT;
    if with_toggle {
        rows.push(Row::Header(RECENT));
        for k in &recent {
            rows.push(Row::Configuration {
                key: (*k).clone(),
                folder: folder_of(k),
                hidden: false,
            });
            shown.push((*k).clone());
        }
        rows.push(Row::Separator);
        rows.push(Row::Toggle { count, expanded });
    }
    // Behind the toggle while it is closed: found by speed search only.
    let collapsed = with_toggle && !expanded;
    let sorted = list.sorted(types);
    let mut i = 0;
    while i < sorted.len() {
        let type_id = &sorted[i].type_id;
        let mut end = i;
        while end < sorted.len() && sorted[end].type_id == *type_id {
            end += 1;
        }
        let group = &sorted[i..end];
        // Folders in the order they first appear, then the rest.
        let mut folders: Vec<(String, Vec<ConfigKey>)> = Vec::new();
        for c in group {
            if let Some(name) = &c.folder {
                match folders.iter_mut().find(|(n, _)| n == name) {
                    Some((_, keys)) => keys.push(c.key()),
                    None => folders.push((name.clone(), vec![c.key()])),
                }
            }
        }
        for (name, keys) in folders {
            rows.push(Row::Folder {
                name,
                keys,
                hidden: collapsed,
            });
        }
        for c in group.iter().filter(|c| c.folder.is_none()) {
            let key = c.key();
            // One already shown above comes again only in the full list.
            if shown.contains(&key) && !expanded {
                continue;
            }
            rows.push(Row::Configuration {
                key,
                folder: None,
                hidden: collapsed,
            });
        }
        // The ones in folders, for speed search.
        for c in group.iter().filter(|c| c.folder.is_some()) {
            let key = c.key();
            if shown.contains(&key) {
                continue;
            }
            rows.push(Row::Configuration {
                key,
                folder: c.folder.clone(),
                hidden: true,
            });
        }
        if !collapsed {
            rows.push(Row::Separator);
        }
        i = end;
    }
    rows.push(Row::CurrentFile);
    rows.push(Row::Separator);
    rows.push(Row::EditConfigurations);
    rows
}

/// What the dropdown's rows show besides the list.
pub struct MenuState<'a> {
    pub list: &'a RunConfigurations,
    pub types: &'a Types,
    pub expanded: bool,
    /// The Run tab whose process runs a configuration.
    pub running: &'a dyn Fn(&ConfigKey) -> Option<usize>,
    /// The file in the editor (its name), whether it can run, and the tab
    /// that runs it.
    pub current_file: Option<(String, bool, Option<usize>)>,
}

fn select_action(c: &RunConfiguration) -> SelectConfiguration {
    SelectConfiguration {
        name: Some(c.name.clone()),
        r#type: Some(c.type_id.clone()),
    }
}

/// A configuration's submenu (`SelectConfigAction.getDefaultChildren`).
fn config_submenu(c: &RunConfiguration, pinned: bool) -> Vec<MenuItem> {
    let name = Some(c.name.clone());
    let ty = Some(c.type_id.clone());
    let mut items = vec![
        MenuItem::action(
            "Run",
            LaunchConfiguration {
                name: name.clone(),
                r#type: ty.clone(),
            },
        )
        .icon(IconName::RUN)
        .enabled(!c.is_unsupported()),
        MenuItem::unavailable("Debug").icon(IconName::DEBUG),
        MenuItem::header(CONFIGURATION),
        MenuItem::action(
            "Edit Configurations\u{2026}",
            EditConfigurations {
                name: name.clone(),
                r#type: ty.clone(),
                run_button: None,
            },
        ),
    ];
    if c.temporary {
        items.push(MenuItem::action(
            "Save Configuration",
            SaveTemporary {
                name: name.clone(),
                r#type: ty.clone(),
            },
        ));
    }
    items.push(MenuItem::action(
        "Delete",
        DeleteConfiguration {
            name: name.clone(),
            r#type: ty.clone(),
        },
    ));
    items.push(MenuItem::action(
        if pinned { "Unpin" } else { "Pin" },
        PinConfiguration { name, r#type: ty },
    ));
    items
}

/// A configuration's row: selects it; Run, Stop and Debug buttons, and
/// More Actions (added by the popup) or Right open its submenu.
fn config_item(
    state: &MenuState<'_>,
    c: &RunConfiguration,
    folder: Option<&str>,
    hidden: bool,
) -> MenuItem {
    let key = c.key();
    let types = state.types;
    let icon = types
        .icon_of(c)
        .map(IconName)
        .unwrap_or(IconName::FILE_UNKNOWN);
    let pinned = state.list.pinned.contains(&key);
    let mut item = MenuItem::submenu(c.name.clone(), config_submenu(c, pinned))
        .perform(select_action(c))
        .icon(icon)
        .description(types.name_of(c).to_owned())
        .hidden(hidden);
    if c.temporary {
        item = item.icon_alpha(TEMPORARY_ICON_ALPHA);
    }
    if c.is_unsupported() {
        item = item.secondary(NOT_SUPPORTED);
    } else if let Some(folder) = folder {
        item = item.secondary(folder.to_owned());
    }
    if !c.is_unsupported() {
        let running = (state.running)(&key);
        let rerun = running.is_some() && !types.allows_parallel(c);
        let (run_icon, run_text) = if rerun {
            (IconName::RERUN, format!("Rerun '{}'", c.name))
        } else {
            (IconName::RUN, format!("Run '{}'", c.name))
        };
        item = item.inline(InlineButton::new(
            run_icon,
            run_text,
            LaunchConfiguration {
                name: Some(c.name.clone()),
                r#type: Some(c.type_id.clone()),
            },
        ));
        if let Some(tab) = running {
            item = item.inline(InlineButton::new(
                IconName::STOP,
                format!("Stop '{}'", c.name),
                Stop { tab: Some(tab) },
            ));
        }
        item = item.inline(InlineButton::disabled(
            IconName::DEBUG,
            format!("Debug '{}'", c.name),
        ));
    }
    item
}

/// The dropdown's items and the row to preselect: the selected
/// configuration's, or Current File.
pub fn menu(state: &MenuState<'_>) -> (Vec<MenuItem>, Option<usize>) {
    let list = state.list;
    let mut items: Vec<MenuItem> = Vec::new();
    let mut preselect = None;
    for row in rows(list, state.types, state.expanded) {
        match row {
            Row::Header(text) => items.push(MenuItem::header(text)),
            Row::Configuration {
                key,
                folder,
                hidden,
            } => {
                let Some(c) = list.get(&key) else {
                    continue;
                };
                if !hidden && preselect.is_none() && list.selected.as_ref() == Some(&key) {
                    preselect = Some(items.len());
                }
                items.push(config_item(state, c, folder.as_deref(), hidden));
            }
            Row::Folder { name, keys, hidden } => {
                let sub: Vec<MenuItem> = keys
                    .iter()
                    .filter_map(|k| list.get(k))
                    .map(|c| config_item(state, c, None, false))
                    .collect();
                items.push(
                    MenuItem::submenu(name, sub)
                        .icon(IconName::FOLDER)
                        .hidden(hidden),
                );
            }
            Row::Toggle { count, expanded } => items.push(
                MenuItem::action(toggle_text(count), ToggleAllConfigurations).icon(if expanded {
                    IconName::CHEVRON_DOWN
                } else {
                    IconName::CHEVRON_RIGHT
                }),
            ),
            Row::Separator => {
                if items
                    .last()
                    .is_some_and(|i| !matches!(i, MenuItem::Separator))
                {
                    items.push(MenuItem::separator());
                }
            }
            Row::CurrentFile => {
                if list.selected_config().is_none() {
                    preselect = Some(items.len());
                }
                items.push(current_file_item(state));
            }
            Row::EditConfigurations => items.push(MenuItem::action(
                "Edit Configurations\u{2026}",
                EditConfigurations {
                    name: None,
                    r#type: None,
                    run_button: None,
                },
            )),
        }
    }
    (items, preselect)
}

/// Current File (`createCurrentFileWithInlineActions`): selects it; runs
/// the file in the editor. Without a file it only selects (no buttons, no
/// submenu), as the toolbar's buttons are disabled then.
fn current_file_item(state: &MenuState<'_>) -> MenuItem {
    let Some((name, runnable, running)) = &state.current_file else {
        return MenuItem::action("Current File", SelectCurrentFile);
    };
    let (runnable, running) = (*runnable, *running);
    let run_text = format!("Run '{name}'");
    let sub = vec![
        MenuItem::action(run_text.clone(), RunFile)
            .icon(IconName::RUN)
            .enabled(runnable),
        MenuItem::unavailable("Debug").icon(IconName::DEBUG),
        MenuItem::separator(),
        MenuItem::unavailable("Run with Parameters\u{2026}"),
    ];
    let mut item = MenuItem::submenu("Current File", sub).perform(SelectCurrentFile);
    if runnable {
        item = item.inline(InlineButton::new(IconName::RUN, run_text, RunFile));
    }
    if let Some(tab) = running {
        item = item.inline(InlineButton::new(
            IconName::STOP,
            "Stop",
            Stop { tab: Some(tab) },
        ));
    }
    item.inline(InlineButton::disabled(IconName::DEBUG, "Debug"))
}

/// A configuration's name cut in the middle for the run widget, as the
/// reference cuts it (`trimRunConfigurationName`, `splitCharCount`): the
/// end keeps 8 characters when it can, the beginning gets the rest.
/// The reference measures widths; Pyoxide counts characters, up to
/// 33 + 8 of them.
pub fn trim_name(name: &str) -> String {
    const MAX: usize = 33 + 8;
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= MAX {
        return name.to_owned();
    }
    let count = MAX - 1;
    let (prefix, suffix) = match count {
        c if c >= 16 => (c - 8, 8),
        c if c > 8 => (8, c - 8),
        c => (c, 0),
    };
    let mut out: String = chars[..prefix].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - suffix..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_config::{Reference, Storage, TypeInfo};

    fn types() -> Types {
        Types::new([
            TypeInfo {
                id: "python",
                name: "Python",
                icon: "python/file",
                reference_id: "PythonConfigurationType",
                allow_parallel: false,
            },
            run_config::shell::TYPE,
        ])
    }

    fn list(names: &[(&str, &str, Option<&str>)]) -> RunConfigurations {
        let mut list = RunConfigurations::default();
        for (type_id, name, folder) in names {
            let mut c = RunConfiguration::new(*type_id, *name, Storage::Personal);
            c.folder = folder.map(str::to_owned);
            list.add(c);
        }
        list
    }

    fn key(type_id: &str, name: &str) -> ConfigKey {
        ConfigKey::new(type_id, name)
    }

    fn config(type_id: &str, name: &str) -> Row {
        Row::Configuration {
            key: key(type_id, name),
            folder: None,
            hidden: false,
        }
    }

    #[test]
    fn fewer_than_five_configurations_list_everything_by_type() {
        let mut list = list(&[
            ("shell-script", "build", None),
            ("python", "worker", None),
            ("python", "api", Some("Servers")),
            ("python", "web", Some("Servers")),
        ]);
        list.history = vec![key("python", "worker")];
        assert_eq!(
            rows(&list, &types(), false),
            [
                Row::Folder {
                    name: "Servers".into(),
                    keys: vec![key("python", "api"), key("python", "web")],
                    hidden: false,
                },
                config("python", "worker"),
                Row::Configuration {
                    key: key("python", "api"),
                    folder: Some("Servers".into()),
                    hidden: true
                },
                Row::Configuration {
                    key: key("python", "web"),
                    folder: Some("Servers".into()),
                    hidden: true
                },
                Row::Separator,
                config("shell-script", "build"),
                Row::Separator,
                Row::CurrentFile,
                Row::Separator,
                Row::EditConfigurations,
            ]
        );
    }

    #[test]
    fn five_or_more_with_a_history_show_recent_and_the_toggle() {
        let mut list = list(&[
            ("python", "a", None),
            ("python", "b", Some("F")),
            ("python", "c", None),
            ("python", "d", None),
            ("python", "e", None),
        ]);
        list.history = vec![key("python", "c"), key("python", "b"), key("python", "a")];
        list.pinned = vec![key("python", "a")];
        let collapsed = rows(&list, &types(), false);
        assert_eq!(
            collapsed[..8],
            [
                Row::Header(PINNED),
                config("python", "a"),
                Row::Separator,
                Row::Header(RECENT),
                config("python", "c"),
                Row::Configuration {
                    key: key("python", "b"),
                    folder: Some("F".into()),
                    hidden: false
                },
                Row::Separator,
                Row::Toggle {
                    count: 5,
                    expanded: false
                },
            ]
        );
        // Behind the toggle: hidden rows only, no separators.
        assert!(collapsed[8..collapsed.len() - 3].iter().all(|r| matches!(
            r,
            Row::Configuration { hidden: true, .. } | Row::Folder { hidden: true, .. }
        )));
        assert_eq!(
            collapsed[collapsed.len() - 3..],
            [Row::CurrentFile, Row::Separator, Row::EditConfigurations]
        );
        let expanded = rows(&list, &types(), true);
        assert_eq!(
            expanded[7],
            Row::Toggle {
                count: 5,
                expanded: true
            }
        );
        assert_eq!(
            expanded[8],
            Row::Folder {
                name: "F".into(),
                keys: vec![key("python", "b")],
                hidden: false
            }
        );
        assert_eq!(expanded[9], config("python", "a"));
        // Without a history the full list shows, even with five.
        let mut plain = list.clone();
        plain.history.clear();
        plain.pinned.clear();
        assert!(
            !rows(&plain, &types(), false)
                .iter()
                .any(|r| matches!(r, Row::Toggle { .. }))
        );
    }

    #[test]
    fn rows_have_the_reference_actions_and_marks() {
        let mut list = list(&[("python", "api", None)]);
        let mut temp = RunConfiguration::new("python", "scratch", Storage::Personal);
        temp.temporary = true;
        list.add(temp);
        let mut tests = RunConfiguration::new("idea:tests", "tests", Storage::Personal);
        tests.reference = Some(Reference {
            type_id: "tests".into(),
            factory: None,
            type_name: Some("Python tests".into()),
            xml: String::new(),
        });
        list.add(tests);
        list.selected = Some(key("python", "scratch"));
        let running = |k: &ConfigKey| (k.name == "api").then_some(0);
        let types = types();
        let state = MenuState {
            list: &list,
            types: &types,
            expanded: false,
            running: &running,
            current_file: Some(("main.py".into(), true, None)),
        };
        let (items, preselect) = menu(&state);
        let labels: Vec<String> = items
            .iter()
            .map(|i| i.label().map(|l| l.to_string()).unwrap_or_default())
            .collect();
        assert_eq!(
            labels,
            [
                "api",
                "scratch",
                "",
                "tests",
                "",
                "Current File",
                "",
                "Edit Configurations\u{2026}"
            ]
        );
        assert_eq!(preselect, Some(1));
        let MenuItem::Submenu {
            items: sub, extra, ..
        } = &items[0]
        else {
            panic!("a submenu row");
        };
        let tooltips: Vec<&str> = extra.inline.iter().map(|b| b.tooltip.as_ref()).collect();
        assert_eq!(tooltips, ["Rerun 'api'", "Stop 'api'", "Debug 'api'"]);
        let sub_labels: Vec<String> = sub
            .iter()
            .map(|i| match i {
                MenuItem::Header(h) => format!("[{h}]"),
                i => i.label().map(|l| l.to_string()).unwrap_or_default(),
            })
            .collect();
        assert_eq!(
            sub_labels,
            [
                "Run",
                "Debug",
                "[Configuration]",
                "Edit Configurations\u{2026}",
                "Delete",
                "Pin"
            ]
        );
        let MenuItem::Submenu {
            items: sub, extra, ..
        } = &items[1]
        else {
            panic!("a submenu row");
        };
        assert_eq!(extra.icon_alpha, Some(TEMPORARY_ICON_ALPHA));
        assert!(
            sub.iter()
                .any(|i| i.label().is_some_and(|l| l == "Save Configuration"))
        );
        let MenuItem::Submenu { extra, .. } = &items[3] else {
            panic!("a submenu row");
        };
        assert_eq!(
            extra.secondary.as_ref().map(|s| s.as_ref()),
            Some(NOT_SUPPORTED)
        );
        assert!(extra.inline.is_empty());
    }

    /// Without a file in the editor Current File only selects: no buttons,
    /// no submenu (#135).
    #[test]
    fn current_file_without_a_file_has_no_buttons() {
        let list = list(&[("python", "api", None)]);
        let types = types();
        let running = |_: &ConfigKey| None;
        let state = MenuState {
            list: &list,
            types: &types,
            expanded: false,
            running: &running,
            current_file: None,
        };
        let (items, _) = menu(&state);
        let item = items
            .iter()
            .find(|i| i.label().is_some_and(|l| l.as_ref() == "Current File"))
            .unwrap();
        assert!(matches!(item, MenuItem::Action { extra, .. } if extra.inline.is_empty()));
    }

    #[test]
    fn names_are_cut_in_the_middle() {
        assert_eq!(trim_name("short"), "short");
        let long = "a-very-long-configuration-name-for-the-server-x";
        let cut = trim_name(long);
        assert_eq!(cut.chars().count(), 41);
        assert!(cut.starts_with("a-very-long-configuration-name-f"));
        assert!(cut.ends_with("\u{2026}server-x"));
    }
}
