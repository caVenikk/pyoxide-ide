//! The grid's part of the status bar: while a table grid has the focus,
//! the sum of the selected numbers, the number of selected cells and rows
//! (when more than one cell is selected) and the current cell's place
//! (`SUM: 0   500 cells, 500 rows   1:3`), as the reference shows them.

use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Global, IntoElement, Render, Subscription,
    WeakEntity, Window, div, prelude::*, px,
};
use themes::UiRole;
use ui::theme::ThemeExt;
use workbench::{StatusWidget, Workbench};

/// Bumped when a grid's selection or focus changes; the widgets observe it.
#[derive(Default)]
struct Changed(u64);

impl Global for Changed {}

pub(super) fn init(cx: &mut App) {
    cx.set_global(Changed::default());
}

/// Tells the widgets to draw again.
pub(super) fn changed(cx: &mut App) {
    cx.default_global::<Changed>().0 += 1;
}

/// The widget of the status bar.
pub fn status_widget() -> StatusWidget {
    StatusWidget {
        id: "status-db-grid",
        create: Rc::new(|_: WeakEntity<Workbench>, window, cx| {
            let handle = window.window_handle();
            cx.new(|cx| GridStatus::new(handle, cx)).into()
        }),
    }
}

struct GridStatus {
    window: AnyWindowHandle,
    _observe: Subscription,
}

impl GridStatus {
    fn new(window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe_global::<Changed>(|_, cx| cx.notify());
        Self {
            window,
            _observe: observe,
        }
    }
}

impl Render for GridStatus {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grid = super::grids_of(self.window, cx)
            .into_iter()
            .rev()
            .find(|g| g.read(cx).outer.contains_focused(window, cx));
        let summary = grid.and_then(|g| g.read(cx).summary());
        let theme = cx.theme();
        let Some(s) = summary else {
            return div().id("status-db-grid").into_any_element();
        };
        let item = |text: String| div().flex_none().px(px(6.0)).child(text);
        div()
            .id("status-db-grid")
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .text_size(theme.ui_size() - px(1.0))
            .text_color(theme.color(UiRole::StatusBarText))
            .child(item(format!("SUM: {}", s.sum)))
            .children(s.counts.map(item))
            .child(item(s.position))
            .into_any_element()
    }
}
