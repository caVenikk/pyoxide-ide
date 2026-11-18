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
use std::path::Path;
use std::rc::Rc;

use db_core::PasswordSave;
use db_session::{DataSourceState, TxPolicy};
use futures::channel::oneshot;
use gpui::{AnyWindowHandle, App, AppContext as _, Context, Entity};
use ui::notification::{Notification, NotificationKind};

use super::{Explorer, Explorers};
use crate::prompt::Answer;
use crate::store::PasswordChange;

/// A connection of `source` in the project `root` found no password: asks
/// in the tool window of that project. `None` when there is no such tool
/// window to ask over.
pub(crate) fn ask_password(
    root: &Path,
    source: &str,
    session: &crate::store::CredentialSession,
    cx: &mut App,
) -> Option<oneshot::Receiver<bool>> {
    let active = cx.active_window();
    let explorers: Vec<(AnyWindowHandle, Entity<Explorer>)> = cx
        .try_global::<Explorers>()?
        .0
        .iter()
        .filter_map(|(w, e)| e.upgrade().map(|e| (*w, e)))
        .filter(|(_, e)| {
            e.read(cx).root.as_deref() == Some(root)
                && e.read(cx)
                    .tree
                    .lives
                    .get(source)
                    .is_some_and(|live| live.credentials.same(session))
        })
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
        let Some(session) = self
            .tree
            .lives
            .get(name)
            .map(|live| live.credentials.clone())
        else {
            let _ = tx.send(false);
            return rx;
        };
        let generation = session.generation();
        let store = crate::store(cx);
        let check = cx.background_spawn({
            let (store, root, source) = (store.clone(), root.clone(), source.clone());
            let session = session.clone();
            async move { store.needs_session_prompt(&root, &source, &session) }
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
            let save = source.profile.gui_options().password_save;
            let original = source.clone();
            let asked = cx.update(|cx| {
                let reply = reply.clone();
                window.update(cx, |_, window, cx| {
                    crate::prompt::open(&source, Some(save), window, cx, move |answer, cx| {
                        let (Some(answer), Some(this)) = (answer, this.upgrade()) else {
                            reply(false);
                            return;
                        };
                        this.update(cx, |this, cx| {
                            this.password_typed(original, answer, (session, generation), reply, cx)
                        });
                    });
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
    fn password_typed(
        &mut self,
        original: crate::DataSource,
        mut answer: Answer,
        (session, generation): (crate::store::CredentialSession, u64),
        reply: impl FnOnce(bool) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root.clone() else {
            reply(false);
            return;
        };
        let name = original.name.clone();
        if !self.sources().contains(&original)
            || !self
                .tree
                .lives
                .get(&name)
                .is_some_and(|live| live.credentials.same(&session))
            || session.generation() != generation
        {
            reply(false);
            return;
        }
        let old = original.profile.gui_options();
        let mut updated = original.clone();
        let using_password = answer.auth == db_core::profile::AuthMethod::UserPassword;
        if !using_password {
            answer.save = PasswordSave::Never;
        }
        if !using_password || answer.save == PasswordSave::Forever || answer.auth != old.auth {
            answer.apply(&mut updated.profile);
        }
        if old.password_save != answer.save {
            updated
                .profile
                .gui
                .get_or_insert_with(Default::default)
                .password_save = answer.save;
        }
        let save_to_store = using_password && answer.save == PasswordSave::Forever;
        let write = updated != original || save_to_store;
        let store = crate::store(cx);
        let saved_profile = updated.profile.clone();
        let current = session.clone();
        let save = cx.background_spawn(async move {
            if session.generation() != generation {
                return Err("The session ended.".to_owned());
            }
            if write {
                let change = if save_to_store {
                    PasswordChange::Set(answer.password.clone())
                } else if old.auth == db_core::profile::AuthMethod::UserPassword
                    && old.password_save == PasswordSave::Forever
                {
                    PasswordChange::Clear
                } else {
                    PasswordChange::Keep
                };
                store.save_prompt(&root, &original, &updated, change)?;
            } else if !store.load(&root)?.contains(&original) {
                return Err(
                    "The data source changed while the credentials dialog was open.".to_owned(),
                );
            }
            if session.generation() != generation {
                return Err("The session ended.".to_owned());
            }
            if !using_password {
                store.clear_rejected(&root, &updated.name);
            } else if matches!(answer.save, PasswordSave::Never | PasswordSave::ForSession) {
                if !session.hold(generation, answer) {
                    return Err("The session ended.".to_owned());
                }
            } else {
                store.hold_answer(&root, &updated, answer);
            }
            Ok::<_, String>(())
        });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let ok = this
                .update(cx, |this, cx| match result {
                    Ok(()) => {
                        // This handle is already opening with the new credentials.
                        // Reloading its saved user must not discard the pending request.
                        if !this
                            .tree
                            .lives
                            .get(&name)
                            .is_some_and(|live| live.credentials.same(&current))
                            || current.generation() != generation
                        {
                            return false;
                        }
                        if let Some(live) = this.tree.lives.get_mut(&name) {
                            live.profile = saved_profile;
                        }
                        if write {
                            this.load(cx);
                        }
                        true
                    }
                    Err(e) => {
                        this.notify(
                            Notification::new(
                                NotificationKind::Error,
                                "Connection credentials",
                                format!("Cannot update the saved credentials: {e}"),
                            ),
                            cx,
                        );
                        false
                    }
                })
                .unwrap_or(false);
            reply(ok);
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
                if let Some(live) = self.tree.lives.get(name) {
                    live.credentials.clear();
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
