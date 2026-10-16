//! The search of the Settings window (the reference's `SettingsFilter`
//! over `SearchableOptionsRegistrarImpl`, and `SearchUtil` with the
//! spotlight of `GlassPanel` on the page).
//!
//! A text finds a page by its name (the whole text in its title, ignoring
//! case: a *name hit*) or by its options (a *content hit*: every word of
//! the text, less the reference's stop words, starts a word of the page's
//! title or search terms). `Group | Page` finds a page by its path. The
//! tree keeps the pages found, their parents, and everything under a page
//! found by its name.
//!
//! On the selected page the labels the text finds light up: those with
//! every word of the text, or when there are none, those with any word.
//! Pages call [`label`] or [`spotlight`] for their labels; a page with a
//! search of its own takes the text instead (`SettingsPage::search`).

use std::collections::HashSet;
use std::sync::OnceLock;

use gpui::{AnyElement, App, Global, IntoElement, SharedString, div, prelude::*, px};

use crate::page::PageSpec;

/// The reference's stop words (`search/ignore.txt`): words a search
/// ignores.
const STOP_WORDS: &str = include_str!("../assets/search-ignore.txt");

/// The spotlight around a label: its offsets from the label
/// (`GlassPanel` insets 7 and 5, plus 1), the outline's width and its
/// colours in light and dark themes (`ColorPalette.Orange6` / `Orange4`).
const SPOT_X: f32 = 8.0;
const SPOT_Y: f32 = 6.0;
const SPOT_WIDTH: f32 = 2.0;
const SPOT_LIGHT: u32 = 0xE08855;
const SPOT_DARK: u32 = 0xA36B4E;

fn stop_words() -> &'static HashSet<&'static str> {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        STOP_WORDS
            .lines()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .collect()
    })
}

/// The lowercase words of a text: runs of letters, digits, `-`, `#` and
/// `+` (`WORD_SEPARATOR_CHARS`).
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '#' | '+')))
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// The words of a search text that count: its words less the stop words,
/// each once (`getProcessedWordsWithoutStemming`).
pub fn query_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words(text) {
        if !stop_words().contains(w.as_str()) && !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// The pages a text finds, by index into the pages searched (tree order).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Hits {
    /// Pages whose name has the text; everything under them shows too.
    pub name: Vec<usize>,
    /// Pages whose options have every word of the text.
    pub content: Vec<usize>,
    /// What lights up labels on the page: the text, or what follows a
    /// page found by its path.
    pub spotlight: String,
}

impl Hits {
    /// Every page found.
    pub fn all(&self) -> HashSet<usize> {
        self.name.iter().chain(&self.content).copied().collect()
    }

    /// Nothing was found.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.content.is_empty()
    }
}

/// The pages `text` finds among `pages` (in tree order).
pub(crate) fn find(pages: &[PageSpec], text: &str) -> Hits {
    let query = text.trim().to_lowercase();
    if let Some(hits) = find_by_path(pages, &query) {
        return hits;
    }
    let counted = query_words(&query);
    let name: Vec<usize> = if counted.is_empty() {
        // Only stop words: every page is a name hit.
        (0..pages.len()).collect()
    } else {
        (0..pages.len())
            .filter(|&i| pages[i].title.to_lowercase().contains(&query))
            .collect()
    };
    let prefixes = if counted.is_empty() {
        words(&query)
    } else {
        counted
    };
    let content = if prefixes.is_empty() {
        Vec::new()
    } else {
        (0..pages.len())
            .filter(|&i| {
                let page_words = page_words(&pages[i]);
                prefixes
                    .iter()
                    .all(|p| page_words.iter().any(|w| w.starts_with(p.as_str())))
            })
            .collect()
    };
    Hits {
        name,
        content,
        spotlight: text.trim().to_string(),
    }
}

/// The words a page is found by: of its title and its search terms.
fn page_words(page: &PageSpec) -> Vec<String> {
    let mut out = words(page.title);
    for term in page.search_terms {
        out.extend(words(term));
    }
    out
}

/// `Group | Page`: the page at that path (top-level titles first, then
/// their children), the rest of the path lighting up its labels
/// (`findGroupsByPath`). A leading `Settings`, `Preferences` or
/// `File | Settings` is skipped.
fn find_by_path(pages: &[PageSpec], query: &str) -> Option<Hits> {
    if !query.contains('|') {
        return None;
    }
    let mut parts: Vec<&str> = query.split('|').map(str::trim).collect();
    for prefix in [&["file", "settings"][..], &["settings"], &["preferences"]] {
        if parts.len() >= prefix.len() && parts[..prefix.len()] == *prefix {
            parts.drain(..prefix.len());
        }
    }
    let mut parent: Option<&str> = None;
    let mut found: Option<(usize, usize)> = None;
    for (n, part) in parts.iter().enumerate() {
        let Some(i) = pages
            .iter()
            .position(|p| p.parent == parent && p.title.eq_ignore_ascii_case(part))
        else {
            break;
        };
        found = Some((i, n));
        parent = Some(pages[i].id);
    }
    let (page, last) = found?;
    Some(Hits {
        name: vec![page],
        content: vec![page],
        spotlight: parts[last + 1..].join(" "),
    })
}

/// Whether the search text `option` lights up a label with `text`
/// (`SearchUtil.isComponentHighlighted`): with `all`, the label has every
/// word of it; otherwise any word, or the whole text. A word is had when a
/// word of the label starts with it.
pub fn label_matches(text: &str, option: &str, all: bool) -> bool {
    let option = option.trim();
    if option.is_empty() {
        return false;
    }
    let contains = || text.to_lowercase().contains(&option.to_lowercase());
    let counted = query_words(option);
    if counted.is_empty() {
        return contains();
    }
    let tokens = words(text);
    let has = |w: &String| tokens.iter().any(|t| t.starts_with(w.as_str()));
    if all {
        counted.iter().all(has)
    } else {
        counted.iter().any(has) || contains()
    }
}

/// The text the selected page lights up its labels with, set by the
/// Settings window while it draws.
#[derive(Debug, Default, Clone)]
pub(crate) struct Spotlight {
    pub text: String,
    /// Labels need every word (some label of the page has them all).
    pub all: bool,
}

impl Global for Spotlight {}

/// Sets what pages light up (`""`: nothing).
pub(crate) fn set_spotlight(cx: &mut App, text: &str, all: bool) {
    let g = cx.default_global::<Spotlight>();
    if g.text != text || g.all != all {
        g.text = text.to_string();
        g.all = all;
    }
}

/// The text labels light up with now (`""` without a search).
pub fn spotlight_text(cx: &App) -> String {
    cx.try_global::<Spotlight>()
        .map(|s| s.text.clone())
        .unwrap_or_default()
}

/// Whether the window's search lights up a label with `text` now.
pub fn is_lit(text: &str, cx: &App) -> bool {
    cx.try_global::<Spotlight>()
        .is_some_and(|s| label_matches(text, &s.text, s.all))
}

/// `element` (a label, or a check box with its label `text`) with the
/// spotlight's outline around it while the window's search finds `text`.
pub fn spotlight(text: &str, element: impl IntoElement, cx: &App) -> AnyElement {
    if !is_lit(text, cx) {
        return element.into_any_element();
    }
    let dark = cx
        .try_global::<ui::theme::Theme>()
        .is_some_and(|t| t.is_dark());
    let color = gpui::rgb(if dark { SPOT_DARK } else { SPOT_LIGHT });
    div()
        .relative()
        .debug_selector(|| "settings-spotlight".into())
        .child(element)
        .child(
            div()
                .absolute()
                .top(px(-SPOT_Y))
                .bottom(px(-SPOT_Y))
                .left(px(-SPOT_X))
                .right(px(-SPOT_X))
                .rounded(px(15.0))
                .border(px(SPOT_WIDTH))
                .border_color(color),
        )
        .into_any_element()
}

/// A plain label of a page, lit up while the search finds it.
pub fn label(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    let text: SharedString = text.into();
    let element = div().whitespace_nowrap().child(text.clone());
    spotlight(&text, element, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(
        id: &'static str,
        parent: Option<&'static str>,
        title: &'static str,
        terms: &'static [&'static str],
    ) -> PageSpec {
        PageSpec {
            search_terms: terms,
            ..PageSpec::group(id, parent, title)
        }
    }

    fn pages() -> Vec<PageSpec> {
        vec![
            page("appearance-behavior", None, "Appearance & Behavior", &[]),
            page(
                "system",
                Some("appearance-behavior"),
                "System Settings",
                &[
                    "Save files if the IDE is idle for",
                    "Sync external changes:",
                ],
            ),
            page("keymap", None, "Keymap", &["Soft-Wrap"]),
            page("editor", None, "Editor", &[]),
            page(
                "editor.general",
                Some("editor"),
                "General",
                &["Soft-wrap these files:", "Use soft wraps in editor"],
            ),
            page("editor.font", Some("editor"), "Font", &["Size:"]),
        ]
    }

    #[test]
    fn words_split_like_the_reference() {
        assert_eq!(
            words("Soft-Wrap, C# and C++!"),
            ["soft-wrap", "c#", "and", "c++"]
        );
        // Stop words do not count in a search text.
        assert_eq!(query_words("the files of the IDE"), ["files", "ide"]);
    }

    #[test]
    fn a_name_hit_needs_the_whole_text_and_content_hits_word_prefixes() {
        let p = pages();
        let hits = find(&p, "soft");
        assert!(hits.name.is_empty());
        assert_eq!(hits.content, [2, 4]);
        let hits = find(&p, "  system set ");
        assert_eq!(hits.name, [1]);
        assert_eq!(hits.spotlight, "system set");
        // Every word, in any of the page's terms.
        assert_eq!(find(&p, "idle sync").content, [1]);
        assert!(find(&p, "idle wrap").is_empty());
        assert!(find(&p, "zzzz").is_empty());
    }

    #[test]
    fn only_stop_words_find_every_page() {
        let p = pages();
        assert_eq!(find(&p, "the").name.len(), p.len());
    }

    #[test]
    fn a_path_finds_the_page_and_lights_up_the_rest() {
        let p = pages();
        let hits = find(&p, "Editor | font | size");
        assert_eq!(hits.name, [5]);
        assert_eq!(hits.spotlight, "size");
        let hits = find(&p, "Settings | Editor");
        assert_eq!(hits.name, [3]);
        assert_eq!(hits.spotlight, "");
        assert_eq!(find(&p, "Nothing | Font").name, Vec::<usize>::new());
    }

    #[test]
    fn labels_light_up_with_all_words_or_any() {
        assert!(label_matches("Use soft wraps in editor", "soft wrap", true));
        assert!(!label_matches(
            "Soft-wrap these files:",
            "soft editor",
            true
        ));
        assert!(label_matches(
            "Soft-wrap these files:",
            "soft editor",
            false
        ));
        assert!(!label_matches("Size:", "soft", false));
        assert!(!label_matches("Size:", "", false));
        // Only stop words: the whole text.
        assert!(label_matches("Do it", "do", false));
    }
}
