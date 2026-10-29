//! The menu of a right click on a cell (shot DB44): the value features of
//! this issue (the value panel, record and aggregate views, display types
//! of binary columns, loading and saving a whole value, the aggregate
//! copy). Other items of the reference's menu (copy formats, editing,
//! filters) come with their own issues; their rows are added in
//! [`TableGrid::cell_menu_items`].

use gpui::{Context, DismissEvent, Pixels, Point, Window};
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};

use super::data::{Pos, Selection};
use super::value::{self, DisplayType};
use super::values::{
    CopyAggregation, LoadFullValue, SaveValue, SetDisplayType, SetView, ShowAggregates, ShowRecord,
    ShowValue,
};
use super::views::ViewMode;
use super::{OpenMenu, TableGrid};

impl TableGrid {
    /// The menu's rows for a click on `pos` of the data page.
    pub(super) fn cell_menu_items(&self, pos: Pos) -> Vec<MenuItem> {
        let mut items = vec![
            MenuItem::action("Show Record View", ShowRecord),
            MenuItem::action("Open in Value Editor", ShowValue),
            MenuItem::action("Show Aggregate View", ShowAggregates),
            MenuItem::separator(),
        ];
        let value = self.cell_value(pos);
        let binary = value
            .as_deref()
            .is_some_and(|v| value::bytes_of(v).is_some());
        let cut = value.as_deref().is_some_and(value::is_truncated);
        if binary {
            let current = self.display_type(pos.1);
            let check = IconName::CHECKED;
            let choices = DisplayType::ALL
                .iter()
                .map(|d| {
                    let label = if *d == DisplayType::Detect {
                        match value.as_deref().and_then(value::bytes_of) {
                            Some(b) if b.len() == 16 => "Detect (UUID)".to_owned(),
                            _ => "Detect (Hex)".to_owned(),
                        }
                    } else {
                        d.title().to_owned()
                    };
                    let mut item = MenuItem::action(
                        label,
                        SetDisplayType {
                            column: pos.1,
                            display: d.id().to_owned(),
                        },
                    );
                    if *d == current {
                        item = item.icon(check);
                    }
                    item
                })
                .collect();
            items.push(MenuItem::submenu("Change Display Type", choices));
        }
        if cut {
            items.push(MenuItem::action("Load Full Value", LoadFullValue));
        }
        if binary || cut {
            items.push(MenuItem::action(
                "Save LOB\u{2026}",
                SaveValue { path: None },
            ));
        }
        if binary || cut {
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::action(
            "Copy Aggregation Result (SUM)",
            CopyAggregation {
                name: Some("SUM".to_owned()),
            },
        ));
        items
    }

    /// Opens the menu at `at` for a click on the cell `pos` of the shown
    /// page; a click outside the selection selects the cell first.
    pub(super) fn show_cell_menu(
        &mut self,
        pos: Pos,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.is_some_and(|s| s.contains(pos)) {
            self.select(Selection::cell(pos), cx);
        }
        let Some(data) = self.data_pos() else {
            return;
        };
        let items = self.cell_menu_items(data);
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription = cx.subscribe(&menu, |g, _, _: &DismissEvent, cx| {
            g.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position: at,
            corner: gpui::Corner::TopLeft,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The View As menu (Transpose, Table, Tree, Text) under its button.
    pub(super) fn show_view_menu(
        &mut self,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.values.view;
        let item = |label: &str, view: ViewMode| {
            let mut item = MenuItem::action(
                label.to_owned(),
                SetView {
                    view: view.id().to_owned(),
                },
            );
            if current == view {
                item = item.icon(IconName::CHECKED);
            }
            item
        };
        let items = vec![
            item("Transpose", ViewMode::Transpose),
            item("Table", ViewMode::Table),
            MenuItem::unavailable("Tree"),
            item("Text", ViewMode::Text),
        ];
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription = cx.subscribe(&menu, |g, _, _: &DismissEvent, cx| {
            g.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position: gpui::point(at.x + gpui::px(12.0), at.y + gpui::px(14.0)),
            corner: gpui::Corner::TopRight,
            _subscription: subscription,
        });
        cx.notify();
    }
}
