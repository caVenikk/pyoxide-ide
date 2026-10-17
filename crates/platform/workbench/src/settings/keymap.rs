//! Keymap (`KeymapPanel`, ADR 0018): the keymap combo box (predefined
//! keymaps bold, custom ones indented under their parent, "Based on {0}
//! keymap" next to a custom one), Import Keymap… in place of the
//! reference's link to the keymap plugins, and the actions tree of the
//! chosen keymap with its shortcuts as key caps: Expand All, Collapse All,
//! the search by name (it also takes the Settings window's search) and
//! Find Actions by Shortcut. The tree is read-only; the keymap chosen is
//! switched to with Apply.
//!
//! Not drawn (ADR 0018): Edit Shortcut and the editing of shortcuts, the
//! scheme gear (Duplicate, Rename, Restore Defaults, Delete), the second
//! stroke and the mouse pad of Find Shortcut, the macOS system shortcut
//! conflicts.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::OnceLock;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, Entity, FocusHandle,
    Focusable as _, FontWeight, IntoElement, Keystroke, Modifiers, Pixels, Render, SharedString,
    Subscription, UniformListScrollHandle, WeakEntity, Window, anchored, canvas, deferred, div,
    point, prelude::*, px, uniform_list,
};
use serde_json::{Value as Json, json};
use settings_ui::commands::EditField;
use settings_ui::{PageContext, SettingsPage};
use themes::UiRole;
use ui::components::{Tooltip, input_frame, popup_shadow, small_icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use crate::keymap::actions::{ActionTable, EDITOR_ACTIONS, MAIN_MENUS, TOP_GROUPS};
use crate::keymap::active::{self as keymaps, IMPORT_KEYMAP, ImportKeymap};
use crate::keymap::derived::canonical_keys;
use crate::keymap::{Keymap, double_tap_text, parse_double_tap, parse_mouse_button};

pub const ID: &str = "keymap";
pub const TITLE: &str = "Keymap";
/// The reference's place: a top-level page after Appearance & Behavior
/// (`groupWeight="65"`).
pub const WEIGHT: i32 = 65;

/// The reference's labels (KeyMapBundle).
const BASED_ON: &str = "Based on {0} keymap";
const EXPAND_ALL: &str = "Expand All";
const COLLAPSE_ALL: &str = "Collapse All";
const FIND_BY_SHORTCUT: &str = "Find Actions by Shortcut";
const CLEAR_FILTERING: &str = "Clear Filtering";
const FIND_SHORTCUT: &str = "Find Shortcut";
const OR: &str = "or";

/// The words the Settings search finds the page by: the actions' texts
/// (the reference indexes them for the Keymap page).
pub fn search_terms() -> &'static [&'static str] {
    static TERMS: OnceLock<Vec<&'static str>> = OnceLock::new();
    TERMS.get_or_init(|| {
        let mut terms = vec![FIND_BY_SHORTCUT];
        terms.extend(
            ActionTable::builtin()
                .actions
                .iter()
                .filter(|a| a.use_shortcut_of.is_none())
                .map(|a| a.text.as_str()),
        );
        terms
    })
}

/// Metrics of the page, from the reference: the combo box, the gap to the
/// info label, the link row, the toolbar's buttons, the tree's rows and
/// indent, and the key caps (`ShortcutTextUi`: 18 high, radius 4, 20 wide
/// for one character, 4 around longer texts, 3 between caps, 8 around
/// "or", 19 from the tree's right edge).
const COMBO_WIDTH: f32 = 197.0;
const COMBO_HEIGHT: f32 = 24.0;
const INFO_GAP: f32 = 10.0;
const LINK_TOP: f32 = 6.0;
const TOOLBAR_TOP: f32 = 12.0;
const TOOLBAR_HEIGHT: f32 = 30.0;
const SEARCH_WIDTH: f32 = 210.0;
const ROW: f32 = 22.0;
const INDENT: f32 = 16.0;
const ROW_LEFT: f32 = 6.0;
const ARROW: f32 = 16.0;
const ICON_GAP: f32 = 5.0;
const CAP_HEIGHT: f32 = 18.0;
const CAP_RADIUS: f32 = 4.0;
const CAP_WIDTH: f32 = 20.0;
const CAP_PAD: f32 = 4.0;
const CAP_GAP: f32 = 3.0;
const OR_GAP: f32 = 8.0;
const CAPS_RIGHT: f32 = 19.0;
const CAP_FONT: f32 = 12.0;
const POPUP_WIDTH: f32 = 220.0;

/// A group of the tree.
#[derive(Debug)]
struct Group {
    name: String,
    /// Its path, the names joined with `/`.
    key: String,
    children: Vec<Child>,
}

#[derive(Debug)]
enum Child {
    Group(Group),
    /// An action by its index in the table.
    Action(usize),
}

fn insert(group: &mut Group, path: &[String], action: usize) {
    let Some((name, rest)) = path.split_first() else {
        group.children.push(Child::Action(action));
        return;
    };
    let at = group
        .children
        .iter()
        .position(|c| matches!(c, Child::Group(g) if g.name == *name));
    let at = at.unwrap_or_else(|| {
        group.children.push(Child::Group(Group {
            name: name.clone(),
            key: format!("{}/{name}", group.key),
            children: Vec::new(),
        }));
        group.children.len() - 1
    });
    if let Child::Group(child) = &mut group.children[at] {
        insert(child, rest, action);
    }
}

/// The tree of the action table: Editor Actions sorted by action id, Main
/// Menu's menus in the reference's order, the rest in the table's order.
fn tree() -> &'static [Group] {
    static TREE: OnceLock<Vec<Group>> = OnceLock::new();
    TREE.get_or_init(|| {
        let table = ActionTable::builtin();
        let mut roots: Vec<Group> = TOP_GROUPS
            .iter()
            .map(|name| Group {
                name: (*name).to_owned(),
                key: (*name).to_owned(),
                children: Vec::new(),
            })
            .collect();
        for (ix, action) in table.actions.iter().enumerate() {
            for path in &action.groups {
                if let Some(root) = roots.iter_mut().find(|g| g.name == path[0]) {
                    insert(root, &path[1..], ix);
                }
            }
        }
        for root in &mut roots {
            if root.name == EDITOR_ACTIONS {
                root.children
                    .sort_by(|a, b| child_id(a, table).cmp(child_id(b, table)));
            } else if root.name == "Main Menu" {
                root.children.sort_by_key(|c| match c {
                    Child::Group(g) => MAIN_MENUS
                        .iter()
                        .position(|m| *m == g.name)
                        .unwrap_or(MAIN_MENUS.len()),
                    Child::Action(_) => MAIN_MENUS.len(),
                });
            }
        }
        roots.retain(|g| !g.children.is_empty());
        roots
    })
}

fn child_id<'a>(child: &'a Child, table: &'a ActionTable) -> &'a str {
    match child {
        Child::Action(ix) => &table.actions[*ix].id,
        Child::Group(g) => &g.name,
    }
}

/// A visible row of the tree.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    depth: usize,
    /// Unique among the rows: a group's path, an action's place.
    key: String,
    kind: RowKind,
}

#[derive(Debug, Clone, PartialEq)]
enum RowKind {
    Group { name: String, open: bool },
    Action(usize),
}

/// The key caps of a binding's keystrokes, in the style of the platform:
/// each modifier and the key a cap (`⌥` `Space`, `Ctrl` `Alt` `S`).
pub fn caps(keys: &str, macos: bool) -> Vec<String> {
    if let Some(key) = parse_double_tap(keys) {
        return vec![double_tap_text(key, macos)];
    }
    if parse_mouse_button(keys).is_some() {
        let n = if keys == "button4" { 4 } else { 5 };
        return vec![format!("Button {n} Click")];
    }
    let mut out = Vec::new();
    for stroke in keys.split(' ') {
        let Ok(k) = Keystroke::parse(stroke) else {
            out.push(stroke.to_owned());
            continue;
        };
        let m = k.modifiers;
        let names: [(bool, &str); 4] = if macos {
            [
                (m.control, "⌃"),
                (m.alt, "⌥"),
                (m.shift, "⇧"),
                (m.platform, "⌘"),
            ]
        } else {
            [
                (m.control, "Ctrl"),
                (m.alt, "Alt"),
                (m.shift, "Shift"),
                (m.platform, "Win"),
            ]
        };
        out.extend(
            names
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, n)| (*n).to_owned()),
        );
        let key = Keystroke {
            modifiers: Modifiers::none(),
            key: k.key,
            key_char: None,
        };
        out.push(ui::keys::format_keystroke(&key, macos));
    }
    out
}

/// The first keystroke of a binding, canonical.
fn first_stroke(keys: &str) -> String {
    let canonical = canonical_keys(keys);
    match canonical.split_once(' ') {
        // A double tap is one gesture.
        Some(_) if parse_double_tap(keys).is_some() => canonical,
        Some((first, _)) => first.to_owned(),
        None => canonical,
    }
}

/// The Find Shortcut popup while it takes keystrokes.
struct Capture {
    focus: FocusHandle,
    /// Escape was the last key: a second one closes the popup.
    escape: bool,
    _subscriptions: Vec<Subscription>,
}

/// Where the combo box and the Find button are drawn.
#[derive(Default)]
struct Anchors {
    combo: Bounds<Pixels>,
    find: Bounds<Pixels>,
}

/// The page: the keymap chosen (applied with Apply), its shortcuts, the
/// tree's state and the filters.
pub struct KeymapPage {
    chosen: String,
    /// The shortcuts of each action of the table in the chosen keymap.
    shortcuts: Vec<Vec<String>>,
    expanded: HashSet<String>,
    rows: Vec<Row>,
    selected: Option<String>,
    search: Entity<Editor>,
    /// The search by name in effect (the field's text).
    filter: String,
    /// Find Actions by Shortcut: the keystrokes looked for, canonical.
    shortcut: Option<String>,
    capture: Option<Capture>,
    list: Option<(Entity<ContextMenu>, Subscription)>,
    anchors: Rc<RefCell<Anchors>>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl KeymapPage {
    pub fn new(_: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = editor::field::new("", None, true, window, cx);
        let subscriptions = vec![cx.subscribe(&search, |this, editor, e: &EditorEvent, cx| {
            if matches!(e, EditorEvent::Edited(_)) {
                let text = editor.read(cx).text();
                this.set_filter(&text, cx);
            }
        })];
        let mut page = Self {
            chosen: keymaps::active_name(cx),
            shortcuts: Vec::new(),
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            search,
            filter: String::new(),
            shortcut: None,
            capture: None,
            list: None,
            anchors: Rc::default(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        page.load_shortcuts(cx);
        page
    }

    /// Reads the chosen keymap's shortcuts.
    fn load_shortcuts(&mut self, cx: &mut Context<Self>) {
        let keymap = keymaps::resolve(&self.chosen, cx).unwrap_or_default();
        self.shortcuts = shortcuts_of(&keymap);
        self.refresh_rows();
        cx.notify();
    }

    fn filtering(&self) -> bool {
        !self.filter.is_empty() || self.shortcut.is_some()
    }

    fn matches(&self, action: usize) -> bool {
        let table = ActionTable::builtin();
        if let Some(shortcut) = &self.shortcut {
            let wanted_first = first_stroke(shortcut);
            let two =
                canonical_keys(shortcut).contains(' ') && parse_double_tap(shortcut).is_none();
            return self.shortcuts[action].iter().any(|keys| {
                if two {
                    canonical_keys(keys) == canonical_keys(shortcut)
                } else {
                    first_stroke(keys) == wanted_first
                }
            });
        }
        if self.filter.is_empty() {
            return true;
        }
        let filter = self.filter.to_lowercase();
        let a = &table.actions[action];
        a.text.to_lowercase().contains(&filter) || a.id.to_lowercase().contains(&filter)
    }

    fn group_matches(&self, group: &Group) -> bool {
        group.children.iter().any(|c| match c {
            Child::Group(g) => self.group_matches(g),
            Child::Action(ix) => self.matches(*ix),
        })
    }

    fn refresh_rows(&mut self) {
        let mut rows = Vec::new();
        for group in tree() {
            self.push_group(group, 0, &mut rows);
        }
        self.rows = rows;
    }

    fn push_group(&self, group: &Group, depth: usize, rows: &mut Vec<Row>) {
        let filtering = self.filtering();
        if filtering && !self.group_matches(group) {
            return;
        }
        let open = filtering || self.expanded.contains(&group.key);
        rows.push(Row {
            depth,
            key: group.key.clone(),
            kind: RowKind::Group {
                name: group.name.clone(),
                open,
            },
        });
        if !open {
            return;
        }
        for child in &group.children {
            match child {
                Child::Group(g) => self.push_group(g, depth + 1, rows),
                Child::Action(ix) => {
                    if !filtering || self.matches(*ix) {
                        rows.push(Row {
                            depth: depth + 1,
                            key: format!(
                                "{}/#{}",
                                group.key,
                                ActionTable::builtin().actions[*ix].id
                            ),
                            kind: RowKind::Action(*ix),
                        });
                    }
                }
            }
        }
    }

    /// The search by name (typing in the field, the Settings search); it
    /// ends a search by shortcut.
    fn set_filter(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.trim().to_owned();
        if text == self.filter && (text.is_empty() || self.shortcut.is_none()) {
            return;
        }
        self.filter = text;
        if !self.filter.is_empty() {
            self.shortcut = None;
        }
        self.refresh_rows();
        cx.notify();
    }

    /// Fills the search field and searches (`showOption`).
    fn search_for(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        self.search
            .update(cx, |e, cx| e.set_text_selected(text, cx));
        self.shortcut = None;
        self.filter.clear();
        self.set_filter(text, cx);
        self.refresh_rows();
        cx.notify();
    }

    /// Find Actions by Shortcut: the actions bound to `keys` (GPUI's
    /// syntax); `None` clears it. The search by name is cleared.
    fn set_shortcut(&mut self, keys: Option<String>, cx: &mut Context<Self>) {
        self.shortcut = keys.map(|k| canonical_keys(&k));
        if self.shortcut.is_some() && !self.filter.is_empty() {
            self.filter.clear();
            self.search.update(cx, |e, cx| e.set_text_selected("", cx));
        }
        self.refresh_rows();
        cx.notify();
    }

    fn choose(&mut self, name: &str, cx: &mut Context<Self>) -> Result<(), String> {
        if !keymaps::is_known(name, cx) {
            return Err(format!("unknown keymap `{name}`"));
        }
        self.chosen = name.to_owned();
        self.load_shortcuts(cx);
        Ok(())
    }

    fn toggle(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_owned());
        }
        self.refresh_rows();
        cx.notify();
    }

    /// Expand All: every group open.
    fn expand_all(&mut self, cx: &mut Context<Self>) {
        fn walk(group: &Group, out: &mut HashSet<String>) {
            out.insert(group.key.clone());
            for child in &group.children {
                if let Child::Group(g) = child {
                    walk(g, out);
                }
            }
        }
        for group in tree() {
            walk(group, &mut self.expanded);
        }
        self.refresh_rows();
        cx.notify();
    }

    /// Collapse All: the top-level groups only.
    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.refresh_rows();
        cx.notify();
    }

    fn open_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list = None;
        let entries = keymaps::entries(cx);
        let selected = entries.iter().position(|e| e.name == self.chosen);
        let items: Vec<MenuItem> = entries
            .iter()
            .map(|e| {
                MenuItem::action(
                    e.name.clone(),
                    EditField {
                        field: "keymap".into(),
                        value: json!(e.name),
                    },
                )
                .bold(e.predefined)
                .indent(u8::from(!e.predefined))
            })
            .collect();
        let width = f32::from(self.anchors.borrow().combo.size.width).max(COMBO_WIDTH);
        // The chosen row runs where the focus is: on the page.
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                selected,
                min_width: Some(width),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some((menu, subscription));
        cx.notify();
    }

    /// Closes the combo box's list; the focus goes back to the page.
    fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((menu, _)) = self.list.take() {
            menu.update(cx, |m, cx| m.dismiss(window, cx));
            cx.notify();
        }
    }

    /// Closes the Find Shortcut popup; the focus goes back to the tree when
    /// the popup had it.
    fn close_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(capture) = self.capture.take() {
            if capture.focus.is_focused(window) {
                window.focus(&self.focus);
            }
            cx.notify();
        }
    }

    /// Opens the Find Shortcut popup: the keystrokes pressed while it has
    /// the focus are looked for; two Escapes in a row close it (the
    /// reference's popup takes a single Escape as a shortcut).
    fn open_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(window, cx);
        let focus = cx.focus_handle();
        let weak: WeakEntity<Self> = cx.weak_entity();
        let watched = focus.clone();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if !watched.is_focused(window) {
                return;
            }
            let keystroke = event.keystroke.clone();
            let _ = weak.update(cx, |page, cx| page.captured(keystroke, window, cx));
            cx.stop_propagation();
        });
        let blur = cx.on_blur(&focus, window, |this, _, cx| {
            this.capture = None;
            cx.notify();
        });
        window.focus(&focus);
        self.capture = Some(Capture {
            focus,
            escape: false,
            _subscriptions: vec![intercept, blur],
        });
        cx.notify();
    }

    fn captured(&mut self, keystroke: Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            keystroke.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        ) {
            return;
        }
        // Escape is not looked for: a second one in a row closes the popup.
        let escape = keystroke.key == "escape" && keystroke.modifiers == Modifiers::none();
        if escape {
            match &mut self.capture {
                Some(capture) if capture.escape => self.close_capture(window, cx),
                Some(capture) => capture.escape = true,
                None => {}
            }
            return;
        }
        if let Some(capture) = &mut self.capture {
            capture.escape = false;
        }
        let keys = Keystroke {
            key_char: None,
            ..keystroke
        }
        .unparse();
        self.set_shortcut(Some(keys), cx);
    }

    /// The row's shortcuts as key caps, right-aligned.
    fn render_caps(&self, action: usize, selected: bool, cx: &App) -> Option<AnyElement> {
        let shortcuts = &self.shortcuts[action];
        if shortcuts.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let macos = cfg!(target_os = "macos");
        let border = theme.color(UiRole::Border);
        let text = if selected {
            theme.color(UiRole::SelectionText)
        } else {
            theme.color(UiRole::Text)
        };
        let muted = theme.color(UiRole::TextMuted);
        let mut out: Vec<AnyElement> = Vec::new();
        for (n, keys) in shortcuts.iter().enumerate() {
            if n > 0 {
                out.push(
                    div()
                        .px(px(OR_GAP))
                        .text_color(muted)
                        .child(OR)
                        .into_any_element(),
                );
            }
            for (i, cap) in caps(keys, macos).into_iter().enumerate() {
                let one = cap.chars().count() == 1;
                out.push(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .when(i > 0, |d| d.ml(px(CAP_GAP)))
                        .h(px(CAP_HEIGHT))
                        .when(one, |d| d.w(px(CAP_WIDTH)))
                        .when(!one, |d| d.px(px(CAP_PAD)))
                        .rounded(px(CAP_RADIUS))
                        .border_1()
                        .border_color(border)
                        .text_color(text)
                        .font_weight(FontWeight::BOLD)
                        .child(cap)
                        .into_any_element(),
                );
            }
        }
        Some(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .pl(px(6.0))
                .pr(px(CAPS_RIGHT))
                .text_size(px(CAP_FONT))
                .children(out)
                .into_any_element(),
        )
    }

    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selected = self.selected.as_deref() == Some(row.key.as_str());
        let selection = if focused {
            theme.color(UiRole::Selection)
        } else {
            theme.color(UiRole::SelectionInactive)
        };
        let key = row.key.clone();
        // A group's arrow: whether it is open.
        let (arrow, icon, label, caps): (Option<bool>, Option<IconName>, String, _) =
            match &row.kind {
                RowKind::Group { name, open } => {
                    (Some(*open), Some(IconName::FOLDER), name.clone(), None)
                }
                RowKind::Action(a) => (
                    None,
                    None,
                    ActionTable::builtin().actions[*a].text.clone(),
                    self.render_caps(*a, selected && focused, cx),
                ),
            };
        let group = matches!(row.kind, RowKind::Group { .. });
        div()
            .id(("keymap-row", ix))
            .debug_selector({
                let label = label.clone();
                move || format!("keymap-row:{label}")
            })
            .h(px(ROW))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .pl(px(ROW_LEFT + INDENT * row.depth as f32))
            .when(selected, |d| d.bg(selection))
            .text_color(if selected && focused {
                theme.color(UiRole::SelectionText)
            } else {
                theme.color(UiRole::TreeText)
            })
            .child(
                div()
                    .flex_none()
                    .w(px(ARROW))
                    .children(arrow.map(|open| ui::components::tree_arrow(open, cx))),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(16.0 + ICON_GAP))
                    .children(icon.map(|i| Icon::new(i).size(px(16.0)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label),
            )
            .children(caps)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                window.focus(&this.focus);
                this.selected = Some(key.clone());
                if group {
                    this.toggle(&key, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_capture(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        let capture = self.capture.as_ref()?;
        let theme = cx.theme();
        let at = self.anchors.borrow().find.bottom_left();
        let at = point(
            (at.x - px(POPUP_WIDTH) + self.anchors.borrow().find.size.width).max(px(0.0)),
            at.y + px(2.0),
        );
        let text = self
            .shortcut
            .as_deref()
            .map(|k| {
                caps(k, cfg!(target_os = "macos")).join(if cfg!(target_os = "macos") {
                    ""
                } else {
                    "+"
                })
            })
            .unwrap_or_default();
        let focused = capture.focus.is_focused(window);
        Some(
            deferred(
                anchored().position(at).child(
                    div()
                        .id("keymap-find-shortcut")
                        .debug_selector(|| "keymap-find-shortcut".into())
                        .track_focus(&capture.focus)
                        .occlude()
                        .w(px(POPUP_WIDTH))
                        .flex()
                        .flex_col()
                        .bg(theme.color(UiRole::SurfaceRaised))
                        .border_1()
                        .border_color(theme.color(UiRole::PopupBorder))
                        .rounded(px(6.0))
                        .shadow(popup_shadow())
                        .child(
                            div()
                                .px(px(10.0))
                                .pt(px(6.0))
                                .pb(px(4.0))
                                .text_color(theme.color(UiRole::TextMuted))
                                .child(FIND_SHORTCUT),
                        )
                        .child(
                            div().p(px(4.0)).child(
                                input_frame(focused, false, cx)
                                    .h(px(ui::form::FIELD_HEIGHT))
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .debug_selector(|| "keymap-find-shortcut-text".into())
                                            .child(text),
                                    ),
                            ),
                        ),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

/// The shortcuts of every action of the table in `keymap`: the bindings
/// of its commands in their contexts, in the keymap's order, once each.
fn shortcuts_of(keymap: &Keymap) -> Vec<Vec<String>> {
    ActionTable::builtin()
        .actions
        .iter()
        .map(|action| {
            let mut keys: Vec<String> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            for b in &keymap.bindings {
                let bound = action.targets.iter().any(|t| {
                    t.command == b.command
                        && (t.args.is_none() || t.args == b.args)
                        && t.contexts.contains(&b.context)
                });
                let canonical = canonical_keys(&b.keys);
                if bound && !seen.contains(&canonical) {
                    seen.push(canonical);
                    keys.push(b.keys.clone());
                }
            }
            keys
        })
        .collect()
}

impl SettingsPage for KeymapPage {
    fn is_modified(&self, cx: &App) -> bool {
        self.chosen != keymaps::active_name(cx)
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        keymaps::switch(&self.chosen, cx)?;
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chosen = keymaps::active_name(cx);
        self.close_list(window, cx);
        self.close_capture(window, cx);
        self.load_shortcuts(cx);
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let table = ActionTable::builtin();
        let entries = keymaps::entries(cx);
        let based_on = entries
            .iter()
            .find(|e| e.name == self.chosen)
            .and_then(|e| e.parent.clone());
        let rows: Vec<Json> = self
            .rows
            .iter()
            .map(|row| match &row.kind {
                RowKind::Group { name, open } => json!({
                    "depth": row.depth,
                    "group": name,
                    "key": row.key,
                    "open": open,
                }),
                RowKind::Action(ix) => json!({
                    "depth": row.depth,
                    "action": table.actions[*ix].id,
                    "text": table.actions[*ix].text,
                    "shortcuts": self.shortcuts[*ix],
                }),
            })
            .collect();
        json!({
            "keymap": self.chosen,
            "active": keymaps::active_name(cx),
            "keymaps": entries.iter().map(|e| json!({
                "name": e.name,
                "predefined": e.predefined,
                "parent": e.parent,
            })).collect::<Vec<_>>(),
            "based_on": based_on,
            "search": self.filter,
            "shortcut": self.shortcut,
            "find_shortcut_open": self.capture.is_some(),
            "selected": self.selected,
            "rows": rows,
            "list": self.list.as_ref().map(|(m, _)| m.read(cx).state_json()),
        })
    }

    fn edit(
        &mut self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = || {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{field}` takes a string"))
        };
        match field {
            "keymap" => self.choose(&text()?, cx)?,
            // Opens (`true`) or closes the combo box's list.
            "list" => match value.as_bool() {
                Some(true) => self.open_list(window, cx),
                _ => self.close_list(window, cx),
            },
            "search" => self.search_for(&text()?, window, cx),
            // Keystrokes in GPUI's syntax (`cmd-/`), `null` to clear.
            "shortcut" => match value {
                Json::Null => self.set_shortcut(None, cx),
                _ => self.set_shortcut(Some(text()?), cx),
            },
            // Opens (`true`) or closes the Find Shortcut popup.
            "find-shortcut" => match value.as_bool() {
                Some(true) => self.open_capture(window, cx),
                _ => self.close_capture(window, cx),
            },
            // A group by its path (`Main Menu/Edit`) opens or closes.
            "toggle" => self.toggle(&text()?, cx),
            "expand-all" => self.expand_all(cx),
            "collapse-all" => self.collapse_all(cx),
            _ => return Err(format!("no field `{field}` on this page")),
        }
        cx.notify();
        Ok(())
    }

    fn search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search_for(text, window, cx);
    }

    fn has_search(&self) -> bool {
        true
    }
}

impl Render for KeymapPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let border = theme.color(UiRole::Border);
        let focus_color = theme.color(UiRole::Focus);
        let entries = keymaps::entries(cx);
        let based_on = entries
            .iter()
            .find(|e| e.name == self.chosen)
            .and_then(|e| e.parent.clone());
        let anchors = self.anchors.clone();
        let combo = ui::form::combo(
            "keymap-combo",
            vec![
                div()
                    .child(SharedString::from(self.chosen.clone()))
                    .into_any_element(),
            ],
            cx,
        )
        .debug_selector(|| "keymap-combo".into())
        .h(px(COMBO_HEIGHT))
        .w(px(COMBO_WIDTH))
        .when(self.list.is_some(), |d| d.border_color(focus_color))
        .child(
            canvas(
                move |bounds, _, _| anchors.borrow_mut().combo = bounds,
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            if this.list.is_some() {
                this.close_list(window, cx);
            } else {
                this.open_list(window, cx);
            }
        }));
        let combo_row = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .child(combo)
            .children(based_on.map(|parent| {
                div()
                    .debug_selector(|| "keymap-based-on".into())
                    .pl(px(INFO_GAP))
                    .text_color(muted)
                    .child(BASED_ON.replace("{0}", &parent))
            }));
        let link = div().flex().flex_none().pt(px(LINK_TOP)).child(
            ui::link::link("keymap-import", IMPORT_KEYMAP, cx)
                .debug_selector(|| "keymap-import".into())
                .on_click(|_: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(ImportKeymap { path: None }), cx)
                }),
        );
        let search_focused = self.search.focus_handle(cx).is_focused(window);
        let anchors = self.anchors.clone();
        let filtering_by_shortcut = self.shortcut.is_some();
        let toolbar = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .mt(px(TOOLBAR_TOP))
            .h(px(TOOLBAR_HEIGHT))
            .border_t_1()
            .border_color(border)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        small_icon_button("keymap-expand-all", IconName::EXPAND_ALL, true, cx)
                            .debug_selector(|| "keymap-expand-all".into())
                            .tooltip(Tooltip::text(EXPAND_ALL, None))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.expand_all(cx)),
                            ),
                    )
                    .child(
                        small_icon_button("keymap-collapse-all", IconName::COLLAPSE_ALL, true, cx)
                            .debug_selector(|| "keymap-collapse-all".into())
                            .tooltip(Tooltip::text(COLLAPSE_ALL, None))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(cx)),
                            ),
                    ),
            )
            .child(
                input_frame(search_focused, false, cx)
                    .debug_selector(|| "keymap-search".into())
                    .w(px(SEARCH_WIDTH))
                    .h(px(ui::form::FIELD_HEIGHT))
                    .pl(px(6.0))
                    .pr(px(6.0))
                    .gap(px(4.0))
                    .flex()
                    .items_center()
                    .child(Icon::new(IconName::SEARCH).size(px(16.0)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(editor::field_line_height_for(true, window, cx))
                            .overflow_hidden()
                            .child(self.search.clone()),
                    ),
            )
            .child(
                small_icon_button(
                    "keymap-find-by-shortcut",
                    IconName::SHORTCUT_FILTER,
                    true,
                    cx,
                )
                .debug_selector(|| "keymap-find-by-shortcut".into())
                .ml(px(6.0))
                .relative()
                .tooltip(Tooltip::text(FIND_BY_SHORTCUT, None))
                .child(
                    canvas(
                        move |bounds, _, _| anchors.borrow_mut().find = bounds,
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.open_capture(window, cx)),
                ),
            )
            .child(
                small_icon_button(
                    "keymap-clear-filtering",
                    IconName::CLOSE_SMALL,
                    filtering_by_shortcut,
                    cx,
                )
                .debug_selector(|| "keymap-clear-filtering".into())
                .tooltip(Tooltip::text(CLEAR_FILTERING, None))
                .when(filtering_by_shortcut, |d| {
                    d.on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.set_shortcut(None, cx)),
                    )
                }),
            );
        let count = self.rows.len();
        let list = uniform_list(
            "keymap-tree",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.focus.contains_focused(window, cx);
                let rows = this.rows.clone();
                range
                    .filter_map(|i| rows.get(i).map(|row| this.render_row(i, row, focused, cx)))
                    .collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "keymap-tree".into())
        .track_scroll(self.scroll.clone())
        .size_full();
        let tree = div()
            .id("keymap-tree-box")
            .debug_selector(|| "keymap-tree-box".into())
            .track_focus(&self.focus)
            .flex_1()
            .min_h(px(120.0))
            .border_1()
            .border_color(border)
            .child(list);
        let list_overlay = self.list.as_ref().map(|(menu, _)| {
            let at = self.anchors.borrow().combo.bottom_left();
            ui::menu::overlay(menu, at, window)
        });
        let capture = self.render_capture(window, cx);
        div()
            .id("keymap-page")
            .debug_selector(|| "keymap-page".into())
            .size_full()
            .flex()
            .flex_col()
            .child(combo_row)
            .child(link)
            .child(toolbar)
            .child(tree)
            .children(list_overlay)
            .children(capture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_caps_in_both_styles() {
        assert_eq!(caps("cmd-shift-a", true), ["⇧", "⌘", "A"]);
        assert_eq!(caps("ctrl-alt-s", false), ["Ctrl", "Alt", "S"]);
        assert_eq!(caps("alt-space", true), ["⌥", "Space"]);
        assert_eq!(caps("ctrl-k ctrl-c", false), ["Ctrl", "K", "Ctrl", "C"]);
        assert_eq!(caps("shift shift", true), ["Double ⇧"]);
        assert_eq!(caps("button4", false), ["Button 4 Click"]);
    }

    #[test]
    fn the_tree_follows_the_reference_groups() {
        let names: Vec<&str> = tree().iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Editor Actions", "Main Menu", "Other"]);
        let menus: Vec<&str> = tree()[1]
            .children
            .iter()
            .filter_map(|c| match c {
                Child::Group(g) => Some(g.name.as_str()),
                Child::Action(_) => None,
            })
            .collect();
        assert_eq!(
            menus,
            [
                "File", "Edit", "View", "Navigate", "Code", "Refactor", "Run", "Window", "Help"
            ]
        );
        // Editor Actions by action id.
        let table = ActionTable::builtin();
        let ids: Vec<&str> = tree()[0]
            .children
            .iter()
            .map(|c| child_id(c, table))
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(search_terms().contains(&"Duplicate Line or Selection"));
    }

    #[test]
    fn shortcuts_of_actions_in_their_contexts() {
        let keymap = Keymap::parse(crate::keymap::MACOS).unwrap();
        let shortcuts = shortcuts_of(&keymap);
        let table = ActionTable::builtin();
        let of = |id: &str| {
            let ix = table.actions.iter().position(|a| a.id == id).unwrap();
            shortcuts[ix].clone()
        };
        assert_eq!(of("CommentByLineComment"), ["cmd-/"]);
        assert_eq!(of("ShowSettings"), ["cmd-,"]);
        // Enter in the find bar is not Find Next.
        assert!(!of("FindNext").contains(&"enter".to_owned()));
        assert!(of("Back").contains(&"button4".to_owned()));
        assert_eq!(first_stroke("ctrl-k ctrl-c"), "ctrl-k");
        assert_eq!(first_stroke("shift shift"), "shift shift");
    }
}
