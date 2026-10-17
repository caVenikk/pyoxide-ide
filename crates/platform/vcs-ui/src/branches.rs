//! The branches popup of the VCS widget, as the reference's New UI draws
//! it (`GitDefaultBranchesPopup`, `GitBranchesTreeSingleRepoModel`,
//! `GitDefaultBranchesTreeRenderer`): the search field "Search for
//! branches and actions" with Fetch and the settings gear, the top actions
//! (Update Project…, Commit…, Push…, then New Branch…, Checkout Tag or
//! Revision…), and a tree of the groups Recent, Local, Remote and Tags
//! with the branches grouped by their prefix folders (`feat/x` in `feat`).
//!
//! A branch row has its icon (the current branch a yellow label, with a
//! star when it is a favourite; favourites a star; tags a label), its
//! incoming and outgoing commits, its upstream right-aligned and the
//! arrow of its actions (Checkout, New Branch from '…'…, Update, Push…,
//! Rename…, Delete), which open beside it on Enter, Right or a click.
//! Typing filters branches and actions; Up and Down move, Left and Right
//! collapse and expand, Escape clears the search, then closes; Backspace
//! deletes the last character of the search, else the selected branch; F2
//! and Shift+F6 rename it.
//!
//! The popup's rows are built by [`build_rows`] from [`BranchData`]
//! (tested without a window); the workbench reads the data off the UI
//! thread and handles the `git.*` commands the rows run.

use std::collections::HashSet;
use std::ops::Range;

use gpui::{
    Action, AnyElement, App, ClickEvent, Context, CursorStyle, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, HighlightStyle, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Window, div, point, prelude::*,
    px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;
use vcs::{Branch, BranchKind};

use crate::widget::in_out_tooltip;
use crate::{Checkout, DeleteBranch, NewBranch, RenameBranch};

ui::ui_commands! {
    COMMANDS, scope = "BranchesPopup";
    /// The next row.
    Down => "branches-popup.down", "Next", "Selects the next row of the branches popup.";
    /// The previous row.
    Up => "branches-popup.up", "Previous", "Selects the previous row of the branches popup.";
    /// Collapse.
    Left => "branches-popup.left", "Collapse",
        "Collapses the selected group or folder of the branches popup, or selects its parent.";
    /// Expand.
    Right => "branches-popup.right", "Expand",
        "Expands the selected group or folder of the branches popup, or opens the actions of the selected branch.";
    /// Choose the selected row.
    Confirm => "branches-popup.confirm", "Choose",
        "Runs the selected action, expands or collapses the selected group, or opens the actions of the selected branch.";
    /// Close.
    Close => "branches-popup.close", "Close",
        "Clears the search text of the branches popup, else closes it.";
    /// Delete.
    Delete => "branches-popup.delete", "Delete",
        "Deletes the last character of the search text, else deletes the selected local branch.";
    /// Rename.
    Rename => "branches-popup.rename", "Rename…",
        "Renames the selected local branch.";
    /// Type into the search field.
    Search { text: String } => "branches-popup.search", "Search",
        "Sets the search text of the branches popup, which filters branches and actions.",
        r#"{"text": "ma"}"#;
}

/// Key context of the popup.
pub const KEY_CONTEXT: &str = "BranchesPopup";

/// Size of the popup (`GitBranchesPopupBase`: at least 375 × 312 in the
/// New UI).
pub const WIDTH: f32 = 375.0;
const MIN_HEIGHT: f32 = 312.0;
const MAX_HEIGHT: f32 = 560.0;
/// Rows (`List.rowHeight`) and the separators between the action groups.
const ROW: f32 = 24.0;
const SEPARATOR: f32 = 9.0;
/// The header: the drag area (8 + 2 × 2) above the search field, the
/// field and the room below it.
const HEADER_TOP: f32 = 12.0;
const FIELD_HEIGHT: f32 = 30.0;
const HEADER_BOTTOM: f32 = 6.0;
/// Selection inset from the sides, content start, the room of a chevron or
/// an action's icon, one tree level, an icon and its gap to the text.
const SELECTION_X: f32 = 10.0;
const CONTENT_X: f32 = 24.0;
const CHEVRON: f32 = 18.0;
const INDENT: f32 = 18.0;
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 6.0;
const BOTTOM: f32 = 8.0;
/// The most branches of the Recent group (`git.recent.checkout.branches.
/// reflog.entries.count`).
pub const RECENT_ENTRIES: usize = 5;

/// Incoming and outgoing commit colours (`GitIncomingOutgoingUi`).
const INCOMING: (u32, u32) = (0x3574F0, 0x548AF7);
const OUTGOING: (u32, u32) = (0x369650, 0x57965C);

/// A group of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Recent,
    Local,
    Remote,
    Tags,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Local => "Local",
            Self::Remote => "Remote",
            Self::Tags => "Tags",
        }
    }
}

/// What the popup lists, read off the UI thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BranchData {
    pub branches: Vec<Branch>,
    /// Local branches checked out lately, newest first.
    pub recent: Vec<String>,
    pub loaded: bool,
}

/// The popup's options (the gear's toggles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Branches in folders by their prefixes (Group by Directory).
    pub group_by_directory: bool,
    pub show_recent: bool,
    pub show_tags: bool,
    /// Actions are searched too (Show Actions in Search Results).
    pub search_actions: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            group_by_directory: true,
            show_recent: true,
            show_tags: true,
            search_actions: true,
        }
    }
}

/// A top action of the popup: a label, an icon, the command it runs
/// (`None`: shown disabled, the feature is not there yet) and its shortcut.
#[derive(Debug)]
pub struct TopAction {
    pub label: &'static str,
    pub icon: Option<IconName>,
    pub action: Option<Box<dyn Action>>,
    pub shortcut: Option<SharedString>,
    /// A separator follows.
    pub separator_after: bool,
}

impl Clone for TopAction {
    fn clone(&self) -> Self {
        Self {
            label: self.label,
            icon: self.icon,
            action: self.action.as_ref().map(|a| a.boxed_clone()),
            shortcut: self.shortcut.clone(),
            separator_after: self.separator_after,
        }
    }
}

/// A node that can be expanded: a group, or a folder of a group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub group: Group,
    /// The folder's path (`feat/sub`); empty for the group itself.
    pub path: String,
}

impl NodeId {
    fn group(group: Group) -> Self {
        Self {
            group,
            path: String::new(),
        }
    }
}

/// One row of the popup.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    /// A top action, by its index.
    Action(usize),
    Separator,
    /// A group or folder: its node, label and whether it is open.
    Node {
        id: NodeId,
        label: String,
        expanded: bool,
    },
    /// A branch or tag: its index in [`BranchData::branches`] and the
    /// group it is shown in.
    Branch {
        index: usize,
        group: Group,
    },
    /// Nothing matches the search.
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    pub depth: usize,
    /// Char ranges of the label the search matched.
    pub matches: Vec<Range<usize>>,
}

/// The default favourites (`GitBranchManager`'s predefined ones): `main`
/// and `master`, locally and on every remote.
pub fn is_favorite(branch: &Branch) -> bool {
    let leaf = match branch.kind {
        BranchKind::Local => branch.name.as_str(),
        BranchKind::Remote => branch.name.split_once('/').map_or("", |(_, rest)| rest),
        BranchKind::Tag => return false,
    };
    matches!(leaf, "main" | "master")
}

/// The search's matcher: the text anywhere in the name, case ignored.
fn matcher(filter: &str) -> Option<Matcher> {
    (!filter.is_empty()).then(|| Matcher::new(&format!("*{filter}"), MatchingMode::IgnoreCase))
}

/// A level of a group's tree.
#[derive(Default)]
struct Level {
    /// Branch indices at this level.
    refs: Vec<usize>,
    /// Folders by name, in the order met.
    folders: Vec<(String, Level)>,
}

impl Level {
    fn insert(&mut self, parts: &[&str], index: usize) {
        match parts {
            [] | [_] => self.refs.push(index),
            [folder, rest @ ..] => {
                let at = match self.folders.iter().position(|(n, _)| n == folder) {
                    Some(at) => at,
                    None => {
                        self.folders.push(((*folder).to_owned(), Level::default()));
                        self.folders.len() - 1
                    }
                };
                self.folders[at].1.insert(rest, index);
            }
        }
    }
}

/// Builds the rows: the top actions (matching the search), a separator,
/// then the groups with their folders and branches, current and favourite
/// branches first at each level, then folders, then the rest by name
/// (`GitBranchesTreeModel.getRefComparator`, `getSubTreeComparator`).
/// While searching every group and folder with a match is open.
pub fn build_rows(
    data: &BranchData,
    actions: &[TopAction],
    filter: &str,
    options: &Options,
    expanded: &HashSet<NodeId>,
) -> Vec<Row> {
    let m = matcher(filter);
    let tree = tree_rows(data, &m, options, expanded);
    let mut rows = Vec::new();
    let mut any_action = false;
    for (i, a) in actions.iter().enumerate() {
        let matches = match &m {
            None => Vec::new(),
            Some(_) if !options.search_actions => continue,
            Some(m) => match m.find(a.label) {
                Some(found) => found,
                None => continue,
            },
        };
        rows.push(Row {
            kind: RowKind::Action(i),
            depth: 0,
            matches,
        });
        any_action = true;
        if a.separator_after && m.is_none() {
            rows.push(Row {
                kind: RowKind::Separator,
                depth: 0,
                matches: Vec::new(),
            });
        }
    }
    if m.is_some() && any_action && !tree.is_empty() {
        rows.push(Row {
            kind: RowKind::Separator,
            depth: 0,
            matches: Vec::new(),
        });
    }
    if m.is_some() && !any_action && tree.is_empty() {
        rows.push(Row {
            kind: RowKind::Empty,
            depth: 0,
            matches: Vec::new(),
        });
    }
    rows.extend(tree);
    rows
}

/// The groups, folders and branches.
fn tree_rows(
    data: &BranchData,
    m: &Option<Matcher>,
    options: &Options,
    expanded: &HashSet<NodeId>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let recent: Vec<usize> = if options.show_recent {
        data.recent
            .iter()
            .filter_map(|name| {
                data.branches
                    .iter()
                    .position(|b| b.kind == BranchKind::Local && &b.name == name)
            })
            .collect()
    } else {
        Vec::new()
    };
    let in_group = |group: Group| -> Vec<usize> {
        match group {
            Group::Recent => recent.clone(),
            Group::Local => (0..data.branches.len())
                .filter(|&i| data.branches[i].kind == BranchKind::Local && !recent.contains(&i))
                .collect(),
            Group::Remote => (0..data.branches.len())
                .filter(|&i| data.branches[i].kind == BranchKind::Remote)
                .collect(),
            Group::Tags if options.show_tags => (0..data.branches.len())
                .filter(|&i| data.branches[i].kind == BranchKind::Tag)
                .collect(),
            Group::Tags => Vec::new(),
        }
    };
    for group in [Group::Recent, Group::Local, Group::Remote, Group::Tags] {
        let mut members = in_group(group);
        if let Some(m) = &m {
            members.retain(|&i| m.matches(&data.branches[i].name));
        }
        if members.is_empty() {
            continue;
        }
        let open = m.is_some() || expanded.contains(&NodeId::group(group));
        rows.push(Row {
            kind: RowKind::Node {
                id: NodeId::group(group),
                label: group.title().to_owned(),
                expanded: open,
            },
            depth: 0,
            matches: Vec::new(),
        });
        if !open {
            continue;
        }
        // Recent keeps the order of the checkouts and no folders.
        if group == Group::Recent || !options.group_by_directory {
            let mut members = members;
            if group != Group::Recent {
                sort_refs(&mut members, data);
            }
            for i in members {
                push_branch(&mut rows, data, i, group, 1, 0, m.as_ref());
            }
            continue;
        }
        let mut root = Level::default();
        for &i in &members {
            let parts: Vec<&str> = data.branches[i].name.split('/').collect();
            root.insert(&parts, i);
        }
        push_level(&mut rows, data, group, &root, "", 1, m.as_ref(), expanded);
    }
    rows
}

/// Orders branches: current, favourites, then by name.
fn sort_refs(refs: &mut [usize], data: &BranchData) {
    refs.sort_by_key(|&i| {
        let b = &data.branches[i];
        (
            !b.current,
            !is_favorite(b),
            b.name.to_lowercase(),
            b.name.clone(),
        )
    });
}

#[allow(clippy::too_many_arguments)]
fn push_level(
    rows: &mut Vec<Row>,
    data: &BranchData,
    group: Group,
    level: &Level,
    prefix: &str,
    depth: usize,
    m: Option<&Matcher>,
    expanded: &HashSet<NodeId>,
) {
    let mut refs = level.refs.clone();
    sort_refs(&mut refs, data);
    let (first, rest): (Vec<usize>, Vec<usize>) = refs.into_iter().partition(|&i| {
        let b = &data.branches[i];
        b.current || is_favorite(b)
    });
    let label_start = prefix.chars().count() + usize::from(!prefix.is_empty());
    for i in first {
        push_branch(rows, data, i, group, depth, label_start, m);
    }
    let mut folders: Vec<&(String, Level)> = level.folders.iter().collect();
    folders.sort_by_key(|(name, _)| (name.to_lowercase(), name.clone()));
    for (name, sub) in folders {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let id = NodeId {
            group,
            path: path.clone(),
        };
        let open = m.is_some() || expanded.contains(&id);
        rows.push(Row {
            kind: RowKind::Node {
                id,
                label: name.clone(),
                expanded: open,
            },
            depth,
            matches: Vec::new(),
        });
        if open {
            push_level(rows, data, group, sub, &path, depth + 1, m, expanded);
        }
    }
    for i in rest {
        push_branch(rows, data, i, group, depth, label_start, m);
    }
}

fn push_branch(
    rows: &mut Vec<Row>,
    data: &BranchData,
    index: usize,
    group: Group,
    depth: usize,
    label_start: usize,
    m: Option<&Matcher>,
) {
    let name = &data.branches[index].name;
    let matches = m
        .and_then(|m| m.find(name))
        .map(|found| {
            found
                .into_iter()
                .filter(|r| r.end > label_start)
                .map(|r| r.start.max(label_start) - label_start..r.end - label_start)
                .collect()
        })
        .unwrap_or_default();
    rows.push(Row {
        kind: RowKind::Branch { index, group },
        depth,
        matches,
    });
}

/// The groups and folders open when the popup opens
/// (`GitBranchesPopupBase.traverseNodesAndExpand`): those on the way to
/// the current branch, and the Recent group.
pub fn default_expanded(data: &BranchData, options: &Options) -> HashSet<NodeId> {
    let mut open = HashSet::new();
    let recent = options.show_recent && !data.recent.is_empty();
    if recent {
        open.insert(NodeId::group(Group::Recent));
    }
    if let Some(current) = data
        .branches
        .iter()
        .find(|b| b.current && b.kind == BranchKind::Local)
    {
        let in_recent = recent && data.recent.contains(&current.name);
        if !in_recent {
            open.insert(NodeId::group(Group::Local));
            if options.group_by_directory {
                let mut path = String::new();
                let parts: Vec<&str> = current.name.split('/').collect();
                for part in &parts[..parts.len() - 1] {
                    if !path.is_empty() {
                        path.push('/');
                    }
                    path.push_str(part);
                    open.insert(NodeId {
                        group: Group::Local,
                        path: path.clone(),
                    });
                }
            }
        }
    }
    open
}

/// The row selected when the popup opens or the search changes: the
/// first matching action, else the current branch (or, searching, the
/// first branch), else the first row that can be chosen.
pub fn preferred_row(
    rows: &[Row],
    data: &BranchData,
    actions: &[TopAction],
    searching: bool,
) -> Option<usize> {
    if searching {
        let action = |r: &Row| matches!(r.kind, RowKind::Action(_)) && selectable(&r.kind, actions);
        if let Some(i) = rows.iter().position(action) {
            return Some(i);
        }
        if let Some(i) = rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::Branch { .. }))
        {
            return Some(i);
        }
    }
    rows.iter()
        .position(
            |r| matches!(r.kind, RowKind::Branch { index, .. } if data.branches[index].current),
        )
        .or_else(|| rows.iter().position(|r| selectable(&r.kind, actions)))
}

/// A row can be selected: not a separator, not a disabled action
/// (`GitBranchesTreeModel.isSelectable`).
fn selectable(kind: &RowKind, actions: &[TopAction]) -> bool {
    match kind {
        RowKind::Separator | RowKind::Empty => false,
        RowKind::Action(a) => actions.get(*a).is_some_and(|a| a.action.is_some()),
        _ => true,
    }
}

/// The same row after the rows were built again (a group opened or
/// closed keeps its selection).
fn same_row(a: &RowKind, b: &RowKind) -> bool {
    match (a, b) {
        (RowKind::Node { id: x, .. }, RowKind::Node { id: y, .. }) => x == y,
        _ => a == b,
    }
}

/// The text a branch is called by in the popup's actions
/// (`getSelectedBranchTruncatedPresentation`): in quotes.
fn quoted(name: &str) -> String {
    format!("'{name}'")
}

/// The actions of a branch: Checkout, New Branch from '…'…, Update,
/// Push…, Rename…, Delete (`Git.Branch`); Update and Push… come with the
/// update and push commands (`update` and `push`, `None` while there are
/// none).
pub fn branch_actions(
    branch: &Branch,
    update: Option<Box<dyn Action>>,
    push: Option<Box<dyn Action>>,
) -> Vec<MenuItem> {
    let name = branch.name.clone();
    let checkout = MenuItem::action(
        "Checkout",
        Checkout {
            r#ref: name.clone(),
            mode: None,
        },
    )
    .enabled(!branch.current);
    let new_branch = MenuItem::action(
        format!("New Branch from {}\u{2026}", quoted(&name)),
        NewBranch {
            name: None,
            from: Some(name.clone()),
            checkout: None,
        },
    );
    let mut items = vec![checkout, new_branch];
    if branch.kind == BranchKind::Local {
        items.push(MenuItem::separator());
        items.push(match update {
            Some(a) if branch.tracking.is_some() => MenuItem::Action {
                label: "Update".into(),
                icon: None,
                action: Some(a),
                enabled: true,
                description: None,
                extra: Default::default(),
            },
            _ => MenuItem::unavailable("Update"),
        });
        items.push(match push {
            Some(a) => MenuItem::Action {
                label: "Push\u{2026}".into(),
                icon: None,
                action: Some(a),
                enabled: true,
                description: None,
                extra: Default::default(),
            },
            None => MenuItem::unavailable("Push\u{2026}"),
        });
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            "Rename\u{2026}",
            RenameBranch {
                from: Some(name.clone()),
                to: None,
            },
        ));
        items.push(
            MenuItem::action("Delete", DeleteBranch { name: name.clone() })
                .enabled(!branch.current),
        );
    }
    items
}

/// What the popup asks of its owner.
pub enum BranchesPopupEvent {
    /// Run a command where the popup was opened from, and close the popup.
    Run(Box<dyn Action>),
    /// The gear's options changed (the owner keeps them).
    OptionsChanged(Options),
}

/// The branches popup.
pub struct BranchesPopup {
    focus: FocusHandle,
    data: BranchData,
    actions: Vec<TopAction>,
    /// The commands of a branch's Update and Push…, when there are.
    update: Option<Box<dyn Action>>,
    push: Option<Box<dyn Action>>,
    fetch: Option<Box<dyn Action>>,
    options: Options,
    filter: String,
    expanded: HashSet<NodeId>,
    rows: Vec<Row>,
    selected: Option<usize>,
    scroll: ScrollHandle,
    /// A branch's actions, open beside its row, and the row.
    submenu: Option<(usize, Entity<ContextMenu>, Subscription)>,
    /// The gear's menu.
    settings: Option<(Entity<ContextMenu>, Subscription)>,
    dismissed: bool,
}

impl EventEmitter<DismissEvent> for BranchesPopup {}
impl EventEmitter<BranchesPopupEvent> for BranchesPopup {}

impl Focusable for BranchesPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// What the popup opens with besides its data.
pub struct PopupSetup {
    pub actions: Vec<TopAction>,
    pub update: Option<Box<dyn Action>>,
    pub push: Option<Box<dyn Action>>,
    pub fetch: Option<Box<dyn Action>>,
    pub options: Options,
}

impl BranchesPopup {
    pub fn new(setup: PopupSetup, cx: &mut Context<Self>) -> Self {
        let mut popup = Self {
            focus: cx.focus_handle(),
            data: BranchData::default(),
            actions: setup.actions,
            update: setup.update,
            push: setup.push,
            fetch: setup.fetch,
            options: setup.options,
            filter: String::new(),
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            scroll: ScrollHandle::new(),
            submenu: None,
            settings: None,
            dismissed: false,
        };
        popup.rebuild(true);
        popup
    }

    /// The branches arrived (or changed).
    pub fn set_data(&mut self, data: BranchData, cx: &mut Context<Self>) {
        let first = !self.data.loaded;
        if first {
            self.expanded = default_expanded(&data, &self.options);
        }
        self.data = data;
        self.rebuild(first);
        cx.notify();
    }

    pub fn data(&self) -> &BranchData {
        &self.data
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The selected branch, if a branch row is selected.
    pub fn selected_branch(&self) -> Option<&Branch> {
        match self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| &r.kind)
        {
            Some(RowKind::Branch { index, .. }) => self.data.branches.get(*index),
            _ => None,
        }
    }

    fn rebuild(&mut self, reselect: bool) {
        let keep = self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| r.kind.clone());
        self.rows = build_rows(
            &self.data,
            &self.actions,
            &self.filter,
            &self.options,
            &self.expanded,
        );
        let kept = keep
            .filter(|_| !reselect)
            .and_then(|k| self.rows.iter().position(|r| same_row(&r.kind, &k)));
        self.selected = kept.or_else(|| {
            preferred_row(
                &self.rows,
                &self.data,
                &self.actions,
                !self.filter.is_empty(),
            )
        });
        if let Some(i) = self.selected {
            self.scroll.scroll_to_item(i);
        }
    }

    fn close_submenu(&mut self) {
        self.submenu = None;
    }

    /// Closes the popup.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.dismissed {
            self.dismissed = true;
            cx.emit(DismissEvent);
        }
    }

    fn run(&mut self, action: Box<dyn Action>, cx: &mut Context<Self>) {
        cx.emit(BranchesPopupEvent::Run(action));
        self.dismiss(cx);
    }

    fn move_selection(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let mut i = self
            .selected
            .map_or(if step > 0 { -1 } else { n }, |s| s as isize);
        for _ in 0..n {
            i = (i + step).rem_euclid(n);
            if selectable(&self.rows[i as usize].kind, &self.actions) {
                self.selected = Some(i as usize);
                self.scroll.scroll_to_item(i as usize);
                break;
            }
        }
        self.close_submenu();
        cx.notify();
    }

    fn set_expanded(&mut self, id: NodeId, open: bool, cx: &mut Context<Self>) {
        if open {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
        self.rebuild(false);
        cx.notify();
    }

    /// Chooses row `i`: an action runs, a group opens or closes, a branch
    /// shows its actions.
    fn choose(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(i).cloned() else {
            return;
        };
        self.selected = Some(i);
        match row.kind {
            RowKind::Action(a) => {
                if let Some(action) = self.actions[a].action.as_ref().map(|a| a.boxed_clone()) {
                    self.run(action, cx);
                }
            }
            RowKind::Node { id, expanded, .. } => {
                if self.filter.is_empty() {
                    self.set_expanded(id, !expanded, cx);
                }
            }
            RowKind::Branch { .. } => self.open_branch_actions(i, window, cx),
            RowKind::Separator | RowKind::Empty => {}
        }
        cx.notify();
    }

    fn open_branch_actions(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(RowKind::Branch { index, .. }) = self.rows.get(row).map(|r| r.kind.clone()) else {
            return;
        };
        let branch = self.data.branches[index].clone();
        let items = branch_actions(
            &branch,
            self.update.as_ref().map(|a| a.boxed_clone()),
            self.push.as_ref().map(|a| a.boxed_clone()),
        );
        window.focus(&self.focus);
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription =
            cx.subscribe_in(&menu, window, |this, menu, _: &DismissEvent, window, cx| {
                if this
                    .submenu
                    .as_ref()
                    .is_some_and(|(_, open, _)| open == menu)
                {
                    this.submenu = None;
                    if !this.dismissed {
                        window.focus(&this.focus);
                    }
                    cx.notify();
                }
            });
        self.submenu = Some((row, menu, subscription));
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let o = self.options;
        let check = |on: bool| if on { Some(IconName::CHECKED) } else { None };
        let toggle = |label: &'static str, on: bool, which: &'static str| {
            let item = MenuItem::action(
                label,
                ToggleOption {
                    option: which.to_owned(),
                },
            );
            match check(on) {
                Some(icon) => item.icon(icon),
                None => item,
            }
        };
        let items = vec![
            MenuItem::header("Add in Search Results"),
            toggle(
                "Show Actions in Search Results",
                o.search_actions,
                "actions",
            ),
            MenuItem::separator(),
            toggle("Group by Directory", o.group_by_directory, "directory"),
            toggle("Show Recent Branches", o.show_recent, "recent"),
            toggle("Show Tags", o.show_tags, "tags"),
        ];
        window.focus(&self.focus);
        let menu = ContextMenu::build(items, window, cx);
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                this.settings = None;
                if !this.dismissed {
                    window.focus(&this.focus);
                }
                cx.notify();
            });
        self.settings = Some((menu, subscription));
        cx.notify();
    }

    fn toggle_option(&mut self, action: &ToggleOption, _: &mut Window, cx: &mut Context<Self>) {
        let o = &mut self.options;
        match action.option.as_str() {
            "actions" => o.search_actions = !o.search_actions,
            "directory" => o.group_by_directory = !o.group_by_directory,
            "recent" => o.show_recent = !o.show_recent,
            "tags" => o.show_tags = !o.show_tags,
            _ => return,
        }
        let options = *o;
        self.rebuild(false);
        cx.emit(BranchesPopupEvent::OptionsChanged(options));
        cx.notify();
    }

    fn set_filter(&mut self, text: String, cx: &mut Context<Self>) {
        self.filter = text;
        self.close_submenu();
        self.rebuild(true);
        cx.notify();
    }

    // ---- commands ---------------------------------------------------------

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else {
            return;
        };
        if self.submenu.take().is_some() {
            cx.notify();
            return;
        }
        let row = self.rows[i].clone();
        if let RowKind::Node {
            id, expanded: true, ..
        } = &row.kind
            && self.filter.is_empty()
        {
            self.set_expanded(id.clone(), false, cx);
            return;
        }
        // The parent: the nearest row above at a smaller depth.
        if row.depth > 0
            && let Some(parent) = (0..i).rev().find(|&j| self.rows[j].depth < row.depth)
        {
            self.selected = Some(parent);
            cx.notify();
        }
    }

    fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else {
            return;
        };
        match self.rows[i].kind.clone() {
            RowKind::Node {
                id,
                expanded: false,
                ..
            } => self.set_expanded(id, true, cx),
            RowKind::Node { expanded: true, .. } => self.move_selection(1, cx),
            RowKind::Branch { .. } => self.open_branch_actions(i, window, cx),
            _ => {}
        }
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selected {
            self.choose(i, window, cx);
        }
    }

    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        if self.submenu.take().is_some() {
            cx.notify();
        } else if !self.filter.is_empty() {
            self.set_filter(String::new(), cx);
        } else {
            self.dismiss(cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if !self.filter.is_empty() {
            let mut text = self.filter.clone();
            text.pop();
            self.set_filter(text, cx);
            return;
        }
        if let Some(b) = self.selected_branch()
            && b.kind == BranchKind::Local
            && !b.current
        {
            let name = b.name.clone();
            self.run(Box::new(DeleteBranch { name }), cx);
        }
    }

    fn rename(&mut self, _: &Rename, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(b) = self.selected_branch()
            && b.kind == BranchKind::Local
        {
            let from = Some(b.name.clone());
            self.run(Box::new(RenameBranch { from, to: None }), cx);
        }
    }

    fn search(&mut self, action: &Search, _: &mut Window, cx: &mut Context<Self>) {
        self.set_filter(action.text.clone(), cx);
    }

    /// A typed character goes to the search.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.platform || k.modifiers.alt || k.modifiers.function {
            return;
        }
        let Some(text) = k
            .key_char
            .as_ref()
            .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
        else {
            return;
        };
        // A space only after the first character (`SpeedSearch`).
        if text == " " && self.filter.is_empty() {
            return;
        }
        let mut filter = self.filter.clone();
        filter.push_str(text);
        self.set_filter(filter, cx);
        cx.stop_propagation();
    }

    /// The state for `vcs.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        let rows: Vec<Json> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut row = match &r.kind {
                    RowKind::Action(a) => json!({
                        "kind": "action",
                        "text": self.actions[*a].label,
                        "enabled": self.actions[*a].action.is_some(),
                    }),
                    RowKind::Separator => json!({ "kind": "separator" }),
                    RowKind::Node {
                        id,
                        label,
                        expanded,
                    } => json!({
                        "kind": if id.path.is_empty() { "group" } else { "folder" },
                        "text": label,
                        "expanded": expanded,
                    }),
                    RowKind::Branch { index, group } => {
                        let b = &self.data.branches[*index];
                        json!({
                            "kind": "branch",
                            "group": group.title(),
                            "text": b.name.rsplit('/').next().unwrap_or(&b.name),
                            "name": b.name,
                            "current": b.current,
                            "favorite": is_favorite(b),
                            "upstream": b.tracking.as_ref().map(|t| t.upstream.clone()),
                            "incoming": b.tracking.as_ref().map_or(0, |t| t.behind),
                            "outgoing": b.tracking.as_ref().map_or(0, |t| t.ahead),
                        })
                    }
                    RowKind::Empty => json!({ "kind": "empty", "text": "Nothing found" }),
                };
                row["depth"] = r.depth.into();
                row["selected"] = (self.selected == Some(i)).into();
                row
            })
            .collect();
        let submenu = self
            .submenu
            .as_ref()
            .map(|(_, m, _)| m.read(cx).state_json());
        json!({
            "search": self.filter,
            "placeholder": PLACEHOLDER,
            "loaded": self.data.loaded,
            "rows": rows,
            "submenu": submenu,
        })
    }
}

/// The search field's placeholder (`git.branches.popup.search.field.
/// empty.text` with actions).
pub const PLACEHOLDER: &str = "Search for branches and actions";

ui::ui_commands! {
    INTERNAL_COMMANDS, scope = "BranchesPopup";
    /// Toggle an option of the popup.
    ToggleOption { option: String } => "branches-popup.toggle-option", "Toggle Popup Option",
        "Toggles an option of the branches popup's gear: actions, directory, recent or tags.",
        r#"{"option": "tags"}"#;
}

// ---- drawing ----------------------------------------------------------------

fn color(pair: (u32, u32), dark: bool) -> gpui::Hsla {
    gpui::rgb(if dark { pair.1 } else { pair.0 }).into()
}

impl BranchesPopup {
    fn render_in_out(&self, incoming: u32, outgoing: u32, cx: &App) -> Option<AnyElement> {
        if incoming == 0 && outgoing == 0 {
            return None;
        }
        let dark = cx.theme().is_dark();
        let count = |n: u32| {
            if n > 99 {
                "99+".to_owned()
            } else {
                n.to_string()
            }
        };
        let label = |icon: IconName, n: u32, c: gpui::Hsla| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(1.0))
                .text_color(c)
                .text_size(cx.theme().ui_size() - px(1.0))
                .child(Icon::new(icon).size(px(12.0)))
                .child(count(n))
        };
        Some(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(4.0))
                .ml(px(6.0))
                .when(incoming > 0, |d| {
                    d.child(label(IconName::INCOMING, incoming, color(INCOMING, dark)))
                })
                .when(outgoing > 0, |d| {
                    d.child(label(IconName::OUTGOING, outgoing, color(OUTGOING, dark)))
                })
                .into_any_element(),
        )
    }

    fn render_row(&self, i: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[i];
        let selected = self.selected == Some(i);
        let text = theme.color(UiRole::Text);
        let muted = theme.color(UiRole::TextMuted);
        let disabled = theme.color(UiRole::TextDisabled);
        let indent = CONTENT_X + row.depth as f32 * INDENT;
        if row.kind == RowKind::Separator {
            return div()
                .h(px(SEPARATOR))
                .flex()
                .items_center()
                .px(px(SELECTION_X))
                .child(div().h(px(1.0)).w_full().bg(theme.color(UiRole::Divider)))
                .into_any_element();
        }
        let highlight = |label: &str, matches: &[Range<usize>]| -> AnyElement {
            if matches.is_empty() {
                return SharedString::from(label.to_owned()).into_any_element();
            }
            let byte = |c: usize| label.char_indices().nth(c).map_or(label.len(), |(b, _)| b);
            let style = HighlightStyle {
                background_color: Some(theme.color(UiRole::SearchMatch)),
                ..Default::default()
            };
            let ranges: Vec<(Range<usize>, HighlightStyle)> = matches
                .iter()
                .map(|r| (byte(r.start)..byte(r.end), style))
                .collect();
            StyledText::new(label.to_owned())
                .with_highlights(ranges)
                .into_any_element()
        };
        let mut content = div()
            .flex()
            .flex_row()
            .items_center()
            .size_full()
            .pl(px(indent - SELECTION_X))
            .pr(px(8.0))
            .min_w(px(0.0));
        let mut has_arrow = false;
        let mut tooltip: Option<String> = None;
        match &row.kind {
            RowKind::Action(a) => {
                let a = &self.actions[*a];
                let enabled = a.action.is_some();
                content = content
                    .child(
                        div()
                            .flex_none()
                            .w(px(CHEVRON))
                            .children(a.icon.map(|icon| Icon::new(icon).size(px(ICON)))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .when(!enabled, |d| d.text_color(disabled))
                            .child(highlight(a.label, &row.matches)),
                    )
                    .children(a.shortcut.clone().map(|s| {
                        div()
                            .flex_none()
                            .ml(px(10.0))
                            .text_color(if selected { text } else { muted })
                            .child(s)
                    }));
            }
            RowKind::Node {
                label,
                expanded,
                id,
                ..
            } => {
                content = content
                    .child(
                        div()
                            .flex_none()
                            .w(px(CHEVRON))
                            .child(ui::components::tree_arrow(*expanded, cx)),
                    )
                    .when(!id.path.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_none()
                                .mr(px(ICON_GAP))
                                .child(Icon::new(IconName::FOLDER).size(px(ICON))),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(label.clone()),
                    );
            }
            RowKind::Branch { index, .. } => {
                let b = &self.data.branches[*index];
                let favorite = is_favorite(b);
                let icon = match (b.current, favorite, b.kind) {
                    (true, true, _) => IconName::CURRENT_BRANCH_FAVORITE,
                    (true, false, _) => IconName::CURRENT_BRANCH,
                    (false, true, _) => IconName::FAVORITE,
                    (false, false, BranchKind::Tag) => IconName::BRANCH_LABEL,
                    _ => IconName::VCS,
                };
                let shown = if self.options.group_by_directory {
                    b.name.rsplit('/').next().unwrap_or(&b.name)
                } else {
                    b.name.as_str()
                };
                let (incoming, outgoing) =
                    b.tracking.as_ref().map_or((0, 0), |t| (t.behind, t.ahead));
                tooltip = in_out_tooltip(incoming, outgoing);
                has_arrow = true;
                content = content
                    .child(div().flex_none().w(px(CHEVRON)))
                    .child(
                        div()
                            .flex_none()
                            .mr(px(ICON_GAP))
                            .child(Icon::new(icon).size(px(ICON))),
                    )
                    .child(
                        div()
                            .flex_shrink()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(highlight(shown, &row.matches)),
                    )
                    .children(self.render_in_out(incoming, outgoing, cx))
                    .child(div().flex_1().min_w(px(10.0)))
                    .children(b.tracking.as_ref().map(|t| {
                        div()
                            .flex_none()
                            .ml(px(10.0))
                            .whitespace_nowrap()
                            .text_color(if selected { text } else { muted })
                            .child(t.upstream.clone())
                    }));
            }
            RowKind::Empty => {
                content = content
                    .justify_center()
                    .text_color(muted)
                    .child("Nothing found");
            }
            RowKind::Separator => unreachable!(),
        }
        if has_arrow {
            content = content.child(
                div()
                    .flex_none()
                    .ml(px(4.0))
                    .child(Icon::new(IconName::CHEVRON_RIGHT).size(px(ICON))),
            );
        }
        let selection = theme.color(UiRole::Selection);
        let radius = px(4.0);
        let mut el = div()
            .id(("branches-row", i))
            .debug_selector(move || format!("branches-row-{i}"))
            .h(px(ROW))
            .px(px(SELECTION_X))
            .child(
                div()
                    .size_full()
                    .rounded(radius)
                    .when(selected, |d| d.bg(selection))
                    .child(content),
            );
        if selectable(&row.kind, &self.actions) {
            el = el
                .pointer(CursorStyle::PointingHand)
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.selected != Some(i) && this.submenu.is_none() {
                        this.selected = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| this.choose(i, window, cx)),
                );
        }
        if let Some(tip) = tooltip {
            el = el.tooltip(Tooltip::text(tip, None));
        }
        let _ = window;
        el.into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let focused_border = theme.color(UiRole::Accent);
        let field = div()
            .id("branches-search")
            .debug_selector(|| "branches-search".into())
            .flex()
            .flex_row()
            .items_center()
            .flex_1()
            .min_w(px(0.0))
            .h(px(FIELD_HEIGHT))
            .px(px(8.0))
            .gap(px(6.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(focused_border)
            .child(Icon::new(IconName::SEARCH).size(px(ICON)))
            .child(if self.filter.is_empty() {
                div()
                    .text_color(muted)
                    .child(PLACEHOLDER)
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(self.filter.clone())
                    .child(div().w(px(1.0)).h(px(14.0)).bg(theme.color(UiRole::Text)))
                    .into_any_element()
            });
        let button = |id: &'static str, icon: IconName, tip: &'static str, enabled: bool| {
            let hover = theme.color(UiRole::Hover);
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded(px(4.0))
                .when(enabled, |d| {
                    d.hover(move |s| s.bg(hover))
                        .pointer(CursorStyle::PointingHand)
                })
                .when(!enabled, |d| d.opacity(0.4))
                .child(Icon::new(icon).size(px(ICON)))
                .tooltip(Tooltip::text(tip, None))
        };
        let fetch_enabled = self.fetch.is_some();
        div()
            .flex()
            .flex_row()
            .items_center()
            .pt(px(HEADER_TOP))
            .pb(px(HEADER_BOTTOM))
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(4.0))
            .child(field)
            .child(div().w(px(2.0)))
            .child(
                button("branches-fetch", IconName::FETCH, "Fetch", fetch_enabled).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(a) = this.fetch.as_ref().map(|a| a.boxed_clone()) {
                            this.run(a, cx);
                        }
                    }),
                ),
            )
            .child(
                button(
                    "branches-settings",
                    IconName::SETTINGS_SMALL,
                    "Settings",
                    true,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.open_settings(window, cx)),
                ),
            )
    }

    /// Height of the rows.
    fn rows_height(&self) -> f32 {
        self.rows
            .iter()
            .map(|r| {
                if r.kind == RowKind::Separator {
                    SEPARATOR
                } else {
                    ROW
                }
            })
            .sum()
    }

    /// Height of the popup.
    pub fn height(&self) -> f32 {
        let header = HEADER_TOP + FIELD_HEIGHT + HEADER_BOTTOM;
        (header + self.rows_height() + BOTTOM).clamp(MIN_HEIGHT, MAX_HEIGHT)
    }

    /// The open menu (a branch's actions or the gear's options) and where
    /// its top left goes from the popup's top left; the owner draws it
    /// over the window (`ui::menu::overlay_at`).
    pub fn open_menu(&self) -> Option<(Entity<ContextMenu>, Point<Pixels>)> {
        if let Some((menu, _)) = &self.settings {
            let at = point(px(WIDTH - 40.0), px(HEADER_TOP + FIELD_HEIGHT));
            return Some((menu.clone(), at));
        }
        self.submenu
            .as_ref()
            .map(|(row, menu, _)| (menu.clone(), self.submenu_offset(*row)))
    }

    /// Where the open submenu is drawn, from the popup's top left.
    fn submenu_offset(&self, row: usize) -> Point<Pixels> {
        let header = HEADER_TOP + FIELD_HEIGHT + HEADER_BOTTOM;
        let above: f32 = self.rows[..row]
            .iter()
            .map(|r| {
                if r.kind == RowKind::Separator {
                    SEPARATOR
                } else {
                    ROW
                }
            })
            .sum();
        point(px(WIDTH - SELECTION_X), px(header + above - 8.0))
    }
}

impl Render for BranchesPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height();
        let header_height = HEADER_TOP + FIELD_HEIGHT + HEADER_BOTTOM;
        let list_height = height - header_height - BOTTOM;
        // Rows of mixed heights (separators): a column, scrolled when long;
        // only the groups the person opened are listed.
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|i| self.render_row(i, window, cx))
            .collect();
        let header = self.render_header(cx).into_any_element();
        let theme = cx.theme();
        let list = div()
            .id("branches-rows")
            .debug_selector(|| "branches-rows".into())
            .flex()
            .flex_col()
            .h(px(list_height))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(rows);
        div()
            .id("branches-popup")
            .debug_selector(|| "branches-popup".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::search))
            .on_action(cx.listener(Self::toggle_option))
            .on_key_down(cx.listener(Self::key_down))
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .w(px(WIDTH))
            .h(px(height))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs::Tracking;

    fn branch(name: &str, kind: BranchKind, current: bool) -> Branch {
        Branch {
            name: name.to_owned(),
            kind,
            oid: "0".repeat(40),
            current,
            tracking: None,
        }
    }

    fn data() -> BranchData {
        let mut main = branch("main", BranchKind::Local, true);
        main.tracking = Some(Tracking {
            upstream: "origin/main".into(),
            ahead: 1,
            behind: 0,
            gone: false,
        });
        BranchData {
            branches: vec![
                branch("zeta", BranchKind::Local, false),
                branch("feat/b", BranchKind::Local, false),
                main,
                branch("feat/a", BranchKind::Local, false),
                branch("alpha", BranchKind::Local, false),
                branch("origin/main", BranchKind::Remote, false),
                branch("origin/feat/a", BranchKind::Remote, false),
                branch("v1.0", BranchKind::Tag, false),
            ],
            recent: Vec::new(),
            loaded: true,
        }
    }

    fn actions() -> Vec<TopAction> {
        let a = |label: &'static str, separator_after: bool| TopAction {
            label,
            icon: None,
            action: None,
            shortcut: None,
            separator_after,
        };
        vec![
            a("Update Project\u{2026}", false),
            a("Commit\u{2026}", false),
            a("Push\u{2026}", true),
            a("New Branch\u{2026}", false),
            a("Checkout Tag or Revision\u{2026}", true),
        ]
    }

    /// Rows as short texts: `>` closed, `v` open, `*` current.
    fn texts(rows: &[Row], d: &BranchData, actions: &[TopAction]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                let pad = "  ".repeat(r.depth);
                match &r.kind {
                    RowKind::Action(a) => format!("{pad}{}", actions[*a].label),
                    RowKind::Separator => "--".to_owned(),
                    RowKind::Node {
                        label, expanded, ..
                    } => format!("{pad}{} {label}", if *expanded { "v" } else { ">" }),
                    RowKind::Branch { index, .. } => {
                        let b = &d.branches[*index];
                        let leaf = b.name.rsplit('/').next().unwrap();
                        format!("{pad}{leaf}{}", if b.current { " *" } else { "" })
                    }
                    RowKind::Empty => "Nothing found".to_owned(),
                }
            })
            .collect()
    }

    #[test]
    fn the_tree_opens_on_the_current_branch() {
        let d = data();
        let o = Options::default();
        let open = default_expanded(&d, &o);
        let rows = build_rows(&d, &actions(), "", &o, &open);
        assert_eq!(
            texts(&rows, &d, &actions()),
            vec![
                "Update Project\u{2026}",
                "Commit\u{2026}",
                "Push\u{2026}",
                "--",
                "New Branch\u{2026}",
                "Checkout Tag or Revision\u{2026}",
                "--",
                "v Local",
                "  main *",
                "  > feat",
                "  alpha",
                "  zeta",
                "> Remote",
                "> Tags",
            ]
        );
        let selected = preferred_row(&rows, &d, &actions(), false).unwrap();
        assert_eq!(texts(&rows, &d, &actions())[selected], "  main *");
    }

    #[test]
    fn folders_and_favourites_inside_groups() {
        let d = data();
        let o = Options::default();
        let mut open = default_expanded(&d, &o);
        open.insert(NodeId {
            group: Group::Local,
            path: "feat".into(),
        });
        open.insert(NodeId::group(Group::Remote));
        open.insert(NodeId {
            group: Group::Remote,
            path: "origin".into(),
        });
        let rows = build_rows(&d, &[], "", &o, &open);
        assert_eq!(
            texts(&rows, &d, &[]),
            vec![
                "v Local",
                "  main *",
                "  v feat",
                "    a",
                "    b",
                "  alpha",
                "  zeta",
                "v Remote",
                "  v origin",
                "    main",
                "    > feat",
                "> Tags",
            ]
        );
    }

    #[test]
    fn recent_branches_come_first_and_leave_local() {
        let mut d = data();
        d.recent = vec!["feat/a".into(), "main".into()];
        let o = Options::default();
        let open = default_expanded(&d, &o);
        let rows = build_rows(&d, &[], "", &o, &open);
        assert_eq!(
            texts(&rows, &d, &[]),
            vec![
                "v Recent", "  a", "  main *", "> Local", "> Remote", "> Tags",
            ]
        );
    }

    #[test]
    fn the_search_filters_branches_and_actions() {
        let d = data();
        let o = Options::default();
        let rows = build_rows(&d, &actions(), "ma", &o, &HashSet::new());
        assert_eq!(
            texts(&rows, &d, &actions()),
            vec!["v Local", "  main *", "v Remote", "  v origin", "    main"]
        );
        let rows = build_rows(&d, &actions(), "new", &o, &HashSet::new());
        assert_eq!(texts(&rows, &d, &actions()), vec!["New Branch\u{2026}"]);
        assert_eq!(preferred_row(&rows, &d, &actions(), true), None);
        let rows = build_rows(&d, &actions(), "qqq", &o, &HashSet::new());
        assert_eq!(texts(&rows, &d, &actions()), vec!["Nothing found"]);
        // Matches are given for the shown part of the name.
        let rows = build_rows(&d, &[], "lph", &o, &HashSet::new());
        let a = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::Branch { index: 4, .. }))
            .unwrap();
        assert_eq!(a.matches, vec![1..4]);
        let rows = build_rows(&d, &[], "feat/b", &o, &HashSet::new());
        let b = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::Branch { index: 1, .. }))
            .unwrap();
        assert_eq!(b.matches, vec![0..1]);
    }

    #[test]
    fn the_actions_of_a_branch() {
        let d = data();
        let labels = |b: &Branch| -> Vec<String> {
            branch_actions(b, None, None)
                .iter()
                .map(|i| i.label().map_or("--".to_owned(), |l| l.to_string()))
                .collect()
        };
        assert_eq!(
            labels(&d.branches[0]),
            vec![
                "Checkout",
                "New Branch from 'zeta'\u{2026}",
                "--",
                "Update",
                "Push\u{2026}",
                "--",
                "Rename\u{2026}",
                "Delete"
            ]
        );
        assert_eq!(
            labels(&d.branches[5]),
            vec!["Checkout", "New Branch from 'origin/main'\u{2026}"]
        );
    }
}
