//! The VCS Operations popup (`Vcs.QuickListPopupAction`, ⌃V on macOS,
//! Alt+` elsewhere): "VCS Operations" in the middle of the window with
//! speed search. Under Git it lists the `Vcs.Operations.Popup` group as
//! the Git quick list provider rearranges it: the "Git" separator,
//! Commit…, Rollback…, then Show History, Annotate with Git Blame and
//! Compare with the Same Repository Version, then Branches…, Push…, Stash
//! Changes… and Unstash Changes…, then Add to VCS, Copy Branch Name and
//! Resolve Conflicts…. Without a repository it offers Create Git
//! Repository…. Commands that are not there yet are shown disabled.

use gpui::{Action, App};
use ui::menu::MenuItem;

/// The popup's title (`vcs.quicklist.popup.title`).
pub const TITLE: &str = "VCS Operations";

/// The command `id` as an action, when it is registered.
fn available(id: &str, cx: &App) -> Option<Box<dyn Action>> {
    ui::command::list(cx)
        .iter()
        .any(|c| c.meta.id == id)
        .then(|| cx.build_action(id, None).ok())
        .flatten()
}

fn item(label: &'static str, id: &str, cx: &App) -> MenuItem {
    match available(id, cx) {
        Some(action) => MenuItem::Action {
            label: label.into(),
            icon: None,
            action: Some(action),
            enabled: true,
            description: None,
            extra: Default::default(),
        },
        None => MenuItem::unavailable(label),
    }
}

/// The rows of the popup: for a project under Git (`git`), else the
/// actions that put it under version control.
pub fn items(git: bool, cx: &App) -> Vec<MenuItem> {
    if !git {
        return vec![item("Create Git Repository\u{2026}", "git.init", cx)];
    }
    vec![
        MenuItem::header("Git"),
        item("Commit\u{2026}", "vcs.commit", cx),
        item("Rollback\u{2026}", "vcs.rollback", cx),
        MenuItem::separator(),
        MenuItem::unavailable("Show History"),
        MenuItem::unavailable("Annotate with Git Blame"),
        MenuItem::unavailable("Compare with the Same Repository Version"),
        MenuItem::separator(),
        item("Branches\u{2026}", "vcs.branches-popup", cx),
        item("Push\u{2026}", "vcs.push", cx),
        MenuItem::unavailable("Stash Changes\u{2026}"),
        MenuItem::unavailable("Unstash Changes\u{2026}"),
        MenuItem::separator(),
        item("Add to VCS", "vcs.add", cx),
        MenuItem::unavailable("Copy Branch Name"),
        MenuItem::unavailable("Resolve Conflicts\u{2026}"),
    ]
}
