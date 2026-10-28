//! Opening a data source's query console from the Database tool window
//! (the toolbar's console button, `db.console.new`) and running a `.sql`
//! file in the session of its data source (`db.console.execute`): the data
//! source's handle of the tree gives the console its session.

use gpui::{Context, Window};
use ui::notification::{Notification, NotificationKind};

use super::Explorer;
use crate::console::{self, ConsoleKind, ConsoleTarget};
use crate::sql::widget::{ChooseDialect, SqlGlobal, is_sql_file};

impl Explorer {
    /// What a console of the data source `name` needs: its handle in the
    /// tree (opened when it is not yet).
    fn console_target(&mut self, name: &str, cx: &mut Context<Self>) -> Option<ConsoleTarget> {
        if !self.tree.lives.contains_key(name) {
            self.open_live(name, cx);
        }
        let Some(live) = self.tree.lives.get(name) else {
            self.notify(
                Notification::new(
                    NotificationKind::Warning,
                    name.to_owned(),
                    "The data source could not be opened",
                ),
                cx,
            );
            return None;
        };
        Some(ConsoleTarget {
            source: name.to_owned(),
            backend: live.profile.backend,
            database: live.profile.database_name().map(str::to_owned),
            handle: live.handle.clone(),
        })
    }

    /// Opens the console of `source`, or of the selected data source.
    pub(crate) fn open_console(
        &mut self,
        source: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = source
            .and_then(|name| self.sources().iter().find(|s| s.name == name).cloned())
            .or_else(|| self.selected_source().cloned())
        else {
            return;
        };
        let Some(target) = self.console_target(&source.name, cx) else {
            return;
        };
        let Some(workbench) = self.workbench.upgrade() else {
            return;
        };
        console::open(workbench, target, window.window_handle(), cx);
        cx.notify();
    }

    /// Execute in the active editor: the statement at the caret of a SQL
    /// file runs in that file's own session (a console's, or the one of the
    /// data source the file is attached to, made on the first run); a SQL
    /// file without a data source asks for one; any other editor does
    /// nothing.
    pub(crate) fn execute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workbench) = self.workbench.upgrade() else {
            return;
        };
        let Some(file) = workbench
            .read(cx)
            .active_file(cx)
            .filter(|f| is_sql_file(f))
        else {
            return;
        };
        let existing = console::console_of(&file, cx);
        if let Some(console) = existing
            .clone()
            .filter(|c| c.read(cx).kind == ConsoleKind::Console)
        {
            console.update(cx, |c, cx| c.execute_at_caret(cx));
            return;
        }
        let hub = cx.global::<SqlGlobal>().0.clone();
        let Some(source) = hub.resolve(&file).data_source else {
            window.dispatch_action(Box::new(ChooseDialect), cx);
            return;
        };
        if let Some(console) = existing.filter(|c| c.read(cx).source == source) {
            console.update(cx, |c, cx| c.execute_at_caret(cx));
            return;
        }
        let Some(target) = self.console_target(&source, cx) else {
            return;
        };
        let console = console::attach_file(workbench, target, file, window, cx);
        console.update(cx, |c, cx| c.execute_at_caret(cx));
        cx.notify();
    }
}
