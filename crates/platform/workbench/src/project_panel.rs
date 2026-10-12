//! The project tool window: the file tree of the open folder, virtualized,
//! loaded one directory at a time off the UI thread. Opening a folder first
//! loads its hide and exclude rules in the background (importing the
//! reference IDE's exclusions once, ADR 0008); directories are listed when
//! the rules are there.
//!
//! The context menu of a row has *Mark Directory As* (*Mark File As* for a
//! file) with *Excluded* and *Cancel Exclusion*; the project settings are
//! written in the background after every change.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, Render,
    ScrollStrategy, SharedString, Styled, Subscription, UniformListScrollHandle, Window, canvas,
    div, point, prelude::*, px, uniform_list,
};
use project::{FileTree, Filter, Node, NodeId, Project, list_dir};
use serde_json::json;
use themes::{Metric, UiRole};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem};
use ui::theme::ThemeExt;

use crate::file_types::FileTypes;

ui::ui_commands! {
    COMMANDS, scope = "ProjectTree";
    SelectPrevious => "project.tree.up", "Up", "Selects the row above.";
    SelectNext => "project.tree.down", "Down", "Selects the row below.";
    ExpandOrNext => "project.tree.expand", "Expand",
        "Expands the selected folder, or selects its first child when it is open.";
    CollapseOrParent => "project.tree.collapse", "Collapse",
        "Collapses the selected folder, or selects its parent.";
    OpenSelected => "project.tree.open", "Open",
        "Opens the selected file, or expands or collapses the selected folder.";
    CollapseAll => "project.tree.collapse-all", "Collapse All", "Collapses every folder.";
    ExpandAll => "project.tree.expand-all", "Expand All",
        "Expands the selected folder and the folders below it that are listed.";
    Reveal { path: String } => "project.tree.reveal", "Select in Project",
        "Expands the folders above a path and selects it.", r#"{"path": "src/app.py"}"#;
    ShowContextMenu => "project.tree.context-menu", "Show Context Menu",
        "Opens the context menu of the selected row.";
    MarkExcluded { path: Option<String> } => "project.tree.mark-excluded", "Excluded",
        "Mark Directory As > Excluded: leaves the selected folder or file (or `path`, relative to the project) out of search, Go to File and indexing, and stores that in the project settings.",
        r#"{"path": "docs"}"#;
    CancelExclusion { path: Option<String> } => "project.tree.cancel-exclusion", "Cancel Exclusion",
        "Mark Directory As > Cancel Exclusion: makes the selected excluded folder or file (or `path`) ordinary project content again.",
        r#"{"path": "docs"}"#;
}

/// Title of the submenu with the exclusion commands, for a folder and for
/// a file.
const MARK_DIRECTORY_AS: &str = "Mark Directory As";
const MARK_FILE_AS: &str = "Mark File As";

/// Space left of the first chevron, inside the island.
const TREE_LEFT: f32 = 12.0;
/// Chevron (or the room for it) to the file icon.
const ICON_X: f32 = 21.0;
/// File icon to the name.
const TEXT_X: f32 = 20.0;
/// Space above the first row.
const TREE_TOP: f32 = 4.0;

#[derive(Debug, Clone)]
pub enum PanelEvent {
    /// The person asked to open a file.
    OpenFile(PathBuf),
}

pub struct ProjectPanel {
    focus: FocusHandle,
    project: Option<Project>,
    tree: Option<FileTree>,
    /// Hide and exclude rules of the project; `None` while they load.
    filter: Option<Filter>,
    /// Directories to list once the rules are loaded.
    waiting: Vec<NodeId>,
    /// Changes with every opened project, so late results of the previous
    /// one are dropped.
    generation: u64,
    selected: Option<NodeId>,
    scroll: UniformListScrollHandle,
    file_types: Rc<FileTypes>,
    /// Directory listings in flight.
    loading: usize,
    /// A path to select once its folders are listed.
    reveal: Option<PathBuf>,
    /// The open context menu and where it opened (window coordinates).
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    /// A save of the project settings is running; `resave` asks for one
    /// more with the latest exclusions.
    saving: bool,
    resave: bool,
    /// Why the project settings could not be written, after a failed save.
    settings_error: Option<String>,
    /// Where the rows were drawn (window coordinates), to place a context
    /// menu opened from the keyboard.
    rows_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl EventEmitter<PanelEvent> for ProjectPanel {}

impl Focusable for ProjectPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ProjectPanel {
    pub fn new(file_types: Rc<FileTypes>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            project: None,
            tree: None,
            filter: None,
            waiting: Vec::new(),
            generation: 0,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            file_types,
            loading: 0,
            reveal: None,
            context_menu: None,
            saving: false,
            resave: false,
            settings_error: None,
            rows_bounds: Rc::default(),
        }
    }

    /// The open context menu, if any.
    pub fn context_menu(&self) -> Option<&Entity<ContextMenu>> {
        self.context_menu.as_ref().map(|(menu, _, _)| menu)
    }

    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// Directory listings still running.
    pub fn is_loading(&self) -> bool {
        self.loading > 0
    }

    /// The selected row's path and whether it is a folder.
    pub fn selected_path(&self) -> Option<(&Path, bool)> {
        let tree = self.tree.as_ref()?;
        let entry = &tree.node(self.selected?).entry;
        Some((entry.path.as_path(), entry.is_dir()))
    }

    /// The hide and exclude rules of the open project, once loaded. They
    /// are replaced (a cheap clone) whenever the person marks or unmarks a
    /// folder, so a search that takes them at its start sees every change
    /// of the session.
    pub fn filter(&self) -> Option<&Filter> {
        self.filter.as_ref()
    }

    /// Shows a folder. Its rules are loaded (and the reference IDE's
    /// exclusions imported, once) in the background, then its root is
    /// listed.
    pub fn set_project(&mut self, project: Project, cx: &mut Context<Self>) {
        let root = project.root().to_owned();
        self.generation += 1;
        let generation = self.generation;
        self.filter = None;
        self.waiting.clear();
        self.context_menu = None;
        self.settings_error = None;
        self.tree = Some(FileTree::new(&root));
        self.project = Some(project);
        self.selected = Some(FileTree::ROOT);
        self.loading += 1;
        let task = cx.background_spawn(async move {
            // A failed import leaves the project as it was; the rules
            // still read `.idea` until the project has its own folder.
            let _ = project::store::import_idea(&root);
            Filter::for_root(&root)
        });
        cx.spawn(async move |this, cx| {
            let filter = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                this.filter = Some(filter);
                for id in std::mem::take(&mut this.waiting) {
                    this.list(id, cx);
                }
                cx.notify();
            });
        })
        .detach();
        self.list(FileTree::ROOT, cx);
        cx.notify();
    }

    fn list(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let Some(filter) = self.filter.clone() else {
            if !self.waiting.contains(&id) {
                self.waiting.push(id);
            }
            return;
        };
        let path = tree.node(id).entry.path.clone();
        let generation = self.generation;
        self.loading += 1;
        let task = cx.background_spawn(async move { list_dir(&path, &filter) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                if let Some(tree) = &mut this.tree {
                    match result {
                        Ok(entries) => tree.set_children(id, entries),
                        Err(e) => tree.set_error(id, e.to_string()),
                    }
                }
                this.continue_reveal(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn expand(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(tree) = &mut self.tree else { return };
        if tree.expand(id) {
            self.list(id, cx);
        }
        cx.notify();
    }

    fn toggle(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(tree) = &mut self.tree else { return };
        if tree.node(id).expanded {
            tree.collapse(id);
            cx.notify();
        } else {
            self.expand(id, cx);
        }
    }

    fn select(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.selected = Some(id);
        if let Some(row) = self.tree.as_ref().and_then(|t| t.row_of(id)) {
            self.scroll_to_row(row, cx);
        }
        cx.notify();
    }

    /// Scrolls as little as shows `row` whole: a row above the view comes
    /// in at the top, a row below it at the bottom, a visible row stays.
    fn scroll_to_row(&self, row: usize, cx: &App) {
        let row_top = cx.theme().metric(Metric::RowHeight) * row as f32;
        let strategy = if row_top < self.scroll_top() {
            ScrollStrategy::Top
        } else {
            ScrollStrategy::Bottom
        };
        self.scroll.scroll_to_item(row, strategy);
    }

    /// How far the rows are scrolled up, in pixels.
    fn scroll_top(&self) -> Pixels {
        -self.scroll.0.borrow().base_handle.offset().y
    }

    /// Opens a file row; expands or collapses a folder row.
    fn activate(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let entry = &tree.node(id).entry;
        if entry.is_dir() {
            self.toggle(id, cx);
        } else {
            cx.emit(PanelEvent::OpenFile(entry.path.clone()));
        }
    }

    /// Selects `path`, listing the folders above it as needed.
    pub fn reveal(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.reveal = Some(path.to_owned());
        self.continue_reveal(cx);
    }

    fn continue_reveal(&mut self, cx: &mut Context<Self>) {
        let (Some(target), Some(tree)) = (self.reveal.clone(), &mut self.tree) else {
            return;
        };
        let root = tree.node(FileTree::ROOT).entry.path.clone();
        let Ok(relative) = target.strip_prefix(&root) else {
            self.reveal = None;
            return;
        };
        let mut current = FileTree::ROOT;
        let mut path = root;
        for part in relative.components() {
            if tree.needs_listing(current) {
                if tree.expand(current) || self.loading == 0 {
                    self.list(current, cx);
                }
                return;
            }
            tree.expand(current);
            path.push(part);
            match tree.find(&path) {
                Some(id) => current = id,
                None => {
                    self.reveal = None;
                    return;
                }
            }
        }
        self.reveal = None;
        self.select(current, cx);
    }

    /// Rows, selection and scroll position as JSON (for `workbench.state`).
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let Some(tree) = &self.tree else {
            return serde_json::Value::Null;
        };
        let root = tree.node(FileTree::ROOT).entry.path.clone();
        let rel = |id: NodeId| {
            let p = &tree.node(id).entry.path;
            p.strip_prefix(&root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default()
        };
        let scroll_top = self.scroll_top();
        let first_visible = (scroll_top / cx.theme().metric(Metric::RowHeight)).floor() as usize;
        json!({
            "rows": tree.rows().iter().map(|&id| {
                let n = tree.node(id);
                json!({
                    "path": rel(id),
                    "depth": n.depth,
                    "dir": n.entry.is_dir(),
                    "expanded": n.expanded,
                    "excluded": n.entry.excluded,
                    "icon": self.icon(n).0,
                    "background": n.entry.excluded.then(|| UiRole::NonProjectFile.name()),
                })
            }).collect::<Vec<_>>(),
            "selected": self.selected.map(rel),
            "loading": self.loading > 0,
            "scroll_top": f32::from(scroll_top),
            "first_visible_row": first_visible,
            "context_menu": self.context_menu().map(|m| m.read(cx).state_json()),
            "exclusions": self.filter.as_ref().map(|f| &f.exclusions().paths),
            "settings_error": self.settings_error,
        })
    }

    // ---- commands ---------------------------------------------------------

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let rows = tree.rows();
        let current = self.selected.and_then(|s| tree.row_of(s)).unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, rows.len() as isize - 1) as usize;
        let id = rows[next];
        self.select(id, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn expand_or_next(&mut self, _: &ExpandOrNext, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(tree), Some(id)) = (&self.tree, self.selected) else {
            return;
        };
        let node = tree.node(id);
        if !node.entry.is_dir() {
            return;
        }
        if node.expanded {
            if let Some(&first) = node.children.as_ref().and_then(|c| c.first()) {
                self.select(first, cx);
            }
        } else {
            self.expand(id, cx);
        }
    }

    fn collapse_or_parent(&mut self, _: &CollapseOrParent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(tree), Some(id)) = (&mut self.tree, self.selected) else {
            return;
        };
        let node = tree.node(id);
        if node.entry.is_dir() && node.expanded && id != FileTree::ROOT {
            tree.collapse(id);
            cx.notify();
        } else if let Some(parent) = node.parent {
            self.select(parent, cx);
        }
    }

    fn open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected {
            self.activate(id, cx);
        }
    }

    fn collapse_all(&mut self, _: &CollapseAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(tree) = &mut self.tree {
            tree.collapse_all();
            self.selected = Some(FileTree::ROOT);
            cx.notify();
        }
    }

    fn expand_all(&mut self, _: &ExpandAll, _: &mut Window, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let start = self
            .selected
            .filter(|&s| tree.node(s).entry.is_dir())
            .unwrap_or(FileTree::ROOT);
        // Folders that are listed already, below the start; excluded ones
        // stay closed.
        let mut stack = vec![start];
        let mut to_expand = Vec::new();
        while let Some(id) = stack.pop() {
            let node = tree.node(id);
            if !node.entry.is_dir() || (node.entry.excluded && id != start) {
                continue;
            }
            to_expand.push(id);
            if let Some(children) = &node.children {
                stack.extend(children.iter().copied());
            }
        }
        for id in to_expand {
            self.expand(id, cx);
        }
    }

    fn reveal_action(&mut self, a: &Reveal, _: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = &self.project else { return };
        let path = Path::new(&a.path);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            project.root().join(path)
        };
        self.reveal(&path, cx);
    }

    // ---- exclusions and the context menu ------------------------------------

    /// The path a command names (relative to the project, or absolute), or
    /// the selected row's, and whether it is a folder.
    fn target(&self, path: Option<&str>) -> Option<(PathBuf, bool)> {
        let tree = self.tree.as_ref()?;
        let path = match path {
            Some(p) => {
                let p = Path::new(p);
                if p.is_absolute() {
                    p.to_owned()
                } else {
                    self.project.as_ref()?.root().join(p)
                }
            }
            None => tree.node(self.selected?).entry.path.clone(),
        };
        let is_dir = match tree.find(&path) {
            Some(id) => tree.node(id).entry.is_dir(),
            None => path.is_dir(),
        };
        Some((path, is_dir))
    }

    fn mark_excluded(&mut self, a: &MarkExcluded, _: &mut Window, cx: &mut Context<Self>) {
        let (Some((path, is_dir)), Some(filter)) = (self.target(a.path.as_deref()), &self.filter)
        else {
            return;
        };
        // Only project content can be excluded: not the root, not what is
        // excluded already.
        let Some(rel) = filter.relative(&path) else {
            return;
        };
        if filter.is_excluded(&path, is_dir) {
            return;
        }
        let mut exclusions = filter.exclusions().clone();
        exclusions.add(&rel);
        self.set_exclusions(exclusions, cx);
    }

    fn cancel_exclusion(&mut self, a: &CancelExclusion, _: &mut Window, cx: &mut Context<Self>) {
        let (Some((path, _)), Some(filter)) = (self.target(a.path.as_deref()), &self.filter) else {
            return;
        };
        // Only a stored exclusion can be cancelled; a product's automatic
        // ones (a virtual environment) cannot, as in the reference IDE,
        // which excludes them again on its own.
        let Some(rel) = filter.relative(&path) else {
            return;
        };
        if !filter.is_excluded_root(&path) {
            return;
        }
        let mut exclusions = filter.exclusions().clone();
        exclusions.remove(&rel);
        self.set_exclusions(exclusions, cx);
    }

    /// Applies new exclusions to the rules and the listed rows, then
    /// writes the project settings in the background.
    fn set_exclusions(&mut self, exclusions: project::Exclusions, cx: &mut Context<Self>) {
        let Some(filter) = &self.filter else { return };
        let filter = filter.with_exclusions(exclusions);
        if let Some(tree) = &mut self.tree {
            tree.refresh_excluded(&filter);
        }
        self.filter = Some(filter);
        self.save_settings(cx);
        cx.notify();
    }

    /// Writes the exclusions into the project settings, one save at a time.
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            self.resave = true;
            return;
        }
        let (Some(filter), Some(project)) = (&self.filter, &self.project) else {
            return;
        };
        let root = project.root().to_owned();
        let exclusions = filter.exclusions().clone();
        let generation = self.generation;
        self.saving = true;
        self.loading += 1;
        let task = cx.background_spawn(async move { project::store::save(&root, &exclusions) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading -= 1;
                this.saving = false;
                let current = this.generation == generation;
                if current {
                    this.settings_error = result.err().map(|e| e.to_string());
                }
                if std::mem::take(&mut this.resave) && current {
                    this.save_settings(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The rows of the context menu of `id`: *Mark Directory As* (or *Mark
    /// File As*) with *Excluded* for project content or *Cancel Exclusion*
    /// for a stored exclusion. The project root, rows inside an excluded
    /// folder and folders a product excludes have nothing to mark.
    fn context_menu_items(&self, id: NodeId) -> Vec<MenuItem> {
        let (Some(tree), Some(filter)) = (&self.tree, &self.filter) else {
            return Vec::new();
        };
        let entry = &tree.node(id).entry;
        let Some(rel) = filter.relative(&entry.path) else {
            return Vec::new();
        };
        let mark = if filter.is_excluded_root(&entry.path) {
            MenuItem::action("Cancel Exclusion", CancelExclusion { path: Some(rel) })
        } else if !entry.excluded {
            MenuItem::action("Excluded", MarkExcluded { path: Some(rel) })
                .icon(IconName::EXCLUDED_FOLDER)
        } else {
            return Vec::new();
        };
        let title = if entry.is_dir() {
            MARK_DIRECTORY_AS
        } else {
            MARK_FILE_AS
        };
        vec![MenuItem::submenu(title, vec![mark])]
    }

    /// Opens the context menu of `id` at `position` (window coordinates).
    fn open_context_menu(
        &mut self,
        id: NodeId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self.context_menu_items(id);
        if items.is_empty() {
            return;
        }
        window.focus(&self.focus);
        let menu = ContextMenu::build(items, window, cx);
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.context_menu = None;
            cx.notify();
        });
        self.context_menu = Some((menu, position, dismissed));
        cx.notify();
    }

    fn show_context_menu(
        &mut self,
        _: &ShowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(tree), Some(id)) = (&self.tree, self.selected) else {
            return;
        };
        let Some(row) = tree.row_of(id) else { return };
        // Below the selected row, where its name starts.
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let indent = theme.metric(Metric::TreeIndent) * tree.node(id).depth as f32;
        let x = px(TREE_LEFT + ICON_X + TEXT_X) + indent;
        let y = row_height * (row + 1) as f32 - self.scroll_top();
        let origin = self.rows_bounds.get().origin;
        self.open_context_menu(id, origin + point(x, y), window, cx);
    }

    /// Selects the row under a right click and opens its context menu.
    fn context_click(
        &mut self,
        id: NodeId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        self.selected = Some(id);
        self.open_context_menu(id, position, window, cx);
        cx.notify();
    }

    /// The icon of a row: the project root, an excluded folder (also one
    /// inside an excluded folder, as in the reference IDE), a folder, or the
    /// file type's.
    fn icon(&self, node: &Node) -> IconName {
        let entry = &node.entry;
        if !entry.is_dir() {
            self.file_types.icon(&entry.path)
        } else if node.depth == 0 {
            IconName::MODULE
        } else if entry.excluded {
            IconName::EXCLUDED_FOLDER
        } else {
            IconName::FOLDER
        }
    }

    // ---- drawing ----------------------------------------------------------

    fn row(
        &self,
        id: NodeId,
        ix: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tree = self.tree.as_ref().expect("rows exist only with a tree");
        let theme = cx.theme();
        let node = tree.node(id);
        let entry = &node.entry;
        let row_height = theme.metric(Metric::RowHeight);
        let indent = theme.metric(Metric::TreeIndent);
        let selected = self.selected == Some(id);
        let left = px(TREE_LEFT) + indent * node.depth as f32;
        let selection = if focused {
            theme.color(UiRole::Selection)
        } else {
            theme.color(UiRole::SelectionInactive)
        };
        let hover = theme.color(UiRole::Hover);
        // Excluded rows keep the normal text colour and get the background
        // of non-project files across the whole row.
        let text_color = if selected && focused {
            theme.color(UiRole::SelectionText)
        } else {
            theme.color(UiRole::Text)
        };
        let non_project = entry.excluded.then(|| theme.color(UiRole::NonProjectFile));
        let icon = self.icon(node);
        let chevron = entry.is_dir().then(|| {
            let name = if node.expanded {
                IconName::CHEVRON_DOWN
            } else {
                IconName::CHEVRON_RIGHT
            };
            div()
                .id(("chevron", ix))
                .absolute()
                .left(left)
                .top((row_height - px(16.0)) / 2.0)
                .child(Icon::new(name))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.selected = Some(id);
                    this.toggle(id, cx);
                }))
        });
        let mut label = div()
            .flex()
            .flex_row()
            .gap(px(6.0))
            .whitespace_nowrap()
            .text_color(text_color)
            .child(SharedString::from(entry.name.clone()));
        if node.depth == 0 {
            label = label.font_weight(FontWeight::SEMIBOLD);
            if let Some(project) = &self.project {
                label = label.child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(theme.color(UiRole::TextMuted))
                        .child(project.display_root()),
                );
            }
        }
        let row = div()
            .id(("row", ix))
            // Tests find a row by its path (no cost in release builds).
            .debug_selector(|| {
                let rel = self.filter.as_ref().and_then(|f| f.relative(&entry.path));
                format!("tree-row:{}", rel.unwrap_or_default())
            })
            .relative()
            .w_full()
            .h(row_height)
            .overflow_hidden()
            .when_some(non_project, |d, bg| d.bg(bg))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(TREE_LEFT))
                    .right(px(TREE_LEFT))
                    .rounded(theme.metric(Metric::CornerRadius))
                    .when(selected, |d| d.bg(selection))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover))),
            )
            .children(chevron)
            .child(
                div()
                    .absolute()
                    .left(left + px(ICON_X))
                    .top((row_height - px(16.0)) / 2.0)
                    .child(Icon::new(icon)),
            )
            .child(
                div()
                    .absolute()
                    .left(left + px(ICON_X + TEXT_X))
                    .top_0()
                    .h(row_height)
                    .flex()
                    .items_center()
                    .child(label),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                window.focus(&this.focus);
                this.selected = Some(id);
                if e.click_count() >= 2 {
                    this.activate(id, cx);
                }
                cx.notify();
            }));
        // The context menu opens on the right button: when it goes down on
        // macOS and Linux, when it comes up on Windows (the platforms'
        // popup triggers).
        #[cfg(not(windows))]
        let row = row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.context_click(id, e.position, window, cx);
            }),
        );
        #[cfg(windows)]
        let row = row.on_mouse_up(
            MouseButton::Right,
            cx.listener(move |this, e: &gpui::MouseUpEvent, window, cx| {
                cx.stop_propagation();
                this.context_click(id, e.position, window, cx);
            }),
        );
        row.into_any_element()
    }
}

macro_rules! panel_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl ProjectPanel {
            fn bind_actions<E: InteractiveElement>(el: E, cx: &mut Context<Self>) -> E {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }

        /// Forwards the tree commands to `panel` (used by the workbench when
        /// the tree is not focused).
        pub fn forward_actions<E: InteractiveElement>(el: E, panel: Entity<ProjectPanel>) -> E {
            el $(.on_action({
                let panel = panel.clone();
                move |a: &$action, window, cx| panel.update(cx, |p, cx| p.$method(a, window, cx))
            }))*
        }
    };
}

panel_actions! {
    SelectPrevious => select_previous,
    SelectNext => select_next,
    ExpandOrNext => expand_or_next,
    CollapseOrParent => collapse_or_parent,
    OpenSelected => open_selected,
    CollapseAll => collapse_all,
    ExpandAll => expand_all,
    Reveal => reveal_action,
    ShowContextMenu => show_context_menu,
    MarkExcluded => mark_excluded,
    CancelExclusion => cancel_exclusion,
}

impl Render for ProjectPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.contains_focused(window, cx);
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let count = self.tree.as_ref().map_or(0, |t| t.rows().len());
        let el = div()
            .id("project-tree")
            .key_context("ProjectTree")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .pt(px(TREE_TOP));
        let el = Self::bind_actions(el, cx);
        if self.tree.is_none() {
            return el
                .items_center()
                .justify_center()
                .text_color(muted)
                .child("No project is open")
                .into_any_element();
        }
        let mut rows = uniform_list(
            "project-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let rows: Vec<NodeId> = this
                    .tree
                    .as_ref()
                    .map(|t| t.rows()[range.clone()].to_vec())
                    .unwrap_or_default();
                rows.into_iter()
                    .zip(range)
                    .map(|(id, ix)| this.row(id, ix, focused, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "project-rows".into())
        .track_scroll(self.scroll.clone())
        .flex_1()
        // The list scrolls vertically (its own `overflow-y: scroll`, which
        // the wheel and trackpad need); names wider than the tool window
        // are cut off.
        .overflow_x_hidden();
        // A sideways swipe does not scroll the rows up or down.
        rows.style().restrict_scroll_to_axis = Some(true);
        let bounds = self.rows_bounds.clone();
        let menu = self
            .context_menu
            .as_ref()
            .map(|(menu, position, _)| ui::menu::overlay(menu, *position, window));
        el.child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(rows)
                .child(
                    canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                ),
        )
        .children(menu)
        .into_any_element()
    }
}
