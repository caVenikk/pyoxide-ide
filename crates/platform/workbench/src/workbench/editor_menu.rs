//! The context menu of the text editor: a right click on the text (or
//! `editor.context-menu`) opens it at the pointer. Its rows are the
//! reference's editor popup (`EditorPopupMenu`) in its order, the ones
//! Pyoxide has: Show Context Actions | Cut, Copy, Paste | Find Usages, Go To
//! | Duplicate, Reformat Code | the rows of the file's type | Run and Debug
//! the file | Open In | Git. Rows whose action cannot run are left out, as
//! the reference's popup hides disabled actions (an editor without a
//! selection has no Cut and Copy).

use std::path::Path;

use editor::{Editor, EditorEvent};
use gpui::{AnyElement, App, Context, DismissEvent, Entity, Pixels, Point, Subscription, Window};
use serde_json::{Value as Json, json};
use ui::menu::{ContextMenu, MenuItem};

use super::Workbench;
use super::tab_menu::{FILE_MANAGER, without_extra_separators};
use crate::commands::{OpenInTerminal, RevealFile};

/// The open menu.
pub(super) struct EditorMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

impl Workbench {
    /// A right click in `editor` (or the command): opens its menu at
    /// `position` (window coordinates) when it is a file's editor.
    pub(super) fn open_editor_menu(
        &mut self,
        editor: &Entity<Editor>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = editor.read(cx).path().cloned() else {
            return;
        };
        let selected = editor.read(cx).selections().iter().any(|s| !s.is_empty());
        let items = self.editor_menu_items(&path, selected, window, cx);
        if items.is_empty() {
            return;
        }
        let menu = ContextMenu::build(items, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.editor_menu = None;
            cx.notify();
        });
        self.editor_menu = Some(EditorMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Whether `event` of a file's editor opens its menu.
    pub(super) fn editor_menu_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let EditorEvent::ContextMenu { position } = event {
            self.open_editor_menu(editor, *position, window, cx);
        }
    }

    pub(super) fn render_editor_menu(&self, window: &Window) -> Option<AnyElement> {
        let open = self.editor_menu.as_ref()?;
        Some(ui::menu::overlay(&open.menu, open.position, window))
    }

    /// The open menu for `workbench.state`.
    pub(super) fn editor_menu_json(&self, cx: &App) -> Json {
        match &self.editor_menu {
            Some(open) => json!({ "menu": open.menu.read(cx).state_json() }),
            None => Json::Null,
        }
    }

    /// The rows of the menu of the editor of `path`.
    fn editor_menu_items(
        &self,
        path: &Path,
        selected: bool,
        window: &Window,
        cx: &App,
    ) -> Vec<MenuItem> {
        use code_intel_ui::commands::{GoToDeclaration, Reformat, ShowIntentions, ShowUsages};
        use editor::commands::{Copy, Cut, DuplicateLine, Paste};

        let language = self
            .file_types
            .for_path(path)
            .is_some_and(|t| t.language_id.is_some());
        let mut items = Vec::new();
        if language {
            items.push(MenuItem::action("Show Context Actions", ShowIntentions));
            items.push(MenuItem::separator());
        }
        if selected {
            items.push(MenuItem::action("Cut", Cut).mnemonic(2));
            items.push(MenuItem::action("Copy", Copy).mnemonic(0));
        }
        items.push(MenuItem::action("Paste", Paste).mnemonic(0));
        if language {
            items.push(MenuItem::separator());
            items.push(MenuItem::action("Find Usages", ShowUsages));
            items.push(MenuItem::submenu(
                "Go To",
                vec![MenuItem::action("Declaration or Usages", GoToDeclaration)],
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Duplicate", DuplicateLine).mnemonic(0));
        if language {
            items.push(MenuItem::action("Reformat Code", Reformat));
        }
        let own = self.file_types.context_menu(path, cx);
        if !own.is_empty() {
            items.push(MenuItem::separator());
            items.extend(own);
        }
        let focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        let keys = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &focus, window).unwrap_or_default()
        };
        items.extend(self.tab_run_items(path, &keys, cx));
        items.push(MenuItem::separator());
        let file = Some(path.display().to_string());
        items.push(MenuItem::submenu(
            "Open In",
            vec![
                MenuItem::action(FILE_MANAGER, RevealFile { path: file.clone() }),
                MenuItem::action("Terminal", OpenInTerminal { path: file }),
            ],
        ));
        items.extend(self.tab_git_items(path, true, &keys, cx));
        without_extra_separators(items)
    }
}
