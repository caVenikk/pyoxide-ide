//! The Settings window (`SettingsEditor` in `SettingsNonModalDialog`): the
//! page tree with its search on the left (`tree.rs`), the banner over the
//! page (bold breadcrumbs, the project mark, "Revert changes" when the
//! page is modified, Back, Forward and Pin Window), the page and the
//! bottom bar with "?" and Cancel, Apply and OK in the platform's order.
//! One window per application, non-modal to the window it was opened
//! from; its size, splitter, last page and search history last across
//! sessions (`settings-window.toml`).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, CursorStyle, DismissEvent,
    Entity, FocusHandle, Focusable, FontWeight, HitboxBehavior, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Render, ScrollHandle, SharedString, Size, Subscription,
    Task, Window, WindowHandle, canvas, div, prelude::*, px, size,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::Tooltip;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use crate::commands::{
    Apply, Back, Cancel, Collapse, Confirm, EditField, Escape, Expand, Find, Forward, GoToPage,
    OpenSettings, Revert, Search, SearchHistory, SelectNext, SelectPage, SelectPrevious,
};
use crate::history::History;
use crate::page::{PageContext, PageHandle, PageSpec};
use crate::tree::Filter;
use crate::window_state;

/// Key of the window among the non-modal dialogs: one per application.
pub const DIALOG_KEY: &str = "settings";
/// Key context of the window.
pub const KEY_CONTEXT: &str = "SettingsWindow";
/// The window's content size, initial and smallest.
pub const WIDTH: f32 = 900.0;
pub const HEIGHT: f32 = 700.0;
/// The tree's share of the width (`SPLITTER_PROPORTION_DEFAULT_VALUE`).
pub const SPLIT: f32 = 0.2;
/// The tree is at least this wide: the reference's splitter honours the
/// tree's preferred width (its widest row).
pub(crate) const TREE_MIN_WIDTH: f32 = 240.0;
/// The page keeps at least this much of the width when the splitter is
/// dragged.
const PAGE_MIN_WIDTH: f32 = 300.0;
/// Width of the splitter's handle over the one-pixel divider.
const SPLITTER_HANDLE: f32 = 6.0;
/// Insets: the banner (11, 6, 0, 10 within the page's 16 left inset), the
/// page's content (11, 16, 16, 16) and the error under it (10, 15, 15, 15).
const BANNER_TOP: f32 = 11.0;
const BANNER_HEIGHT: f32 = 26.0;
pub(crate) const PAGE_X: f32 = 16.0;
const PAGE_TOP: f32 = 11.0;
/// Gap between the breadcrumbs and "Revert changes".
const REVERT_GAP: f32 = 16.0;
/// The history toolbar's buttons (`ActionToolbar` in the banner).
const BANNER_BUTTON: f32 = 22.0;

/// The breadcrumb separator.
const CRUMB_SEPARATOR: &str = "\u{203a}";
/// The banner's reset link (`configurable.reset.action.name`).
pub const REVERT: &str = "Revert changes";
/// The project mark's tooltip (`configurable.current.project.tooltip`).
pub const PROJECT_MARK: &str = "For current project";

/// How the window is opened.
#[derive(Debug, Clone, Default)]
pub struct OpenRequest {
    /// The project's name for the title (`Settings – <project>`).
    pub project_name: Option<String>,
    pub context: PageContext,
    /// The page by id; `None`: the last one (first time: Appearance).
    pub page: Option<String>,
    /// Search for this text (`settings.open {filter}`).
    pub filter: Option<String>,
    /// Give the window the focus (off in the background).
    pub activate: bool,
    /// Modal to the window it opens over (from the first-run window, as
    /// the reference's welcome screen opens it); else non-modal.
    pub modal: bool,
}

/// The window's title for a project name.
pub fn title(project: Option<&str>) -> String {
    match project {
        Some(name) => format!("Settings \u{2013} {name}"),
        None => "Settings".into(),
    }
}

/// Opens the Settings window over `parent`, or brings the open one to the
/// front and selects the requested page there. One open for another
/// project closes first, after Apply / Don't Save / Cancel when it has
/// changes (then the new one opens after the answer).
pub fn open(
    parent: &mut Window,
    request: OpenRequest,
    cx: &mut App,
) -> Option<WindowHandle<SettingsWindow>> {
    if let Some(open) = find(cx) {
        let same = open
            .read(cx)
            .map(|v| v.context.project == request.context.project)
            .unwrap_or(true);
        if same {
            let _ = open.update(cx, |view, window, cx| {
                view.navigate(
                    request.page.as_deref(),
                    request.filter.as_deref(),
                    window,
                    cx,
                );
                if request.activate {
                    window.activate_window();
                }
            });
            return Some(open);
        }
        let modified = open.read(cx).is_ok_and(|v| v.any_modified(cx));
        if !modified {
            let _ = open.update(cx, |view, window, cx| view.close(window, cx));
            return open_new(parent, request, cx);
        }
        let parent_handle = parent.window_handle();
        let to = request.project_name.clone().unwrap_or_default();
        crate::leave::resolve(crate::leave::Leaving::Switch(to), cx, move |cx| {
            let _ = parent_handle.update(cx, |_, window, cx| {
                open_new(window, request, cx);
            });
        });
        return Some(open);
    }
    open_new(parent, request, cx)
}

fn open_new(
    parent: &mut Window,
    request: OpenRequest,
    cx: &mut App,
) -> Option<WindowHandle<SettingsWindow>> {
    let title_text = title(request.project_name.as_deref());
    let saved = window_state::get(cx);
    let title_height = ui::dialog_window::title_height(true);
    let width = saved.width.unwrap_or(WIDTH).max(WIDTH);
    let height = saved
        .height
        .unwrap_or(HEIGHT + title_height)
        .max(HEIGHT + title_height);
    let options = ui::dialog_window::DialogOptions {
        title: title_text.into(),
        size: size(px(width), px(height)),
        min_size: Some(size(px(WIDTH), px(HEIGHT))),
        activate: request.activate,
        themed_title: true,
        modal: request.modal,
        key: (!request.modal).then(|| DIALOG_KEY.into()),
    };
    ui::dialog_window::open(parent, options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(request, window, cx))
    })
}

/// The open Settings window.
pub fn find(cx: &App) -> Option<WindowHandle<SettingsWindow>> {
    ui::dialog_window::find(DIALOG_KEY, cx)?.downcast::<SettingsWindow>()
}

pub struct SettingsWindow {
    title: SharedString,
    pub(crate) project_name: Option<String>,
    context: PageContext,
    /// Every page in tree order (depth first, siblings by weight).
    pub(crate) pages: Vec<PageSpec>,
    pub(crate) depths: Vec<usize>,
    pub(crate) expanded: HashSet<&'static str>,
    pub(crate) selected: usize,
    /// Views of the pages shown so far, with their drafts.
    views: HashMap<&'static str, (PageHandle, Subscription)>,
    errors: HashMap<&'static str, String>,
    focus: FocusHandle,
    pub(crate) tree_focus: FocusHandle,
    /// The search field and the search in effect.
    pub(crate) search: Entity<Editor>,
    pub(crate) filter: Filter,
    /// The search's text as last seen, to tell typing from deleting.
    search_text: String,
    /// The search waiting for the typing to pause.
    search_task: Option<Task<()>>,
    /// The page that was selected when a search found nothing
    /// (`isLastSelected`): selected again when the search changes.
    pub(crate) last_selected: Option<usize>,
    /// The text last handed to each page's own search
    /// (`configurableOption`).
    page_search: HashMap<&'static str, String>,
    history: History,
    /// Back or Forward is selecting a page: it is not recorded.
    navigating: bool,
    pub(crate) tree_scroll: ScrollHandle,
    /// The tree is to scroll to the selected page once it is laid out.
    reveal_pending: bool,
    /// The tree's share of the width.
    split: f32,
    /// The splitter being dragged: where the drag started and the tree's
    /// width then.
    resizing: Option<(Pixels, Pixels)>,
    /// Where the search field is (the history popup opens below it).
    pub(crate) search_anchor: Rc<Cell<Bounds<Pixels>>>,
    history_menu: Option<(Entity<ContextMenu>, Subscription)>,
    /// Whether each page was modified when it was left, or when the window
    /// was deactivated (`myLeaveState`): one that was not and is modified
    /// on return was changed elsewhere and is reloaded.
    leave_state: HashMap<&'static str, bool>,
    /// The window's size when it was last drawn.
    size: Size<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    pub fn new(request: OpenRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (pages, depths) = tree_order(crate::page::pages(cx));
        let saved = window_state::get(cx);
        let search = editor::field::new("", None, true, window, cx);
        let search_focus = search.focus_handle(cx);
        let mut subscriptions = vec![
            cx.subscribe_in(&search, window, |this, _, e: &EditorEvent, window, cx| {
                if matches!(e, EditorEvent::Edited(_)) {
                    this.search_edited(window, cx);
                }
            }),
            // A search is remembered when the field loses the focus
            // (`SearchTextField.onFocusLost`).
            cx.on_blur(&search_focus, window, |this, _, cx| {
                let text = this.search.read(cx).text();
                window_state::update(cx, |s| s.remember_search(&text));
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.activation_changed(window, cx)
            }),
        ];
        subscriptions.push(cx.on_release(|this: &mut Self, cx| this.remember(cx)));
        let mut this = Self {
            title: title(request.project_name.as_deref()).into(),
            project_name: request.project_name.clone(),
            context: request.context,
            pages,
            depths,
            expanded: HashSet::new(),
            selected: 0,
            views: HashMap::new(),
            errors: HashMap::new(),
            focus: cx.focus_handle(),
            tree_focus: cx.focus_handle(),
            search,
            filter: Filter::default(),
            search_text: String::new(),
            search_task: None,
            last_selected: None,
            page_search: HashMap::new(),
            history: History::default(),
            navigating: false,
            tree_scroll: ScrollHandle::new(),
            reveal_pending: false,
            split: saved.split.unwrap_or(SPLIT).clamp(0.05, 0.95),
            resizing: None,
            search_anchor: Rc::default(),
            history_menu: None,
            leave_state: HashMap::new(),
            size: window.viewport_size(),
            _subscriptions: subscriptions,
        };
        let first = request
            .page
            .as_deref()
            .and_then(|id| this.index_of(id))
            .or_else(|| saved.page.as_deref().and_then(|id| this.index_of(id)))
            .or_else(|| this.index_of("appearance"))
            .or_else(|| this.pages.iter().position(|p| p.create.is_some()))
            .unwrap_or(0);
        if !this.pages.is_empty() {
            this.select(first, window, cx);
        }
        window.focus(&this.tree_focus);
        if let Some(text) = request.filter.as_deref().filter(|t| !t.trim().is_empty()) {
            this.set_search_text(text, request.page.is_none(), window, cx);
        }
        this
    }

    pub(crate) fn index_of(&self, id: &str) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    pub(crate) fn parent_index(&self, i: usize) -> Option<usize> {
        self.pages[i].parent.and_then(|p| self.index_of(p))
    }

    pub(crate) fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let id = self.pages[i].id;
        (0..self.pages.len()).filter(move |&c| self.pages[c].parent == Some(id))
    }

    pub(crate) fn has_children(&self, i: usize) -> bool {
        self.children(i).next().is_some()
    }

    /// Selects a page by id; false for an unknown id.
    pub fn select_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.index_of(id) {
            Some(i) => {
                self.select(i, window, cx);
                true
            }
            None => false,
        }
    }

    /// Opening the window again: the page asked for (the search cleared,
    /// as the reference's `selectWithFilter`), and the search text.
    fn navigate(
        &mut self,
        page: Option<&str>,
        filter: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(i) = page.and_then(|id| self.index_of(id)) {
            self.set_search_text(filter.unwrap_or(""), false, window, cx);
            self.select(i, window, cx);
        } else if let Some(text) = filter {
            self.set_search_text(text, true, window, cx);
        }
    }

    pub(crate) fn select(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.selected;
        if old != i
            && let Some(modified) = self.view(old).map(|v| v.is_modified(cx))
        {
            self.leave_state.insert(self.pages[old].id, modified);
        }
        self.selected = i;
        let mut at = self.parent_index(i);
        while let Some(p) = at {
            self.expanded.insert(self.pages[p].id);
            if self.filter.visible.is_some() {
                self.filter.expanded.insert(self.pages[p].id);
            }
            at = self.parent_index(p);
        }
        self.ensure_view(i, window, cx);
        self.reload_if_changed(i, window, cx);
        let id = self.pages[i].id;
        if !self.navigating {
            self.history.push(id);
        }
        window_state::update(cx, |s| s.page = Some(id.to_string()));
        self.reveal_selected();
        self.hand_over_search(window, cx);
        cx.notify();
    }

    /// Scrolls the tree to the selected page's row; before the tree is
    /// laid out, once it is.
    pub(crate) fn reveal_selected(&mut self) {
        let Some(row) = self.rows().iter().position(|r| r.page == self.selected) else {
            self.reveal_pending = false;
            return;
        };
        self.reveal_pending = self.tree_scroll.bounds_for_item(0).is_none();
        if !self.reveal_pending {
            self.tree_scroll.scroll_to_item(row);
        }
    }

    /// A page that had no changes when it was left and now differs from
    /// the store was changed elsewhere: it shows the stored state again
    /// (`postUpdateCurrent`, `resetUnmodifiedOnWindowFocus`).
    fn reload_if_changed(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.pages[i].id;
        if self.leave_state.remove(id) == Some(false)
            && let Some(view) = self.view(i).cloned()
            && view.is_modified(cx)
        {
            view.reset(window, cx);
        }
    }

    fn activation_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.selected;
        if i >= self.pages.len() {
            return;
        }
        if window.is_window_active() {
            self.reload_if_changed(i, window, cx);
        } else if ui::dialog_window::modal_of(window.window_handle(), cx).is_some() {
            // A dialog of the page took the focus (`NonModalWindowWrapper`:
            // an owned child): what it changes is meant, nothing is reset
            // when it closes.
        } else if let Some(view) = self.view(i) {
            let modified = view.is_modified(cx);
            self.leave_state.insert(self.pages[i].id, modified);
        }
    }

    /// Creates the page's view the first time it is shown.
    fn ensure_view(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let spec = &self.pages[i];
        if self.views.contains_key(spec.id) {
            return;
        }
        let Some(create) = spec.create.clone() else {
            return;
        };
        let id = spec.id;
        let handle = create(&self.context, window, cx);
        let weak = cx.weak_entity();
        let subscription = handle.observe(window, cx, move |_, cx| {
            let _ = weak.update(cx, |_, cx| cx.notify());
        });
        self.views.insert(id, (handle, subscription));
    }

    fn view(&self, i: usize) -> Option<&PageHandle> {
        self.views.get(self.pages[i].id).map(|(h, _)| h)
    }

    pub(crate) fn is_modified(&self, i: usize, cx: &App) -> bool {
        self.view(i).is_some_and(|v| v.is_modified(cx))
    }

    pub(crate) fn has_error(&self, i: usize) -> bool {
        self.errors.contains_key(self.pages[i].id)
    }

    /// What takes the focus on the selected page (its first field).
    pub fn page_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.view(self.selected).and_then(|v| v.focus(cx))
    }

    /// The tree's focus.
    pub fn tree_focus(&self) -> FocusHandle {
        self.tree_focus.clone()
    }

    /// The search field's focus.
    pub fn search_focus(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }

    /// Some page has changes to apply.
    pub fn any_modified(&self, cx: &App) -> bool {
        self.views.values().any(|(v, _)| v.is_modified(cx))
    }

    /// Applies every modified page; false when one fails (it is selected,
    /// its error shown under it). A page left without changes that now
    /// differs from the store was changed elsewhere and is not applied
    /// over it.
    pub fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let modified: Vec<usize> = (0..self.pages.len())
            .filter(|&i| self.is_modified(i, cx))
            .filter(|&i| {
                i == self.selected || self.leave_state.get(self.pages[i].id) != Some(&false)
            })
            .collect();
        self.errors.clear();
        let mut failed = None;
        for i in modified {
            let Some(view) = self.view(i).cloned() else {
                continue;
            };
            if let Err(message) = view.apply(window, cx) {
                self.errors.insert(self.pages[i].id, message);
                failed.get_or_insert(i);
            }
        }
        if let Some(i) = failed {
            if self
                .filter
                .visible
                .as_ref()
                .is_some_and(|v| !v.contains(&i))
            {
                self.set_search_text("", false, window, cx);
            }
            self.select(i, window, cx);
        }
        cx.notify();
        failed.is_none()
    }

    /// Writes what the window keeps: the page, size and splitter.
    fn remember(&mut self, cx: &mut App) {
        self.remember_now(cx);
        window_state::save(cx);
    }

    /// Takes what the window keeps into the state (written later).
    pub(crate) fn remember_now(&mut self, cx: &mut App) {
        let id = self.pages.get(self.selected).map(|p| p.id.to_string());
        let (width, height) = (f32::from(self.size.width), f32::from(self.size.height));
        let split = self.split;
        let text = self.search.read(cx).text();
        window_state::update(cx, |s| {
            if id.is_some() {
                s.page = id;
            }
            if width > 0.0 && height > 0.0 {
                s.width = Some(width);
                s.height = Some(height);
            }
            s.split = Some(split);
            s.remember_search(&text);
        });
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember(cx);
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    fn on_select(&mut self, a: &SelectPage, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.index_of(&a.page) {
            // As the reference's `Settings.select`: the search is cleared.
            self.set_search_text("", false, window, cx);
            self.select(i, window, cx);
        }
    }

    fn on_open(&mut self, a: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(a.page.as_deref(), a.filter.as_deref(), window, cx);
    }

    fn on_edit(&mut self, a: &EditField, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.view(self.selected).cloned() else {
            return;
        };
        if let Err(e) = view.edit(&a.field, &a.value, window, cx) {
            eprintln!("settings.edit: {e}");
        }
        cx.notify();
    }

    fn on_apply(&mut self, _: &Apply, window: &mut Window, cx: &mut Context<Self>) {
        self.apply(window, cx);
    }

    fn on_ok(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.apply(window, cx) {
            self.close(window, cx);
        }
    }

    fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// Escape clears the search first (`SettingsEditor.cancel` while the
    /// filter holds), then cancels.
    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).text().is_empty() && self.filter.text.is_empty() {
            self.close(window, cx);
        } else {
            self.set_search_text("", false, window, cx);
        }
    }

    fn on_revert(&mut self, _: &Revert, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.selected;
        if let Some(view) = self.view(i).cloned() {
            view.reset(window, cx);
        }
        self.errors.remove(self.pages[i].id);
        cx.notify();
    }

    fn on_search(&mut self, a: &Search, window: &mut Window, cx: &mut Context<Self>) {
        self.set_search_text(&a.text, true, window, cx);
    }

    fn on_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |e, cx| {
            let text = e.text();
            e.set_text_selected(&text, cx);
        });
        window.focus(&self.search.focus_handle(cx));
        cx.notify();
    }

    fn exists(&self) -> impl Fn(&str) -> bool + '_ {
        |id| self.index_of(id).is_some()
    }

    fn on_back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        let pages: Vec<&'static str> = self.pages.iter().map(|p| p.id).collect();
        if let Some(id) = self.history.back(|id| pages.contains(&id)) {
            self.go_to_place(id, window, cx);
        }
    }

    fn on_forward(&mut self, _: &Forward, window: &mut Window, cx: &mut Context<Self>) {
        let pages: Vec<&'static str> = self.pages.iter().map(|p| p.id).collect();
        if let Some(id) = self.history.forward(|id| pages.contains(&id)) {
            self.go_to_place(id, window, cx);
        }
    }

    /// Back or Forward: the page, the search cleared (`navigateTo` selects
    /// through `Settings.select`).
    fn go_to_place(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.navigating = true;
        self.set_search_text("", false, window, cx);
        self.select_id(id, window, cx);
        self.navigating = false;
    }

    fn on_go_to_page(&mut self, _: &GoToPage, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.page_focus(cx).unwrap_or_else(|| self.focus.clone());
        window.focus(&target);
        cx.notify();
    }

    fn on_search_history(
        &mut self,
        _: &SearchHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let history = window_state::get(cx).search_history;
        if history.is_empty() {
            return;
        }
        let items = history
            .into_iter()
            .map(|text| MenuItem::action(text.clone(), Search { text }))
            .collect();
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                min_width: Some(f32::from(self.search_anchor.get().size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.history_menu = None;
            cx.notify();
        });
        self.history_menu = Some((menu, subscription));
        cx.notify();
    }

    /// The text typed in the search field; the search runs when the
    /// typing pauses (`SettingsFilter`, 100 ms).
    fn search_edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.search.read(cx).text();
        let kind = crate::tree::EditKind::between(&self.search_text, &text);
        self.search_text = text.clone();
        // Typing elsewhere puts the focus in the field.
        let focus = self.search.focus_handle(cx);
        if !focus.is_focused(window) {
            window.focus(&focus);
        }
        self.search_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(crate::tree::SEARCH_DELAY)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.search_task = None;
                this.update_filter(&text, kind, true, window, cx);
            });
        }));
    }

    /// Puts `text` in the search field and searches at once; `adjust`
    /// moves the selection to what is found, as typing does.
    pub(crate) fn set_search_text(
        &mut self,
        text: &str,
        adjust: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = crate::tree::EditKind::between(&self.search_text, text);
        self.search_text = text.to_string();
        self.search_task = None;
        let text_owned = text.to_string();
        self.search.update(cx, |e, cx| {
            e.set_text_selected(&text_owned, cx);
            let end = text_owned.chars().count();
            e.set_selections(
                ::text::SelectionSet::single(::text::Selection::caret(end)),
                cx,
            );
        });
        self.update_filter(text, kind, adjust, window, cx);
    }

    /// Hands the search text to the selected page's own search, when the
    /// search did not find the page by its name (`SpotlightPainter.update`).
    pub(crate) fn hand_over_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.selected;
        let Some(view) = self.view(i).cloned() else {
            return;
        };
        if !view.has_search(cx) {
            return;
        }
        let text = self.spotlight_text();
        let id = self.pages[i].id;
        let by_name = self
            .filter
            .hits
            .as_ref()
            .is_some_and(|h| h.name.contains(&i));
        let last = self.page_search.insert(id, text.clone());
        if !by_name && last.as_deref() != Some(text.as_str()) {
            view.search(&text, window, cx);
        }
    }

    /// What the selected page lights up: the search's text, or what
    /// follows a path.
    pub(crate) fn spotlight_text(&self) -> String {
        self.filter
            .hits
            .as_ref()
            .map(|h| h.spotlight.clone())
            .unwrap_or_default()
    }

    /// The state for `settings.state`.
    pub fn state(&self, window: &Window, cx: &App) -> Json {
        let selected = self.pages.get(self.selected);
        let modified: Vec<&str> = (0..self.pages.len())
            .filter(|&i| self.is_modified(i, cx))
            .map(|i| self.pages[i].id)
            .collect();
        let rows: Vec<Json> = self
            .rows()
            .into_iter()
            .map(|r| {
                let p = &self.pages[r.page];
                json!({"id": p.id, "title": p.title, "depth": r.depth})
            })
            .collect();
        let bounds = window.bounds();
        let exists = self.exists();
        json!({
            "open": true,
            "title": self.title,
            "page": selected.map(|p| p.id),
            "breadcrumbs": self.breadcrumbs(self.selected),
            "rows": rows,
            "modified": modified,
            "errors": self.errors,
            "apply_enabled": !modified.is_empty(),
            "fields": self.view(self.selected).map(|v| v.state(cx)),
            "tree_focused": self.tree_focus.is_focused(window),
            "search": {
                "text": self.search.read(cx).text(),
                "filter": self.filter.text,
                "no_match": self.filter.no_match(),
                "spotlight": self.spotlight_text(),
                "focused": self.search.focus_handle(cx).is_focused(window),
                "history": window_state::get(cx).search_history,
                "history_open": self.history_menu.is_some(),
            },
            "sticky_header": self.sticky_header(&self.rows()).map(|i| self.pages[i].title),
            "can_back": self.history.can_back(&exists),
            "can_forward": self.history.can_forward(&exists),
            "split": self.split,
            "tree_width": f32::from(self.tree_width(window)),
            "window": {"width": f32::from(bounds.size.width), "height": f32::from(bounds.size.height)},
        })
    }

    pub(crate) fn breadcrumbs(&self, i: usize) -> Vec<&'static str> {
        let mut crumbs = vec![self.pages[i].title];
        let mut at = self.parent_index(i);
        while let Some(p) = at {
            crumbs.push(self.pages[p].title);
            at = self.parent_index(p);
        }
        crumbs.reverse();
        crumbs
    }

    /// The tree's width: its share of the window, at least its smallest
    /// width, leaving the page its own.
    pub(crate) fn tree_width(&self, window: &Window) -> Pixels {
        let width = f32::from(window.viewport_size().width);
        let max = (width - PAGE_MIN_WIDTH).max(TREE_MIN_WIDTH);
        px((width * self.split).clamp(TREE_MIN_WIDTH, max).round())
    }

    fn start_resize(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.resizing = Some((e.position.x, self.tree_width(window)));
        cx.stop_propagation();
        cx.notify();
    }

    fn resize(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((start, from)) = self.resizing else {
            return;
        };
        if e.pressed_button != Some(MouseButton::Left) {
            self.end_resize(cx);
            return;
        }
        let width = f32::from(window.viewport_size().width);
        if width <= 0.0 {
            return;
        }
        let max = (width - PAGE_MIN_WIDTH).max(TREE_MIN_WIDTH);
        let tree = f32::from(from + e.position.x - start).clamp(TREE_MIN_WIDTH, max);
        self.split = tree / width;
        cx.notify();
    }

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resizing.take().is_some() {
            let split = self.split;
            window_state::update(cx, |s| s.split = Some(split));
            cx.notify();
        }
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let crumbs = self.breadcrumbs(self.selected);
        let count = crumbs.len();
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .font_weight(FontWeight::BOLD);
        for (n, crumb) in crumbs.into_iter().enumerate() {
            row = row.child(div().whitespace_nowrap().child(crumb));
            if n + 1 < count {
                row = row.child(
                    div()
                        .text_color(muted)
                        .font_weight(FontWeight::NORMAL)
                        .child(CRUMB_SEPARATOR),
                );
            }
        }
        // The project mark of a page with project settings (borders 8, 4,
        // 8, 8 around `General.ProjectConfigurable`).
        let project = self.pages[self.selected].scope.is_project().then(|| {
            div()
                .id("settings-banner-project")
                .debug_selector(|| "settings-banner-project".into())
                .flex_none()
                .ml(px(4.0))
                .child(Icon::new(IconName::PROJECT_CONFIGURABLE))
                .tooltip(Tooltip::text(PROJECT_MARK, None))
        });
        let modified = self.is_modified(self.selected, cx) || self.has_error(self.selected);
        let revert = modified.then(|| {
            ui::link::link("settings-revert", REVERT, cx)
                .debug_selector(|| "settings-revert".into())
                .ml(px(REVERT_GAP))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Revert), cx))
        });
        let exists = self.exists();
        let can_back = self.history.can_back(&exists);
        let can_forward = self.history.can_forward(&exists);
        let back = ui::components::small_icon_button("settings-back", IconName::BACK, can_back, cx)
            .debug_selector(|| "settings-back".into())
            .tooltip(Tooltip::text("Back", None))
            .when(can_back, |d| {
                d.on_click(|_, window, cx| window.dispatch_action(Box::new(Back), cx))
            });
        let forward = ui::components::small_icon_button(
            "settings-forward",
            IconName::FORWARD,
            can_forward,
            cx,
        )
        .tooltip(Tooltip::text("Forward", None))
        .when(can_forward, |d| {
            d.on_click(|_, window, cx| window.dispatch_action(Box::new(Forward), cx))
        });
        // Pin Window: the reference's float mode, on by default; switching
        // to an independent window is not built yet.
        let pin = ui::components::icon_button(
            "settings-pin",
            IconName::PIN,
            px(BANNER_BUTTON),
            px(16.0),
            true,
            cx,
        )
        .tooltip(Tooltip::text("Pin Window", None));
        let history = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(back)
            .child(forward)
            .child(pin);
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(BANNER_HEIGHT))
            .mt(px(BANNER_TOP))
            .pl(px(PAGE_X))
            .pr(px(10.0))
            .child(row)
            .children(project)
            .children(revert)
            .child(div().flex_1())
            .child(history)
            .into_any_element()
    }

    fn render_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let i = self.selected;
        if let Some(view) = self.view(i) {
            return view.view.clone().into_any_element();
        }
        // A group page: its description and links to its children
        // (`createConfigurablesListPanel`).
        let page = self.pages[i].clone();
        let children: Vec<usize> = self.children(i).collect();
        let links: Vec<AnyElement> = children
            .into_iter()
            .map(|c| {
                let child = &self.pages[c];
                let id = child.id;
                ui::link::link(("settings-link", c), child.title, cx)
                    .debug_selector(move || format!("settings-link:{id}"))
                    .mb(px(4.0))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.select(c, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .children(
                page.description
                    .map(|d| div().mb(px(11.0)).child(d).into_any_element()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .pl(px(20.0))
                    .children(links),
            )
            .into_any_element()
    }

    fn render_footer(&self, cx: &App) -> AnyElement {
        BottomBar::new()
            .help(ui::form::help_button("settings-help", None, cx))
            .button(BarButton::new(
                "settings-ok",
                "OK",
                ButtonRole::Ok,
                Box::new(Confirm),
            ))
            .button(BarButton::new(
                "settings-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(Cancel),
            ))
            .button(
                BarButton::new(
                    "settings-apply",
                    "Apply",
                    ButtonRole::Apply,
                    Box::new(Apply),
                )
                .enabled(self.any_modified(cx)),
            )
            .render(cx)
    }

    /// The one-pixel divider between the tree and the page, with a wider
    /// handle that drags it.
    fn render_splitter(&self, cx: &mut Context<Self>) -> AnyElement {
        let border = cx.theme().color(UiRole::Border);
        div()
            .relative()
            .flex_none()
            .w(px(1.0))
            .h_full()
            .bg(border)
            .child(
                div()
                    .id("settings-splitter")
                    .debug_selector(|| "settings-splitter".into())
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-(SPLITTER_HANDLE - 1.0) / 2.0))
                    .w(px(SPLITTER_HANDLE))
                    .child(
                        canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, HitboxBehavior::Normal)
                            },
                            |_, hitbox, window, _| {
                                ui::pointer::request(CursorStyle::ResizeLeftRight, &hitbox, window)
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::start_resize)),
            )
            .into_any_element()
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let surface = theme.color(UiRole::Surface);
        let side = theme.color(UiRole::SurfaceAlt);
        let danger = theme.color(UiRole::Danger);
        self.size = window.viewport_size();
        if self.reveal_pending {
            self.reveal_selected();
            if self.reveal_pending {
                window.request_animation_frame();
            }
        }
        let tree_width = self.tree_width(window);
        // The page lights up what the search finds while the focus is in
        // the tree or the search field (`installSpotlightRemover`).
        let spotlight_on =
            self.tree_focus.is_focused(window) || self.search.focus_handle(cx).is_focused(window);
        let spot = if spotlight_on {
            self.spotlight_text()
        } else {
            String::new()
        };
        let all = self.pages.get(self.selected).is_some_and(|p| {
            p.search_terms
                .iter()
                .any(|t| crate::search::label_matches(t, &spot, true))
        });
        crate::search::set_spotlight(cx, &spot, all);
        let tree = self.render_tree(window, cx);
        let banner = (!self.pages.is_empty()).then(|| self.render_banner(cx));
        let page = (!self.pages.is_empty()).then(|| self.render_page(cx));
        let error = self
            .pages
            .get(self.selected)
            .and_then(|p| self.errors.get(p.id).cloned());
        let footer = self.render_footer(cx);
        let splitter = self.render_splitter(cx);
        let history_menu = self.history_menu.as_ref().map(|(menu, _)| {
            let anchor = self.search_anchor.get();
            ui::menu::overlay(menu, anchor.bottom_left(), window)
        });
        let resizing = self.resizing.is_some().then(|| {
            canvas(
                |_, _, _| {},
                |_, _, window, _| ui::pointer::request_window(CursorStyle::ResizeLeftRight, window),
            )
            .absolute()
            .size_full()
        });
        ui::components::root(cx)
            .id("settings-window")
            .debug_selector(|| "settings-window".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_select))
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_edit))
            .on_action(cx.listener(Self::on_apply))
            .on_action(cx.listener(Self::on_ok))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_revert))
            .on_action(cx.listener(Self::on_search))
            .on_action(cx.listener(Self::on_find))
            .on_action(cx.listener(Self::on_back))
            .on_action(cx.listener(Self::on_forward))
            .on_action(cx.listener(Self::on_go_to_page))
            .on_action(cx.listener(Self::on_search_history))
            .on_action(
                cx.listener(|this, _: &SelectNext, window, cx| this.move_selection(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPrevious, window, cx| {
                this.move_selection(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Expand, window, cx| this.expand(window, cx)))
            .on_action(cx.listener(|this, _: &Collapse, window, cx| this.collapse(window, cx)))
            .on_mouse_move(cx.listener(Self::resize))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_resize(cx)),
            )
            .relative()
            .flex()
            .flex_col()
            .bg(surface)
            .children(ui::dialog_window::title_bar(self.title.clone(), true, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .flex_none()
                            .w(tree_width)
                            .h_full()
                            .bg(side)
                            .child(tree),
                    )
                    .child(splitter)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .children(banner)
                            .child(
                                div()
                                    .id("settings-page")
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .overflow_y_scroll()
                                    .px(px(PAGE_X))
                                    .pt(px(PAGE_TOP))
                                    .pb(px(16.0))
                                    .children(page),
                            )
                            .children(error.map(|e| {
                                div()
                                    .debug_selector(|| "settings-error".into())
                                    .flex_none()
                                    .pt(px(10.0))
                                    .px(px(15.0))
                                    .pb(px(15.0))
                                    .text_color(danger)
                                    .text_size(px(16.0))
                                    .child(e)
                            })),
                    ),
            )
            .child(footer)
            .children(resizing)
            .children(history_menu)
            .children(ui::dialog_window::blocker(window, cx))
            // The links and the divider ask for their pointer shapes.
            .child(ui::pointer::frame())
    }
}

/// Pages in tree order and their depths: children after their parent,
/// siblings by weight (the larger first), then registration order. A page
/// whose parent is unknown is left out.
fn tree_order(pages: Vec<PageSpec>) -> (Vec<PageSpec>, Vec<usize>) {
    fn visit(
        parent: Option<&'static str>,
        depth: usize,
        all: &[PageSpec],
        out: &mut Vec<(PageSpec, usize)>,
    ) {
        let mut children: Vec<&PageSpec> = all.iter().filter(|p| p.parent == parent).collect();
        children.sort_by_key(|p| std::cmp::Reverse(p.weight));
        for child in children {
            out.push((child.clone(), depth));
            visit(Some(child.id), depth + 1, all, out);
        }
    }
    let mut out = Vec::new();
    visit(None, 0, &pages, &mut out);
    out.into_iter().unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &'static str, parent: Option<&'static str>, weight: i32) -> PageSpec {
        PageSpec {
            weight,
            ..PageSpec::group(id, parent, id)
        }
    }

    #[test]
    fn the_tree_puts_children_after_their_parent_by_weight() {
        let (pages, depths) = tree_order(vec![
            page("editor", None, 10),
            page("system", Some("appearance-behavior"), 120),
            page("appearance-behavior", None, 100),
            page("appearance", Some("appearance-behavior"), 150),
            page("python", None, 1000),
            page("orphan", Some("nothing"), 0),
        ]);
        let ids: Vec<_> = pages.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "python",
                "appearance-behavior",
                "appearance",
                "system",
                "editor"
            ]
        );
        assert_eq!(depths, [0, 0, 1, 1, 0]);
    }

    #[test]
    fn the_title_names_the_project() {
        assert_eq!(title(Some("demo")), "Settings \u{2013} demo");
        assert_eq!(title(None), "Settings");
    }
}
