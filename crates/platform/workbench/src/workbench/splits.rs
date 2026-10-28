//! The editor area's split windows: tab groups (the reference's
//! `EditorWindow`s) in a tree of splitters (`EditorsSplitters`). Every tab
//! belongs to a group; `Workbench::tabs` holds the tabs of all groups, a
//! group's tabs in the order of that list, and `Workbench::active` is the
//! selected tab of the focused group. The commands are the reference's
//! `SplitVertically` (Split Right), `SplitHorizontally` (Split Down),
//! `MoveTabRight` and `MoveTabDown` (Split and Move), `NextSplitter` and
//! `PrevSplitter`.
//!
//! A file shown in two groups has one tab per group with an editor each
//! (the carets and the scroll positions are separate). The first editor of
//! a file is the one the language services, the file watcher and the diff
//! know; the others are mirrors that copy every edit of the text to the
//! other editors of the file (`sync_views`) and take over the role when
//! the first one closes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, Entity, EntityId, HitboxBehavior, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, ScrollHandle,
    StatefulInteractiveElement, Styled, Window, canvas, div, prelude::*, px, relative,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::menu::MenuItem;
use ui::theme::ThemeExt;

use super::{Tab, TabContent, Workbench};
use crate::commands::*;
use crate::split_tree::{GroupId, Node, Path, clamp_proportion};

/// Width of a splitter's line and of the strip around it that takes the
/// mouse.
const SPLITTER_LINE: f32 = 1.0;
const SPLITTER_GRAB: f32 = 7.0;

/// The tab strip's scroll position of a group, and the selected tab and
/// tab count it was last brought to show (a change of either scrolls the
/// strip just enough to show the whole selected tab).
#[derive(Clone, Default)]
struct StripScroll {
    handle: ScrollHandle,
    shown: Rc<Cell<Option<(usize, usize)>>>,
}

/// A splitter being dragged: its path in the tree.
#[derive(Debug, Clone)]
pub(super) struct SplitDrag {
    path: Path,
}

/// The split windows of the editor area.
pub(super) struct Splits {
    /// The groups' layout; one leaf while nothing is split.
    pub root: Node,
    /// The group that has the focus.
    pub focused: GroupId,
    next: GroupId,
    /// The selected tab of each group (by its editor or view).
    selected: HashMap<GroupId, EntityId>,
    scroll: RefCell<HashMap<GroupId, StripScroll>>,
    /// Where each splitter was drawn last, for dragging it.
    bounds: Rc<RefCell<HashMap<Path, Bounds<Pixels>>>>,
    drag: Option<SplitDrag>,
    /// Editors that just got an edit copied from another view of their
    /// file: the version after it, so the event of that edit is not copied
    /// back.
    echo: HashMap<EntityId, u64>,
}

impl Default for Splits {
    fn default() -> Self {
        Self {
            root: Node::Leaf(0),
            focused: 0,
            next: 1,
            selected: HashMap::new(),
            scroll: RefCell::default(),
            bounds: Rc::default(),
            drag: None,
            echo: HashMap::new(),
        }
    }
}

impl Splits {
    fn strip(&self, group: GroupId) -> StripScroll {
        self.scroll.borrow_mut().entry(group).or_default().clone()
    }

    /// A group id nobody used yet.
    fn new_group(&mut self) -> GroupId {
        let id = self.next;
        self.next += 1;
        id
    }

    /// Nothing is split.
    pub fn is_single(&self) -> bool {
        self.root.is_single()
    }
}

impl Tab {
    /// The editor or the view the tab shows, as its identity.
    pub(super) fn key(&self) -> EntityId {
        match &self.content {
            TabContent::File { editor, .. } => editor.entity_id(),
            TabContent::View(view) => view.entity_id(),
        }
    }
}

impl Workbench {
    /// The handlers of the split commands.
    pub(super) fn split_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        root.on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::split_down))
            .on_action(cx.listener(Self::split_and_move_right))
            .on_action(cx.listener(Self::split_and_move_down))
            .on_action(cx.listener(Self::unsplit))
            .on_action(cx.listener(Self::unsplit_all))
            .on_action(cx.listener(Self::change_splitter_orientation))
            .on_action(cx.listener(Self::next_splitter))
            .on_action(cx.listener(Self::previous_splitter))
            .on_mouse_move(cx.listener(Self::resize_split))
    }

    // ---- groups and their tabs ---------------------------------------------------

    /// The indices of a group's tabs, in order.
    pub(super) fn group_tabs(&self, group: GroupId) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&i| self.tabs[i].group == group)
            .collect()
    }

    /// The selected tab of a group: the active tab of the focused group, of
    /// the others the one recorded, else the last.
    pub(super) fn group_selected(&self, group: GroupId) -> Option<usize> {
        if group == self.splits.focused {
            return self.active.filter(|&a| self.tabs[a].group == group);
        }
        self.recorded_selection(group)
    }

    /// The tab recorded as the group's selected one, else its last tab.
    fn recorded_selection(&self, group: GroupId) -> Option<usize> {
        let order = self.group_tabs(group);
        self.splits
            .selected
            .get(&group)
            .and_then(|key| order.iter().copied().find(|&i| self.tabs[i].key() == *key))
            .or_else(|| order.last().copied())
    }

    /// Whether tab `index` is the selected tab of its group.
    pub(super) fn is_selected_in_group(&self, index: usize) -> bool {
        self.group_selected(self.tabs[index].group) == Some(index)
    }

    /// Tab `index` was made active: its group has the focus now and shows
    /// it.
    pub(super) fn note_selected(&mut self, index: usize) {
        let group = self.tabs[index].group;
        self.splits.focused = group;
        self.splits.selected.insert(group, self.tabs[index].key());
    }

    /// The tabs a command acts on besides the tab itself: those of its
    /// group.
    pub(super) fn same_group(&self, a: usize, b: usize) -> bool {
        self.tabs[a].group == self.tabs[b].group
    }

    /// The group has the focus now: its selected tab is the active one.
    fn focus_group(&mut self, group: GroupId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.group_selected(group) {
            self.activate(index, window, cx);
        }
    }

    /// Everything about the groups is forgotten (another project).
    pub(super) fn reset_splits(&mut self) {
        self.splits = Splits::default();
    }

    /// Fixes the list after tab `index`, of `group` and identified by
    /// `key`, was taken out of `tabs`: the active tab is the neighbour of
    /// the closed one in its group; an empty group goes, the focus moves
    /// to the group beside it. The focus follows when `focus` is set.
    pub(super) fn tab_removed(
        &mut self,
        index: usize,
        group: GroupId,
        key: EntityId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let in_focused = self.splits.focused == group;
        let was_active = in_focused && self.active == Some(index);
        self.active = match self.active {
            Some(a) if a == index => None,
            Some(a) if a > index => Some(a - 1),
            other => other,
        };
        let remaining = self.group_tabs(group);
        if remaining.is_empty() {
            self.splits.selected.remove(&group);
            if !self.splits.is_single() {
                self.remove_group(group);
            }
        } else {
            let before = remaining.iter().filter(|&&i| i < index).count();
            let neighbour = remaining[before.saturating_sub(1)];
            if was_active {
                self.active = Some(neighbour);
                self.splits
                    .selected
                    .insert(group, self.tabs[neighbour].key());
            } else if self.splits.selected.get(&group) == Some(&key) {
                self.splits
                    .selected
                    .insert(group, self.tabs[neighbour].key());
            }
        }
        if focus && (in_focused || self.splits.focused != group) {
            match self.active {
                Some(a) => window.focus(&self.tabs[a].focus_handle(cx)),
                None => window.focus(&self.focus),
            }
        }
    }

    /// An empty group goes: its splitter is replaced by the other side. If
    /// it had the focus, the first group of that side has it.
    fn remove_group(&mut self, group: GroupId) {
        let beside = self.splits.root.siblings(group);
        let root = std::mem::replace(&mut self.splits.root, Node::Leaf(0));
        self.splits.root = root.without(group).unwrap_or(Node::Leaf(0));
        self.splits.scroll.borrow_mut().remove(&group);
        if self.splits.focused == group {
            let next = beside
                .first()
                .copied()
                .or_else(|| self.splits.root.leaves().first().copied())
                .unwrap_or(0);
            self.splits.focused = next;
            self.active = self.recorded_selection(next);
        }
        self.normalize_single();
    }

    /// With nothing split the one group is group 0 again, as at the start.
    fn normalize_single(&mut self) {
        let Node::Leaf(only) = self.splits.root else {
            return;
        };
        if only == 0 && self.splits.focused == 0 {
            return;
        }
        for tab in &mut self.tabs {
            tab.group = 0;
        }
        let mut scroll = self.splits.scroll.borrow_mut();
        let strip = scroll.remove(&only);
        scroll.clear();
        if let Some(strip) = strip {
            scroll.insert(0, strip);
        }
        drop(scroll);
        let selected = self.splits.selected.remove(&only);
        self.splits.selected.clear();
        if let Some(key) = selected {
            self.splits.selected.insert(0, key);
        }
        self.splits.root = Node::Leaf(0);
        self.splits.focused = 0;
        self.splits.next = 1;
    }

    // ---- splitting -----------------------------------------------------------------

    fn split_right(&mut self, a: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_command(a.index, false, false, window, cx);
    }

    fn split_down(&mut self, a: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_command(a.index, true, false, window, cx);
    }

    fn split_and_move_right(
        &mut self,
        a: &SplitAndMoveRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_command(a.index, false, true, window, cx);
    }

    fn split_and_move_down(
        &mut self,
        a: &SplitAndMoveDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_command(a.index, true, true, window, cx);
    }

    fn split_command(
        &mut self,
        index: Option<usize>,
        vertical: bool,
        close_source: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = index.or(self.active).filter(|&i| i < self.tabs.len())
            && self.can_split(index, close_source)
        {
            self.split_tab(index, vertical, close_source, window, cx);
        }
    }

    /// `SplitAction.isEnabled`: a tab can be split off when it is a file's
    /// (a view cannot show twice: `FORBID_TAB_SPLIT`); it can be moved
    /// into a new split when its group has another tab.
    pub(super) fn can_split(&self, index: usize, close_source: bool) -> bool {
        let tab = &self.tabs[index];
        if close_source {
            self.group_tabs(tab.group).len() >= 2
        } else {
            tab.path().is_some()
        }
    }

    /// The tab menu's rows: Split Right, Split and Move Right, Split Down,
    /// Split and Move Down (those that apply).
    pub(super) fn tab_split_items(
        &self,
        index: usize,
        keys: &dyn Fn(&dyn gpui::Action) -> String,
    ) -> Vec<MenuItem> {
        let some = Some(index);
        let mut items = Vec::new();
        if self.can_split(index, false) {
            items.push(
                MenuItem::action("Split Right", SplitRight { index: some })
                    .shortcut(keys(&SplitRight { index: None })),
            );
        }
        if self.can_split(index, true) {
            items.push(
                MenuItem::action("Split and Move Right", SplitAndMoveRight { index: some })
                    .shortcut(keys(&SplitAndMoveRight { index: None })),
            );
        }
        if self.can_split(index, false) {
            items.push(
                MenuItem::action("Split Down", SplitDown { index: some })
                    .shortcut(keys(&SplitDown { index: None })),
            );
        }
        if self.can_split(index, true) {
            items.push(
                MenuItem::action("Split and Move Down", SplitAndMoveDown { index: some })
                    .shortcut(keys(&SplitAndMoveDown { index: None })),
            );
        }
        // In a split: the splitter's rows (`SplitterAction`), Unsplit All
        // only with more than two groups.
        if !self.splits.is_single() {
            items.push(
                MenuItem::action("Change Splitter Orientation", ChangeSplitterOrientation)
                    .shortcut(keys(&ChangeSplitterOrientation))
                    .mnemonic(17),
            );
            items.push(
                MenuItem::action("Unsplit", Unsplit)
                    .shortcut(keys(&Unsplit))
                    .mnemonic(2),
            );
            if self.splits.root.leaves().len() > 2 {
                items.push(
                    MenuItem::action("Unsplit All", UnsplitAll)
                        .shortcut(keys(&UnsplitAll))
                        .mnemonic(1),
                );
            }
        }
        items
    }

    /// `EditorWindow.split`: the tab's group is divided in two, below or
    /// right of it; the new group shows the tab's file again (a second
    /// view of it) or, when `close_source`, the tab itself (it moves), and
    /// has the focus.
    pub(super) fn split_tab(
        &mut self,
        index: usize,
        vertical: bool,
        close_source: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.tabs[index].group;
        let new = self.splits.new_group();
        let mut root = std::mem::replace(&mut self.splits.root, Node::Leaf(0));
        root.split(source, vertical, new, false);
        self.splits.root = root;
        let target = if close_source {
            // The tab leaves its group: that one selects another.
            let order = self.group_tabs(source);
            let key = self.tabs[index].key();
            if self.splits.selected.get(&source) == Some(&key) || self.active == Some(index) {
                let at = order.iter().position(|&i| i == index).unwrap_or(0);
                let neighbour = if at > 0 { order[at - 1] } else { order[1] };
                self.splits
                    .selected
                    .insert(source, self.tabs[neighbour].key());
            }
            self.tabs[index].group = new;
            index
        } else {
            match self.open_mirror(index, new, window, cx) {
                Some(mirror) => mirror,
                None => {
                    // Not a file: nothing to show in the new group.
                    let root = std::mem::replace(&mut self.splits.root, Node::Leaf(0));
                    self.splits.root = root.without(new).unwrap_or(Node::Leaf(0));
                    return;
                }
            }
        };
        self.activate(target, window, cx);
    }

    /// A second tab of the file of tab `source`, in `group`, with its own
    /// editor on a copy of the text: the caret and the scroll position
    /// where the first view has them.
    fn open_mirror(
        &mut self,
        source: usize,
        group: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let tab = &self.tabs[source];
        let TabContent::File { path, editor, .. } = &tab.content else {
            return None;
        };
        let (path, from, pinned) = (path.clone(), editor.clone(), tab.pinned);
        let (buffer, caret, view_y) = {
            let e = from.read(cx);
            (e.buffer().clone(), e.caret_point(), e.caret_view_y())
        };
        let editor = self.new_file_editor(&path, buffer, window, cx);
        editor.update(cx, |e, cx| e.go_to_place(caret, view_y, cx));
        let (find, subscriptions) = self.file_tab_parts(&editor, window, cx);
        self.vcs_tab_opened(&path, &editor, window, cx);
        self.debug_tab_opened(&editor, window, cx);
        self.tabs.push(Tab {
            content: TabContent::File { path, editor, find },
            pinned,
            group,
            mirror: true,
            _subscriptions: subscriptions,
        });
        Some(self.tabs.len() - 1)
    }

    /// The first view of a file closed while mirrors remain: the first of
    /// them is the file's editor from now on, for the language services,
    /// the file watcher and the diff.
    pub(super) fn promote_mirror(
        &mut self,
        closed: &Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if closed.mirror {
            return;
        }
        let Some(path) = closed.path() else {
            return;
        };
        let focused = self.splits.focused;
        let Some(next) = (0..self.tabs.len())
            .filter(|&i| self.tabs[i].mirror && self.tabs[i].is_file(path))
            .min_by_key(|&i| self.tabs[i].group != focused)
        else {
            return;
        };
        self.tabs[next].mirror = false;
        let Some(editor) = self.tabs[next].editor().cloned() else {
            return;
        };
        let path = path.clone();
        if let Some(view) = &self.code_intel {
            let language = self.file_types.for_path(&path).and_then(|t| t.language_id);
            view.update(cx, |v, cx| {
                v.editor_opened(&path, language, &editor, window, cx)
            });
        }
        self.sync_tab_opened(&path, &editor, window, cx);
        self.diff_tab_opened(&path, &editor, cx);
    }

    /// Every editor of the file of the tab at `index`, the first view
    /// first.
    pub(crate) fn views_of(&self, path: &std::path::Path) -> Vec<Entity<Editor>> {
        let mut views: Vec<&Tab> = self.tabs.iter().filter(|t| t.is_file(path)).collect();
        views.sort_by_key(|t| t.mirror);
        views
            .into_iter()
            .filter_map(|t| t.editor().cloned())
            .collect()
    }

    /// The index of the tab that saves the file of tab `index`: the first
    /// view's.
    pub(super) fn saving_tab(&self, index: usize) -> usize {
        let tab = &self.tabs[index];
        if !tab.mirror {
            return index;
        }
        tab.path()
            .and_then(|path| {
                (0..self.tabs.len()).find(|&i| !self.tabs[i].mirror && self.tabs[i].is_file(path))
            })
            .unwrap_or(index)
    }

    /// An editor of a file that is open in more than one group changed:
    /// the other editors of the file get the same edit (or the same text
    /// after a reload).
    pub(super) fn sync_views(
        &mut self,
        from: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.iter().any(|t| t.mirror) {
            return;
        }
        let applied = match event {
            EditorEvent::Edited(applied) => Some(applied),
            EditorEvent::Reloaded => None,
            _ => return,
        };
        let version = from.read(cx).buffer().version();
        if self.splits.echo.remove(&from.entity_id()) == Some(version) {
            return;
        }
        let Some(path) = from.read(cx).path().cloned() else {
            return;
        };
        for other in self.views_of(&path) {
            if other == *from {
                continue;
            }
            match applied {
                Some(applied) => crate::file_preview::mirror(applied, from, &other, cx),
                None => {
                    let text = from.read(cx).text();
                    other.update(cx, |e, cx| {
                        let len = e.buffer().len_chars();
                        e.edit(
                            vec![text::Edit::new(0..len, text)],
                            text::EditKind::Other,
                            text::CaretPlacement::AfterEachEdit,
                            cx,
                        )
                    });
                }
            }
            let version = other.read(cx).buffer().version();
            self.splits.echo.insert(other.entity_id(), version);
        }
    }

    // ---- merging ---------------------------------------------------------------------

    fn unsplit(&mut self, _: &Unsplit, window: &mut Window, cx: &mut Context<Self>) {
        self.unsplit_group(self.splits.focused, window, cx);
    }

    fn unsplit_all(&mut self, _: &UnsplitAll, window: &mut Window, cx: &mut Context<Self>) {
        // `EditorWindow.unsplitAll`: the focused group takes the others one
        // splitter after the other.
        while !self.splits.is_single() {
            let before = self.splits.root.leaves().len();
            self.unsplit_group(self.splits.focused, window, cx);
            if self.splits.root.leaves().len() >= before {
                break;
            }
        }
    }

    /// `ChangeManager.changeSplitterOrientation`: the splitter beside the
    /// focused group turns.
    fn change_splitter_orientation(
        &mut self,
        _: &ChangeSplitterOrientation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.splits.focused;
        if let Some(splitter) = self.splits.root.parent_mut(focused) {
            splitter.vertical = !splitter.vertical;
            cx.notify();
        }
    }

    /// `EditorWindow.unsplit`: the groups beside `group` give it their tabs
    /// (after its own, except the files it shows already, whose tabs
    /// close) and the splitter goes. `group` shows the tab it showed, else
    /// the first selected one of the others, and has the focus.
    fn unsplit_group(&mut self, group: GroupId, window: &mut Window, cx: &mut Context<Self>) {
        let beside = self.splits.root.siblings(group);
        if beside.is_empty() {
            return;
        }
        let select = self
            .group_selected(group)
            .or_else(|| beside.iter().find_map(|&g| self.group_selected(g)))
            .map(|i| self.tabs[i].key());
        // The tabs of files the group shows already close (the tab of the
        // first view when it is the other group's: the group's takes over).
        let mut shown: Vec<std::path::PathBuf> = self
            .group_tabs(group)
            .into_iter()
            .filter_map(|i| self.tabs[i].path().cloned())
            .collect();
        let mut duplicates = Vec::new();
        for &g in &beside {
            for i in self.group_tabs(g) {
                match self.tabs[i].path() {
                    Some(path) if shown.contains(path) => duplicates.push(i),
                    Some(path) => shown.push(path.clone()),
                    None => {}
                }
            }
        }
        for index in duplicates.into_iter().rev() {
            self.close_tab(index, window, cx);
        }
        // The rest move to the end of the list: after the group's tabs.
        let (moved, kept): (Vec<usize>, Vec<usize>) =
            (0..self.tabs.len()).partition(|&i| beside.contains(&self.tabs[i].group));
        for &i in &moved {
            self.tabs[i].group = group;
        }
        self.permute_tabs(kept.into_iter().chain(moved).collect());
        self.splits.root.collapse(group);
        for g in beside {
            self.splits.selected.remove(&g);
            self.splits.scroll.borrow_mut().remove(&g);
        }
        if let Some(key) = select {
            self.splits.selected.insert(group, key);
        }
        self.splits.focused = group;
        self.normalize_single();
        if let Some(index) = self.recorded_selection(self.splits.focused) {
            self.activate(index, window, cx);
        }
        cx.notify();
    }

    /// Puts the tabs in a new order: `order[k]` is the old index of the
    /// tab that becomes the `k`-th.
    fn permute_tabs(&mut self, order: Vec<usize>) {
        let mut old: Vec<Option<Tab>> = self.tabs.drain(..).map(Some).collect();
        self.tabs = order.iter().map(|&o| old[o].take().unwrap()).collect();
        self.active = self.active.and_then(|a| order.iter().position(|&o| o == a));
    }

    // ---- moving between the groups ---------------------------------------------------

    fn next_splitter(&mut self, _: &NextSplitter, window: &mut Window, cx: &mut Context<Self>) {
        self.goto_splitter(true, window, cx);
    }

    fn previous_splitter(
        &mut self,
        _: &PreviousSplitter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.goto_splitter(false, window, cx);
    }

    /// `SplitterAction.Goto`: the next (previous) group in tree order,
    /// round.
    fn goto_splitter(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        let leaves = self.splits.root.leaves();
        let Some(at) = leaves.iter().position(|&g| g == self.splits.focused) else {
            return;
        };
        if leaves.len() < 2 {
            return;
        }
        let to = if next {
            (at + 1) % leaves.len()
        } else {
            (at + leaves.len() - 1) % leaves.len()
        };
        self.focus_group(leaves[to], window, cx);
    }

    // ---- drawing -----------------------------------------------------------------------

    /// The groups of the editor area as the splitters lay them out.
    pub(super) fn render_groups(&self, cx: &mut Context<Self>) -> AnyElement {
        let root = self.splits.root.clone();
        self.render_node(&root, Vec::new(), cx)
    }

    fn render_node(&self, node: &Node, path: Path, cx: &mut Context<Self>) -> AnyElement {
        let splitter = match node {
            Node::Leaf(group) => return self.render_group(*group, cx),
            Node::Split(splitter) => splitter,
        };
        let vertical = splitter.vertical;
        let theme = cx.theme();
        let line = theme.color(UiRole::Border);
        let mut first_path = path.clone();
        first_path.push(false);
        let mut second_path = path.clone();
        second_path.push(true);
        let first = self.render_node(&splitter.first, first_path, cx);
        let second = self.render_node(&splitter.second, second_path, cx);
        let side = |share: f32, child: AnyElement| {
            let mut side = div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_hidden();
            let style = side.style();
            style.flex_grow = Some(share);
            style.flex_shrink = Some(1.0);
            style.flex_basis = Some(relative(0.0).into());
            side.child(child)
        };
        let bounds = self.splits.bounds.clone();
        let measured = path.clone();
        let measure = canvas(
            move |b, _, _| {
                bounds.borrow_mut().insert(measured.clone(), b);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let cursor = if vertical {
            CursorStyle::ResizeUpDown
        } else {
            CursorStyle::ResizeLeftRight
        };
        let grab_path = path.clone();
        let id = format!(
            "splitter-{}",
            path.iter()
                .map(|&b| if b { '1' } else { '0' })
                .collect::<String>()
        );
        let debug_id = id.clone();
        let grab = div()
            .id(gpui::SharedString::from(id))
            .debug_selector(move || debug_id.clone())
            .absolute()
            .when(vertical, |d| {
                d.left_0()
                    .right_0()
                    .top(px(-(SPLITTER_GRAB - SPLITTER_LINE) / 2.0))
                    .h(px(SPLITTER_GRAB))
            })
            .when(!vertical, |d| {
                d.top_0()
                    .bottom_0()
                    .left(px(-(SPLITTER_GRAB - SPLITTER_LINE) / 2.0))
                    .w(px(SPLITTER_GRAB))
            })
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |_, hitbox, window, _| ui::pointer::request(cursor, &hitbox, window),
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.splits.drag = Some(SplitDrag {
                        path: grab_path.clone(),
                    });
                    cx.stop_propagation();
                }),
            );
        let divider = div()
            .flex_none()
            .relative()
            .bg(line)
            .when(vertical, |d| d.h(px(SPLITTER_LINE)).w_full())
            .when(!vertical, |d| d.w(px(SPLITTER_LINE)).h_full())
            .child(grab);
        div()
            .flex()
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .when(vertical, |d| d.flex_col())
            .when(!vertical, |d| d.flex_row())
            .child(measure)
            .child(side(splitter.proportion, first))
            .child(divider)
            .child(side(1.0 - splitter.proportion, second))
            .into_any_element()
    }

    /// Moves the splitter that is being dragged to the mouse.
    pub(super) fn resize_split(
        &mut self,
        e: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.splits.drag.clone() else {
            return;
        };
        if e.pressed_button != Some(MouseButton::Left) {
            self.splits.drag = None;
            return;
        }
        let Some(bounds) = self.splits.bounds.borrow().get(&drag.path).copied() else {
            return;
        };
        let Some(splitter) = self.splits.root.at_mut(&drag.path) else {
            return;
        };
        let share = if splitter.vertical {
            f32::from(e.position.y - bounds.top()) / f32::from(bounds.size.height).max(1.0)
        } else {
            f32::from(e.position.x - bounds.left()) / f32::from(bounds.size.width).max(1.0)
        };
        splitter.proportion = clamp_proportion(share);
        cx.notify();
    }

    // ---- state ------------------------------------------------------------------------

    /// The groups for `workbench.state`: the tree and, per group in tree
    /// order, its tabs and its selected tab.
    pub(super) fn splits_json(&self) -> Json {
        let groups: Vec<Json> = self
            .splits
            .root
            .leaves()
            .into_iter()
            .map(|g| {
                json!({
                    "id": g,
                    "tabs": self.group_tabs(g),
                    "active": self.group_selected(g),
                })
            })
            .collect();
        json!({
            "focused": self.splits.focused,
            "tree": serde_json::to_value(&self.splits.root).unwrap_or(Json::Null),
            "groups": groups,
        })
    }

    /// One group: its tab strip, the find bar of the selected tab and the
    /// selected tab's editor or view.
    fn render_group(&self, group: GroupId, cx: &mut Context<Self>) -> AnyElement {
        let order = self.group_tabs(group);
        let selected = self.group_selected(group);
        let theme = cx.theme();
        let tab_h = theme.metric(themes::Metric::TabHeight);
        let separator = theme.color(UiRole::Border);
        let tabs: Vec<AnyElement> = order.iter().map(|&i| self.render_tab(i, cx)).collect();
        let strip = self.splits.strip(group);
        if let Some(selected) = selected {
            let at = order.iter().position(|&i| i == selected).unwrap_or(0);
            let shown = Some((at, order.len()));
            if strip.shown.replace(shown) != shown {
                strip.handle.scroll_to_item(at);
            }
        }
        let separator = match selected.map(|s| &self.tabs[s].content) {
            Some(TabContent::View(view)) if !view.separator(cx) => gpui::transparent_black(),
            _ => separator,
        };
        let focused = group == self.splits.focused;
        // The selector names the focused group's strip, the others by id.
        let debug_name: gpui::SharedString = if focused {
            "editor-tabs".into()
        } else {
            format!("editor-tabs-{group}").into()
        };
        let find = selected
            .and_then(|s| self.tabs[s].find())
            .filter(|find| find.read(cx).is_open())
            .cloned();
        let body = selected.map(|s| match &self.tabs[s].content {
            TabContent::File { editor, .. } => self.editor_container(editor, cx),
            TabContent::View(view) => div().flex_1().min_h(px(0.0)).child(view.view()),
        });
        div()
            .id(("editor-group", group as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            // A press anywhere in a group gives it the focus.
            .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                if this.splits.focused != group {
                    this.focus_group(group, window, cx);
                }
            }))
            // Files dropped on a group open in it.
            .on_drop(
                cx.listener(move |this, dropped: &gpui::ExternalPaths, window, cx| {
                    this.focus_group(group, window, cx);
                    this.open_dropped(dropped.paths().to_vec(), window, cx)
                }),
            )
            .on_drop(cx.listener(
                move |this, dragged: &crate::project_panel::DraggedFiles, window, cx| {
                    this.focus_group(group, window, cx);
                    this.open_dropped(dragged.paths.clone(), window, cx)
                },
            ))
            .child(
                div()
                    .id(("editor-tabs", group as usize))
                    .debug_selector(move || debug_name.to_string())
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .h(tab_h)
                    .pl(px(super::FIRST_TAB_OFFSET))
                    .overflow_x_scroll()
                    .track_scroll(&strip.handle)
                    .border_b_1()
                    .border_color(separator)
                    .children(tabs),
            )
            // The product's headers are one view each, shown above the
            // focused group's file.
            .when(
                focused && selected.is_some_and(|s| self.tabs[s].path().is_some()),
                |d| d.children(self.editor_headers.iter().cloned()),
            )
            .children(find)
            .children(body)
            .into_any_element()
    }
}
