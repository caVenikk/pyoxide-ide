//! The Actions tab: the registered commands, found by their titles (and
//! descriptions), as the reference's Go to Action finds actions.
//!
//! A command matches when its title matches `*pattern` (or, for a pattern
//! with spaces, when every word of the pattern starts a word of the
//! title), else when every word starts a word of its description. Its
//! weight is the matching degree of the title (start matches preferred),
//! plus 100 for a pattern with spaces; equal weights go: the title equal
//! to the pattern, starting with it, containing it; available before
//! unavailable; title matches before description matches; then by title.

use gpui::{App, FocusHandle, SharedString, Window};
use name_match::{Matcher, MatchingMode, WordPrefixMatcher};

/// Key contexts whose commands work only inside their popup or list.
const INNER_SCOPES: &[&str] = &[
    "ContextMenu",
    "FindHistory",
    "FindInFiles",
    "GoToLine",
    "SearchEverywhere",
    "RecentFiles",
];

/// Added to the weight of every match of a pattern with spaces.
const SPACE_BONUS: i32 = 100;

/// A command as the Actions tab shows it.
#[derive(Debug, Clone)]
pub struct ActionItem {
    pub id: &'static str,
    pub title: SharedString,
    pub description: SharedString,
    /// Its shortcut where the focus was when the popup opened.
    pub shortcut: Option<SharedString>,
    /// It does something there.
    pub available: bool,
}

/// The commands to offer: everything a person can run without arguments,
/// with its shortcut and availability for `focus` (the focus before the
/// popup opened).
pub fn collect(focus: Option<&FocusHandle>, window: &Window, cx: &mut App) -> Vec<ActionItem> {
    let mut items = Vec::new();
    for command in ui::command::list(cx) {
        let id = command.meta.id;
        if INNER_SCOPES.contains(&command.scope) || ui::command::is_query(cx, id) {
            continue;
        }
        let Ok(action) = cx.build_action(id, Some(serde_json::json!({}))) else {
            continue;
        };
        let available = window.is_action_available(action.as_ref(), cx)
            || cx.is_action_available(action.as_ref());
        let shortcut = match crate::keymap::double_tap_for(id, cx) {
            Some(key) => Some(crate::keymap::double_tap_text(
                key,
                cfg!(target_os = "macos"),
            )),
            None => focus.and_then(|f| ui::keys::shortcut_in(action.as_ref(), f, window)),
        };
        items.push(ActionItem {
            id,
            title: command.meta.title.into(),
            description: command.meta.summary.into(),
            shortcut: shortcut.map(Into::into),
            available,
        });
    }
    items
}

/// How a command matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchMode {
    Name,
    Description,
}

/// A command found.
#[derive(Debug, Clone)]
pub struct ActionHit {
    pub index: usize,
    pub weight: i32,
    pub mode: MatchMode,
}

/// The commands `pattern` finds, best first; unavailable ones only with
/// `include_disabled`.
pub fn search(items: &[ActionItem], pattern: &str, include_disabled: bool) -> Vec<ActionHit> {
    let pattern_trimmed = pattern.trim();
    if pattern_trimmed.is_empty() {
        return Vec::new();
    }
    let spaces = pattern_trimmed.contains(' ');
    let star = Matcher::new(&format!("*{pattern}"), MatchingMode::IgnoreCase);
    let words = WordPrefixMatcher::new(pattern);
    let weigh = Matcher::new(&format!("*{pattern}"), MatchingMode::IgnoreCase).prefer_start();
    let mut hits: Vec<ActionHit> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| include_disabled || item.available)
        .filter_map(|(index, item)| {
            let title_matches = if spaces {
                words.matches(&item.title)
            } else {
                star.matches(&item.title)
            };
            let mode = if title_matches {
                MatchMode::Name
            } else if !item.description.is_empty()
                && item.description != item.title
                && words.matches(&item.description)
            {
                MatchMode::Description
            } else {
                return None;
            };
            let title: Vec<char> = item
                .title
                .trim_matches(|c: char| !c.is_alphanumeric())
                .chars()
                .collect();
            let mut weight = weigh.match_chars(&title).map_or(0, |m| m.degree);
            if spaces {
                weight += SPACE_BONUS;
            }
            Some(ActionHit {
                index,
                weight: weight.max(0),
                mode,
            })
        })
        .collect();
    let lower = pattern_trimmed.to_lowercase();
    let rank = |title: &str| {
        let t = title
            .trim_end_matches("...")
            .trim_end_matches('…')
            .to_lowercase();
        if t == lower {
            3
        } else if t.starts_with(&lower) {
            2
        } else if t.contains(&lower) {
            1
        } else {
            0
        }
    };
    hits.sort_by(|a, b| {
        let (ia, ib) = (&items[a.index], &items[b.index]);
        b.weight
            .cmp(&a.weight)
            .then_with(|| rank(&ib.title).cmp(&rank(&ia.title)))
            .then_with(|| ib.available.cmp(&ia.available))
            .then_with(|| a.mode.cmp(&b.mode))
            .then_with(|| ia.title.to_lowercase().cmp(&ib.title.to_lowercase()))
            .then_with(|| ia.title.len().cmp(&ib.title.len()))
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &'static str, description: &'static str, available: bool) -> ActionItem {
        ActionItem {
            id: title,
            title: title.into(),
            description: description.into(),
            shortcut: None,
            available,
        }
    }

    fn titles(items: &[ActionItem], pattern: &str, disabled: bool) -> Vec<&'static str> {
        search(items, pattern, disabled)
            .iter()
            .map(|h| items[h.index].id)
            .collect()
    }

    #[test]
    fn titles_then_descriptions() {
        let items = [
            item("Find in Files…", "Searches the project for text.", true),
            item("Go to File…", "Opens a file by its name.", true),
            item("Save All", "Saves every modified file.", true),
            item("File Mask", "Narrows the files searched.", false),
        ];
        // "File" ends "Go to File": a final match counts.
        assert_eq!(
            titles(&items, "file", false),
            ["Go to File…", "Find in Files…", "Save All"]
        );
        assert_eq!(
            titles(&items, "file", true),
            ["File Mask", "Go to File…", "Find in Files…", "Save All"]
        );
        assert_eq!(titles(&items, "go file", false), ["Go to File…"]);
        assert_eq!(titles(&items, "fif", false), ["Find in Files…"]);
        assert!(titles(&items, "", false).is_empty());
    }
}
