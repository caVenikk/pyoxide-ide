//! Connecting a data source from the Database tool window: the password
//! asked for when a connection finds none (every way of connecting goes
//! through the handle's connections, see `live::Prompting`), kept as the
//! person chose and forgotten when the data source is disconnected; and
//! Disconnect.
//!
//! - "Never": the typed password serves the data source while it is
//!   connected and is forgotten when it is disconnected (by the person, by
//!   the idle time) or when the connect did not come about, so the next
//!   connect asks again;
//! - "Until restart": kept for the run, until the server refuses it;
//! - "Forever": the data source is saved with that choice and the password
//!   goes to the credential store like the Data Sources dialog's.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

use db_core::PasswordSave;
use db_session::{DataSourceState, TxPolicy};
use futures::channel::oneshot;
use gpui::{AnyWindowHandle, App, AppContext as _, Context, Entity};
use ui::notification::{Notification, NotificationKind};

use super::{Explorer, Explorers};
use crate::prompt::Answer;
use crate::store::{PasswordChange, SaveRequest};

/// What the tool window keeps of the passwords it asked for.
#[derive(Default)]
pub(super) struct State {
    /// Data sources whose typed password is forgotten when they are
    /// disconnected.
    never: HashSet<String>,
}

/// A connection of `source` in the project `root` found no password: asks
/// in the tool window of that project. `None` when there is no such tool
/// window to ask over.
pub(crate) fn ask_password(
    root: &Path,
    source: &str,
    cx: &mut App,
) -> Option<oneshot::Receiver<bool>> {
    let active = cx.active_window();
    let explorers: Vec<(AnyWindowHandle, Entity<Explorer>)> = cx
        .try_global::<Explorers>()?
        .0
        .iter()
        .filter_map(|(w, e)| e.upgrade().map(|e| (*w, e)))
        .filter(|(_, e)| e.read(cx).root.as_deref() == Some(root))
        .collect();
    let (window, explorer) = explorers
        .iter()
        .find(|(w, _)| Some(*w) == active)
        .or(explorers.first())
        .cloned()?;
    Some(explorer.update(cx, |e, cx| e.ask_password(window, source, cx)))
}

impl Explorer {
    /// Asks for the password of `name` over `window` unless it is at hand
    /// by now (another connect may have asked meanwhile); the receiver
    /// tells whether one is held.
    fn ask_password(
        &mut self,
        window: AnyWindowHandle,
        name: &str,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let (Some(root), Some(source)) = (
            self.root.clone(),
            self.sources().iter().find(|s| s.name == name).cloned(),
        ) else {
            let _ = tx.send(false);
            return rx;
        };
        let store = crate::store(cx);
        let check = cx.background_spawn({
            let (store, root, source) = (store.clone(), root.clone(), source.clone());
            async move { store.needs_prompt(&root, &source) }
        });
        let tx = Rc::new(RefCell::new(Some(tx)));
        let reply = move |ok: bool| {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(ok);
            }
        };
        cx.spawn(async move |this, cx| {
            if !check.await {
                reply(true);
                return;
            }
            let user = source.profile.user.clone();
            let save = source.profile.gui_options().password_save;
            let name = source.name.clone();
            let asked = cx.update(|cx| {
                let reply = reply.clone();
                window.update(cx, |_, window, cx| {
                    crate::prompt::ask_to_connect(
                        &name.clone(),
                        &user,
                        save,
                        window,
                        cx,
                        move |answer, cx| {
                            let (Some(answer), Some(this)) = (answer, this.upgrade()) else {
                                reply(false);
                                return;
                            };
                            this.update(cx, |this, cx| this.password_typed(&name, answer, cx));
                            reply(true);
                        },
                    );
                })
            });
            if !matches!(asked, Ok(Ok(()))) {
                reply(false);
            }
        })
        .detach();
        rx
    }

    /// The person typed the password of `name`: it is held for the
    /// connection that asks, for as long as chosen.
    fn password_typed(&mut self, name: &str, answer: Answer, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let store = crate::store(cx);
        store.hold_password(&root, name, answer.password.clone());
        if answer.save == PasswordSave::Never {
            self.connect.never.insert(name.to_owned());
        } else {
            self.connect.never.remove(name);
        }
        if answer.save == PasswordSave::Forever {
            self.keep_forever(name, answer.password, cx);
        }
    }

    /// Saves the data source with "Forever" and the password typed.
    fn keep_forever(&mut self, name: &str, password: db_core::Secret, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut sources = self.sources().to_vec();
        let Some(source) = sources.iter_mut().find(|s| s.name == name) else {
            return;
        };
        source
            .profile
            .gui
            .get_or_insert_with(Default::default)
            .password_save = PasswordSave::Forever;
        let request = SaveRequest {
            sources,
            passwords: vec![(name.to_owned(), PasswordChange::Set(password))],
            ..SaveRequest::default()
        };
        let store = crate::store(cx);
        let run_root = root.clone();
        let save = cx.background_spawn(async move { store.save(&run_root, request) });
        let name = name.to_owned();
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.load(cx),
                Err(e) => this.notify(
                    Notification::new(
                        NotificationKind::Error,
                        name,
                        format!("Cannot keep the password: {e}"),
                    ),
                    cx,
                ),
            });
        })
        .detach();
    }

    /// A data source handle moved to `state`.
    pub(super) fn state_reached(
        &mut self,
        name: &str,
        state: &DataSourceState,
        cx: &mut Context<Self>,
    ) {
        match state {
            DataSourceState::Disconnected | DataSourceState::Failed { .. } => {
                // A password typed for one connect is not kept for the
                // next: a connect that did not come about is one, too.
                if self.connect.never.remove(name)
                    && let Some(root) = self.root.clone()
                {
                    crate::store(cx).release_password(&root, name);
                }
                if *state == DataSourceState::Disconnected {
                    crate::console::disconnected(name, cx);
                }
            }
            DataSourceState::Connected => self.reload_totals(name, cx),
            DataSourceState::Connecting => {}
        }
    }

    /// The data source is connected (or nothing is connecting it): a
    /// disconnect can end something.
    pub(super) fn is_connected(&self, name: &str) -> bool {
        self.tree
            .lives
            .get(name)
            .is_some_and(|l| l.state == DataSourceState::Connected)
    }

    /// Whether Disconnect has something to end for the selected data
    /// source.
    pub(super) fn can_disconnect(&self) -> bool {
        self.selected_source().is_some_and(|s| {
            self.tree.lives.get(&s.name).is_some_and(|l| {
                matches!(
                    l.state,
                    DataSourceState::Connected | DataSourceState::Connecting
                )
            })
        })
    }

    /// Disconnect: closes the connections of the selected data source and
    /// its consoles' sessions; the data source stays in the tree with its
    /// model. Pending changes of a console are asked about first.
    pub(super) fn disconnect_selected(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected_source().map(|s| s.name.clone()) else {
            return;
        };
        if let Some(live) = self.tree.lives.get(&name)
            && matches!(
                live.state,
                DataSourceState::Connected | DataSourceState::Connecting
            )
        {
            live.handle.disconnect(TxPolicy::Ask);
        }
        cx.notify();
    }
}
