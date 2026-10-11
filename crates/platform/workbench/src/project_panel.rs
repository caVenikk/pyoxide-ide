//! The project tool window: the file tree of the open folder, virtualized,
//! loaded one directory at a time off the UI thread.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollStrategy, SharedString, Styled,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use project::{FileTree, Filter, NodeId, Project, list_dir};
use serde_json::json;
use themes::{Metric, UiRole};
use ui::icons::{Icon, IconName};
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
}

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
    filter: Filter,
    selected: Option<NodeId>,
    scroll: UniformListScrollHandle,
    file_types: Rc<FileTypes>,
    /// Directory listings in flight.
    loading: usize,
    /// A path to select once its folders are listed.
    reveal: Option<PathBuf>,
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
            filter: Filter::default(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            file_types,
            loading: 0,
            reveal: None,
        }
    }

    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// Directory listings still running.
    pub fn is_loading(&self) -> bool {
        self.loading > 0
    }

    /// Shows a folder; its root is listed in the background.
    pub fn set_project(&mut self, project: Project, cx: &mut Context<Self>) {
        self.filter = Filter::for_root(project.root());
        self.tree = Some(FileTree::new(project.root()));
        self.project = Some(project);
        self.selected = Some(FileTree::ROOT);
        self.list(FileTree::ROOT, cx);
        cx.notify();
    }

    fn list(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let path = tree.node(id).entry.path.clone();
        let filter = self.filter.clone();
        self.loading += 1;
        let task = cx.background_spawn(async move { list_dir(&path, &filter) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading -= 1;
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
            self.scroll.scroll_to_item(row, ScrollStrategy::Top);
        }
        cx.notify();
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

    /// Rows and selection as JSON (for `workbench.state`).
    pub fn state_json(&self) -> serde_json::Value {
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
        json!({
            "rows": tree.rows().iter().map(|&id| {
                let n = tree.node(id);
                json!({
                    "path": rel(id),
                    "depth": n.depth,
                    "dir": n.entry.is_dir(),
                    "expanded": n.expanded,
                    "excluded": n.entry.excluded,
                })
            }).collect::<Vec<_>>(),
            "selected": self.selected.map(rel),
            "loading": self.loading > 0,
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
        let text_color = if entry.excluded {
            theme.color(UiRole::GitIgnored)
        } else if selected && focused {
            theme.color(UiRole::SelectionText)
        } else {
            theme.color(UiRole::Text)
        };
        let icon = if entry.is_dir() {
            if node.depth == 0 {
                IconName::MODULE
            } else if entry.excluded {
                IconName::EXCLUDED_FOLDER
            } else {
                IconName::FOLDER
            }
        } else {
            self.file_types.icon(&entry.path)
        };
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
        div()
            .id(("row", ix))
            .relative()
            .w_full()
            .h(row_height)
            .overflow_hidden()
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
            }))
            .into_any_element()
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
        el.child(
            uniform_list(
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
            .track_scroll(self.scroll.clone())
            .flex_1()
            .overflow_hidden(),
        )
        .into_any_element()
    }
}
