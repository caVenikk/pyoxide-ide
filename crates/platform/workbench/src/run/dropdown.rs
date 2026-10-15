//! What the run widget's configuration dropdown lists, in the reference's
//! structure (`RunToolbarPopup.createRunConfigurationsActionGroup`):
//!
//! 1. "Pinned" and the pinned configurations, when there are any.
//! 2. "Recent Configurations" (the start history without the pinned ones,
//!    at most [`RECENT_COUNT`]) and the "All Configurations  N" toggle,
//!    when something is pinned or was started and the project has at
//!    least [`RECENT_COUNT`] configurations.
//! 3. Every configuration by type in list order, with a separator after
//!    each type and folders as submenus (behind the toggle when 2. is
//!    shown).
//! 4. Current File, a separator, Edit Configurations….

use run_config::{ConfigKey, RECENT_COUNT, RunConfigurations, Types};

/// A row of the dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A section title ("Pinned", "Recent Configurations").
    Header(&'static str),
    /// A configuration; `folder` is shown after its name in the Pinned and
    /// Recent sections.
    Configuration {
        key: ConfigKey,
        folder: Option<String>,
    },
    /// A folder's configurations, as a submenu.
    Folder {
        name: String,
        keys: Vec<ConfigKey>,
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

/// The toggle's text (`run.toolbar.widget.all.configurations`).
pub fn toggle_text(count: usize) -> String {
    format!("All Configurations  {count}")
}

/// The rows of the dropdown; `show_all` expands the full list behind the
/// toggle.
pub fn rows(list: &RunConfigurations, types: &Types, show_all: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let folder_of = |k: &ConfigKey| list.get(k).and_then(|c| c.folder.clone());
    let pinned: Vec<&ConfigKey> = list.pinned.iter().filter(|k| list.contains(k)).collect();
    let recent: Vec<&ConfigKey> = list
        .history
        .iter()
        .filter(|k| !list.pinned.contains(k) && list.contains(k))
        .take(RECENT_COUNT)
        .collect();
    if !pinned.is_empty() {
        rows.push(Row::Header(PINNED));
        for k in &pinned {
            rows.push(Row::Configuration {
                key: (*k).clone(),
                folder: folder_of(k),
            });
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
            });
        }
        rows.push(Row::Separator);
        rows.push(Row::Toggle {
            count,
            expanded: show_all,
        });
    }
    if !with_toggle || show_all {
        let sorted = list.sorted(types);
        let mut i = 0;
        while i < sorted.len() {
            let type_id = &sorted[i].type_id;
            let mut end = i;
            while end < sorted.len() && sorted[end].type_id == *type_id {
                end += 1;
            }
            // Folders in the order they first appear, then the rest.
            let mut folders: Vec<(String, Vec<ConfigKey>)> = Vec::new();
            for c in &sorted[i..end] {
                if let Some(name) = &c.folder {
                    match folders.iter_mut().find(|(n, _)| n == name) {
                        Some((_, keys)) => keys.push(c.key()),
                        None => folders.push((name.clone(), vec![c.key()])),
                    }
                }
            }
            let mut folder_rows: Vec<Row> = folders
                .into_iter()
                .map(|(name, keys)| Row::Folder { name, keys })
                .collect();
            let mut plain: Vec<Row> = sorted[i..end]
                .iter()
                .filter(|c| c.folder.is_none())
                .map(|c| Row::Configuration {
                    key: c.key(),
                    folder: None,
                })
                .collect();
            rows.append(&mut folder_rows);
            rows.append(&mut plain);
            rows.push(Row::Separator);
            i = end;
        }
    }
    rows.push(Row::CurrentFile);
    rows.push(Row::Separator);
    rows.push(Row::EditConfigurations);
    rows
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
    use run_config::{RunConfiguration, Storage, TypeInfo};

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
                    keys: vec![key("python", "api"), key("python", "web")]
                },
                config("python", "worker"),
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
            collapsed,
            [
                Row::Header(PINNED),
                config("python", "a"),
                Row::Separator,
                Row::Header(RECENT),
                config("python", "c"),
                Row::Configuration {
                    key: key("python", "b"),
                    folder: Some("F".into())
                },
                Row::Separator,
                Row::Toggle {
                    count: 5,
                    expanded: false
                },
                Row::CurrentFile,
                Row::Separator,
                Row::EditConfigurations,
            ]
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
                keys: vec![key("python", "b")]
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
    fn names_are_cut_in_the_middle() {
        assert_eq!(trim_name("short"), "short");
        let long = "a-very-long-configuration-name-for-the-server-x";
        let cut = trim_name(long);
        assert_eq!(cut.chars().count(), 41);
        assert!(cut.starts_with("a-very-long-configuration-name-f"));
        assert!(cut.ends_with("\u{2026}server-x"));
    }
}
