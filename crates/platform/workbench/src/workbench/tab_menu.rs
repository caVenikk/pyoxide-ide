//! The context menu of an editor tab and the commands of its Close group
//! and Pin Tab: the reference's `EditorTabPopupMenu` with
//! `EditorTabPopupMenuEx` (in their order, the items Pyoxide has), the
//! actions of `CloseEditorsGroup` (`CloseAllEditorsButActiveAction`,
//! `CloseAllEditorsAction`, `CloseAllUnmodifiedEditorsAction`,
//! `CloseAllUnpinnedEditorsAction`, `CloseAllEditorsToTheLeftAction`,
//! `CloseAllEditorsToTheRightAction`) and `PinActiveTabAction`.
//!
//! A right click on a tab opens the menu where the pointer is; the active
//! tab stays, the items act on the clicked tab. Items whose condition does
//! not hold are left out, as the reference's popup hides disabled actions
//! of `CloseEditorsActionBase` (Close Other Tabs stays, disabled with one
//! tab). Pinned tabs stay on the left ("Keep pinned tabs on the left",
//! `JBTabsImpl.groupPinnedFirst`) and stay open on every Close action but
//! Close and Close All Tabs.

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, InteractiveElement, Pixels, Point,
    Subscription, Window, point, px,
};
use serde_json::{Value as Json, json};
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem};
use vcs::FileStatus;

use super::Workbench;
use crate::commands::*;

/// The open menu, the tab it acts on and where it opened.
pub(super) struct TabMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    index: usize,
    _subscription: Subscription,
}

/// The reference's file manager name in Open In (`RevealFileAction.
/// getFileManagerName`, the short text of the `RevealInPopup` place).
#[cfg(target_os = "macos")]
const FILE_MANAGER: &str = "Finder";
#[cfg(windows)]
const FILE_MANAGER: &str = "Explorer";
#[cfg(not(any(target_os = "macos", windows)))]
const FILE_MANAGER: &str = "File Manager";

/// Moves the pinned tabs before the others as `JBTabsImpl.
/// groupPinnedFirst` does: a pinned tab after an unpinned one trades places
/// with the first unpinned one. Returns the new order as indices into
/// `pinned`.
pub(super) fn group_pinned_first(pinned: &[bool]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..pinned.len()).collect();
    let mut first_unpinned: Option<usize> = None;
    for i in 0..order.len() {
        if pinned[order[i]] {
            if let Some(f) = first_unpinned {
                order.swap(f, i);
                first_unpinned = Some(f + 1);
            }
        } else if first_unpinned.is_none() {
            first_unpinned = Some(i);
        }
    }
    order
}

impl Workbench {
    /// The handlers of the tab commands.
    pub(super) fn tab_menu_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        root.on_action(cx.listener(Self::close_other_tabs))
            .on_action(cx.listener(Self::close_all_tabs))
            .on_action(cx.listener(Self::close_unmodified_tabs))
            .on_action(cx.listener(Self::close_unpinned_tabs))
            .on_action(cx.listener(Self::close_tabs_to_left))
            .on_action(cx.listener(Self::close_tabs_to_right))
            .on_action(cx.listener(Self::toggle_pin_tab))
            .on_action(cx.listener(Self::show_tab_menu))
            .on_action(cx.listener(Self::reveal_file))
            .on_action(cx.listener(Self::open_in_terminal))
    }

    /// Whether a tab is pinned.
    pub fn is_tab_pinned(&self, index: usize) -> bool {
        self.tabs.get(index).is_some_and(|t| t.pinned)
    }

    /// The tab a command acts on: `index`, else the active one.
    fn tab_of(&self, index: Option<usize>) -> Option<usize> {
        index.or(self.active).filter(|&i| i < self.tabs.len())
    }

    // ---- what each Close action closes -----------------------------------------

    /// Close Other Tabs: all but `index` and the pinned ones.
    fn others_of(&self, index: usize) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&i| i != index && !self.tabs[i].pinned)
            .collect()
    }

    /// Close Tabs to the Left (`left`) or Right of `index`: the unpinned
    /// ones there.
    fn beside(&self, index: usize, left: bool) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&i| if left { i < index } else { i > index })
            .filter(|&i| !self.tabs[i].pinned)
            .collect()
    }

    /// Close All but Pinned.
    fn unpinned(&self) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&i| !self.tabs[i].pinned)
            .collect()
    }

    /// Close Unmodified Tabs: the unpinned tabs whose files have no version
    /// control status (`FileEditorManagerImpl.isChanged`: a status other
    /// than unknown or not changed is a change). Nothing without version
    /// control (the action is enabled only with an active VCS).
    fn unmodified(&self, cx: &App) -> Vec<usize> {
        if self.version_control.service.is_none() {
            return Vec::new();
        }
        (0..self.tabs.len())
            .filter(|&i| !self.tabs[i].pinned)
            .filter(|&i| match self.tabs[i].path() {
                Some(path) => matches!(
                    self.vcs_file_status(path, cx),
                    FileStatus::NotChanged | FileStatus::Unversioned
                ),
                None => true,
            })
            .collect()
    }

    /// Closes the tabs at `indices` (`closeFilesWithChecks`), from the
    /// right so the others keep their indices; stops at a tab that asks
    /// first (its file changed on disk).
    fn close_tabs(&mut self, mut indices: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            let count = self.tabs.len();
            self.close_tab(index, window, cx);
            if self.tabs.len() == count {
                break;
            }
        }
    }

    // ---- commands ---------------------------------------------------------------

    fn close_other_tabs(
        &mut self,
        a: &CloseOtherTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.tab_of(a.index) {
            self.close_tabs(self.others_of(index), window, cx);
        }
    }

    fn close_all_tabs(&mut self, _: &CloseAllTabs, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs((0..self.tabs.len()).collect(), window, cx);
    }

    fn close_unmodified_tabs(
        &mut self,
        _: &CloseUnmodifiedTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = self.unmodified(cx);
        self.close_tabs(tabs, window, cx);
    }

    fn close_unpinned_tabs(
        &mut self,
        _: &CloseUnpinnedTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_tabs(self.unpinned(), window, cx);
    }

    fn close_tabs_to_left(
        &mut self,
        a: &CloseTabsToLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.tab_of(a.index) {
            self.close_tabs(self.beside(index, true), window, cx);
        }
    }

    fn close_tabs_to_right(
        &mut self,
        a: &CloseTabsToRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.tab_of(a.index) {
            self.close_tabs(self.beside(index, false), window, cx);
        }
    }

    /// Pin Tab / Unpin Tab (`EditorWindow.setFilePinned`); the tabs are
    /// then grouped with the pinned ones first. The active tab stays
    /// active.
    fn toggle_pin_tab(&mut self, a: &TogglePinTab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tab_of(a.index) {
            self.toggle_pin(index, cx);
        }
    }

    /// Pins tab `index` or unpins it.
    pub(super) fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs[index].pinned = !self.tabs[index].pinned;
        let pinned: Vec<bool> = self.tabs.iter().map(|t| t.pinned).collect();
        let order = group_pinned_first(&pinned);
        if order.iter().enumerate().any(|(i, &o)| i != o) {
            let mut old: Vec<Option<super::Tab>> = self.tabs.drain(..).map(Some).collect();
            self.tabs = order.iter().map(|&o| old[o].take().unwrap()).collect();
            self.active = self.active.and_then(|a| order.iter().position(|&o| o == a));
        }
        cx.notify();
    }

    /// `tabs.context-menu`: the menu below the tab.
    fn show_tab_menu(&mut self, a: &ShowTabMenu, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tab_of(a.index) else {
            return;
        };
        let bounds = self.tab_bounds.borrow().get(index).copied();
        let position = match bounds {
            Some(b) => point(b.left(), b.bottom()),
            None => point(px(0.0), px(0.0)),
        };
        self.open_editor_tab_menu(index, position, window, cx);
    }

    /// `project.reveal-file`: Open In > Finder (`RevealFileAction`).
    fn reveal_file(&mut self, a: &RevealFile, _: &mut Window, cx: &mut Context<Self>) {
        let path = match &a.path {
            Some(p) => self.resolve(p, cx),
            None => match self.active_file(cx) {
                Some(p) => p,
                None => return,
            },
        };
        if crate::shot::native_windows() {
            cx.reveal_path(&path);
        }
    }

    /// `terminal.open-in`: Open In > Terminal, a new Terminal tab in the
    /// file's folder (the terminal plugin's `OpenInTerminal`).
    fn open_in_terminal(
        &mut self,
        a: &OpenInTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = match &a.path {
            Some(p) => self.resolve(p, cx),
            None => match self.active_file(cx) {
                Some(p) => p,
                None => return,
            },
        };
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().map(Path::to_owned).unwrap_or(path)
        };
        self.open_terminal_in(dir, window, cx);
    }

    // ---- the menu ---------------------------------------------------------------

    /// Opens the context menu of tab `index` at `position` (window
    /// coordinates).
    pub(super) fn open_editor_tab_menu(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        let items = self.tab_menu_items(index, window, cx);
        let menu = ContextMenu::build(items, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.tab_menu = None;
            cx.notify();
        });
        self.tab_menu = Some(TabMenu {
            menu,
            position,
            index,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The tab whose menu is open.
    pub(super) fn tab_menu_index(&self) -> Option<usize> {
        self.tab_menu.as_ref().map(|m| m.index)
    }

    pub(super) fn render_tab_menu(&self, window: &Window) -> Option<AnyElement> {
        let open = self.tab_menu.as_ref()?;
        Some(ui::menu::overlay(&open.menu, open.position, window))
    }

    /// The open menu for `workbench.state`.
    pub(super) fn tab_menu_json(&self, cx: &App) -> Json {
        match &self.tab_menu {
            Some(open) => json!({
                "tab": open.index,
                "menu": open.menu.read(cx).state_json(),
            }),
            None => Json::Null,
        }
    }

    /// The rows of the menu of tab `index`, in the reference's order:
    /// the Close group | Copy Path/Reference… | Pin Tab | Run and Debug
    /// the file | Open In | Git | Rename File….
    fn tab_menu_items(&self, index: usize, window: &Window, cx: &App) -> Vec<MenuItem> {
        let focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        let keys = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &focus, window).unwrap_or_default()
        };
        let count = self.tabs.len();
        let some = Some(index);
        let mut items = vec![
            MenuItem::action("Close", CloseTab { index: some })
                .shortcut(keys(&CloseTab { index: None }))
                .mnemonic(0),
            MenuItem::action("Close Other Tabs", CloseOtherTabs { index: some })
                .shortcut(keys(&CloseOtherTabs { index: None }))
                .enabled(count > 1)
                .mnemonic(6),
            MenuItem::action("Close All Tabs", CloseAllTabs).mnemonic(6),
        ];
        if !self.unmodified(cx).is_empty() {
            items.push(MenuItem::action("Close Unmodified Tabs", CloseUnmodifiedTabs).mnemonic(6));
        }
        let pinned = self.tabs.iter().filter(|t| t.pinned).count();
        if pinned > 0 && pinned < count {
            items.push(MenuItem::action("Close All but Pinned", CloseUnpinnedTabs));
        }
        if !self.beside(index, true).is_empty() {
            items.push(MenuItem::action(
                "Close Tabs to the Left",
                CloseTabsToLeft { index: some },
            ));
        }
        if !self.beside(index, false).is_empty() {
            items.push(MenuItem::action(
                "Close Tabs to the Right",
                CloseTabsToRight { index: some },
            ));
        }
        let file = self.tabs[index].path().cloned();
        // Copy Path/Reference… for a file of the project.
        let in_project = file.as_ref().filter(|f| {
            self.project_root(cx)
                .is_some_and(|root| f.starts_with(&root) && **f != root)
        });
        if let Some(file) = in_project {
            let paths = Some(vec![file.display().to_string()]);
            let copy = |label: &'static str, kind: &str| {
                MenuItem::action(
                    label,
                    crate::project_panel::CopyPath {
                        kind: kind.to_owned(),
                        paths: paths.clone(),
                    },
                )
            };
            items.push(MenuItem::separator());
            items.push(MenuItem::submenu(
                "Copy Path/Reference\u{2026}",
                vec![
                    copy("Absolute Path", "absolute"),
                    copy("File Name", "file-name"),
                    copy("Path from Content Root", "content-root"),
                ],
            ));
        }
        items.push(MenuItem::separator());
        let (pin, mnemonic) = if self.tabs[index].pinned {
            ("Unpin Tab", 3)
        } else {
            ("Pin Tab", 1)
        };
        items.push(
            MenuItem::action(pin, TogglePinTab { index: some })
                .shortcut(keys(&TogglePinTab { index: None }))
                .mnemonic(mnemonic),
        );
        if let Some(file) = &file {
            items.extend(self.tab_run_items(file, &keys, cx));
            items.push(MenuItem::separator());
            let path = Some(file.display().to_string());
            items.push(MenuItem::submenu(
                "Open In",
                vec![
                    MenuItem::action(FILE_MANAGER, RevealFile { path: path.clone() }),
                    MenuItem::action("Terminal", OpenInTerminal { path }),
                ],
            ));
            let active = self.active == Some(index);
            items.extend(self.tab_git_items(file, active, &keys, cx));
        }
        if let Some(file) = in_project {
            items.push(MenuItem::separator());
            items.push(MenuItem::action(
                "Rename File\u{2026}",
                crate::project_panel::Rename {
                    path: Some(file.display().to_string()),
                    name: None,
                },
            ));
        }
        without_extra_separators(items)
    }

    /// Run and Debug the file with its context configuration
    /// (`RunContextPopupGroup`: `Run 'main'`, `Debug 'main'`).
    fn tab_run_items(
        &self,
        file: &Path,
        keys: &dyn Fn(&dyn gpui::Action) -> String,
        cx: &App,
    ) -> Vec<MenuItem> {
        let path = file.display().to_string();
        let Some((_, true)) = self.run_file_of(Some(&path), cx) else {
            return Vec::new();
        };
        let name = self
            .run_configs(cx)
            .read(cx)
            .context_name(file)
            .or_else(|| file.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let mut items = vec![
            MenuItem::separator(),
            MenuItem::action(
                format!("Run '{name}'"),
                crate::run::commands::RunCurrentFile {
                    file: Some(path.clone()),
                },
            )
            .icon(IconName::RUN)
            .shortcut(keys(&crate::run::commands::RunCurrentFile { file: None }))
            .mnemonic(1),
        ];
        if self.product.debug.is_some() {
            use debugger_ui::session::commands::DebugCurrentFile;
            items.push(
                MenuItem::action(
                    format!("Debug '{name}'"),
                    DebugCurrentFile {
                        file: Some(path.clone()),
                    },
                )
                .icon(IconName::DEBUG)
                .shortcut(keys(&DebugCurrentFile { file: None }))
                .mnemonic(0),
            );
        }
        items
    }

    /// Git (`Git.ContextMenu`): the file's actions Pyoxide has (Add for an
    /// unversioned file, Show Diff and, for the active tab, Rollback… for a
    /// changed one), then
    /// the repository's (Push…, Fetch, Branches…, New Branch…).
    fn tab_git_items(
        &self,
        file: &Path,
        active: bool,
        keys: &dyn Fn(&dyn gpui::Action) -> String,
        cx: &App,
    ) -> Vec<MenuItem> {
        use vcs_ui::commit::{AddToVcs, Rollback};
        if self.version_control.service.is_none() {
            return Vec::new();
        }
        let status = self.vcs_file_status(file, cx);
        let paths = Some(vec![file.display().to_string()]);
        let mut git = Vec::new();
        if status == FileStatus::Unversioned {
            git.push(
                MenuItem::action(
                    "Add",
                    AddToVcs {
                        paths: paths.clone(),
                    },
                )
                .icon(IconName::ADD)
                .shortcut(keys(&AddToVcs { paths: None })),
            );
        }
        let changed = !matches!(
            status,
            FileStatus::NotChanged | FileStatus::Unversioned | FileStatus::Ignored
        );
        if changed {
            let path = Some(file.display().to_string());
            git.push(MenuItem::separator());
            git.push(
                MenuItem::action("Show Diff", vcs_ui::diff::ShowDiff { path })
                    .shortcut(keys(&vcs_ui::diff::ShowDiff { path: None })),
            );
            // Without paths the command asks in the Rollback Changes
            // dialog for the active file; with paths it would not ask.
            if active {
                let rollback = Rollback {
                    paths: None,
                    delete_added: None,
                };
                git.push(
                    MenuItem::action("Rollback\u{2026}", rollback.clone())
                        .shortcut(keys(&rollback)),
                );
            }
        }
        let item = |label: &'static str, action: Box<dyn gpui::Action>| {
            let shortcut = keys(action.as_ref());
            MenuItem::Action {
                label: label.into(),
                icon: None,
                action: Some(action),
                enabled: true,
                description: None,
                extra: Default::default(),
            }
            .shortcut(shortcut)
        };
        git.extend([
            MenuItem::separator(),
            item(
                "Push\u{2026}",
                Box::new(vcs_ui::VcsPush {
                    branch: None,
                    target: None,
                    force: None,
                    tags: None,
                    set_upstream: None,
                    dialog: None,
                }),
            ),
            item("Fetch", Box::new(vcs_ui::Fetch { remote: None })),
            MenuItem::separator(),
            item("Branches\u{2026}", Box::new(vcs_ui::ShowBranches)),
            item(
                "New Branch\u{2026}",
                Box::new(vcs_ui::NewBranch {
                    name: None,
                    from: None,
                    checkout: None,
                }),
            ),
        ]);
        vec![
            MenuItem::separator(),
            MenuItem::submenu("Git", without_extra_separators(git)).mnemonic(0),
        ]
    }

    /// Opens a new Terminal tab in `dir` and shows the tool window.
    fn open_terminal_in(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.new_terminal_tab_in(None, Some(dir), window, cx);
        self.activate_tool_window(super::terminal_window::TERMINAL, window, cx);
    }
}

/// Drops leading, trailing and doubled separators (the reference's popup
/// does so for groups that turn out empty).
pub(super) fn without_extra_separators(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        let separator = matches!(item, MenuItem::Separator);
        if separator && out.last().is_none_or(|l| matches!(l, MenuItem::Separator)) {
            continue;
        }
        out.push(item);
    }
    while out.last().is_some_and(|l| matches!(l, MenuItem::Separator)) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_tabs_trade_places_with_the_first_unpinned_one() {
        // [A, B, C], C pinned: C and A trade places.
        assert_eq!(group_pinned_first(&[false, false, true]), [2, 1, 0]);
        // Already grouped: nothing moves.
        assert_eq!(group_pinned_first(&[true, false, false]), [0, 1, 2]);
        // [P, A, B, Q]: Q trades places with A.
        assert_eq!(
            group_pinned_first(&[true, false, false, true]),
            [0, 3, 2, 1]
        );
        // [A, P, Q]: P with A, then Q with A again.
        assert_eq!(group_pinned_first(&[false, true, true]), [1, 2, 0]);
    }

    #[test]
    fn separators_are_dropped_at_the_ends_and_when_doubled() {
        let items = without_extra_separators(vec![
            MenuItem::separator(),
            MenuItem::action("a", CloseAllTabs),
            MenuItem::separator(),
            MenuItem::separator(),
            MenuItem::action("b", CloseAllTabs),
            MenuItem::separator(),
        ]);
        let labels: Vec<_> = items
            .iter()
            .map(|i| i.label().map(|l| l.to_string()).unwrap_or("-".into()))
            .collect();
        assert_eq!(labels, ["a", "-", "b"]);
    }
}
