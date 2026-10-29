//! Models for the SQL files' code intelligence. A file attached to a data
//! source gets table and column completion from the source's metadata
//! model; when the source has no cached model and the Database tool window
//! has not loaded it, the model is loaded here, in the background, at most
//! once per data source in a run:
//!
//! - when no password has to be typed (none is needed, or the environment,
//!   a kept password or one held for this run gives it), the data source is
//!   opened like the tree opens it, and a failure is told in a balloon;
//! - else nothing is asked of the person: the hub is told, the file's
//!   toolbar shows a hint with a "Connect" link, and the link connects as
//!   the tree does, which asks for the password (`connect`).
//!
//! Nothing here waits for the database or the credential store on the UI
//! thread.

use std::collections::HashSet;
use std::path::Path;

use gpui::{App, AppContext as _, BorrowAppContext as _, Context};
use ui::notification::{Notification, NotificationKind};

use super::{Explorer, Explorers};
use crate::sql::widget::SqlGlobal;

/// What the tool window keeps of the loads for code intelligence.
#[derive(Default)]
pub(super) struct State {
    /// Data sources asked for before their list was read.
    pending: Vec<String>,
    /// Data sources loading for code intelligence: a failure is told in a
    /// balloon, whether or not the tree shows them.
    loading: HashSet<String>,
}

/// Code intelligence of a file in the project `root` needs the model of
/// `source` and there is none cached (called once per data source in a
/// run): the tool window of that project loads it.
pub(crate) fn request(root: &Path, source: &str, cx: &mut App) {
    let explorers: Vec<_> = cx
        .try_global::<Explorers>()
        .map(|e| e.0.iter().filter_map(|(_, e)| e.upgrade()).collect())
        .unwrap_or_default();
    let explorer = explorers
        .into_iter()
        .find(|e| e.read(cx).root.as_deref() == Some(root));
    if let Some(explorer) = explorer {
        explorer.update(cx, |e, cx| e.model_wanted(source, cx));
    }
}

impl Explorer {
    /// The model of `name` is wanted by code intelligence.
    fn model_wanted(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(sources) = &self.sources else {
            self.model_load.pending.push(name.to_owned());
            return;
        };
        let (Some(root), Some(source)) = (
            self.root.clone(),
            sources.iter().find(|s| s.name == name).cloned(),
        ) else {
            return;
        };
        // The tree has it open: its model arrives with its refresh.
        if self.tree.lives.contains_key(name) {
            return;
        }
        let store = crate::store(cx);
        let hub = cx.global::<SqlGlobal>().0.clone();
        let check = cx.background_spawn({
            let (root, source) = (root.clone(), source.clone());
            async move { store.needs_prompt(&root, &source) }
        });
        cx.spawn(async move |this, cx| {
            let needs_prompt = check.await;
            let _ = this.update(cx, |this, cx| {
                if needs_prompt {
                    hub.set_needs_connect(&root, &source.name, true);
                    // The toolbars show the hint.
                    cx.update_global::<SqlGlobal, _>(|_, _| {});
                } else {
                    this.start_model_load(&source.name, cx);
                }
            });
        })
        .detach();
    }

    /// Asks again for what was asked for before the list was read.
    pub(super) fn run_pending_model_loads(&mut self, cx: &mut Context<Self>) {
        for name in std::mem::take(&mut self.model_load.pending) {
            self.model_wanted(&name, cx);
        }
    }

    /// Opens the data source for the model's sake, unless it is open.
    fn start_model_load(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(live) = self.tree.lives.get_mut(name) {
            live.refresh();
        } else {
            self.model_load.loading.insert(name.to_owned());
            self.open_live(name, cx);
        }
        self.rebuild_rows();
        cx.notify();
    }

    /// The "Connect" link of a SQL file's toolbar: connects to `name` as
    /// the tree does; the connection asks for the password when none is to
    /// be had without asking.
    pub(crate) fn connect_for_sql(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.sources().iter().any(|s| s.name == name) {
            self.start_model_load(name, cx);
        }
    }

    /// A load of the data source finished with a model.
    pub(super) fn model_load_finished(&mut self, name: &str) {
        self.model_load.loading.remove(name);
    }

    /// A refresh of the data source failed: tells whether the failure
    /// belongs in a balloon (the load was ours, and nothing else tells it;
    /// a password that was not typed is no failure to tell).
    pub(super) fn model_load_failed(&mut self, name: &str, error: &db_core::Error) -> bool {
        let ours = self.model_load.loading.remove(name);
        ours && error.kind != db_core::ErrorKind::Cancelled
    }

    /// The notification of a failed load for code intelligence.
    pub(super) fn model_load_notification(name: &str, message: String) -> Notification {
        Notification::new(
            NotificationKind::Warning,
            name.to_owned(),
            format!("Cannot load the metadata for code completion: {message}"),
        )
    }
}
