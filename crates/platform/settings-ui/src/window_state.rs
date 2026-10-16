//! What the Settings window keeps between its openings and sessions, in
//! `settings-window.toml` of the configuration folder (ADR 0017): its size,
//! the tree's share of the width, the last page and the search history
//! (the reference keeps them as the window state `SettingsEditor`,
//! `settings.editor.splitter.proportion`,
//! `settings.editor.selected.configurable` and `SettingsSearchHistory`).

use std::path::PathBuf;

use gpui::{App, AppContext as _, Global};
use serde::{Deserialize, Serialize};

/// The file's name in the configuration folder.
pub const FILE: &str = "settings-window.toml";

/// Most searches the history keeps (`SearchTextField`).
pub const HISTORY: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WindowState {
    /// The window's content size.
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// The tree's share of the window's width.
    pub split: Option<f32>,
    /// The page shown last, by id.
    pub page: Option<String>,
    /// Recent searches, the latest first.
    pub search_history: Vec<String>,
}

impl WindowState {
    /// Puts a search first in the history (once, ignoring case).
    pub fn remember_search(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let lower = text.to_lowercase();
        self.search_history.retain(|t| t.to_lowercase() != lower);
        self.search_history.insert(0, text.to_string());
        self.search_history.truncate(HISTORY);
    }
}

#[derive(Default)]
struct Saved {
    file: Option<PathBuf>,
    state: WindowState,
    /// The state changed since it was written.
    dirty: bool,
}

impl Global for Saved {}

/// Reads the state from `file` and writes it there from now on.
pub fn load(cx: &mut App, file: PathBuf) {
    let state = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| toml::from_str::<WindowState>(&text).ok())
        .unwrap_or_default();
    let g = cx.default_global::<Saved>();
    g.state = state;
    g.file = Some(file);
    g.dirty = false;
}

/// The state now.
pub fn get(cx: &App) -> WindowState {
    cx.try_global::<Saved>()
        .map(|g| g.state.clone())
        .unwrap_or_default()
}

/// Changes the state; it is written when the window closes (or the
/// application quits).
pub(crate) fn update(cx: &mut App, f: impl FnOnce(&mut WindowState)) {
    let g = cx.default_global::<Saved>();
    let before = g.state.clone();
    f(&mut g.state);
    if g.state != before {
        g.dirty = true;
    }
}

/// Writes a changed state in the background.
pub(crate) fn save(cx: &mut App) {
    if let Some((file, text)) = take_save(cx) {
        cx.background_spawn(async move { write(&file, &text) })
            .detach();
    }
}

/// Writes a changed state now (on quit).
pub fn flush(cx: &mut App) {
    if let Some((file, text)) = take_save(cx) {
        write(&file, &text);
    }
}

fn take_save(cx: &mut App) -> Option<(PathBuf, String)> {
    let g = cx.try_global::<Saved>()?;
    if !g.dirty {
        return None;
    }
    let file = g.file.clone()?;
    let text = toml::to_string(&g.state).ok()?;
    cx.global_mut::<Saved>().dirty = false;
    Some((file, text))
}

fn write(file: &std::path::Path, text: &str) {
    if let Err(e) = settings::write_atomic(file, text) {
        eprintln!("cannot save {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_keeps_five_recent_searches_once() {
        let mut s = WindowState::default();
        for t in ["a", "b", "c", "d", "e", "f", " B ", ""] {
            s.remember_search(t);
        }
        assert_eq!(s.search_history, ["B", "f", "e", "d", "c"]);
    }

    #[test]
    fn the_state_reads_back() {
        let s = WindowState {
            width: Some(1000.0),
            height: Some(720.0),
            split: Some(0.3),
            page: Some("system".into()),
            search_history: vec!["wrap".into()],
        };
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("search-history"), "{text}");
        assert_eq!(toml::from_str::<WindowState>(&text).unwrap(), s);
        assert_eq!(
            toml::from_str::<WindowState>("unknown = 1").unwrap(),
            WindowState::default()
        );
    }
}
