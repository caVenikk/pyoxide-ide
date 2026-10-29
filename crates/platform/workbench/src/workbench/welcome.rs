//! The Welcome tab of a main window (`crate::welcome`): `welcome.open`
//! opens it or selects it, `welcome.theme` and `welcome.keymap` open the
//! lists of its combo boxes.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Entity, Window};

use super::{Tab, TabContent, Workbench};
use crate::tab_view::downcast;
use crate::welcome::{ChooseKeymap, ChooseTheme, Combo, NewScript, OpenWelcome, WelcomeView};

/// Creates the first free `scratch.<ext>`, `scratch_1.<ext>`, ... in `dir`
/// (and `dir` itself), empty.
fn create_scratch(dir: &Path, ext: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut n = 0usize;
    loop {
        let name = match n {
            0 => format!("scratch.{ext}"),
            n => format!("scratch_{n}.{ext}"),
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

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
            pinned: false,
            group: self.splits.focused,
            mirror: false,
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

    /// New Script: the file is made off the UI thread, then opens in a tab.
    pub(crate) fn new_script(
        &mut self,
        a: &NewScript,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext = a.extension.clone().unwrap_or_else(|| "txt".to_owned());
        let dir = match &a.directory {
            Some(dir) => Some(PathBuf::from(dir)),
            None => settings::config_dir().map(|d| d.join("scratches")),
        };
        let Some(dir) = dir else {
            self.message = Some("Cannot create a file: no configuration folder".into());
            cx.notify();
            return;
        };
        let made = cx.background_spawn(async move { create_scratch(&dir, &ext) });
        cx.spawn_in(window, async move |this, cx| {
            let made = made.await;
            let _ = this.update_in(cx, |this, window, cx| match made {
                Ok(path) => this.open_file(path, window, cx),
                Err(e) => {
                    this.message = Some(format!("Cannot create a file: {e}").into());
                    cx.notify();
                }
            });
        })
        .detach();
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
