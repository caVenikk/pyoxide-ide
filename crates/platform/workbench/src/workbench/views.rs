//! Editor tabs of a module's views ([`crate::tab_view`]): a module opens
//! its view in a tab after the open ones, or selects the tab that already
//! shows the same thing (a table's data, a console).

use gpui::{App, Context, Entity, Window};

use super::{Tab, TabContent, Workbench};
use crate::tab_view::{TabView, downcast};

impl Workbench {
    /// Selects the tab whose view of type `T` `same` accepts, or opens the
    /// view `create` makes in a new tab after the open ones and selects it.
    pub fn open_tab_view<T: TabView + 'static>(
        &mut self,
        same: impl Fn(&T, &App) -> bool,
        create: impl FnOnce(&mut Window, &mut Context<Self>) -> Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<T> {
        let open = self.tabs.iter().enumerate().find_map(|(ix, tab)| {
            let view = downcast::<T>(tab.view()?)?;
            same(view.read(cx), cx).then(|| (ix, view.clone()))
        });
        if let Some((ix, view)) = open {
            self.activate(ix, window, cx);
            return view;
        }
        let view = create(window, cx);
        let observed = cx.observe(&view, |_, _, cx| cx.notify());
        self.tabs.push(Tab {
            content: TabContent::View(Box::new(view.clone())),
            pinned: false,
            group: self.splits.focused,
            mirror: false,
            _subscriptions: vec![observed],
        });
        self.activate(self.tabs.len() - 1, window, cx);
        view
    }

    /// The views of type `T` in the editor tabs, the selected one first.
    pub fn tab_views<T: TabView + 'static>(&self) -> Vec<Entity<T>> {
        let mut views: Vec<(bool, Entity<T>)> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(ix, tab)| {
                let view = downcast::<T>(tab.view()?)?;
                Some((Some(ix) == self.active, view.clone()))
            })
            .collect();
        views.sort_by_key(|(active, _)| !active);
        views.into_iter().map(|(_, v)| v).collect()
    }
}
