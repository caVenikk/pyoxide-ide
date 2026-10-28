//! The actions of a value that open a window of their own (the reference's
//! `XDebugger.ValueGroup`):
//!
//! - Inspect… (`XInspectAction`, `XInspectDialog`): a non-modal dialog with
//!   the value's tree under a toolbar (Set As Root, Back, Forward,
//!   `DebuggerTreeWithHistoryPanel`); it shows the value again when the
//!   program stops next;
//! - Compare Value with Clipboard (`XCompareWithClipboardAction`): a diff of
//!   the clipboard's text and the value's text in a window;
//! - Show Referring Objects… (`XShowReferringObjectsAction`): the dialog of
//!   Inspect with the objects that refer to the value as its children
//!   (title "Referring Objects For"), where the product's runtime can tell.

use std::path::PathBuf;
use std::sync::Arc;

use debugger::{DebugAdapterProvider, EvaluateContext, Phase, Session};
use editor::{Editor, EditorConfig};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Task, Window, WindowHandle, div, px, size,
};
use serde_json::{Value as Json, json};
use text::Buffer;
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, Tooltip, icon_button};
use ui::icons::IconName;
use ui::theme::ThemeExt as _;
use vcs_ui::diff::{DiffRequest, DiffView, ScrollTo, SideTitle};

use crate::session::commands::{
    AddToWatches, CancelEdit, CommitEdit, CopyName, CopyValue, JumpToTypeSource, JumpToValueSource,
    SetValue, ViewAs,
};
use crate::variables::{PopupRoot, VariablesView};
use crate::view::DebuggerView;
use crate::{Highlighters, icons};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Inspect….
    InspectValue => "debug.variables.inspect", "Inspect\u{2026}",
        "Opens the selected value of the Variables view (or of a value's popup) with its tree in a dialog of its own, which shows it again when the program stops next.";
    /// Compare Value with Clipboard.
    CompareWithClipboard => "debug.variables.compare-with-clipboard", "Compare Value with Clipboard",
        "Shows the differences between the clipboard's text and the selected value's text in a window.";
    /// Show Referring Objects….
    ShowReferringObjects => "debug.variables.show-referring-objects", "Show Referring Objects\u{2026}",
        "Opens a dialog with the objects that refer to the selected value as the children of the value (the Python runtime's garbage collector tells).";
}

ui::ui_commands! {
    DIALOG_COMMANDS, scope = "InspectDialog";
    /// Set As Root.
    SetAsRoot => "debug.inspect.set-as-root", "Set As Root",
        "Shows the selected node of the Inspect dialog's tree as the root of a new tree, after the one shown (the history's next step).";
    /// Back.
    InspectBack => "debug.inspect.back", "Back",
        "Goes back to the tree shown before in the Inspect dialog.";
    /// Forward.
    InspectForward => "debug.inspect.forward", "Forward",
        "Goes forward to the tree of the Inspect dialog's history that was left with Back.";
}

/// Registers the commands.
pub(crate) fn register(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, DIALOG_COMMANDS);
}

/// The history's length (`DebuggerTreeWithHistoryContainer.HISTORY_SIZE`).
const HISTORY_SIZE: usize = 11;
/// The dialog's content size, and the smallest it can be resized to.
const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 380.0;
const MIN_WIDTH: f32 = 300.0;
const MIN_HEIGHT: f32 = 160.0;
/// The toolbar's height and its buttons.
const TOOLBAR: f32 = 30.0;
const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;

/// A value to show in a window of its own: what the tree's root is and
/// where it is evaluated.
pub(crate) struct InspectTarget {
    pub root: PopupRoot,
    /// The root's expression, evaluated again at the next stop.
    pub expression: Option<String>,
    pub session: Session,
    pub frame: i64,
    pub location: Option<(PathBuf, u32)>,
    pub provider: Option<Arc<dyn DebugAdapterProvider>>,
    pub highlighters: Option<Highlighters>,
}

impl VariablesView {
    /// The selected value as the root of a window's tree.
    pub(crate) fn inspect_target(&mut self, cx: &mut Context<Self>) -> Option<InspectTarget> {
        let frame = self.frame_id()?;
        let row = self.selected_row(cx)?;
        let v = row.value()?;
        let root = PopupRoot {
            name: v.name.clone(),
            variable: v.variable.clone(),
            shape: v.shape.clone(),
            kind: v.kind,
            parent: v.parent,
        };
        let location = self
            .frame
            .as_ref()
            .and_then(|f| f.file.clone().map(|file| (file, f.line)));
        Some(InspectTarget {
            root,
            expression: self.selected_expression(cx),
            session: self.session.clone(),
            frame,
            location,
            provider: self.provider.clone(),
            highlighters: self.highlighters.clone(),
        })
    }

    /// The selected value's whole text, fetched when the adapter has more
    /// than the row shows (the text Copy Value puts on the clipboard).
    pub(crate) fn fetch_value(&mut self, cx: &mut Context<Self>) -> Option<Task<String>> {
        let row = self.selected_row(cx)?;
        let v = row.value()?.clone();
        let shown = v.variable.value.clone();
        let again = matches!(row.kind, crate::variables::RowKind::Value(_));
        let whole = match (&v.variable.evaluate_name, self.frame_id()) {
            (Some(name), Some(frame)) if again && self.session.evaluates_for_clipboard() => Some(
                self.session
                    .evaluate_in(name, Some(frame), EvaluateContext::Clipboard),
            ),
            _ => None,
        };
        Some(cx.spawn(async move |_, _| match whole {
            Some(pending) => pending.await.map(|e| e.value).unwrap_or(shown),
            None => shown,
        }))
    }
}

impl DebuggerView {
    /// The tree the command is for: the dialog's, or the open value popup's
    /// or the selected tab's.
    pub(crate) fn with_tree(
        &mut self,
        tree: Option<&Entity<VariablesView>>,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut VariablesView, &mut Context<VariablesView>),
    ) {
        match tree {
            Some(tree) => tree.update(cx, f),
            None => self.with_variables(cx, f),
        }
    }

    /// Inspect….
    pub(crate) fn inspect(
        &mut self,
        tree: Option<&Entity<VariablesView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut target = None;
        self.with_tree(tree, cx, |t, cx| target = t.inspect_target(cx));
        let Some(target) = target else {
            return;
        };
        let title = format!("Inspect '{}'", target.root.name);
        let host = cx.entity();
        if let Some(dialog) = InspectDialog::open(host, title, target, self.activate, window, cx) {
            self.inspects.retain(|d| d.read(cx).is_ok());
            self.inspects.push(dialog);
        }
    }

    /// Show Referring Objects….
    pub(crate) fn show_referring_objects(
        &mut self,
        tree: Option<&Entity<VariablesView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut target = None;
        self.with_tree(tree, cx, |t, cx| target = t.inspect_target(cx));
        let Some(mut target) = target else {
            return;
        };
        let asked = target
            .provider
            .as_ref()
            .zip(target.expression.as_deref())
            .and_then(|(p, e)| p.referrers_expression(e));
        let Some(asked) = asked else {
            return;
        };
        let pending =
            target
                .session
                .evaluate_in(&asked, Some(target.frame), EvaluateContext::Watch);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(referrers) = pending.await else {
                return;
            };
            if referrers.reference <= 0 {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                // The value itself, whose children are its referrers
                // (`PyReferringObjectsValue`).
                target.root.variable.reference = referrers.reference;
                target.expression = None;
                let title = format!("Referring Objects For {}", target.root.name);
                let host = cx.entity();
                if let Some(dialog) =
                    InspectDialog::open(host, title, target, this.activate, window, cx)
                {
                    this.inspects.retain(|d| d.read(cx).is_ok());
                    this.inspects.push(dialog);
                }
            });
        })
        .detach();
    }

    /// Compare Value with Clipboard.
    pub(crate) fn compare_with_clipboard(
        &mut self,
        tree: Option<&Entity<VariablesView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut fetch = None;
        self.with_tree(tree, cx, |t, cx| fetch = t.fetch_value(cx));
        let Some(fetch) = fetch else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let value = fetch.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let clipboard = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .unwrap_or_default();
                CompareDialog::open(clipboard, value, this.activate, window, cx);
            });
        })
        .detach();
    }

    /// The open Inspect dialogs as `debug.state` shows them.
    pub(crate) fn inspects_json(&self, cx: &App) -> Json {
        Json::Array(
            self.inspects
                .iter()
                .filter_map(|d| d.read(cx).ok())
                .map(|d| d.state_json(cx))
                .collect(),
        )
    }
}

/// The commands of a value's window actions, at the window's root.
pub(crate) fn forward_actions<E: InteractiveElement>(el: E, view: Entity<DebuggerView>) -> E {
    let v = view.clone();
    let el = el.on_action(move |_: &InspectValue, window, cx| {
        v.update(cx, |v, cx| v.inspect(None, window, cx))
    });
    let v = view.clone();
    let el = el.on_action(move |_: &CompareWithClipboard, window, cx| {
        v.update(cx, |v, cx| v.compare_with_clipboard(None, window, cx))
    });
    let v = view;
    el.on_action(move |_: &ShowReferringObjects, window, cx| {
        v.update(cx, |v, cx| v.show_referring_objects(None, window, cx))
    })
}

/// A step of the dialog's history: a root and the expression that
/// evaluates to it.
#[derive(Clone)]
struct Entry {
    root: PopupRoot,
    expression: Option<String>,
}

/// A tree of a step of the history, in the frame.
fn build_tree(
    session: &Session,
    provider: &Option<Arc<dyn DebugAdapterProvider>>,
    frame: i64,
    location: &Option<(PathBuf, u32)>,
    highlighters: &Option<Highlighters>,
    entry: &Entry,
    cx: &mut Context<InspectDialog>,
) -> Entity<VariablesView> {
    let (session, provider, location, highlighters) = (
        session.clone(),
        provider.clone(),
        location.clone(),
        highlighters.clone(),
    );
    let root = entry.root.clone();
    cx.new(|cx| {
        let mut view = VariablesView::for_popup(session, provider, frame, location, root, cx);
        view.highlighters = highlighters;
        view.in_dialog = true;
        view
    })
}

/// The Inspect dialog (`XInspectDialog`).
pub(crate) struct InspectDialog {
    host: Entity<DebuggerView>,
    session: Session,
    provider: Option<Arc<dyn DebugAdapterProvider>>,
    highlighters: Option<Highlighters>,
    frame: i64,
    location: Option<(PathBuf, u32)>,
    title: SharedString,
    history: Vec<Entry>,
    current: usize,
    tree: Entity<VariablesView>,
    focus: FocusHandle,
    /// The session's generation the tree is for.
    generation: u64,
    _refresh: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl InspectDialog {
    pub(crate) fn open(
        host: Entity<DebuggerView>,
        title: String,
        target: InspectTarget,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<DebuggerView>,
    ) -> Option<WindowHandle<Self>> {
        let title_height = ui::dialog_window::title_height(true);
        let options = ui::dialog_window::DialogOptions {
            title: title.clone().into(),
            size: size(px(WIDTH), px(HEIGHT + title_height)),
            min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT + title_height))),
            activate,
            themed_title: true,
            modal: false,
            key: None,
        };
        ui::dialog_window::open(window, options, cx, move |window, cx| {
            cx.new(|cx| Self::new(host, title.into(), target, window, cx))
        })
    }

    fn new(
        host: Entity<DebuggerView>,
        title: SharedString,
        target: InspectTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let InspectTarget {
            root,
            expression,
            session,
            frame,
            location,
            provider,
            highlighters,
        } = target;
        let generation = session.snapshot().generation;
        let first = Entry { root, expression };
        let tree = build_tree(
            &session,
            &provider,
            frame,
            &location,
            &highlighters,
            &first,
            cx,
        );
        window.focus(&tree.focus_handle(cx));
        Self {
            host: host.clone(),
            session,
            provider,
            highlighters,
            frame,
            location,
            title,
            history: vec![first],
            current: 0,
            tree,
            focus: cx.focus_handle(),
            generation,
            _refresh: None,
            // The program stopped again: the value is shown again
            // (`XInspectDialog`'s `sessionPaused`).
            _subscriptions: vec![cx.observe(&host, |this, _, cx| this.host_changed(cx))],
        }
    }

    fn make_tree(&self, entry: &Entry, cx: &mut Context<Self>) -> Entity<VariablesView> {
        build_tree(
            &self.session,
            &self.provider,
            self.frame,
            &self.location,
            &self.highlighters,
            entry,
            cx,
        )
    }

    /// Shows the history's step `index`.
    fn go(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.history.get(index).cloned() else {
            return;
        };
        self.current = index;
        self.tree = self.make_tree(&entry, cx);
        window.focus(&self.tree.focus_handle(cx));
        cx.notify();
    }

    fn can_back(&self) -> bool {
        self.current > 0
    }

    fn can_forward(&self) -> bool {
        self.current + 1 < self.history.len()
    }

    /// The selected node's value, when it can be the root of a tree
    /// (`SetAsRootAction.update`: a node below the root that has children).
    fn root_candidate(&self, cx: &mut Context<Self>) -> Option<Entry> {
        let current = self.history.get(self.current)?;
        self.tree.update(cx, |t, cx| {
            let row = t.selected_row(cx)?;
            let v = row.value()?;
            if v.variable.reference <= 0 || row.path == current.root.name {
                return None;
            }
            Some(Entry {
                root: PopupRoot {
                    name: v.name.clone(),
                    variable: v.variable.clone(),
                    shape: v.shape.clone(),
                    kind: v.kind,
                    parent: v.parent,
                },
                expression: t.selected_expression(cx),
            })
        })
    }

    fn set_as_root(&mut self, _: &SetAsRoot, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.root_candidate(cx) else {
            return;
        };
        // What was ahead in the history is gone.
        self.history.truncate(self.current + 1);
        if self.history.len() >= HISTORY_SIZE {
            self.history.remove(0);
        }
        self.history.push(entry);
        let last = self.history.len() - 1;
        self.go(last, window, cx);
    }

    fn back(&mut self, _: &InspectBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_back() {
            self.go(self.current - 1, window, cx);
        }
    }

    fn forward(&mut self, _: &InspectForward, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_forward() {
            self.go(self.current + 1, window, cx);
        }
    }

    /// The host changed: when the program stopped again the root is
    /// evaluated in the new frame and the tree built again.
    fn host_changed(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.session.snapshot();
        if snapshot.generation == self.generation {
            return;
        }
        self.generation = snapshot.generation;
        if snapshot.phase != Phase::Suspended {
            return;
        }
        let Some(frame) = snapshot.selected_frame else {
            return;
        };
        let Some(expression) = self
            .history
            .get(self.current)
            .and_then(|e| e.expression.clone())
        else {
            return;
        };
        self.frame = frame;
        self.location = snapshot
            .frames
            .iter()
            .find(|f| f.id == frame)
            .and_then(|f| {
                f.source
                    .as_ref()
                    .and_then(|s| s.path.clone())
                    .map(|p| (p, f.line))
            });
        let evaluation = crate::watches::evaluate(
            self.session.clone(),
            self.provider.clone(),
            frame,
            expression,
            EvaluateContext::Watch,
        );
        let generation = self.generation;
        self._refresh = Some(cx.spawn(async move |this, cx| {
            let eval = evaluation.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let crate::watches::Eval::Value { variable, shape } = eval else {
                    return;
                };
                let Some(entry) = this.history.get_mut(this.current) else {
                    return;
                };
                entry.root.variable = *variable;
                entry.root.shape = shape;
                let entry = entry.clone();
                this.tree = this.make_tree(&entry, cx);
                cx.notify();
            });
        }));
    }

    /// The dialog as `debug.state` shows it.
    pub(crate) fn state_json(&self, cx: &App) -> Json {
        let history: Vec<&str> = self.history.iter().map(|e| e.root.name.as_str()).collect();
        json!({
            "title": self.title,
            "history": history,
            "current": self.current,
            "can_back": self.can_back(),
            "can_forward": self.can_forward(),
            "tree": self.tree.read(cx).state_json(cx),
        })
    }

    fn tree_action(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut VariablesView, &mut Context<VariablesView>),
    ) {
        self.tree.update(cx, f);
    }

    fn button(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        enabled: bool,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
            .debug_selector(move || id.to_owned())
            .tooltip(Tooltip::text(tooltip, None))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
            })
            .into_any_element()
    }
}

impl Focusable for InspectDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let _ = &self.focus;
        self.tree.focus_handle(cx)
    }
}

impl Render for InspectDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root_enabled = self.root_candidate(cx).is_some();
        let (back, forward) = (self.can_back(), self.can_forward());
        let buttons = vec![
            self.button(
                "inspect-set-as-root",
                icons::SET_AS_ROOT,
                "Set As Root",
                root_enabled,
                |this, window, cx| this.set_as_root(&SetAsRoot, window, cx),
                cx,
            ),
            self.button(
                "inspect-back",
                IconName::BACK,
                "Back",
                back,
                |this, window, cx| this.back(&InspectBack, window, cx),
                cx,
            ),
            self.button(
                "inspect-forward",
                IconName::FORWARD,
                "Forward",
                forward,
                |this, window, cx| this.forward(&InspectForward, window, cx),
                cx,
            ),
        ];
        let theme = cx.theme();
        let tree = self.tree.clone();
        let host = self.host.clone();
        div()
            .id("inspect-dialog")
            .key_context("InspectDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::set_as_root))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::forward))
            // The value's actions act on the dialog's tree.
            .on_action(cx.listener(|this, _: &CopyValue, _, cx| {
                this.tree_action(cx, |t, cx| t.copy_value(cx))
            }))
            .on_action(cx.listener(|this, _: &CopyName, _, cx| {
                this.tree_action(cx, |t, cx| t.copy_name(cx))
            }))
            .on_action(cx.listener(|this, a: &SetValue, window, cx| {
                let value = a.value.clone();
                this.tree_action(cx, |t, cx| t.set_value(value, window, cx))
            }))
            .on_action(cx.listener(|this, _: &CommitEdit, window, cx| {
                this.tree_action(cx, |t, cx| t.commit_edit(window, cx))
            }))
            .on_action(cx.listener(|this, _: &CancelEdit, window, cx| {
                this.tree_action(cx, |t, cx| t.cancel_edit(window, cx))
            }))
            .on_action(cx.listener(|this, a: &ViewAs, _, cx| {
                let renderer = a.renderer.clone();
                this.tree_action(cx, |t, cx| t.view_as(&renderer, cx))
            }))
            .on_action(cx.listener(|this, _: &AddToWatches, _, cx| {
                this.tree_action(cx, |t, cx| t.add_to_watches(cx))
            }))
            .on_action({
                let (host, tree) = (host.clone(), tree.clone());
                move |_: &InspectValue, window, cx| {
                    host.update(cx, |h, cx| h.inspect(Some(&tree), window, cx))
                }
            })
            .on_action({
                let (host, tree) = (host.clone(), tree.clone());
                move |_: &CompareWithClipboard, window, cx| {
                    host.update(cx, |h, cx| {
                        h.compare_with_clipboard(Some(&tree), window, cx)
                    })
                }
            })
            .on_action({
                let (host, tree) = (host.clone(), tree.clone());
                move |_: &ShowReferringObjects, window, cx| {
                    host.update(cx, |h, cx| {
                        h.show_referring_objects(Some(&tree), window, cx)
                    })
                }
            })
            .on_action({
                let (host, tree) = (host.clone(), tree.clone());
                move |_: &JumpToValueSource, _, cx| {
                    host.update(cx, |h, cx| h.jump_to_value_source(Some(&tree), cx))
                }
            })
            .on_action({
                let (host, tree) = (host, tree.clone());
                move |_: &JumpToTypeSource, _, cx| {
                    host.update(cx, |h, cx| h.jump_to_type_source(Some(&tree), cx))
                }
            })
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .children(ui::dialog_window::title_bar(self.title.clone(), true, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .h(px(TOOLBAR))
                    .px(px(4.0))
                    .gap(px(2.0))
                    .children(buttons),
            )
            .child(div().flex_1().min_h(px(0.0)).child(tree))
            .child(ui::pointer::frame())
    }
}

/// The window of Compare Value with Clipboard: a diff of the clipboard's
/// text (left) and the value's text (right, read-only), as the reference
/// shows it (`DiffRequestFactory.createClipboardVsValue`).
pub(crate) struct CompareDialog {
    diff: Entity<DiffView>,
}

/// The reference's strings (`DiffBundle`).
const COMPARE_TITLE: &str = "Clipboard vs Selected Value";
const CLIPBOARD_TITLE: &str = "From clipboard";
const VALUE_TITLE: &str = "Selected Value";
const COMPARE_WIDTH: f32 = 900.0;
const COMPARE_HEIGHT: f32 = 560.0;

impl CompareDialog {
    pub(crate) fn open(
        clipboard: String,
        value: String,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<DebuggerView>,
    ) -> Option<WindowHandle<Self>> {
        let title_height = ui::dialog_window::title_height(true);
        let options = ui::dialog_window::DialogOptions {
            title: COMPARE_TITLE.into(),
            size: size(px(COMPARE_WIDTH), px(COMPARE_HEIGHT + title_height)),
            min_size: Some(size(px(400.0), px(240.0))),
            activate,
            themed_title: true,
            modal: false,
            key: None,
        };
        ui::dialog_window::open(window, options, cx, move |window, cx| {
            cx.new(|cx| Self::new(clipboard, value, window, cx))
        })
    }

    fn new(clipboard: String, value: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let right = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(&value), window, cx);
            e.buffer_mut().set_read_only(true);
            e
        });
        let diff = cx.new(|cx| DiffView::new(right, window, cx));
        let side = |label: &str| SideTitle {
            label: label.to_owned().into(),
            path: None,
            tooltip: None,
            read_only: true,
        };
        let request = DiffRequest {
            path: PathBuf::from("value.txt"),
            title: COMPARE_TITLE.into(),
            left: side(CLIPBOARD_TITLE),
            right: side(VALUE_TITLE),
            included: None,
            excluded: Vec::new(),
            one_side: None,
            files: (1, 1),
            steps: (false, false),
            preview: false,
        };
        diff.update(cx, |d, cx| {
            d.set_request(
                request,
                Some(Arc::from(clipboard.as_str())),
                None,
                EditorConfig::default(),
                ScrollTo::FirstChange,
                cx,
            )
        });
        window.focus(&diff.focus_handle(cx));
        Self { diff }
    }
}

impl Focusable for CompareDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.diff.focus_handle(cx)
    }
}

impl Render for CompareDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("compare-dialog")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .children(ui::dialog_window::title_bar(COMPARE_TITLE, true, cx))
            .child(div().flex_1().min_h(px(0.0)).child(self.diff.clone()))
            .child(ui::pointer::frame())
    }
}
