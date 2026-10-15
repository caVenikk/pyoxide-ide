//! Navigation history of a main window: Back and Forward, as the reference
//! IDE keeps them (`IdeDocumentHistoryImpl`).
//!
//! A navigation (another file becoming the active editor's, a jump of the
//! caret to a declaration, a search result, a line, ...) records the place
//! it leaves: the file and the caret there. Back returns to the last place
//! recorded and keeps the place it leaves for Forward; a new navigation
//! drops the places Forward would go to. A place next to the last one
//! recorded (the same file, less than [`SAME_PLACE_LINES`] lines away)
//! takes its slot instead of adding another, so moving around one spot
//! leaves one place. Places in files that no longer exist are skipped.
//!
//! The model here knows no toolkit; the workbench records the places and
//! goes to them (`workbench/history.rs`).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::json;

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Go back.
    Back => "workbench.back", "Back",
        "Goes back to the place the last navigation left (another file, or a jump within a file).";
    /// Go forward.
    Forward => "workbench.forward", "Forward",
        "Goes forward to the place Back left.";
}

/// Registers the commands.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, COMMANDS);
}

/// Most places Back keeps (the reference's registry value
/// `editor.navigation.history.stack.size`); the oldest go first.
pub const LIMIT: usize = 150;

/// Two places in a file are the same place when their carets are less than
/// this many lines apart (the reference's text editor state merges such
/// places, `TextEditorState.MIN_CHANGE_DISTANCE`).
pub const SAME_PLACE_LINES: usize = 4;

/// A place to come back to: a file, the caret in it and where the caret's
/// line was in the view.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub path: PathBuf,
    /// Zero-based line and column of the caret.
    pub caret: text::Point,
    /// How far the caret's line was from the top of the view, in pixels
    /// (the reference's relative caret position); going back puts it
    /// there again. `None` when the editor had not been drawn: the line
    /// goes to the middle of the view.
    pub view_y: Option<f32>,
}

impl Place {
    pub fn new(path: impl Into<PathBuf>, caret: text::Point) -> Self {
        Self {
            path: path.into(),
            caret,
            view_y: None,
        }
    }

    /// The place with the caret's line `view_y` from the top of the view.
    pub fn with_view_y(mut self, view_y: Option<f32>) -> Self {
        self.view_y = view_y;
        self
    }

    /// The same file with the caret less than [`SAME_PLACE_LINES`] lines
    /// away.
    pub fn is_same(&self, other: &Place) -> bool {
        self.path == other.path && self.caret.line.abs_diff(other.caret.line) < SAME_PLACE_LINES
    }
}

/// The places Back and Forward go to, the most recent last.
#[derive(Debug, Clone, Default)]
pub struct NavigationHistory {
    back: VecDeque<Place>,
    forward: VecDeque<Place>,
}

impl NavigationHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// A navigation left `place`: Back will return there, and the places
    /// Forward would go to are dropped.
    pub fn record(&mut self, place: Place) {
        self.push_back(place);
        self.forward.clear();
    }

    /// Adds a place for Back; it takes the last one's slot when it is the
    /// same place, and the oldest goes beyond the limit.
    fn push_back(&mut self, place: Place) {
        if self.back.back().is_some_and(|last| last.is_same(&place)) {
            self.back.pop_back();
        }
        self.back.push_back(place);
        if self.back.len() > LIMIT {
            self.back.pop_front();
        }
    }

    /// Back: the place to go to (the last recorded); `current`, the place
    /// Back leaves, is kept for Forward. `None` when there is nothing to go
    /// back to.
    pub fn back(&mut self, current: Option<Place>) -> Option<Place> {
        let target = self.back.pop_back()?;
        if let Some(current) = current {
            self.forward.push_back(current);
        }
        Some(target)
    }

    /// Forward: the place to go to (the last one Back left, skipping those
    /// that are the current place while there are more); `current` becomes
    /// a place for Back, the other places Forward goes to are kept. `None`
    /// when there is nothing to go forward to.
    pub fn forward(&mut self, current: Option<Place>) -> Option<Place> {
        let mut target = self.forward.pop_back()?;
        if let Some(current) = &current {
            while target.is_same(current) {
                match self.forward.pop_back() {
                    Some(next) => target = next,
                    None => break,
                }
            }
        }
        if let Some(current) = current {
            self.push_back(current);
        }
        Some(target)
    }

    /// Drops the places in files `exists` says are gone (asked once per
    /// file).
    pub fn retain_existing(&mut self, mut exists: impl FnMut(&Path) -> bool) {
        let mut known: HashMap<PathBuf, bool> = HashMap::new();
        let mut keep = |place: &Place| {
            *known
                .entry(place.path.clone())
                .or_insert_with(|| exists(&place.path))
        };
        self.back.retain(&mut keep);
        self.forward.retain(&mut keep);
    }

    /// `from` was renamed or moved to `to`: the places in it (and in the
    /// files inside a folder) follow, as the reference's places follow the
    /// file.
    pub fn rename(&mut self, from: &Path, to: &Path) {
        for place in self.back.iter_mut().chain(self.forward.iter_mut()) {
            if let Ok(rest) = place.path.strip_prefix(from) {
                place.path = if rest.as_os_str().is_empty() {
                    to.to_owned()
                } else {
                    to.join(rest)
                };
            }
        }
    }

    /// `path` was deleted: the places in it (and in the files inside a
    /// folder) are dropped.
    pub fn remove(&mut self, path: &Path) {
        self.back.retain(|p| !p.path.starts_with(path));
        self.forward.retain(|p| !p.path.starts_with(path));
    }

    /// Forgets every place (another project opened).
    pub fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The places Back goes to, the next one last.
    pub fn back_places(&self) -> impl ExactSizeIterator<Item = &Place> {
        self.back.iter()
    }

    /// The places Forward goes to, the next one last.
    pub fn forward_places(&self) -> impl ExactSizeIterator<Item = &Place> {
        self.forward.iter()
    }

    /// The history as JSON (`workbench.state`): the places of Back and
    /// Forward, the next one last; paths as `path` shows them.
    pub fn state_json(&self, path: impl Fn(&Path) -> String) -> serde_json::Value {
        let places = |list: &VecDeque<Place>| {
            list.iter()
                .map(|p| {
                    json!({
                        "path": path(&p.path),
                        "line": p.caret.line,
                        "column": p.caret.column,
                        "view_y": p.view_y,
                    })
                })
                .collect::<Vec<_>>()
        };
        json!({
            "back": places(&self.back),
            "forward": places(&self.forward),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, line: usize) -> Place {
        Place::new(path, text::Point::new(line, 0))
    }

    fn lines(places: impl Iterator<Item = Place>) -> Vec<(String, usize)> {
        places
            .map(|p| (p.path.display().to_string(), p.caret.line))
            .collect()
    }

    fn back(h: &NavigationHistory) -> Vec<(String, usize)> {
        lines(h.back_places().cloned())
    }

    fn forward(h: &NavigationHistory) -> Vec<(String, usize)> {
        lines(h.forward_places().cloned())
    }

    fn p(path: &str, line: usize) -> (String, usize) {
        (path.to_owned(), line)
    }

    #[test]
    fn nothing_recorded_goes_nowhere() {
        let mut h = NavigationHistory::new();
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
        assert_eq!(h.back(Some(at("a", 0))), None);
        assert_eq!(h.forward(Some(at("a", 0))), None);
        assert!(!h.can_go_forward(), "a failed Back keeps nothing");
    }

    #[test]
    fn records_the_places_navigation_leaves() {
        let mut h = NavigationHistory::new();
        h.record(at("a", 10));
        assert!(h.can_go_back());
        assert!(!h.can_go_forward());
        h.record(at("b", 0));
        assert_eq!(back(&h), [p("a", 10), p("b", 0)]);
    }

    #[test]
    fn back_and_forward_walk_the_places() {
        // a:1 -> a:20 -> a:40, now at a:60.
        let mut h = NavigationHistory::new();
        h.record(at("a", 1));
        h.record(at("a", 20));
        h.record(at("a", 40));
        let mut current = at("a", 60);

        current = h.back(Some(current)).unwrap();
        assert_eq!(current, at("a", 40));
        assert_eq!(forward(&h), [p("a", 60)]);
        current = h.back(Some(current)).unwrap();
        assert_eq!(current, at("a", 20));
        current = h.back(Some(current)).unwrap();
        assert_eq!(current, at("a", 1));
        assert!(!h.can_go_back());
        assert_eq!(forward(&h), [p("a", 60), p("a", 40), p("a", 20)]);

        current = h.forward(Some(current)).unwrap();
        assert_eq!(current, at("a", 20));
        assert_eq!(back(&h), [p("a", 1)]);
        current = h.forward(Some(current)).unwrap();
        assert_eq!(current, at("a", 40));
        current = h.forward(Some(current)).unwrap();
        assert_eq!(current, at("a", 60));
        assert!(!h.can_go_forward());
        assert_eq!(back(&h), [p("a", 1), p("a", 20), p("a", 40)]);
    }

    #[test]
    fn a_new_navigation_drops_the_forward_places() {
        let mut h = NavigationHistory::new();
        h.record(at("a", 0));
        h.record(at("b", 0));
        let current = h.back(Some(at("c", 0))).unwrap();
        assert_eq!(current, at("b", 0));
        assert!(h.can_go_forward());

        h.record(current);
        assert!(!h.can_go_forward());
        assert_eq!(back(&h), [p("a", 0), p("b", 0)]);
    }

    #[test]
    fn a_place_near_the_last_one_takes_its_slot() {
        let mut h = NavigationHistory::new();
        h.record(at("a", 10));
        // Three lines away: the same place, the newer one stays.
        h.record(at("a", 13));
        assert_eq!(back(&h), [p("a", 13)]);
        h.record(at("a", 10));
        assert_eq!(back(&h), [p("a", 10)]);
        // Four lines away is another place; another file too.
        h.record(at("a", 14));
        h.record(at("b", 14));
        assert_eq!(back(&h), [p("a", 10), p("a", 14), p("b", 14)]);
        // Only the last place is compared.
        h.record(at("a", 11));
        assert_eq!(back(&h), [p("a", 10), p("a", 14), p("b", 14), p("a", 11)]);
        // The view does not count; the newer place's view stays.
        h.record(at("a", 12).with_view_y(Some(40.0)));
        assert_eq!(h.back_places().last().unwrap().view_y, Some(40.0));
        assert_eq!(h.back_places().len(), 4);
    }

    #[test]
    fn forward_skips_the_current_place() {
        let mut h = NavigationHistory::new();
        h.record(at("a", 0));
        h.record(at("a", 30));
        let current = h.back(Some(at("a", 60))).unwrap();
        let current = h.back(Some(current)).unwrap();
        assert_eq!(current, at("a", 0));
        assert_eq!(forward(&h), [p("a", 60), p("a", 30)]);
        // The caret moved next to the first forward place.
        let target = h.forward(Some(at("a", 31))).unwrap();
        assert_eq!(target, at("a", 60));
        assert!(!h.can_go_forward());
        assert_eq!(back(&h), [p("a", 31)]);
    }

    #[test]
    fn forward_to_a_near_place_when_it_is_the_only_one() {
        // Go to a declaration two lines up, Back, Forward.
        let mut h = NavigationHistory::new();
        h.record(at("a", 2));
        let current = h.back(Some(at("a", 0))).unwrap();
        assert_eq!(current, at("a", 2));
        assert_eq!(h.forward(Some(current)), Some(at("a", 0)));
    }

    #[test]
    fn keeps_at_most_the_limit() {
        let mut h = NavigationHistory::new();
        for i in 0..LIMIT + 5 {
            h.record(at("a", i * SAME_PLACE_LINES));
        }
        assert_eq!(h.back_places().len(), LIMIT);
        assert_eq!(h.back_places().next(), Some(&at("a", 5 * SAME_PLACE_LINES)));
    }

    #[test]
    fn drops_the_places_of_removed_files() {
        let mut h = NavigationHistory::new();
        h.record(at("a", 0));
        h.record(at("gone", 0));
        h.record(at("a", 20));
        h.record(at("gone", 20));
        let current = h.back(Some(at("b", 0))).unwrap();
        assert_eq!(current, at("gone", 20));
        h.back(Some(at("gone", 20)));
        let mut asked = Vec::new();
        h.retain_existing(|path| {
            asked.push(path.to_owned());
            path != Path::new("gone")
        });
        assert_eq!(back(&h), [p("a", 0)]);
        assert_eq!(forward(&h), [p("b", 0)]);
        assert_eq!(asked.len(), 3, "each file is asked about once: {asked:?}");

        h.retain_existing(|_| false);
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
    }

    #[test]
    fn places_follow_renamed_and_deleted_files() {
        let mut h = NavigationHistory::new();
        h.record(at("src/a.py", 0));
        h.record(at("src/pkg/b.py", 10));
        h.record(at("srcx.py", 0));
        h.record(at("c.py", 0));
        h.back(Some(at("src/pkg/b.py", 30)));
        h.rename(Path::new("src/pkg"), Path::new("lib"));
        h.rename(Path::new("src/a.py"), Path::new("main.py"));
        let moved = Path::new("lib").join("b.py").display().to_string();
        assert_eq!(back(&h), [p("main.py", 0), p(&moved, 10), p("srcx.py", 0)]);
        assert_eq!(forward(&h), [p(&moved, 30)]);
        h.remove(Path::new("lib"));
        assert_eq!(back(&h), [p("main.py", 0), p("srcx.py", 0)]);
        assert!(!h.can_go_forward());
    }

    #[test]
    fn clear_forgets_everything() {
        let mut h = NavigationHistory::new();
        h.record(at("a", 0));
        h.back(Some(at("b", 0)));
        h.record(at("c", 0));
        h.back(Some(at("d", 0)));
        h.clear();
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
    }

    #[test]
    fn state_lists_the_places() {
        let mut h = NavigationHistory::new();
        h.record(Place::new("/p/a.py", text::Point::new(3, 4)));
        h.back(Some(at("/p/b.py", 7)));
        let state = h.state_json(|p| p.display().to_string());
        assert_eq!(
            state,
            json!({
                "back": [],
                "forward": [{ "path": "/p/b.py", "line": 7, "column": 0, "view_y": null }],
            })
        );
    }
}
