//! Go to Declaration or Usages and Show Usages, as the reference's
//! `GotoDeclarationAction` and `ShowUsagesAction` behave: the declaration
//! of the symbol at the caret (or at a Cmd/Ctrl+click) opens at its name;
//! several give a chooser; on the declaration itself the usages show in a
//! popup (rows of code with the file and line); a declaration in a file
//! outside the project (a library, a stub in a cache folder) opens
//! read-only; "Cannot find declaration to go to" when there is none. While
//! the navigation modifier is held over a symbol with a declaration, the
//! symbol is drawn as a link.
//!
//! The list popups of code intelligence (these and the intention actions)
//! are opened here too: one at a time, under the line they are about.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use code_intel::{DocumentId, Location, Point, Rope};
use editor::{Editor, EditorEvent};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, DismissEvent, Entity, EntityId, Modifiers,
    Subscription, Task, WeakEntity, Window,
};
use text::SelectionSet;

use crate::completion::{identifier_end, identifier_start, is_identifier_char};
use crate::document::point_to_offset;
use crate::hover::symbol_offset;
use crate::list_popup::{ListEvent, ListItem, ListOptions, ListPopup, ListRow};
use crate::popup::{CaretPopup, Placement};
use crate::view::{CodeIntelEvent, CodeIntelView};

/// The key context of the list popups of code intelligence (intention
/// actions, declarations, usages); one at a time.
pub const LIST_CONTEXT: &str = "CodeList";

/// The hint when the symbol has no declaration.
pub const NO_DECLARATION: &str = "Cannot find declaration to go to";
/// The hint when the symbol has no usages.
pub const NO_USAGES: &str = "No usages found";
/// The title of the chooser of several declarations.
pub const CHOOSE_DECLARATION: &str = "Choose Declaration";

/// The navigation state of a window.
#[derive(Default)]
pub(crate) struct NavState {
    /// The window of the editors (list popups take its focus).
    window: Option<AnyWindowHandle>,
    /// The request of Go to Declaration or Show Usages.
    request: Option<Task<()>>,
    /// The symbol drawn as a link (editor, range) and the request that
    /// checks the symbol under the mouse has a declaration.
    link: Option<(EntityId, Range<usize>)>,
    link_request: Option<Task<()>>,
    /// The list popup shown.
    list: Option<OpenList>,
    /// Files opened by navigation that open read-only (outside the
    /// project).
    read_only: HashSet<PathBuf>,
    /// The editors of the window's tabs (documents or not).
    tabs: HashMap<PathBuf, WeakEntity<Editor>>,
}

/// What the rows of the list popup do.
pub(crate) enum ListTargets {
    Locations(Vec<Location>),
    Intentions,
}

struct OpenList {
    popup: Entity<ListPopup>,
    targets: ListTargets,
    editor: Entity<Editor>,
    _subscriptions: Vec<Subscription>,
}

/// A line of code as a usage row shows it: its text without the leading
/// indentation, and the byte range in it of `range` (chars of the line).
pub fn usage_line(line: &str, range: Range<usize>) -> (String, Range<usize>) {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.len() - line.trim_start().len();
    let byte = |col: usize| line.char_indices().nth(col).map_or(line.len(), |(i, _)| i);
    let (start, end) = (byte(range.start), byte(range.end.max(range.start)));
    let text = line[indent..].to_owned();
    let start = start.saturating_sub(indent).min(text.len());
    let end = end.saturating_sub(indent).clamp(start, text.len());
    (text, start..end)
}

/// The rows of locations: the line of code with the symbol in bold, the
/// file and line at the right. `texts` holds the text of each file.
pub fn location_rows(locations: &[Location], texts: &HashMap<PathBuf, Rope>) -> Vec<ListItem> {
    locations
        .iter()
        .map(|location| {
            let name = location
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let line = location.range.start.line;
            let (text, bold) = texts
                .get(&location.path)
                .filter(|rope| line < rope.len_lines())
                .map(|rope| {
                    let end = if location.range.end.line == line {
                        location.range.end.column
                    } else {
                        location.range.start.column
                    };
                    usage_line(
                        &rope.line(line).to_string(),
                        location.range.start.column..end,
                    )
                })
                .unwrap_or_default();
            ListItem::Row(ListRow {
                text: text.into(),
                bold: vec![bold],
                detail: Some(format!("{name}:{}", line + 1).into()),
                code: true,
                ..ListRow::default()
            })
        })
        .collect()
}

impl CodeIntelView {
    /// Go to Declaration or Usages at the caret of an editor.
    pub(crate) fn go_to_declaration_at_caret(
        &mut self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        let offset = symbol_offset(editor.read(cx).buffer(), query.offset);
        self.go_to_declaration_at(editor, offset, cx);
    }

    /// Go to Declaration or Usages of the symbol at `offset` of an editor.
    pub(crate) fn go_to_declaration_at(
        &mut self,
        editor: &Entity<Editor>,
        offset: usize,
        cx: &mut Context<Self>,
    ) {
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let doc = document.id.clone();
        let at = document.versions.point(offset);
        let request = services.definition(&doc, at);
        let editor = editor.clone();
        self.nav.request = Some(cx.spawn(async move |this, cx| {
            let locations = request.await.unwrap_or_default();
            let _ = this.update(cx, |v, cx| {
                v.nav.request = None;
                v.declarations_answered(&editor, &doc, offset, at, locations, cx)
            });
        }));
    }

    fn declarations_answered(
        &mut self,
        editor: &Entity<Editor>,
        doc: &DocumentId,
        offset: usize,
        at: Point,
        locations: Vec<Location>,
        cx: &mut Context<Self>,
    ) {
        if locations.is_empty() {
            self.show_hint(editor, offset, NO_DECLARATION, cx);
            return;
        }
        // On the declaration itself: its usages.
        let on_declaration = locations
            .iter()
            .any(|l| l.path == doc.path() && l.range.start <= at && at <= l.range.end);
        if on_declaration {
            self.show_usages_at(editor, offset, cx);
            return;
        }
        match locations.as_slice() {
            [location] => self.navigate(editor, location.clone(), cx),
            _ => self.show_locations(
                editor,
                offset,
                locations,
                ListOptions {
                    title: Some(CHOOSE_DECLARATION.into()),
                    name: "declarations",
                    ..ListOptions::default()
                },
                cx,
            ),
        }
    }

    /// Show Usages at the caret of an editor.
    pub(crate) fn show_usages_at_caret(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        let offset = symbol_offset(editor.read(cx).buffer(), query.offset);
        self.show_usages_at(editor, offset, cx);
    }

    /// The usages of the symbol at `offset` in a popup.
    pub(crate) fn show_usages_at(
        &mut self,
        editor: &Entity<Editor>,
        offset: usize,
        cx: &mut Context<Self>,
    ) {
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let doc = document.id.clone();
        let at = document.versions.point(offset);
        let buffer = editor.read(cx).buffer();
        let name = buffer.slice(identifier_start(buffer, offset)..identifier_end(buffer, offset));
        let request = services.references(&doc, at, false);
        let editor = editor.clone();
        self.nav.request = Some(cx.spawn(async move |this, cx| {
            let mut locations = request.await.unwrap_or_default();
            locations.sort_by(|a, b| {
                (&a.path, a.range.start.line, a.range.start.column).cmp(&(
                    &b.path,
                    b.range.start.line,
                    b.range.start.column,
                ))
            });
            locations.dedup();
            let _ = this.update(cx, |v, cx| {
                v.nav.request = None;
                if locations.is_empty() {
                    v.show_hint(&editor, offset, NO_USAGES, cx);
                    return;
                }
                let count = locations.len();
                let options = ListOptions {
                    title: Some(format!("Usages of {name}").into()),
                    subtitle: Some(
                        format!("{count} usage{}", if count == 1 { "" } else { "s" }).into(),
                    ),
                    name: "usages",
                    ..ListOptions::default()
                };
                v.show_locations(&editor, offset, locations, options, cx);
            });
        }));
    }

    /// A popup of locations, with their lines of code: the texts of open
    /// documents, other files read off the UI thread.
    fn show_locations(
        &mut self,
        editor: &Entity<Editor>,
        offset: usize,
        locations: Vec<Location>,
        options: ListOptions,
        cx: &mut Context<Self>,
    ) {
        let mut texts: HashMap<PathBuf, Rope> = HashMap::new();
        for location in &locations {
            if let Some(document) = self.document(&location.path) {
                let rope = document.editor.read(cx).buffer().rope().clone();
                texts.insert(location.path.clone(), rope);
            }
        }
        let missing: HashSet<PathBuf> = locations
            .iter()
            .map(|l| l.path.clone())
            .filter(|p| !texts.contains_key(p))
            .collect();
        let read = cx.background_spawn(async move {
            for path in missing {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    texts.insert(path, Rope::from_str(&text.replace("\r\n", "\n")));
                }
            }
            texts
        });
        let editor = editor.clone();
        self.nav.request = Some(cx.spawn(async move |this, cx| {
            let texts = read.await;
            let _ = this.update(cx, |v, cx| {
                v.nav.request = None;
                let rows = location_rows(&locations, &texts);
                let targets = ListTargets::Locations(locations);
                v.open_list(&editor, offset, rows, options, targets, cx);
            });
        }));
    }

    /// Opens a list popup under the line of `offset` of an editor; it
    /// takes the focus. Called when an answer arrives (not while the
    /// window is being updated).
    pub(crate) fn open_list(
        &mut self,
        editor: &Entity<Editor>,
        offset: usize,
        items: Vec<ListItem>,
        options: ListOptions,
        targets: ListTargets,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return;
        }
        let Some(window) = self.nav.window else {
            return;
        };
        let popup = window.update(cx, |_, window: &mut Window, cx: &mut App| {
            cx.new(|cx| ListPopup::new(items, options, window, cx))
        });
        let Ok(popup) = popup else {
            return;
        };
        self.close_list(cx);
        let chosen = cx.subscribe(&popup, |this, _, event: &ListEvent, cx| match event {
            ListEvent::Chosen(path) => this.list_chosen(path.clone(), cx),
        });
        // A choice is reported right after the popup closes: the closed
        // popup is forgotten after it.
        let dismissed = cx.subscribe(&popup, |_, popup, _: &DismissEvent, cx| {
            let id = popup.entity_id();
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |v, cx| v.list_dismissed(id, cx));
            })
            .detach();
        });
        self.nav.list = Some(OpenList {
            popup: popup.clone(),
            targets,
            editor: editor.clone(),
            _subscriptions: vec![chosen, dismissed],
        });
        let height = popup.read(cx).height(cx);
        self.popups_mut().show(CaretPopup {
            context: LIST_CONTEXT,
            editor: editor.clone(),
            anchor: offset,
            placement: Placement::Below,
            inset: gpui::px(0.0),
            height: Some(height),
            content: popup.into(),
        });
        cx.emit(CodeIntelEvent::Redraw);
        cx.notify();
    }

    /// Closes the list popup.
    pub(crate) fn close_list(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.nav.list.take() {
            if let Some(window) = self.nav.window {
                let popup = list.popup.clone();
                let _ = window.update(cx, |_, window, cx| {
                    popup.update(cx, |p, cx| p.dismiss(window, cx))
                });
            }
            self.popups_mut().hide(LIST_CONTEXT);
            cx.notify();
        }
    }

    /// A row of the list popup was chosen.
    fn list_chosen(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        let Some(list) = self.nav.list.take() else {
            return;
        };
        self.popups_mut().hide(LIST_CONTEXT);
        cx.notify();
        match list.targets {
            ListTargets::Locations(locations) => {
                if let Some(location) = path.first().and_then(|&i| locations.get(i)) {
                    self.navigate(&list.editor, location.clone(), cx);
                }
            }
            ListTargets::Intentions => self.intention_chosen(&list.editor, &path, cx),
        }
    }

    /// The list popup closed without a choice.
    fn list_dismissed(&mut self, popup: EntityId, cx: &mut Context<Self>) {
        let ours = self
            .nav
            .list
            .as_ref()
            .is_some_and(|l| l.popup.entity_id() == popup);
        if ours {
            self.nav.list = None;
            self.popups_mut().hide(LIST_CONTEXT);
            cx.notify();
        }
    }

    /// The list popup's state for `code.state`.
    pub(crate) fn list_state_json(&self, cx: &App) -> serde_json::Value {
        self.nav
            .list
            .as_ref()
            .filter(|_| self.popups().is_shown(LIST_CONTEXT))
            .map(|l| l.popup.read(cx).state_json())
            .unwrap_or_default()
    }

    /// Opens a navigation target: in the editor it was asked from, the
    /// caret goes to it; another file opens in a tab with the caret there
    /// (read-only when it is outside the project).
    pub(crate) fn navigate(
        &mut self,
        editor: &Entity<Editor>,
        location: Location,
        cx: &mut Context<Self>,
    ) {
        if let Some(document) = self.document(&location.path)
            && document.editor == *editor
        {
            let offset = point_to_offset(document.versions.text(), location.range.start);
            editor.update(cx, |e, cx| {
                let from = e.selections().primary().head;
                e.set_selections(SelectionSet::caret(offset), cx);
                e.reveal(offset, cx);
                // A navigation (the reference's `navigateRequest`).
                e.jumped_from(from, cx);
            });
            return;
        }
        let inside = self
            .project_root()
            .is_some_and(|root| location.path.starts_with(root));
        if !inside {
            self.nav.read_only.insert(location.path.clone());
        }
        cx.emit(CodeIntelEvent::OpenLocation {
            path: location.path,
            at: location.range.start,
        });
    }

    /// An editor tab was opened (any file): navigation targets outside
    /// the project open read-only.
    pub(crate) fn tab_opened(
        &mut self,
        path: &Path,
        editor: &Entity<Editor>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.nav.window = Some(window.window_handle());
        self.nav.tabs.insert(path.to_owned(), editor.downgrade());
        if self.nav.read_only.contains(path) {
            editor.update(cx, |e, cx| {
                e.buffer_mut().set_read_only(true);
                cx.notify();
            });
        }
    }

    /// The editor of an open tab of a file (a document or not).
    pub(crate) fn tab_editor(&self, path: &Path) -> Option<Entity<Editor>> {
        self.nav.tabs.get(path).and_then(WeakEntity::upgrade)
    }

    /// Events of a document's editor: a click with the navigation modifier
    /// goes to the declaration; the mouse over a symbol with the modifier
    /// held draws it as a link when it has a declaration.
    pub(crate) fn nav_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::ModifiedClick { offset, modifiers } if modifiers.secondary() => {
                self.clear_link(cx);
                let buffer = editor.read(cx).buffer();
                if buffer.char_at(*offset).is_some_and(is_identifier_char) {
                    self.go_to_declaration_at(editor, *offset, cx);
                }
            }
            EditorEvent::Hovered {
                offset, modifiers, ..
            } => self.link_hovered(editor, *offset, *modifiers, cx),
            EditorEvent::Edited(_) | EditorEvent::Reloaded => self.clear_link(cx),
            _ => {}
        }
    }

    fn link_hovered(
        &mut self,
        editor: &Entity<Editor>,
        offset: Option<usize>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let buffer = editor.read(cx).buffer();
        let word = offset
            .filter(|_| modifiers.secondary())
            .filter(|&o| buffer.char_at(o).is_some_and(is_identifier_char))
            .map(|o| identifier_start(buffer, o)..identifier_end(buffer, o));
        let Some(word) = word else {
            self.clear_link(cx);
            return;
        };
        let id = editor.entity_id();
        if self.nav.link.as_ref() == Some(&(id, word.clone())) {
            return;
        }
        self.clear_link(cx);
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let request = services.definition(&document.id, document.versions.point(word.start));
        let editor = editor.clone();
        self.nav.link_request = Some(cx.spawn(async move |this, cx| {
            let found = request.await.is_ok_and(|l| !l.is_empty());
            let _ = this.update(cx, |v, cx| {
                v.nav.link_request = None;
                if found {
                    v.nav.link = Some((editor.entity_id(), word.clone()));
                    editor.update(cx, |e, cx| e.set_link_highlight(Some(word), cx));
                }
            });
        }));
    }

    /// Removes the link from the symbol under the mouse.
    pub(crate) fn clear_link(&mut self, cx: &mut Context<Self>) {
        self.nav.link_request = None;
        let Some((id, _)) = self.nav.link.take() else {
            return;
        };
        if let Some(editor) = self.editor_by_id(id) {
            editor.update(cx, |e, cx| e.set_link_highlight(None, cx));
        }
    }

    /// Forgets the requests of an editor that went away or changed.
    pub(crate) fn reset_navigation(&mut self, cx: &mut Context<Self>) {
        self.nav.request = None;
        self.clear_link(cx);
        self.close_list(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_lines_lose_their_indentation() {
        let (text, bold) = usage_line("    return norm(p)\n", 11..15);
        assert_eq!(text, "return norm(p)");
        assert_eq!(&text[bold], "norm");
        let (text, bold) = usage_line("x = ñame", 4..8);
        assert_eq!(&text[bold], "ñame");
        let (_, bold) = usage_line("ab", 5..9);
        assert_eq!(bold, 2..2);
    }

    #[test]
    fn rows_of_locations() {
        let path = PathBuf::from("/p/app.py");
        let locations = vec![Location {
            path: path.clone(),
            range: Point::new(1, 4)..Point::new(1, 8),
        }];
        let texts = HashMap::from([(path, Rope::from_str("import os\nx = norm(1)\n"))]);
        let rows = location_rows(&locations, &texts);
        let ListItem::Row(row) = &rows[0] else {
            panic!("a row");
        };
        assert_eq!(row.text.as_ref(), "x = norm(1)");
        assert_eq!(&row.text[row.bold[0].clone()], "norm");
        assert_eq!(row.detail.as_ref().unwrap().as_ref(), "app.py:2");
    }
}
