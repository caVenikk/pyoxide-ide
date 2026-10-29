//! Drawing of the grid's tab: the toolbar, the WHERE and ORDER BY fields,
//! the column headers, the cells ([`super::body`]), the page bar floating
//! over the bottom of the cells and the error balloon under the WHERE
//! field. Metrics are read off the reference at its main window of 1710 px.

use gpui::{
    AnyElement, App, ClickEvent, Context, DismissEvent, FontWeight, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollWheelEvent, SharedString,
    TextRun, Window, canvas, div, point, prelude::*, px,
};
use themes::{Metric, TextRole, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::pointer::PointerExt;
use ui::theme::ThemeExt;

use super::body::{self, CELL_PAD, Hit};
use super::data::{KeyMark, Selection, SortDir};
use super::{
    ApplyFields, ClearField, CountRows, Dismiss, FIELD_CONTEXT, Field, FirstPage, KEY_CONTEXT,
    LastPage, NextPage, OpenMenu, PAGE_SIZES, PreviousPage, Reload, Resize, SetDefaultPageSize,
    SetPageSize, Stop, TableGrid, VIEW_CONTEXT,
};
use crate::icons;

const TOOLBAR_HEIGHT: f32 = 34.0;
const TOOLBAR_LEFT: f32 = 8.0;
const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;
const SEPARATOR_GAP: f32 = 5.0;
const FIELDS_HEIGHT: f32 = 30.0;
/// The header's icon, gaps and sort mark.
const HEADER_ICON: f32 = 16.0;
const SORT_MARK: f32 = 14.0;
/// Width of the grip on a header's right edge that resizes the column.
const RESIZE_GRIP: f32 = 6.0;
const MIN_COLUMN: f32 = 30.0;
/// Row height over the editor font's line height.
const ROW_EXTRA: f32 = 9.0;
/// The page bar.
const BAR_HEIGHT: f32 = 32.0;
const BAR_BOTTOM: f32 = 10.0;
const BAR_BUTTON: f32 = 24.0;

impl TableGrid {
    fn separator(cx: &App) -> AnyElement {
        div()
            .mx(px(SEPARATOR_GAP))
            .w(px(1.0))
            .h(px(16.0))
            .bg(cx.theme().color(UiRole::Divider))
            .into_any_element()
    }

    pub(super) fn tool(
        id: &'static str,
        icon: IconName,
        tip: &'static str,
        on: bool,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
            .debug_selector(move || id.to_owned())
            .tooltip(Tooltip::text(tip, None))
            .when(!on, |d| d.opacity(DISABLED_ALPHA))
    }

    pub(super) fn text_tool(
        id: &'static str,
        label: &'static str,
        tip: &'static str,
        chevron: bool,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.0))
            .h(px(BUTTON))
            .px(px(4.0))
            .text_size(theme.ui_size() - px(1.0))
            .text_color(theme.color(UiRole::Text))
            .opacity(DISABLED_ALPHA)
            .tooltip(Tooltip::text(tip, None))
            .child(label)
            .when(chevron, |d| {
                d.child(Icon::new(IconName::CHEVRON_DOWN).size(px(10.0)))
            })
            .into_any_element()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let running = self.is_running();
        let has_page = self.page.is_some();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(TOOLBAR_HEIGHT))
            .pl(px(TOOLBAR_LEFT))
            .pr(px(TOOLBAR_LEFT))
            .gap(px(2.0))
            .child(
                Self::tool("grid-reload", IconName::REFRESH, "Reload Page", true, cx)
                    .on_click(cx.listener(|g, _, _, cx| g.reload(cx))),
            )
            .child(Self::tool(
                "grid-history",
                IconName::HISTORY,
                "Query History",
                false,
                cx,
            ))
            .child(
                Self::tool(
                    "grid-stop",
                    if running {
                        IconName::STOP
                    } else {
                        icons::STOP_DISABLED
                    },
                    "Cancel Running Statements",
                    running,
                    cx,
                )
                .when(running, |d| {
                    d.on_click(cx.listener(|g, _, _, cx| g.stop(cx)))
                }),
            )
            .child(Self::separator(cx))
            .child(Self::tool(
                "grid-add-row",
                IconName::ADD,
                "Add Row",
                false,
                cx,
            ))
            .child(Self::tool(
                "grid-delete-row",
                IconName::REMOVE,
                "Delete Row",
                false,
                cx,
            ))
            .child(Self::tool(
                "grid-revert",
                IconName::ROLLBACK,
                "Revert Selected",
                false,
                cx,
            ))
            .child(Self::tool(
                "grid-submit",
                IconName::UPLOAD,
                "Submit",
                false,
                cx,
            ))
            .child(Self::separator(cx))
            .child(Self::text_tool(
                "grid-tx",
                "Tx: Auto",
                "Transaction Mode",
                true,
                cx,
            ))
            .child(Self::separator(cx))
            .child(Self::text_tool("grid-ddl", "DDL", "Go to DDL", false, cx))
            .child(Self::tool(
                "grid-search",
                IconName::SEARCH_SMALL,
                "Find",
                false,
                cx,
            ))
            .child(
                icon_button(
                    "grid-value-panel",
                    IconName::PREVIEW,
                    px(BUTTON),
                    px(ICON),
                    self.values.panel.is_some(),
                    cx,
                )
                .debug_selector(|| "grid-value-panel".to_owned())
                .tooltip(Tooltip::text("Open in Value Editor", None))
                .when(!has_page, |d| d.opacity(DISABLED_ALPHA))
                .when(has_page, |d| {
                    d.on_click(cx.listener(|g, _, window, cx| {
                        if g.values.panel.is_some() {
                            g.close_panel(window, cx)
                        } else {
                            g.open_panel(super::panel::PanelTab::Value, window, cx)
                        }
                    }))
                }),
            )
            .child(div().flex_1())
            .child(self.extractor_button(cx))
            .child(Self::separator(cx))
            .child(
                icon_button(
                    "grid-view-as",
                    IconName::DIFF_SIDE_BY_SIDE,
                    px(BUTTON),
                    px(ICON),
                    self.values.view != super::views::ViewMode::Table,
                    cx,
                )
                .debug_selector(|| "grid-view-as".to_owned())
                .tooltip(Tooltip::text("View As", None))
                .when(!has_page, |d| d.opacity(DISABLED_ALPHA))
                .when(has_page, |d| {
                    d.on_click(cx.listener(|g, e: &ClickEvent, window, cx| {
                        g.show_view_menu(e.position(), window, cx)
                    }))
                }),
            )
            .child(self.transfer_buttons(cx))
            .child(Self::tool(
                "grid-view-options",
                IconName::SHOW,
                "View Options",
                has_page,
                cx,
            ))
            .child(Self::tool(
                "grid-settings",
                IconName::SETTINGS_SMALL,
                "Grid Options",
                false,
                cx,
            ))
            .into_any_element()
    }

    fn render_field(&self, field: Field, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (editor, label, icon, id) = match field {
            Field::Where => (&self.filter, "WHERE", IconName::FILTER, "grid-where"),
            Field::Order => (&self.order, "ORDER BY", icons::ORDER_BY, "grid-order"),
        };
        let empty = editor.read(cx).text().is_empty();
        let label_color = if empty {
            theme.color(UiRole::TextMuted)
        } else {
            theme.text_color(TextRole::Keyword)
        };
        let font = theme.editor_font();
        let line = editor::field_line_height_for(false, window, cx);
        let clear_id: SharedString = format!("{id}-clear").into();
        let name = if field == Field::Order {
            "order"
        } else {
            "where"
        };
        div()
            .id(id)
            .key_context(FIELD_CONTEXT)
            .flex()
            .flex_row()
            .items_center()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .pl(px(8.0))
            .pr(px(6.0))
            .gap(px(6.0))
            .child(Icon::new(icon).size(px(ICON)))
            .child(
                div()
                    .flex_none()
                    .font_family(font.family.clone())
                    .text_size(font.size)
                    .text_color(label_color)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(line)
                    .overflow_hidden()
                    .child(editor.clone()),
            )
            .when(!empty, |d| {
                d.child(
                    icon_button(
                        clear_id,
                        IconName::CLOSE_SMALL,
                        px(18.0),
                        px(16.0),
                        false,
                        cx,
                    )
                    .tooltip(Tooltip::text("Clear", None))
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(ClearField {
                                field: name.to_owned(),
                            }),
                            cx,
                        )
                    }),
                )
            })
            .on_action(cx.listener(|g, _: &ApplyFields, _, cx| g.apply_fields(cx)))
            .into_any_element()
    }

    fn render_fields(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let line = theme.color(UiRole::Border);
        div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(FIELDS_HEIGHT))
            .border_t_1()
            .border_b_1()
            .border_color(line)
            .child(self.render_field(Field::Where, window, cx))
            .child(div().w(px(1.0)).h_full().bg(line))
            .child(self.render_field(Field::Order, window, cx))
            .into_any_element()
    }

    /// Column widths from the content (or the user's) and the editor
    /// font's character width.
    fn column_edges(&self, char_width: Pixels) -> Vec<Pixels> {
        let mut edges = vec![px(0.0)];
        let mut x = px(0.0);
        if let Some(page) = self.shown_page() {
            for c in &page.columns {
                // The content, or the header: name, icon, gaps, sort mark.
                let header = char_width * c.name.chars().count() as f32
                    + px(2.0 * CELL_PAD + 2.0 + HEADER_ICON + 2.0 * SORT_MARK + 2.0 + 24.0);
                let auto = (char_width * c.chars as f32 + px(2.0 * CELL_PAD + 2.0)).max(header);
                let w = if self.hidden.contains(&c.name) {
                    px(0.0)
                } else {
                    self.widths
                        .get(&c.name)
                        .copied()
                        .unwrap_or(auto)
                        .max(px(MIN_COLUMN))
                };
                x += w;
                edges.push(x);
            }
        }
        edges
    }

    fn render_header(&self, header_height: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let layout = self.body.layout().clone();
        let lines = theme.color(UiRole::Border);
        let font = theme.editor_font();
        let mut row = div()
            .relative()
            .flex_none()
            .h(header_height)
            .w_full()
            .overflow_hidden()
            .border_b_1()
            .border_color(lines)
            .bg(theme.color(UiRole::EditorBackground))
            .font_family(font.family.clone())
            .text_size(font.size)
            .text_color(theme.color(UiRole::EditorText))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(layout.numbers)
                    .border_r_1()
                    .border_color(lines),
            );
        let Some(page) = self.shown_page().cloned() else {
            return row.into_any_element();
        };
        let sort = if self.is_transposed() {
            Vec::new()
        } else {
            self.sort_marks(&page)
        };
        let view_width = layout.bounds.size.width.max(px(4000.0));
        let mut cells = div()
            .absolute()
            .top_0()
            .h_full()
            .left(layout.numbers)
            .right_0()
            .overflow_hidden();
        for (i, c) in page.columns.iter().enumerate() {
            let left = layout.edges[i] - self.scroll.x;
            let width = layout.width(i);
            if width <= px(0.0) || left + width < px(0.0) || left > view_width {
                continue;
            }
            let icon = match c.key {
                KeyMark::Primary => icons::COLUMN_KEY,
                KeyMark::Foreign => icons::COLUMN_FOREIGN,
                KeyMark::None => icons::COLUMN,
            };
            let mark = sort.iter().find(|(ix, _)| *ix == i).map(|(_, d)| *d);
            let mark_icon = match mark {
                Some(SortDir::Asc) => IconName::CHEVRON_UP,
                Some(SortDir::Desc) => IconName::CHEVRON_DOWN,
                None => icons::SORTABLE,
            };
            let tip: SharedString = format!("{}: {}", c.name, c.type_name).into();
            let column = i;
            cells = cells.child(
                div()
                    .id(("grid-header", i))
                    .absolute()
                    .top_0()
                    .h_full()
                    .left(left)
                    .w(width)
                    .flex()
                    .flex_row()
                    .items_center()
                    .pl(px(CELL_PAD + 2.0))
                    .pr(px(CELL_PAD))
                    .gap(px(4.0))
                    .border_r_1()
                    .border_color(lines)
                    .tooltip(Tooltip::text(tip, None))
                    .on_click(cx.listener(move |g, _: &ClickEvent, window, cx| {
                        window.focus(&g.focus);
                        g.select_column(column, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |g, e: &MouseDownEvent, window, cx| {
                            if !g.is_transposed() {
                                g.show_header_menu(column, e.position, window, cx);
                            }
                        }),
                    )
                    .child(Icon::new(icon).size(px(HEADER_ICON)))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_weight(FontWeight::BOLD)
                            .child(self.header_label(i)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .opacity(0.8)
                            .child(Icon::new(IconName::FILTER).size(px(SORT_MARK))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(("grid-sort", i))
                            .flex_none()
                            .size(px(SORT_MARK + 2.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme.metric(Metric::ButtonRadius))
                            .hover(|s| s.bg(cx.theme().color(UiRole::Hover)))
                            .debug_selector(move || format!("grid-sort-{column}"))
                            .on_click(cx.listener(move |g, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                g.sort_by(column, window, cx);
                            }))
                            .child(Icon::new(mark_icon).size(px(SORT_MARK))),
                    )
                    .child(
                        div()
                            .id(("grid-resize", i))
                            .absolute()
                            .top_0()
                            .right(px(-RESIZE_GRIP / 2.0))
                            .w(px(RESIZE_GRIP))
                            .h_full()
                            .pointer(gpui::CursorStyle::ResizeColumn)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |g, e: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    g.resize = Some(Resize {
                                        column,
                                        start_x: e.position.x,
                                        start_width: g.body.layout().width(column),
                                    });
                                }),
                            ),
                    ),
            );
        }
        row = row.child(cells);
        row.into_any_element()
    }

    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let focused = self.focus.is_focused(window);
        let colors = body::Colors {
            background: theme.color(UiRole::EditorBackground),
            text: theme.color(UiRole::EditorText),
            null: theme.color(UiRole::TextMuted),
            number: theme.color(UiRole::LineNumber),
            number_current: theme.color(UiRole::LineNumberActive),
            lines: theme.color(UiRole::Divider),
            selection: if focused {
                theme.color(UiRole::Selection)
            } else {
                theme.color(UiRole::SelectionInactive)
            },
            current_row: theme.color(UiRole::CurrentLine),
            expansion_border: theme.color(UiRole::PopupBorder),
            thumb: theme.color(UiRole::Scrollbar),
        };
        let font = theme.editor_font();
        let paint = self.shown_page().cloned().map(|page| body::Paint {
            page,
            layout: self.body.shared(),
            scroll: self.scroll,
            selection: self.selection,
            hover: self.hover,
            font: font.font(),
            font_size: font.size,
            colors,
        });
        let background = colors.background;
        div()
            .id("grid-cells")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_hidden()
            .bg(background)
            .on_hover(cx.listener(|g, hovered: &bool, _, cx| {
                if !*hovered && g.hover.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|g, e: &MouseDownEvent, window, cx| {
                    window.focus(&g.focus);
                    let hit = g.body.hit(e.position, g.scroll);
                    let Some(page) = &g.page else { return };
                    let last_column = page.column_count().saturating_sub(1);
                    match hit {
                        Hit::Cell(pos) => {
                            let selection = match g.selection {
                                Some(s) if e.modifiers.shift => Selection {
                                    anchor: s.anchor,
                                    lead: pos,
                                },
                                _ => Selection::cell(pos),
                            };
                            g.drag_select = true;
                            g.select(selection, cx);
                        }
                        Hit::RowNumber(row) => {
                            g.select(
                                Selection {
                                    anchor: (row, 0),
                                    lead: (row, last_column),
                                },
                                cx,
                            );
                        }
                        Hit::None => {}
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|g, e: &MouseDownEvent, window, cx| {
                    window.focus(&g.focus);
                    if let Hit::Cell(pos) = g.body.hit(e.position, g.scroll) {
                        g.show_cell_menu(pos, e.position, window, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|g, _: &MouseUpEvent, _, _| g.drag_select = false),
            )
            .on_mouse_move(cx.listener(|g, e: &MouseMoveEvent, _, cx| {
                if g.resize.is_some() {
                    return;
                }
                let hit = g.body.hit(e.position, g.scroll);
                let hover = match hit {
                    Hit::Cell(pos) => Some(pos),
                    _ => None,
                };
                if g.drag_select && e.pressed_button == Some(MouseButton::Left) {
                    if let (Some(pos), Some(s)) = (hover, g.selection)
                        && s.lead != pos
                    {
                        g.select(
                            Selection {
                                anchor: s.anchor,
                                lead: pos,
                            },
                            cx,
                        );
                    }
                } else {
                    g.drag_select = false;
                }
                if hover != g.hover {
                    g.hover = hover;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|g, e: &ScrollWheelEvent, _, cx| {
                let line = g.body.row_height();
                let mut delta = e.delta.pixel_delta(line);
                if e.modifiers.shift && delta.x == px(0.0) {
                    delta = point(delta.y, px(0.0));
                }
                let scroll = g
                    .body
                    .clamp(point(g.scroll.x - delta.x, g.scroll.y - delta.y));
                if scroll != g.scroll {
                    g.scroll = scroll;
                    g.hover = None;
                    cx.notify();
                }
            }))
            .children(paint.map(|p| {
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| p.paint(bounds, window, cx),
                )
                .size_full()
            }))
            .into_any_element()
    }

    fn bar_button(
        id: &'static str,
        icon: IconName,
        tip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        run: impl Fn(&mut TableGrid, &mut Context<TableGrid>) + 'static,
    ) -> AnyElement {
        icon_button(id, icon, px(BAR_BUTTON), px(ICON), false, cx)
            .debug_selector(move || id.to_owned())
            .tooltip(Tooltip::text(tip, None))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |g, _: &ClickEvent, _, cx| run(g, cx)))
            })
            .into_any_element()
    }

    fn render_page_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.page.as_ref()?;
        let range = self.range_text()?;
        let theme = cx.theme();
        let radius = theme.metric(Metric::ButtonRadius);
        let hover = theme.color(UiRole::Hover);
        let divider = theme.color(UiRole::Divider);
        let border = theme.color(UiRole::PopupBorder);
        let background = theme.color(UiRole::EditorBackground);
        let text_size = theme.ui_size() - px(1.0);
        let single = page.first_row == 0 && !page.has_more;
        let paged = self.page_size.rows().is_some();
        let has_previous = page.first_row > 0;
        let has_next = page.has_more;
        let muted = theme.color(UiRole::TextMuted);
        let text = theme.color(UiRole::Text);
        let bar_bounds = self.page_bar_bounds.clone();
        let mut bar = div()
            .id("grid-page-bar")
            .flex()
            .flex_row()
            .items_center()
            .h(px(BAR_HEIGHT))
            .px(px(6.0))
            .gap(px(2.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(border)
            .bg(background)
            .text_size(text_size)
            .text_color(text)
            .child(
                canvas(move |b, _, _| bar_bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            );
        if !single {
            bar = bar
                .child(Self::bar_button(
                    "grid-first-page",
                    icons::FIRST_PAGE,
                    "First Page",
                    has_previous && paged,
                    cx,
                    |g, cx| g.first_page(cx),
                ))
                .child(Self::bar_button(
                    "grid-previous-page",
                    IconName::CHEVRON_LEFT,
                    "Previous Page",
                    has_previous && paged,
                    cx,
                    |g, cx| g.previous_page(cx),
                ));
        }
        bar = bar.child(
            div()
                .id("grid-page-size")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .h(px(BAR_BUTTON))
                .px(px(5.0))
                .rounded(radius)
                .hover(move |s| s.bg(hover))
                .debug_selector(|| "grid-page-size".to_owned())
                .on_click(cx.listener(|g, _: &ClickEvent, window, cx| {
                    g.show_page_size_menu(None, window, cx)
                }))
                .child(range)
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(12.0))),
        );
        if let Some(total) = self.total_text() {
            let link = self.total.is_none() && self.counting.is_none();
            bar = bar.child(
                div()
                    .id("grid-count")
                    .flex()
                    .items_center()
                    .h(px(BAR_BUTTON))
                    .px(px(5.0))
                    .rounded(radius)
                    .text_color(if self.counting.is_some() { muted } else { text })
                    .debug_selector(|| "grid-count".to_owned())
                    .when(link, |d| {
                        d.hover(move |s| s.bg(hover))
                            .tooltip(Tooltip::text("Count Rows", None))
                            .on_click(cx.listener(|g, _: &ClickEvent, _, cx| g.count(cx)))
                    })
                    .child(total),
            );
            bar = bar
                .child(Self::bar_button(
                    "grid-next-page",
                    IconName::CHEVRON_RIGHT,
                    "Next Page",
                    has_next && paged,
                    cx,
                    |g, cx| g.next_page(cx),
                ))
                .child(Self::bar_button(
                    "grid-last-page",
                    icons::LAST_PAGE,
                    "Last Page",
                    has_next && paged,
                    cx,
                    |g, cx| g.last_page(cx),
                ));
        }
        bar = bar
            .child(div().mx(px(4.0)).w(px(1.0)).h(px(18.0)).bg(divider))
            .child(
                icon_button(
                    "grid-page-options",
                    IconName::MORE_VERTICAL,
                    px(BAR_BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .opacity(DISABLED_ALPHA),
            );
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(BAR_BOTTOM))
                .flex()
                .justify_center()
                .child(bar)
                .into_any_element(),
        )
    }

    fn render_error(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let error = self.error.as_ref()?;
        let theme = cx.theme();
        Some(
            div()
                .id("grid-error")
                .absolute()
                .top(px(2.0))
                .left(px(24.0))
                .max_w(px(420.0))
                .flex()
                .flex_row()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(theme.metric(Metric::CornerRadius) * 2.0)
                .border_1()
                .border_color(theme.color(UiRole::Danger))
                .bg(theme.color(UiRole::SurfaceRaised))
                .shadow(ui::components::popup_shadow())
                .text_size(theme.ui_size())
                .text_color(theme.color(UiRole::Text))
                .debug_selector(|| "grid-error".to_owned())
                .on_click(cx.listener(|g, _: &ClickEvent, _, cx| {
                    g.error = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_none()
                        .pt(px(1.0))
                        .child(Icon::new(IconName("status/error")).size(px(ICON))),
                )
                .child(div().flex_1().min_w(px(0.0)).child(error.text.clone()))
                .into_any_element(),
        )
    }

    /// The page size menu above the page bar's range.
    pub(super) fn show_page_size_menu(
        &mut self,
        at: Option<gpui::Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.page_size.rows();
        let default = super::default_page_size(cx).rows();
        let check = IconName("actions/checked");
        let mut items = vec![MenuItem::header("Page Size")];
        for size in PAGE_SIZES {
            let mut item =
                MenuItem::action(super::group_thousands(size as u64), SetPageSize { size });
            if current == Some(size) {
                item = item.icon(check);
            }
            if default == Some(size) {
                item = item.secondary("Default");
            }
            items.push(item);
        }
        let mut all = MenuItem::action("All", SetPageSize { size: 0 });
        if current.is_none() {
            all = all.icon(check);
        }
        items.push(all);
        items.push(MenuItem::unavailable("Custom\u{2026}"));
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Set as Default", SetDefaultPageSize));
        let bounds = self.page_bar_bounds.get();
        let position =
            at.unwrap_or_else(|| point(bounds.left() + px(40.0), bounds.top() - px(4.0)));
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription = cx.subscribe(&menu, |g, _, _: &DismissEvent, cx| {
            g.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position,
            corner: gpui::Corner::BottomLeft,
            _subscription: subscription,
        });
        cx.notify();
    }
}

impl Render for TableGrid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let font = theme.editor_font();
        let background = theme.color(UiRole::EditorBackground);
        // The editor font's character width and line height.
        let run = TextRun {
            len: 1,
            font: font.font(),
            color: theme.color(UiRole::EditorText),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let char_width = window
            .text_system()
            .shape_line("0".into(), font.size, &[run], None)
            .width;
        let line_height = (font.size * font.line_spacing).round();
        let row_height = line_height + px(ROW_EXTRA);
        self.sync_panel(cx);
        self.sync_text_view(cx);
        let text_view = self.values.view == super::views::ViewMode::Text;
        let rows = self.shown_page().map_or(0, |p| p.row_count());
        let last_number = self.shown_page().map_or(1, |p| p.first_row + rows as u64);
        let digits = last_number.to_string().len().max(2);
        let numbers = char_width * digits as f32 + px(CELL_PAD * 4.0 + 8.0);
        let edges = self.column_edges(char_width);
        self.body.update(row_height, numbers, edges, rows);
        self.scroll = self.body.clamp(self.scroll);

        let toolbar = self.render_toolbar(cx);
        let fields = (!self.is_result()).then(|| self.render_fields(window, cx));
        let header = self.render_header(row_height + px(1.0), cx);
        let cells = self.render_body(window, cx);
        let text = self.values.text.as_ref().filter(|_| text_view).map(|t| {
            div()
                .id("grid-text-view")
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .overflow_hidden()
                .child(t.editor.clone())
                .into_any_element()
        });
        let panel = self.render_panel(cx);
        let bar = self.render_page_bar(cx);
        let error = self.render_error(cx);
        let popup = self
            .menu
            .as_ref()
            .map(|m| ui::menu::overlay_at(&m.menu, m.position, m.corner, window));
        let grid = div()
            .id("grid-area")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .on_mouse_move(cx.listener(|g, e: &MouseMoveEvent, _, cx| {
                if let Some(r) = g.resize {
                    if e.pressed_button != Some(MouseButton::Left) {
                        g.resize = None;
                        return;
                    }
                    let Some(name) = g
                        .page
                        .as_ref()
                        .and_then(|p| p.columns.get(r.column))
                        .map(|c| c.name.clone())
                    else {
                        return;
                    };
                    let w = (r.start_width + e.position.x - r.start_x).max(px(MIN_COLUMN));
                    g.widths.insert(name, w);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|g, _: &MouseUpEvent, _, _| {
                    g.resize = None;
                    g.drag_select = false;
                }),
            )
            .children((!text_view).then_some(header))
            .children(if text_view { text } else { Some(cells) })
            .children(bar)
            .children(error);
        let content = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .child(grid)
            .children(panel);
        div()
            .id("table-grid")
            .key_context(VIEW_CONTEXT)
            .track_focus(&self.outer)
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .on_action(cx.listener(|g, _: &Reload, _, cx| g.reload(cx)))
            .on_action(cx.listener(|g, _: &Stop, _, cx| g.stop(cx)))
            .on_action(cx.listener(|g, _: &Dismiss, _, cx| g.dismiss(cx)))
            .on_action(cx.listener(|g, _: &FirstPage, _, cx| g.first_page(cx)))
            .on_action(cx.listener(|g, _: &PreviousPage, _, cx| g.previous_page(cx)))
            .on_action(cx.listener(|g, _: &NextPage, _, cx| g.next_page(cx)))
            .on_action(cx.listener(|g, _: &LastPage, _, cx| g.last_page(cx)))
            .on_action(cx.listener(|g, _: &CountRows, _, cx| g.count(cx)))
            .child(toolbar)
            .children(fields)
            .child(content)
            .children(popup)
    }
}
