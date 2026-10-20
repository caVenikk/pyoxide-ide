//! The Welcome tab of a main window (`crate::welcome`): `welcome.open`
//! opens it or selects it, `welcome.theme` and `welcome.keymap` open the
//! lists of its combo boxes.

use gpui::{AppContext as _, Context, Entity, Window};

use super::{Tab, TabContent, Workbench};
use crate::tab_view::downcast;
use crate::welcome::{ChooseKeymap, ChooseTheme, Combo, OpenWelcome, WelcomeView};

impl Workbench {
    /// The Welcome tab's index and view, when it is open.
    fn welcome_tab(&self) -> Option<(usize, Entity<WelcomeView>)> {
        self.tabs.iter().enumerate().find_map(|(ix, tab)| {
            let view = downcast::<WelcomeView>(tab.view()?)?;
            Some((ix, view.clone()))
        })
    }

    /// Opens the Welcome tab after the tabs open, or selects it.
    pub fn open_welcome(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<WelcomeView> {
        if let Some((ix, view)) = self.welcome_tab() {
            self.activate(ix, window, cx);
            return view;
        }
        let product = self.product.clone();
        let view = cx.new(|cx| WelcomeView::new(product.name, &product.welcome, cx));
        let observed = cx.observe(&view, |_, _, cx| cx.notify());
        self.tabs.push(Tab {
            content: TabContent::View(Box::new(view.clone())),
            _subscriptions: vec![observed],
        });
        self.activate(self.tabs.len() - 1, window, cx);
        view
    }

    pub(crate) fn open_welcome_action(
        &mut self,
        _: &OpenWelcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_welcome(window, cx);
    }

    pub(crate) fn welcome_theme(
        &mut self,
        _: &ChooseTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.open_welcome(window, cx);
        view.update(cx, |v, cx| v.open_list(Combo::Theme, window, cx));
    }

    pub(crate) fn welcome_keymap(
        &mut self,
        _: &ChooseKeymap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.open_welcome(window, cx);
        view.update(cx, |v, cx| v.open_list(Combo::Keymap, window, cx));
    }
}
