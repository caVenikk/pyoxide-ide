//! The Settings window (`SettingsEditor` in `SettingsNonModalDialog`): the
//! page tree on the left (`SettingsTreeView`: top-level rows bold, modified
//! pages in `Tree.modifiedItemForeground`, pages with errors in
//! `Tree.errorForeground`, the project mark), the banner over the page
//! (bold breadcrumbs, "Revert changes" when the page is modified), the page
//! and the bottom bar with "?" and Cancel, Apply and OK in the platform's
//! order. One window per application, non-modal to the window it was
//! opened from.

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, FocusHandle, Focusable, FontWeight,
    IntoElement, Render, SharedString, Subscription, Window, WindowHandle, div, prelude::*, px,
    size,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole, Hover};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::commands::{Apply, Cancel, Confirm, EditField, OpenSettings, Revert, SelectPage};
use crate::page::{PageContext, PageHandle, PageSpec};

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
const TREE_MIN_WIDTH: f32 = 240.0;
/// Tree metrics: row height, indent per level, where the arrow column
/// starts and its width (the text of a top-level row starts at 32).
const ROW: f32 = 25.0;
const INDENT: f32 = 16.0;
const ROW_LEFT: f32 = 12.0;
const ARROW: f32 = 16.0;
const ARROW_GAP: f32 = 4.0;
/// Insets: the search panel (`searchPanel` border 7, 5, 6, 5), the banner
/// (11, 6, 0, 10 within the page's 16 left inset), the page's content
/// (11, 16, 16, 16) and the error under it (10, 15, 15, 15).
const SEARCH_INSETS: (f32, f32) = (7.0, 5.0);
const SEARCH_HEIGHT: f32 = 24.0;
const BANNER_TOP: f32 = 11.0;
const BANNER_HEIGHT: f32 = 26.0;
const PAGE_X: f32 = 16.0;
const PAGE_TOP: f32 = 11.0;
/// Gap between the breadcrumbs and "Revert changes".
const REVERT_GAP: f32 = 16.0;

/// The breadcrumb separator.
const CRUMB_SEPARATOR: &str = "\u{203a}";
/// The banner's reset link (`configurable.reset.action.name`).
pub const REVERT: &str = "Revert changes";

/// How the window is opened.
#[derive(Debug, Clone, Default)]
pub struct OpenRequest {
    /// The project's name for the title (`Settings – <project>`).
    pub project_name: Option<String>,
    pub context: PageContext,
    /// The page by id; `None`: the last one (first time: Appearance).
    pub page: Option<String>,
    /// Give the window the focus (off in the background).
    pub activate: bool,
}

/// The window's title for a project name.
pub fn title(project: Option<&str>) -> String {
    match project {
        Some(name) => format!("Settings \u{2013} {name}"),
        None => "Settings".into(),
    }
}

/// Opens the Settings window over `parent`, or brings the open one to the
/// front and selects the requested page there.
pub fn open(
    parent: &mut Window,
    request: OpenRequest,
    cx: &mut App,
) -> Option<WindowHandle<SettingsWindow>> {
    if let Some(open) = find(cx) {
        let page = request.page.clone();
        let _ = open.update(cx, |view, window, cx| {
            if let Some(page) = page {
                view.select_id(&page, window, cx);
            }
            if request.activate {
                window.activate_window();
            }
        });
        return Some(open);
    }
    let title_text = title(request.project_name.as_deref());
    let options = ui::dialog_window::DialogOptions {
        title: title_text.into(),
        size: size(
            px(WIDTH),
            px(HEIGHT + ui::dialog_window::title_height(true)),
        ),
        min_size: Some(size(px(WIDTH), px(HEIGHT))),
        activate: request.activate,
        themed_title: true,
        modal: false,
        key: Some(DIALOG_KEY.into()),
    };
    ui::dialog_window::open(parent, options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(request, window, cx))
    })
}

/// The open Settings window.
pub fn find(cx: &App) -> Option<WindowHandle<SettingsWindow>> {
    ui::dialog_window::find(DIALOG_KEY, cx)?.downcast::<SettingsWindow>()
}

/// A row of the tree: a page and its depth.
#[derive(Debug, Clone, Copy)]
struct Row {
    page: usize,
    depth: usize,
}

pub struct SettingsWindow {
    title: SharedString,
    context: PageContext,
    /// Every page in tree order (depth first, siblings by weight).
    pages: Vec<PageSpec>,
    depths: Vec<usize>,
    expanded: HashSet<&'static str>,
    selected: usize,
    /// Views of the pages shown so far, with their drafts.
    views: HashMap<&'static str, (PageHandle, Subscription)>,
    errors: HashMap<&'static str, String>,
    focus: FocusHandle,
    tree_focus: FocusHandle,
    revert_hover: Hover,
    link_hovers: HashMap<&'static str, Hover>,
}

impl SettingsWindow {
    pub fn new(request: OpenRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (pages, depths) = tree_order(crate::page::pages(cx));
        let mut this = Self {
            title: title(request.project_name.as_deref()).into(),
            context: request.context,
            pages,
            depths,
            expanded: HashSet::new(),
            selected: 0,
            views: HashMap::new(),
            errors: HashMap::new(),
            focus: cx.focus_handle(),
            tree_focus: cx.focus_handle(),
            revert_hover: Hover::default(),
            link_hovers: HashMap::new(),
        };
        let first = request
            .page
            .as_deref()
            .and_then(|id| this.index_of(id))
            .or_else(|| this.index_of("appearance"))
            .or_else(|| this.pages.iter().position(|p| p.create.is_some()))
            .unwrap_or(0);
        if !this.pages.is_empty() {
            this.select(first, window, cx);
        }
        window.focus(&this.tree_focus);
        this
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    fn parent_index(&self, i: usize) -> Option<usize> {
        self.pages[i].parent.and_then(|p| self.index_of(p))
    }

    fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let id = self.pages[i].id;
        (0..self.pages.len()).filter(move |&c| self.pages[c].parent == Some(id))
    }

    fn has_children(&self, i: usize) -> bool {
        self.children(i).next().is_some()
    }

    /// The rows shown: the pages whose ancestors are expanded.
    fn rows(&self) -> Vec<Row> {
        (0..self.pages.len())
            .filter(|&i| {
                let mut at = self.parent_index(i);
                while let Some(p) = at {
                    if !self.expanded.contains(self.pages[p].id) {
                        return false;
                    }
                    at = self.parent_index(p);
                }
                true
            })
            .map(|i| Row {
                page: i,
                depth: self.depths[i],
            })
            .collect()
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

    fn select(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = i;
        let mut at = self.parent_index(i);
        while let Some(p) = at {
            self.expanded.insert(self.pages[p].id);
            at = self.parent_index(p);
        }
        self.ensure_view(i, window, cx);
        cx.notify();
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

    fn is_modified(&self, i: usize, cx: &App) -> bool {
        self.view(i).is_some_and(|v| v.is_modified(cx))
    }

    /// What takes the focus on the selected page (its first field).
    pub fn page_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.view(self.selected).and_then(|v| v.focus(cx))
    }

    /// The tree's focus.
    pub fn tree_focus(&self) -> FocusHandle {
        self.tree_focus.clone()
    }

    /// Some page has changes to apply.
    pub fn any_modified(&self, cx: &App) -> bool {
        self.views.values().any(|(v, _)| v.is_modified(cx))
    }

    /// Applies every modified page; false when one fails (it is selected,
    /// its error shown under it).
    pub fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let modified: Vec<usize> = (0..self.pages.len())
            .filter(|&i| self.is_modified(i, cx))
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
            self.select(i, window, cx);
        }
        cx.notify();
        failed.is_none()
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    fn on_select(&mut self, a: &SelectPage, window: &mut Window, cx: &mut Context<Self>) {
        self.select_id(&a.page, window, cx);
    }

    fn on_open(&mut self, a: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = &a.page {
            self.select_id(page, window, cx);
        }
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

    fn on_revert(&mut self, _: &Revert, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.selected;
        if let Some(view) = self.view(i).cloned() {
            view.reset(window, cx);
        }
        self.errors.remove(self.pages[i].id);
        cx.notify();
    }

    fn toggle(&mut self, i: usize, cx: &mut Context<Self>) {
        let id = self.pages[i].id;
        if !self.expanded.remove(id) {
            self.expanded.insert(id);
        }
        cx.notify();
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
            "window": {"width": f32::from(bounds.size.width), "height": f32::from(bounds.size.height)},
        })
    }

    fn breadcrumbs(&self, i: usize) -> Vec<&'static str> {
        let mut crumbs = vec![self.pages[i].title];
        let mut at = self.parent_index(i);
        while let Some(p) = at {
            crumbs.push(self.pages[p].title);
            at = self.parent_index(p);
        }
        crumbs.reverse();
        crumbs
    }

    fn render_tree(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let focused = self.tree_focus.is_focused(window);
        let selection = theme.color(if focused {
            UiRole::Selection
        } else {
            UiRole::SelectionInactive
        });
        let text = theme.color(UiRole::Text);
        let selected_text = theme.color(UiRole::SelectionText);
        let modified = theme.color(UiRole::TextLink);
        let error = theme.color(UiRole::Danger);
        let muted = theme.color(UiRole::TextMuted);
        let rows: Vec<AnyElement> =
            self.rows()
                .into_iter()
                .map(|row| {
                    let i = row.page;
                    let page = &self.pages[i];
                    let selected = i == self.selected;
                    let parent = self.has_children(i);
                    let expanded = self.expanded.contains(page.id);
                    // The modified colour shows on a selected row too, as the
                    // reference draws it.
                    let color = if self.errors.contains_key(page.id) {
                        error
                    } else if self.is_modified(i, cx) {
                        modified
                    } else if selected && focused {
                        selected_text
                    } else {
                        text
                    };
                    let selector = format!("settings-row:{}", page.id);
                    div()
                        .id(("settings-row", i))
                        .debug_selector(move || selector.clone())
                        .flex()
                        .flex_none()
                        .flex_row()
                        .items_center()
                        .h(px(ROW))
                        .pl(px(ROW_LEFT + INDENT * row.depth as f32))
                        .pr(px(10.0))
                        .when(selected, |d| d.bg(selection))
                        .child(
                            div()
                                .id(("settings-arrow", i))
                                .flex_none()
                                .w(px(ARROW))
                                .when(parent, |d| {
                                    d.child(Icon::new(if expanded {
                                        IconName::CHEVRON_DOWN
                                    } else {
                                        IconName::CHEVRON_RIGHT
                                    }))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle(i, cx);
                                    }))
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .ml(px(ARROW_GAP))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_color(color)
                                .when(row.depth == 0, |d| d.font_weight(FontWeight::BOLD))
                                .child(page.title),
                        )
                        .when(page.scope.is_project(), |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_color(muted)
                                    .child(Icon::new(IconName::TOOL_PROJECT)),
                            )
                        })
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            window.focus(&this.tree_focus);
                            this.select(i, window, cx);
                            if e.click_count() == 2 && this.has_children(i) {
                                this.toggle(i, cx);
                            }
                        }))
                        .into_any_element()
                })
                .collect();
        let search = div()
            .flex_none()
            .px(px(SEARCH_INSETS.1))
            .pt(px(SEARCH_INSETS.0))
            .pb(px(6.0))
            .child(
                ui::components::input_frame(false, false, cx)
                    .h(px(SEARCH_HEIGHT))
                    .px(px(6.0))
                    .child(Icon::new(IconName::SEARCH)),
            );
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(search)
            .child(
                div()
                    .id("settings-tree")
                    .debug_selector(|| "settings-tree".into())
                    .track_focus(&self.tree_focus)
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element()
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
        let modified = self.is_modified(self.selected, cx);
        let revert = modified.then(|| {
            ui::form::link("settings-revert", REVERT, &self.revert_hover, cx)
                .debug_selector(|| "settings-revert".into())
                .ml(px(REVERT_GAP))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Revert), cx))
        });
        let history = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(ui::components::small_icon_button(
                "settings-back",
                IconName::CHEVRON_LEFT,
                false,
                cx,
            ))
            .child(ui::components::small_icon_button(
                "settings-forward",
                IconName::CHEVRON_RIGHT,
                false,
                cx,
            ))
            .child(ui::components::small_icon_button(
                "settings-pin",
                IconName::PIN,
                false,
                cx,
            ));
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
                let hover = self.link_hovers.entry(child.id).or_default().clone();
                let id = child.id;
                ui::form::link(("settings-link", c), child.title, &hover, cx)
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
        let border = theme.color(UiRole::Border);
        let danger = theme.color(UiRole::Danger);
        let width = f32::from(window.viewport_size().width);
        let tree_width = (width * SPLIT).max(TREE_MIN_WIDTH).floor();
        let tree = self.render_tree(window, cx);
        let banner = (!self.pages.is_empty()).then(|| self.render_banner(cx));
        let page = (!self.pages.is_empty()).then(|| self.render_page(cx));
        let error = self
            .pages
            .get(self.selected)
            .and_then(|p| self.errors.get(p.id).cloned());
        let footer = self.render_footer(cx);
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
            .on_action(cx.listener(Self::on_revert))
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
                            .w(px(tree_width))
                            .h_full()
                            .bg(side)
                            .child(tree),
                    )
                    .child(div().flex_none().w(px(1.0)).h_full().bg(border))
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
            .children(ui::dialog_window::blocker(window, cx))
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
