//! The navigation history of the main window (Back and Forward, see
//! [`crate::navigation`]): the places that tab switches and jumps leave,
//! and going back and forward to them. As in the reference IDE, where
//! every change of the selected editor tab is a navigation
//! (`EditorTabbedContainer`), another file becoming the active editor's
//! records the place in the file left: opening a file, selecting a tab,
//! closing the active tab. A jump within the active editor records the
//! place it left when the caret left its line.

use std::path::PathBuf;

use gpui::{App, Context, Window};

use super::Workbench;
use crate::navigation::{Back, Forward, NavigationHistory, Place};

/// The navigation history of a window and what Back and Forward are doing.
#[derive(Debug, Default)]
pub(super) struct Navigation {
    pub history: NavigationHistory,
    /// Back or Forward is going to a place: tab switches record nothing.
    moving: bool,
    /// The file Back or Forward is opening: selecting its tab when it is
    /// read records nothing.
    opening: Option<PathBuf>,
}

impl Workbench {
    /// The place of the active editor: its file and caret.
    pub fn current_place(&self, cx: &App) -> Option<Place> {
        self.active.map(|i| self.tab_place(i, cx))
    }

    fn tab_place(&self, index: usize, cx: &App) -> Place {
        let tab = &self.tabs[index];
        Place::new(tab.path.clone(), tab.editor.read(cx).caret_point())
    }

    /// The window's navigation history.
    pub fn navigation_history(&self) -> &NavigationHistory {
        &self.navigation.history
    }

    /// Records `left`, the place a navigation left, for Back: a component
    /// that moves the active editor's caret as a navigation (a jump, not
    /// typing or caret keys) takes [`Self::current_place`] before the move
    /// and passes it here after it. It is recorded when the caret is now
    /// in another file or on another line. Opening files through
    /// [`Self::open_file`] or [`Self::open_file_at`] and switching tabs
    /// record by themselves.
    pub fn record_place(&mut self, left: Place, cx: &App) {
        if self.navigation.moving {
            return;
        }
        let moved = self
            .current_place(cx)
            .is_none_or(|now| now.path != left.path || now.caret.line != left.caret.line);
        if moved {
            self.navigation.history.record(left);
        }
    }

    /// The tab at `index` became active, `previous` was: the place in the
    /// previous one is recorded, unless Back or Forward is going there.
    pub(super) fn tab_activated(&mut self, previous: Option<usize>, index: usize, cx: &App) {
        // Any activation ends Back's or Forward's opening of a file.
        let opening = self.navigation.opening.take();
        let quiet =
            self.navigation.moving || opening.is_some_and(|path| path == self.tabs[index].path);
        if let Some(previous) = previous.filter(|p| *p != index)
            && !quiet
        {
            self.navigation.history.record(self.tab_place(previous, cx));
        }
    }

    /// The tab at `index` is about to close: when it is the active one and
    /// another tab takes its place, its place is recorded (Back opens the
    /// file again).
    pub(super) fn tab_closing(&mut self, index: usize, cx: &App) {
        if self.active == Some(index) && self.tabs.len() > 1 && !self.navigation.moving {
            self.navigation.history.record(self.tab_place(index, cx));
        }
    }

    /// Another project: the places of the old one are forgotten.
    pub(super) fn clear_navigation(&mut self) {
        self.navigation = Navigation::default();
    }

    /// Drops the places in files that are neither open nor on disk any
    /// more (asked once per file).
    fn drop_missing_places(&mut self) {
        let open: Vec<PathBuf> = self.tab_paths();
        self.navigation
            .history
            .retain_existing(|path| open.iter().any(|p| p == path) || path.is_file());
    }

    pub(super) fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_missing_places();
        let current = self.current_place(cx);
        if let Some(place) = self.navigation.history.back(current) {
            self.go_to_place(place, window, cx);
        }
    }

    pub(super) fn forward(&mut self, _: &Forward, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_missing_places();
        let current = self.current_place(cx);
        if let Some(place) = self.navigation.history.forward(current) {
            self.go_to_place(place, window, cx);
        }
    }

    /// Opens a place's file (or selects its tab) with the caret at the
    /// place, recording nothing.
    fn go_to_place(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        let Place { path, caret } = place;
        match self.tabs.iter().position(|t| t.path == path) {
            Some(i) => {
                self.navigation.moving = true;
                self.activate(i, window, cx);
                self.navigation.moving = false;
                self.tabs[i].editor.update(cx, |e, cx| {
                    e.go_to(caret.line + 1, Some(caret.column + 1), cx)
                });
            }
            None => {
                self.navigation.opening = Some(path.clone());
                self.pending_go_to = Some((path.clone(), caret));
                self.open_file(path, window, cx);
            }
        }
        cx.notify();
    }
}
