//! Drawing the Data Sources dialog. Metrics were read off the reference
//! (logical px, PyCharm 2026.2.3, New UI).

use db_core::schema::SchemaFilter;
use db_core::{Backend, Dialect, TlsMode};
use gpui::{
    AnyElement, App, Context, ElementId, FontWeight, IntoElement, KeyDownEvent, Render,
    SharedString, Window, canvas, div, prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip, checkbox, header_tab, small_icon_button};
use ui::form::{BarButton, BottomBar, ButtonRole, FIELD_HEIGHT};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::*;

/// The window's title (the reference's window holds drivers too).
pub(super) const WINDOW_TITLE: &str = "Data Sources and Drivers";
pub(super) const WIDTH: f32 = 980.0;
pub(super) const HEIGHT: f32 = 704.0;
pub(super) const MIN_SIZE: (f32, f32) = (760.0, 520.0);

/// The columns: the pages' icons, the list, the data source.
const ICON_COLUMN: f32 = 44.0;
const LIST_WIDTH: f32 = 248.0;
/// The narrowest the list and the page get when the splitter is dragged,
/// and the width of the splitter's handle over the one-pixel line.
const LIST_MIN_WIDTH: f32 = 150.0;
const PAGE_MIN_WIDTH: f32 = 460.0;
const SPLITTER_HANDLE: f32 = 6.0;
const PAGE_ICON: f32 = 27.0;
/// The list: header, toolbar, group label, rows.
const LIST_SIDE: f32 = 10.0;
const LIST_HEADER: f32 = 32.0;
const LIST_TOOLBAR: f32 = 34.0;
const LIST_ROW: f32 = 24.0;
/// The data source: the name row, the tabs, the form.
const PAGE_SIDE: f32 = 14.0;
const PAGE_RIGHT: f32 = 15.0;
const NAME_TOP: f32 = 10.0;
const TABS_TOP: f32 = 12.0;
const TAB_PAD: f32 = 10.0;
const TAB_GAP: f32 = 6.0;
const FORM_TOP: f32 = 12.0;
const LABEL_WIDTH: f32 = 118.0;
const SSL_LABEL_WIDTH: f32 = 141.0;
/// Port and Save (the reference: 94 for "Forever"; room for "Until restart").
const SHORT_FIELD: f32 = 118.0;
const SIDE_LABEL_GAP: f32 = 24.0;
/// The column of Port and Save, their gaps included.
const SIDE_LABEL_WIDTH: f32 = 70.0;
const ROW_GAP: f32 = 11.0;
const GROUP_GAP: f32 = 21.0;
const URL_GAP: f32 = 38.0;
const OPTION_INDENT: f32 = 24.0;
const NUMBER_FIELD: f32 = 196.0;
const MODE_FIELD: f32 = 160.0;
const TEST_BAR: f32 = 34.0;
/// The test result above Test Connection.
const RESULT_WIDTH: f32 = 386.0;
const RESULT_LEFT: f32 = -50.0;
const FAILURE_WIDTH: f32 = 368.0;

/// Texts (plain field names; our own wording for messages).
const DATA_SOURCES: &str = "Data Sources";
const PROJECT_DATA_SOURCES: &str = "Project Data Sources";
const NAME: &str = "Name:";
const ADD_COMMENT: &str = "Add comment";
const COMMENT: &str = "Comment:";
const DRIVER: &str = "Driver:";
const CONNECTION_TYPE: &str = "Connection type:";
const HOST: &str = "Host:";
const PORT: &str = "Port:";
const AUTHENTICATION: &str = "Authentication:";
const USER: &str = "User:";
const PASSWORD: &str = "Password:";
const SAVE: &str = "Save:";
const DATABASE: &str = "Database:";
const FILE: &str = "File:";
const URL: &str = "URL:";
const URL_COMMENT: &str = "Overrides settings above";
const CONNECT_STRING: &str = "Connect string:";
const TEST_CONNECTION: &str = "Test Connection";
const CANCEL_TEST: &str = "Cancel";
const SUCCEEDED: &str = "Succeeded";
const FAILED: &str = "Failed";
const COPY: &str = "Copy";
const EMPTY_LIST: &str = "No data sources";
const PROBLEMS: &str = "Problems";
/// The Problems row under the list: the gap above its line, its indent.
const PROBLEMS_GAP: f32 = 22.0;
const PROBLEMS_INDENT: f32 = 26.0;
/// The pointer of the test result towards the status icon (width,
/// height) and where its tip is from the result's left edge.
const POINTER: (f32, f32) = (14.0, 7.0);
const POINTER_TIP: f32 = 182.0;

/// The pointer under a balloon: a triangle in the balloon's fill with its
/// outline on the two slanted sides.
fn pointer(fill: gpui::Hsla, edge: gpui::Hsla) -> AnyElement {
    use gpui::{PathBuilder, point};
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x0, x1) = (bounds.left(), bounds.right());
            let mid = bounds.left() + px(POINTER.0 / 2.0);
            let (base, tip) = (bounds.top() - px(1.0), bounds.bottom());
            let mut path = PathBuilder::fill();
            path.move_to(point(x0, base));
            path.line_to(point(mid, tip));
            path.line_to(point(x1, base));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, fill);
            }
            let mut line = PathBuilder::stroke(px(1.0));
            line.move_to(point(x0, base));
            line.line_to(point(mid, tip));
            line.line_to(point(x1, base));
            if let Ok(path) = line.build() {
                window.paint_path(path, edge);
            }
        },
    )
    .absolute()
    .left(px(POINTER_TIP - POINTER.0 / 2.0))
    .bottom(px(-POINTER.1))
    .w(px(POINTER.0))
    .h(px(POINTER.1))
    .into_any_element()
}

/// Height of the expanded editor of a field.
const EXPANDED_HEIGHT: f32 = 120.0;

/// The colour of a data source's colour name (the IDE's file colours of
/// the dark themes, as a dot).
fn color_value(name: &str) -> gpui::Hsla {
    let rgb = match name {
        "blue" => 0x2f4d73,
        "green" => 0x3b5139,
        "orange" => 0x6e4b2f,
        "rose" => 0x6e3a43,
        "violet" => 0x55416e,
        "yellow" => 0x6b6332,
        _ => 0x4d4f53,
    };
    gpui::rgb(rgb).into()
}
const NOTHING_SELECTED: &str = "Select a data source or add one with +";

impl DataSourcesDialog {
    fn render_page_icons(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .w(px(ICON_COLUMN))
            .pt(px(4.0))
            .border_r_1()
            .border_color(theme.color(UiRole::Border))
            .child(
                div()
                    .id("data-sources-page")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(PAGE_ICON))
                    .rounded(theme.metric(Metric::ButtonRadius))
                    .bg(theme.color(UiRole::Selection))
                    .tooltip(Tooltip::text(DATA_SOURCES, None))
                    .child(Icon::new(icons::DATA_SOURCE).size(px(16.0))),
            )
            .into_any_element()
    }

    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let focused = self.focus.contains_focused(window, cx);
        let has_item = self.item().is_some();
        let anchors = self.anchors.clone();
        let add = small_icon_button("data-sources-add", IconName::ADD, true, cx)
            .debug_selector(|| "data-sources-add".into())
            .tooltip(Tooltip::text("Add", None))
            .relative()
            .child(
                canvas(
                    move |b, _, _| {
                        anchors.borrow_mut().insert(ListId::Add, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_click(cx.listener(|this, _, window, cx| this.open_list(ListId::Add, window, cx)));
        let tool = |id: &'static str, icon: IconName, tip: &'static str, enabled: bool| {
            small_icon_button(id, icon, enabled, cx)
                .debug_selector(move || id.to_owned())
                .tooltip(Tooltip::text(tip, None))
        };
        let rows: Vec<AnyElement> =
            self.items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let selected = self.selected == Some(i);
                    let name = item.source.name.clone();
                    let selector = format!("data-source-item-{i}");
                    div()
                        .id(ElementId::NamedInteger("data-source-item".into(), i as u64))
                        .debug_selector(move || selector.clone())
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .h(px(LIST_ROW))
                        .px(px(10.0))
                        .rounded(theme.metric(Metric::CornerRadius))
                        .when(selected, |d| {
                            d.bg(theme.color(if focused {
                                UiRole::Selection
                            } else {
                                UiRole::SelectionInactive
                            }))
                        })
                        .child(Icon::new(icons::backend(item.profile().backend)).size(px(16.0)))
                        .child(div().whitespace_nowrap().overflow_hidden().child(
                            if name.is_empty() {
                                " ".to_owned()
                            } else {
                                name
                            },
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            window.focus(&this.focus);
                            this.select(Some(i), cx);
                        }))
                        .into_any_element()
                })
                .collect();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.list_width(window, cx)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(LIST_HEADER))
                    .px(px(LIST_SIDE + 2.0))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(DATA_SOURCES),
                    )
                    .child(tool(
                        "data-sources-back",
                        IconName::CHEVRON_LEFT,
                        "Back",
                        false,
                    ))
                    .child(tool(
                        "data-sources-forward",
                        IconName::CHEVRON_RIGHT,
                        "Forward",
                        false,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .h(px(LIST_TOOLBAR))
                    .px(px(LIST_SIDE))
                    .child(add)
                    .child(
                        tool("data-sources-remove", IconName::REMOVE, "Remove", has_item)
                            .when(has_item, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.remove(cx)))
                            }),
                    )
                    .child(
                        tool(
                            "data-sources-duplicate",
                            IconName::COPY,
                            "Duplicate",
                            has_item,
                        )
                        .when(has_item, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.duplicate(cx)))
                        }),
                    ),
            )
            .child(
                div()
                    .px(px(LIST_SIDE + 2.0))
                    .pb(px(4.0))
                    .text_color(muted)
                    .child(PROJECT_DATA_SOURCES),
            )
            .child(
                div()
                    .id("data-sources-list")
                    .debug_selector(|| "data-sources-list".into())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .px(px(LIST_SIDE))
                    .children(rows)
                    .when(self.items.is_empty(), |d| {
                        d.child(div().px(px(10.0)).text_color(muted).child(EMPTY_LIST))
                    })
                    .child(
                        // The data sources' problems (`Problems`): what keeps
                        // one from being saved.
                        div()
                            .id("data-sources-problems")
                            .debug_selector(|| "data-sources-problems".into())
                            .min_w(px(0.0))
                            .mt(px(PROBLEMS_GAP))
                            .pt(px(PROBLEMS_GAP - 6.0))
                            .border_t_1()
                            .border_color(theme.color(UiRole::Divider))
                            .pl(px(PROBLEMS_INDENT))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(PROBLEMS)
                            // One line cut with an ellipsis at the list's
                            // edge; the whole text in its tooltip.
                            .children(self.problem().map(|(_, p)| {
                                let tip = p.clone();
                                div()
                                    .id("data-sources-problem")
                                    .debug_selector(|| "data-sources-problem".into())
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .min_w(px(0.0))
                                    .pr(px(LIST_SIDE))
                                    .text_color(theme.color(UiRole::Danger))
                                    .tooltip(move |window, cx| {
                                        Tooltip::text(tip.clone(), None)(window, cx)
                                    })
                                    .child(
                                        div()
                                            .flex_none()
                                            .child(Icon::new(IconName::ERROR).size(px(14.0))),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(p),
                                    )
                            })),
                    ),
            )
            .into_any_element()
    }

    /// The list's width: as dragged, else as last left, kept so that the
    /// page keeps its least width.
    pub(super) fn list_width(&self, window: &Window, cx: &App) -> f32 {
        let width = self
            .drag_width
            .or_else(|| super::layout::list_width(cx))
            .unwrap_or(LIST_WIDTH);
        let room = f32::from(window.viewport_size().width) - ICON_COLUMN - PAGE_MIN_WIDTH;
        width.min(room).max(LIST_MIN_WIDTH).round()
    }

    /// The one-pixel line between the list and the page, with a wider
    /// handle that drags it (the reference's `OnePixelSplitter`).
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
                    .id("data-sources-splitter")
                    .debug_selector(|| "data-sources-splitter".into())
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-(SPLITTER_HANDLE - 1.0) / 2.0))
                    .w(px(SPLITTER_HANDLE))
                    .child(
                        canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
                            },
                            |_, hitbox, window, _| {
                                ui::pointer::request(
                                    gpui::CursorStyle::ResizeLeftRight,
                                    &hitbox,
                                    window,
                                )
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                            let from = this.list_width(window, cx);
                            this.resizing = Some((e.position.x, from));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .into_any_element()
    }

    fn drag_splitter(
        &mut self,
        e: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start, from)) = self.resizing else {
            return;
        };
        if e.pressed_button != Some(gpui::MouseButton::Left) {
            self.end_splitter(cx);
            return;
        }
        self.drag_width = Some(from + f32::from(e.position.x - start));
        self.drag_width = Some(self.list_width(window, cx));
        cx.notify();
    }

    fn end_splitter(&mut self, cx: &mut Context<Self>) {
        if self.resizing.take().is_some() {
            if let Some(width) = self.drag_width.take() {
                super::layout::set_list_width(width, cx);
            }
            cx.notify();
        }
    }

    /// A text field of the form.
    fn field(&self, id: FieldId, error: bool, window: &Window, cx: &App) -> gpui::Div {
        let selector = format!("data-source-{}", id.id());
        let anchors = self.field_anchors.clone();
        editor::field::frame(self.editor(id), id.ui_font(), error, window, cx)
            .relative()
            .child(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .absolute()
                    .size_0(),
            )
            .child(
                canvas(
                    move |b, _, _| {
                        anchors.borrow_mut().insert(id, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    /// A field with the Expand button at its end (`ExpandableTextField`).
    fn expandable(&self, id: FieldId, error: bool, window: &Window, cx: &App) -> gpui::Div {
        self.field(id, error, window, cx)
            .child(ui::form::field_button(
                SharedString::from(format!("data-source-expand-{}", id.id())),
                IconName::EXPAND,
                "Expand",
                Some(Box::new(DialogExpand {
                    field: id.id().to_owned(),
                })),
            ))
    }

    /// The Database field: editable, with the list of the server's
    /// databases behind its arrow.
    fn database_field(&self, window: &Window, cx: &App) -> gpui::Div {
        let anchors = self.anchors.clone();
        self.field(FieldId::Database, false, window, cx)
            .child(
                canvas(
                    move |b, _, _| {
                        anchors.borrow_mut().insert(ListId::Database, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(ui::form::field_button(
                "data-source-database-list",
                IconName::CHEVRON_DOWN,
                "Show Databases",
                Some(Box::new(DialogOpenList {
                    list: ListId::Database.id().to_owned(),
                })),
            ))
    }

    /// The Color choice at the end of the Name field.
    fn color_choice(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let color = self.item().and_then(|i| i.gui().color);
        let anchors = self.anchors.clone();
        let label = color
            .as_deref()
            .and_then(|c| COLORS.iter().find(|(id, _)| *id == c))
            .map(|(_, l)| *l)
            .unwrap_or("Color");
        div()
            .id("data-source-color")
            .debug_selector(|| "data-source-color".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .text_color(theme.color(UiRole::TextMuted))
            .children(
                color
                    .as_deref()
                    .map(|c| div().size(px(10.0)).rounded_full().bg(color_value(c))),
            )
            .child(label)
            .child(Icon::new(IconName::CHEVRON_DOWN).size(px(14.0)))
            .child(
                canvas(
                    move |b, _, _| {
                        anchors.borrow_mut().insert(ListId::Color, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_click(cx.listener(|this, _, window, cx| this.open_list(ListId::Color, window, cx)))
            .into_any_element()
    }

    /// The expanded editor over its field, with a backdrop that closes it.
    fn render_expanded(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.expanded?;
        let bounds = self.field_anchors.borrow().get(&id).copied()?;
        let theme = cx.theme();
        let focused = self.expand_editor.focus_handle(cx).is_focused(window);
        Some(
            div()
                .id("data-source-expanded-backdrop")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.collapse(window, cx)),
                )
                .child(
                    ui::components::input_frame(focused, false, cx)
                        .id("data-source-expanded")
                        .debug_selector(|| "data-source-expanded".into())
                        .occlude()
                        .absolute()
                        .left(bounds.left())
                        .top(bounds.top())
                        .w(bounds.size.width)
                        .h(px(EXPANDED_HEIGHT))
                        .p(px(6.0))
                        .bg(theme.color(UiRole::Input))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(self.expand_editor.clone()),
                )
                .into_any_element(),
        )
    }

    /// A closed combo box showing its choice; a click opens its list.
    fn combo(&self, list: ListId, enabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let anchors = self.anchors.clone();
        let label = self.choice_label(list);
        let icon = match list {
            ListId::Driver => self.item().map(|i| icons::backend(i.profile().backend)),
            _ => None,
        };
        let selector = format!("data-source-combo-{}", list.id());
        let mut content: Vec<AnyElement> = Vec::new();
        if let Some(icon) = icon {
            content.push(Icon::new(icon).size(px(16.0)).into_any_element());
        }
        content.push(div().child(label).into_any_element());
        ui::form::combo(SharedString::from(selector.clone()), content, cx)
            .debug_selector(move || selector.clone())
            .child(
                canvas(
                    move |b, _, _| {
                        anchors.borrow_mut().insert(list, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| this.open_list(list, window, cx)))
            })
            .into_any_element()
    }

    fn row(label: &'static str, width: f32, value: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .w(px(width))
                    .whitespace_nowrap()
                    .child(label),
            )
            .child(div().flex_1().min_w(px(0.0)).child(value))
    }

    /// A field with a short labelled one at its right (Host and Port,
    /// Password and Save).
    fn pair(
        label: &'static str,
        value: impl IntoElement,
        side_label: &'static str,
        side: impl IntoElement,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(div().flex_none().w(px(LABEL_WIDTH)).child(label))
            .child(div().flex_1().min_w(px(0.0)).child(value))
            .child(
                div()
                    .flex_none()
                    .w(px(SIDE_LABEL_WIDTH))
                    .pl(px(SIDE_LABEL_GAP))
                    .child(side_label),
            )
            .child(div().flex_none().w(px(SHORT_FIELD)).child(side))
    }

    fn check(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        on: bool,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label: SharedString = label.into();
        let selector = format!("data-source-check-{id}");
        ui::form::check_row(
            SharedString::from(selector.clone()),
            SharedString::from(format!("{selector}-box")),
            on,
            div().child(label),
            cx,
        )
        .debug_selector(move || selector.clone())
        .h(px(FIELD_HEIGHT))
        .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                this.set_check(id, !on, cx);
            }))
        })
        .into_any_element()
    }

    fn render_name_row(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let comment_shown = self
            .item()
            .is_some_and(|i| i.comment_shown || i.gui().comment.is_some());
        let name_error = self
            .item()
            .is_some_and(|i| db_core::profile::validate_name(&i.source.name).is_err());
        let color = self.color_choice(cx);
        let link = ui::link::link("data-source-add-comment", ADD_COMMENT, cx)
            .debug_selector(|| "data-source-add-comment".into())
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(item) = this.item_mut() {
                    item.comment_shown = true;
                }
                let focus = this.editor(FieldId::Comment).focus_handle(cx);
                window.focus(&focus);
                cx.notify();
            }));
        div()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .px(px(PAGE_SIDE))
            .pr(px(PAGE_RIGHT))
            .pt(px(NAME_TOP))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().flex_none().child(NAME))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            self.field(FieldId::Name, name_error, window, cx)
                                .child(color),
                        ),
                    )
                    .when(!comment_shown, |d| d.child(link)),
            )
            .when(comment_shown, |d| {
                d.child(Self::row(
                    COMMENT,
                    56.0,
                    self.field(FieldId::Comment, false, window, cx),
                ))
            })
            .into_any_element()
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs: Vec<AnyElement> = Tab::ALL
            .into_iter()
            .map(|tab| {
                let selector = format!("data-source-tab-{}", tab.id());
                header_tab(
                    SharedString::from(selector.clone()),
                    tab == self.tab,
                    true,
                    cx,
                )
                .debug_selector(move || selector.clone())
                .px(px(TAB_PAD))
                .h(px(26.0))
                .child(tab.title())
                .on_click(cx.listener(move |this, _, _, cx| this.show_tab(tab, cx)))
                .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(TAB_GAP))
            .px(px(PAGE_SIDE - 8.0))
            .pt(px(TABS_TOP))
            .pb(px(8.0))
            .border_b_1()
            .border_color(cx.theme().color(UiRole::Divider))
            .children(tabs)
            .into_any_element()
    }

    fn render_general(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item().cloned() else {
            return div().into_any_element();
        };
        let p = item.profile().clone();
        let gui = item.gui();
        let dialect = p.dialect();
        let url_only = gui.url_only;
        let tns = dialect == Dialect::Oracle && p.dsn.is_some();
        let mut rows: Vec<(f32, AnyElement)> = Vec::new();
        rows.push((
            0.0,
            Self::row(DRIVER, LABEL_WIDTH, self.combo(ListId::Driver, true, cx)).into_any_element(),
        ));
        let mut kinds: Vec<(&'static str, &'static str)> = vec![("default", "default")];
        if dialect == Dialect::Oracle {
            kinds.push(("tns", "TNS"));
        }
        if dialect != Dialect::Sqlite {
            kinds.push(("url-only", "URL only"));
        }
        let selected_kind = if url_only {
            "url-only"
        } else if tns {
            "tns"
        } else {
            "default"
        };
        let labels: Vec<SharedString> = kinds.iter().map(|(_, l)| (*l).into()).collect();
        let ids: Vec<&'static str> = kinds.iter().map(|(id, _)| *id).collect();
        let selected = ids.iter().position(|id| *id == selected_kind);
        let this = cx.entity().downgrade();
        if kinds.len() > 1 {
            rows.push((
                ROW_GAP,
                Self::row(
                    CONNECTION_TYPE,
                    LABEL_WIDTH,
                    div().flex().child(ui::form::segmented(
                        "data-source-connection-type",
                        &labels,
                        selected,
                        move |ix, _, cx| {
                            if let Some(this) = this.upgrade() {
                                this.update(cx, |this, cx| {
                                    this.set_connection_type(ids[ix], cx);
                                });
                            }
                        },
                        cx,
                    )),
                )
                .into_any_element(),
            ));
        }

        if dialect == Dialect::Sqlite {
            rows.push((
                GROUP_GAP,
                Self::row(
                    FILE,
                    LABEL_WIDTH,
                    self.field(FieldId::Path, false, window, cx),
                )
                .into_any_element(),
            ));
        } else if !url_only && !tns {
            rows.push((
                GROUP_GAP,
                Self::pair(
                    HOST,
                    self.field(FieldId::Host, false, window, cx),
                    PORT,
                    self.field(FieldId::Port, self.port_error, window, cx),
                )
                .into_any_element(),
            ));
        } else if tns {
            rows.push((
                GROUP_GAP,
                Self::row(
                    CONNECT_STRING,
                    LABEL_WIDTH,
                    self.field(FieldId::Dsn, false, window, cx),
                )
                .into_any_element(),
            ));
        }
        if dialect != Dialect::Sqlite {
            rows.push((
                GROUP_GAP,
                Self::row(
                    AUTHENTICATION,
                    LABEL_WIDTH,
                    self.combo(ListId::Auth, true, cx),
                )
                .into_any_element(),
            ));
            if gui.auth == db_core::profile::AuthMethod::UserPassword {
                rows.push((
                    ROW_GAP,
                    Self::pair(
                        USER,
                        self.field(FieldId::User, false, window, cx),
                        "",
                        div(),
                    )
                    .into_any_element(),
                ));
                rows.push((
                    ROW_GAP,
                    Self::pair(
                        PASSWORD,
                        self.password.clone(),
                        SAVE,
                        self.combo(ListId::Save, true, cx),
                    )
                    .into_any_element(),
                ));
            }
        }
        if !url_only && !tns {
            match dialect {
                Dialect::Postgres => rows.push((
                    GROUP_GAP,
                    Self::row(DATABASE, LABEL_WIDTH, self.database_field(window, cx))
                        .into_any_element(),
                )),
                Dialect::Oracle => rows.push((
                    GROUP_GAP,
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(div().flex_none().w(px(LABEL_WIDTH)).child(
                            div().w(px(LABEL_WIDTH - 8.0)).child(self.combo(
                                ListId::ServiceKind,
                                true,
                                cx,
                            )),
                        ))
                        .child(div().flex_1().child(self.field(
                            FieldId::Service,
                            false,
                            window,
                            cx,
                        )))
                        .into_any_element(),
                )),
                Dialect::Sqlite => {}
            }
        }
        if dialect != Dialect::Sqlite || url_only {
            rows.push((
                if url_only { GROUP_GAP } else { URL_GAP },
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(Self::row(
                        URL,
                        LABEL_WIDTH,
                        self.expandable(FieldId::Url, self.url_error.is_some(), window, cx),
                    ))
                    .child(
                        div().pl(px(LABEL_WIDTH)).child(
                            ui::form::comment(
                                self.url_error.clone().unwrap_or_else(|| {
                                    if url_only {
                                        String::new()
                                    } else {
                                        URL_COMMENT.to_owned()
                                    }
                                }),
                                cx,
                            )
                            .text_size(cx.theme().ui_size() - px(1.0))
                            .when(self.url_error.is_some(), |d| {
                                d.text_color(cx.theme().color(UiRole::Danger))
                            }),
                        ),
                    )
                    .into_any_element(),
            ));
        }
        Self::stack(rows)
    }

    fn stack(rows: Vec<(f32, AnyElement)>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .children(
                rows.into_iter()
                    .map(|(gap, row)| div().pt(px(gap)).child(row)),
            )
            .into_any_element()
    }

    fn render_options(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item().cloned() else {
            return div().into_any_element();
        };
        let gui = item.gui();
        let read_only = item.profile().read_only;
        let keep_alive = gui.keep_alive_secs.is_some();
        let auto_disconnect = gui.auto_disconnect_secs.is_some();
        let label_w = 150.0;
        let indent = |e: AnyElement| div().pl(px(OPTION_INDENT)).child(e).into_any_element();
        let seconds = |check: AnyElement, field: gpui::Div, on: bool| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(div().flex_none().w(px(214.0)).child(check))
                .child(
                    div()
                        .w(px(NUMBER_FIELD))
                        .when(!on, |d| d.opacity(DISABLED_ALPHA))
                        .child(field),
                )
                .child("sec.")
                .into_any_element()
        };
        let rows: Vec<(f32, AnyElement)> = vec![
            (
                0.0,
                ui::form::group_header("Connection", cx).into_any_element(),
            ),
            (
                ROW_GAP - 4.0,
                indent(self.check("read-only", "Read-only", read_only, true, cx)),
            ),
            (
                ROW_GAP - 4.0,
                indent(
                    Self::row(
                        "Transaction control:",
                        label_w,
                        div()
                            .w(px(160.0))
                            .child(self.combo(ListId::TxMode, true, cx)),
                    )
                    .into_any_element(),
                ),
            ),
            (
                ROW_GAP,
                indent(
                    Self::row(
                        "Time zone:",
                        label_w,
                        self.field(FieldId::TimeZone, false, window, cx),
                    )
                    .into_any_element(),
                ),
            ),
            (
                ROW_GAP,
                indent(seconds(
                    self.check(
                        "keep-alive",
                        "Run keep-alive query each",
                        keep_alive,
                        true,
                        cx,
                    ),
                    self.field(FieldId::KeepAlive, false, window, cx),
                    keep_alive,
                )),
            ),
            (
                ROW_GAP,
                indent(seconds(
                    self.check(
                        "auto-disconnect",
                        "Auto-disconnect after",
                        auto_disconnect,
                        true,
                        cx,
                    ),
                    self.field(FieldId::AutoDisconnect, false, window, cx),
                    auto_disconnect,
                )),
            ),
            (
                ROW_GAP,
                indent(
                    Self::row(
                        "Session control:",
                        label_w,
                        div()
                            .w(px(240.0))
                            .child(self.combo(ListId::Session, true, cx)),
                    )
                    .into_any_element(),
                ),
            ),
            (
                ROW_GAP,
                indent(
                    Self::row(
                        "Startup script:",
                        label_w,
                        self.expandable(FieldId::StartupScript, false, window, cx),
                    )
                    .into_any_element(),
                ),
            ),
            (
                GROUP_GAP + 8.0,
                ui::form::group_header("Introspection", cx).into_any_element(),
            ),
            (
                ROW_GAP - 4.0,
                indent(self.check("auto-sync", "Auto sync", !gui.no_auto_sync, true, cx)),
            ),
            (
                ROW_GAP - 4.0,
                indent(
                    Self::row(
                        "Default level:",
                        label_w,
                        div()
                            .w(px(200.0))
                            .child(self.combo(ListId::Level, true, cx)),
                    )
                    .into_any_element(),
                ),
            ),
            (
                ROW_GAP,
                indent(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().flex_none().child("Automatic introspection interval"))
                        .child(div().w(px(76.0)).child(self.field(
                            FieldId::RefreshMinutes,
                            false,
                            window,
                            cx,
                        )))
                        .child("minutes")
                        .into_any_element(),
                ),
            ),
        ];
        Self::stack(rows)
    }

    fn render_ssh_ssl(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item().cloned() else {
            return div().into_any_element();
        };
        let p = item.profile();
        let use_ssl = p.tls_mode() != TlsMode::Disable;
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let divider = theme.color(UiRole::Divider);
        let dim = |on: bool, e: AnyElement| {
            div()
                .when(!on, |d| d.opacity(DISABLED_ALPHA))
                .child(e)
                .into_any_element()
        };
        let path_row = |label: &'static str, id: FieldId, cx: &App| {
            dim(
                use_ssl,
                Self::row(
                    label,
                    SSL_LABEL_WIDTH,
                    editor::field::browse_frame(
                        self.editor(id),
                        false,
                        SharedString::from(format!("data-source-browse-{}", id.id())),
                        None,
                        window,
                        cx,
                    ),
                )
                .into_any_element(),
            )
        };
        let disabled_box = |text: &'static str, cx: &App| {
            ui::components::input_frame(false, false, cx)
                .h(px(FIELD_HEIGHT))
                .px(px(8.0))
                .flex()
                .items_center()
                .text_color(muted)
                .child(text)
        };
        let mut rows: Vec<(f32, AnyElement)> = vec![
            (
                0.0,
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(self.check("use-ssh", "Use SSH tunnel", false, false, cx))
                    .child(
                        div()
                            .text_color(muted)
                            .child("SSH tunnels are not supported yet"),
                    )
                    .into_any_element(),
            ),
            (
                ROW_GAP,
                dim(
                    false,
                    Self::row(
                        "SSH configuration:",
                        SSL_LABEL_WIDTH,
                        disabled_box("<none>", cx),
                    )
                    .into_any_element(),
                ),
            ),
            (
                ROW_GAP,
                dim(
                    false,
                    Self::row(
                        "Local port:",
                        SSL_LABEL_WIDTH,
                        div().w(px(108.0)).child(disabled_box("<Dynamic>", cx)),
                    )
                    .into_any_element(),
                ),
            ),
            (GROUP_GAP, div().h(px(1.0)).bg(divider).into_any_element()),
            (
                GROUP_GAP - 6.0,
                self.check("use-ssl", "Use SSL", use_ssl, true, cx),
            ),
            (ROW_GAP, path_row("CA file:", FieldId::CaFile, cx)),
            (
                ROW_GAP,
                path_row("Client certificate file:", FieldId::ClientCert, cx),
            ),
            (
                ROW_GAP,
                path_row("Client key file:", FieldId::ClientKey, cx),
            ),
        ];
        if p.dialect() == Dialect::Oracle {
            rows.push((ROW_GAP, path_row("Wallet folder:", FieldId::WalletDir, cx)));
        }
        rows.push((
            ROW_GAP,
            dim(
                use_ssl,
                Self::row(
                    "Mode:",
                    SSL_LABEL_WIDTH,
                    div()
                        .w(px(MODE_FIELD))
                        .child(self.combo(ListId::SslMode, use_ssl, cx)),
                )
                .into_any_element(),
            ),
        ));
        Self::stack(rows)
    }

    fn render_schemas(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item().cloned() else {
            return div().into_any_element();
        };
        let muted = cx.theme().color(UiRole::TextMuted);
        let divider = cx.theme().color(UiRole::Divider);
        let scope = item
            .gui()
            .scope
            .unwrap_or_else(db_core::schema::Scope::current);
        let all = scope.filter == SchemaFilter::All;
        let listed = |name: &str, current: bool| match &scope.filter {
            SchemaFilter::All => true,
            SchemaFilter::Current => current,
            SchemaFilter::List(names) => names.iter().any(|n| n == name),
            SchemaFilter::Patterns(_) => false,
        };
        let schemas = item.schemas.clone().unwrap_or_default();
        let current_name = schemas
            .iter()
            .find(|s| s.is_current)
            .map(|s| s.name.clone());
        let tree_row = |id: String,
                        label: AnyElement,
                        on: bool,
                        enabled: bool,
                        indent: f32,
                        cx: &mut Context<Self>| {
            let field = id.clone();
            let selector = format!("data-source-schema-row-{id}");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(LIST_ROW))
                .pl(px(indent))
                .child(checkbox(SharedString::from(format!("{id}-box")), on, cx))
                .child(label)
                .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.set_check(&field, !on, cx);
                    }))
                })
                .into_any_element()
        };
        let mut rows: Vec<AnyElement> = vec![tree_row(
            "all-schemas".into(),
            div()
                .flex()
                .gap(px(6.0))
                .child("All schemas")
                .into_any_element(),
            all,
            true,
            0.0,
            cx,
        )];
        let current_on = matches!(scope.filter, SchemaFilter::Current)
            || current_name
                .as_deref()
                .is_some_and(|c| listed(c, true) && !all);
        rows.push(tree_row(
            "current-schema".into(),
            div()
                .flex()
                .gap(px(6.0))
                .child("Current schema")
                .children(
                    current_name
                        .clone()
                        .map(|c| div().text_color(muted).child(format!("({c})"))),
                )
                .into_any_element(),
            current_on || all,
            !all,
            0.0,
            cx,
        ));
        for s in &schemas {
            // The SQL catalogs show; the other system schemas (toast, temporary,
            // Oracle's) only with "Show internal system schemas".
            let internal =
                s.is_system && !matches!(s.name.as_str(), "information_schema" | "pg_catalog");
            if internal && !scope.system && !listed(&s.name, s.is_current) {
                continue;
            }
            let on = all || listed(&s.name, s.is_current);
            rows.push(tree_row(
                format!("schema:{}", s.name),
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(s.name.clone())
                    .when(s.is_system, |d| {
                        d.child(div().text_color(muted).child("system"))
                    })
                    .when(s.is_current, |d| {
                        d.child(div().text_color(muted).child("(current schema)"))
                    })
                    .into_any_element(),
                on,
                !all,
                22.0,
                cx,
            ));
        }
        let status = if self.schemas_loading {
            Some("Reading the schemas\u{2026}")
        } else if item.schemas.is_none() {
            Some("Test the connection settings to read the schemas")
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .pb(px(4.0))
                    .child(
                        small_icon_button(
                            "data-source-schemas-refresh",
                            IconName::REFRESH,
                            !self.schemas_loading,
                            cx,
                        )
                        .debug_selector(|| "data-source-schemas-refresh".into())
                        .tooltip(Tooltip::text("Refresh", None))
                        .on_click(cx.listener(|this, _, _, cx| this.load_schemas(cx))),
                    )
                    .child(small_icon_button(
                        "data-source-schemas-expand",
                        IconName::EXPAND_ALL,
                        false,
                        cx,
                    ))
                    .child(small_icon_button(
                        "data-source-schemas-collapse",
                        IconName::COLLAPSE_ALL,
                        false,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("data-source-schemas-tree")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(120.0))
                    .overflow_y_scroll()
                    .children(rows)
                    .children(status.map(|s| div().pt(px(8.0)).text_color(muted).child(s))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .pt(px(12.0))
                    .border_t_1()
                    .border_color(divider)
                    .child(Self::row(
                        "Schema pattern:",
                        LABEL_WIDTH,
                        self.field(FieldId::SchemaPattern, false, window, cx),
                    ))
                    .child(self.check(
                        "show-system",
                        "Show internal system schemas",
                        scope.system,
                        true,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_test_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let running = self.test == TestState::Running;
        let driver = self
            .item()
            .map(|i| backend_title(i.profile().backend).to_owned())
            .unwrap_or_default();
        let (status_icon, status_text): (Option<AnyElement>, String) = match &self.test {
            TestState::Idle => (None, driver),
            TestState::Running => (
                Some(ui::components::spinner("data-source-test-spinner", cx).into_any_element()),
                driver,
            ),
            TestState::Ok(info) => (
                Some(
                    Icon::new(IconName::CHECKED)
                        .size(px(16.0))
                        .tint(theme.color(UiRole::Success))
                        .into_any_element(),
                ),
                format!("{} {}", info.product, info.version),
            ),
            TestState::Failed(_) => (
                Some(Icon::new(IconName::ERROR).size(px(16.0)).into_any_element()),
                driver,
            ),
        };
        let enabled = self.item().is_some();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(TEST_BAR))
            .px(px(PAGE_SIDE - 1.0))
            .border_t_1()
            .border_color(theme.color(UiRole::Divider))
            .child(
                ui::link::link("data-source-test", TEST_CONNECTION, cx)
                    .debug_selector(|| "data-source-test".into())
                    .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(|this, _, window, cx| {
                            if this.test != TestState::Running {
                                this.start_test(window, cx)
                            }
                        }))
                    }),
            )
            .children(status_icon.map(|icon| {
                div()
                    .id("data-source-test-status")
                    .debug_selector(|| "data-source-test-status".into())
                    .flex()
                    .items_center()
                    .child(icon)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if matches!(this.test, TestState::Ok(_) | TestState::Failed(_)) {
                            this.result_shown = !this.result_shown;
                            cx.notify();
                        }
                    }))
            }))
            .child(
                div()
                    .text_color(muted)
                    .whitespace_nowrap()
                    .child(status_text),
            )
            .when(running, |d| {
                d.child(
                    ui::link::link("data-source-test-cancel", CANCEL_TEST, cx)
                        .debug_selector(|| "data-source-test-cancel".into())
                        .on_click(cx.listener(|this, _, window, cx| this.start_test(window, cx))),
                )
            })
            .into_any_element()
    }

    /// The details of the last test over the form, above Test Connection.
    fn render_result(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.result_shown {
            return None;
        }
        let theme = cx.theme();
        let (title, color, text) = match &self.test {
            TestState::Ok(_) => (SUCCEEDED, theme.color(UiRole::Success), self.result_text()?),
            TestState::Failed(_) => (FAILED, theme.color(UiRole::Danger), self.result_text()?),
            _ => return None,
        };
        let lines: Vec<AnyElement> = text
            .lines()
            .map(|l| div().child(l.to_owned()).into_any_element())
            .collect();
        Some(
            div()
                .id("data-source-test-result")
                .debug_selector(|| "data-source-test-result".into())
                .occlude()
                .absolute()
                .left(px(RESULT_LEFT))
                .bottom(px(TEST_BAR + POINTER.1))
                .w(px(RESULT_WIDTH))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .p(px(14.0))
                .rounded(theme.metric(Metric::CornerRadius))
                .border_1()
                .border_color(theme.color(UiRole::PopupBorder))
                .bg(theme.color(UiRole::SurfaceRaised))
                .shadow(ui::components::popup_shadow())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .pb(px(8.0))
                        .child(div().flex_1().text_color(color).child(title))
                        .child(
                            ui::link::link("data-source-test-copy", COPY, cx)
                                .debug_selector(|| "data-source-test-copy".into())
                                .on_click(cx.listener(|this, _, _, cx| this.copy_result(cx))),
                        ),
                )
                .children(lines)
                .child(pointer(
                    theme.color(UiRole::SurfaceRaised),
                    theme.color(UiRole::PopupBorder),
                ))
                .into_any_element(),
        )
    }

    /// The message of a failed test, over the form (`Connecting to …`).
    fn render_failure(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let failure = self.failure.as_ref()?;
        let theme = cx.theme();
        let text = failure_text(&failure.failure);
        let auth = failure.failure.auth;
        let close = BarButton::new(
            "data-source-failure-close",
            "Close",
            ButtonRole::Cancel,
            Box::new(DialogDismiss),
        )
        .primary(!auth);
        let mut bar = BottomBar::new()
            .separator(false)
            .metrics(44.0, 0.0)
            .button(close);
        if auth {
            bar = bar.button(BarButton::new(
                "data-source-failure-credentials",
                "Change Credentials",
                ButtonRole::Ok,
                Box::new(DialogChangeCredentials),
            ));
        }
        Some(
            div()
                .id("data-source-failure-backdrop")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    div()
                        .id("data-source-failure")
                        .debug_selector(|| "data-source-failure".into())
                        .absolute()
                        .top(px(150.0))
                        .left(px(PAGE_SIDE))
                        .w(px(FAILURE_WIDTH))
                        .flex()
                        .flex_col()
                        .pt(px(16.0))
                        .px(px(16.0))
                        .rounded(theme.metric(Metric::CornerRadius))
                        .border_1()
                        .border_color(theme.color(UiRole::PopupBorder))
                        .bg(theme.color(UiRole::SurfaceRaised))
                        .shadow(ui::components::popup_shadow())
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(12.0))
                                .child(Icon::new(IconName::ERROR_DIALOG).size(px(28.0)))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap(px(6.0))
                                        .child(div().font_weight(FontWeight::SEMIBOLD).child(
                                            format!(
                                                "Could not connect to \u{201c}{}\u{201d}",
                                                failure.name
                                            ),
                                        ))
                                        .children(
                                            text.lines()
                                                .map(|l| div().child(l.to_owned()))
                                                .collect::<Vec<_>>(),
                                        ),
                                ),
                        )
                        .child(bar.render(cx)),
                )
                .into_any_element(),
        )
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.banner.clone()?;
        let theme = cx.theme();
        Some(
            div()
                .id("data-source-banner")
                .debug_selector(|| "data-source-banner".into())
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .px(px(PAGE_SIDE))
                .py(px(6.0))
                .bg(theme.color(UiRole::DangerBg))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(format!("Error: {text}")),
                )
                .child(
                    small_icon_button("data-source-banner-close", IconName::CLOSE, true, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.banner = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_page(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if self.item().is_none() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.color(UiRole::TextMuted))
                .child(NOTHING_SELECTED)
                .into_any_element();
        }
        let body = match self.tab {
            Tab::General => self.render_general(window, cx),
            Tab::Options => self.render_options(window, cx),
            Tab::SshSsl => self.render_ssh_ssl(window, cx),
            Tab::Schemas => self.render_schemas(window, cx),
        };
        let result = self.render_result(cx);
        let failure = self.render_failure(cx);
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .child(self.render_name_row(window, cx))
            .child(self.render_tabs(cx))
            .child(
                div()
                    .id("data-source-form")
                    .debug_selector(|| "data-source-form".into())
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .px(px(PAGE_SIDE))
                    .pr(px(PAGE_RIGHT))
                    .pt(px(FORM_TOP))
                    .pb(px(12.0))
                    .child(body),
            )
            .children(self.render_banner(cx))
            .child(self.render_test_bar(cx))
            .children(result)
            .children(failure)
            .into_any_element()
    }

    fn render_footer(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let status = self.save_error.clone().map(|e| {
            div()
                .id("data-sources-save-error")
                .debug_selector(|| "data-sources-save-error".into())
                .text_color(theme.color(UiRole::Danger))
                .child(e)
        });
        let mut bar = BottomBar::new()
            .help(ui::form::help_button("data-sources-help", None, cx))
            .button(BarButton::new(
                "data-sources-ok",
                "OK",
                ButtonRole::Ok,
                Box::new(DialogOk),
            ))
            .button(BarButton::new(
                "data-sources-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(DialogCancel),
            ))
            .button(
                BarButton::new(
                    "data-sources-apply",
                    "Apply",
                    ButtonRole::Apply,
                    Box::new(DialogApply),
                )
                .enabled(self.modified()),
            );
        if let Some(status) = status {
            bar = bar.status(status);
        }
        bar.render(cx)
    }
}

impl Render for DataSourcesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (surface, text) = (theme.color(UiRole::Surface), theme.color(UiRole::Text));
        let border = theme.color(UiRole::Border);
        let family = theme.ui_family();
        let size = theme.ui_size();
        let icons_column = self.render_page_icons(cx);
        let list = self.render_list(window, cx);
        let page = self.render_page(window, cx);
        let footer = self.render_footer(cx);
        let expanded = self.render_expanded(window, cx);
        let splitter = self.render_splitter(cx);
        let resizing = self.resizing.is_some().then(|| {
            canvas(
                |_, _, _| {},
                |_, _, window, _| {
                    ui::pointer::request_window(gpui::CursorStyle::ResizeLeftRight, window)
                },
            )
            .absolute()
            .size_full()
        });
        div()
            .id("data-sources-dialog")
            .debug_selector(|| "data-sources-dialog".into())
            .key_context("DataSourcesDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_set))
            .on_action(cx.listener(|this, a: &DialogOpenList, window, cx| {
                if let Some(list) = ListId::from_id(&a.list) {
                    this.open_list(list, window, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &DialogSelect, _, cx| {
                let at = this.items.iter().position(|i| i.source.name == a.name);
                if at.is_some() {
                    this.select(at, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &DialogTab, _, cx| {
                if let Some(tab) = Tab::from_id(&a.tab) {
                    this.show_tab(tab, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &DialogAdd, window, cx| {
                if let Some(backend) = Backend::from_id(&a.backend) {
                    this.add(backend, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &DialogRemove, _, cx| this.remove(cx)))
            .on_action(cx.listener(|this, _: &DialogDuplicate, _, cx| this.duplicate(cx)))
            .on_action(cx.listener(|this, _: &DialogTest, window, cx| this.start_test(window, cx)))
            .on_action(cx.listener(|this, _: &DialogShowResult, _, cx| {
                this.result_shown = matches!(this.test, TestState::Ok(_) | TestState::Failed(_));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DialogCopyResult, _, cx| this.copy_result(cx)))
            .on_action(
                cx.listener(|this, _: &DialogChangeCredentials, window, cx| {
                    this.failure = None;
                    this.tab = Tab::General;
                    let focus = this.password.focus_handle(cx);
                    window.focus(&focus);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this, _: &DialogDismiss, _, cx| {
                if this.failure.take().is_none() {
                    this.result_shown = false;
                    this.banner = None;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DialogLoadSchemas, _, cx| this.load_schemas(cx)))
            .on_action(cx.listener(|this, _: &DialogAddComment, _, cx| {
                if let Some(item) = this.item_mut() {
                    item.comment_shown = true;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DialogApply, window, cx| {
                // Disabled while nothing differs from what is kept.
                if this.modified() {
                    this.save(false, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &DialogOk, window, cx| this.save(true, window, cx)))
            .on_action(cx.listener(|this, _: &DialogCancel, window, cx| this.cancel(window, cx)))
            .capture_action(
                cx.listener(|this, _: &editor::commands::Escape, window, cx| {
                    this.escape(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(|this, a: &DialogExpand, window, cx| {
                if let Some(id) = FieldId::from_id(&a.field) {
                    this.expand(id, window, cx);
                }
            }))
            .capture_action(
                cx.listener(|this, _: &editor::commands::Newline, window, cx| {
                    // The expanded editor takes new lines.
                    if this.expanded.is_some() {
                        return;
                    }
                    this.save(true, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if m.control || m.alt || m.platform || m.shift {
                    return;
                }
                if this.expanded.is_some() && e.keystroke.key == "enter" {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "escape" => this.escape(window, cx),
                    "enter" => this.save(true, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_mouse_move(cx.listener(Self::drag_splitter))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_splitter(cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.result_shown {
                        this.result_shown = false;
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(surface)
            .text_color(text)
            .font_family(family)
            .text_size(size)
            .children(ui::dialog_window::title_bar(WINDOW_TITLE, true, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .border_b_1()
                    .border_color(border)
                    .child(icons_column)
                    .child(list)
                    .child(splitter)
                    .child(page),
            )
            .child(footer)
            .children(expanded)
            .children(resizing)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}

impl DataSourcesDialog {
    /// Escape: closes the message or the result, else the dialog.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.expanded.is_some() {
            self.collapse(window, cx);
            return;
        }
        if self.failure.take().is_some() || self.result_shown {
            self.result_shown = false;
            cx.notify();
            return;
        }
        self.cancel(window, cx);
    }
}
