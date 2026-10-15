//! The kinds of files and folders a product adds to *New* (the reference's
//! actions in `NewGroup`, such as Python's *Python File* and *Python
//! Package*): a row after *Directory*, a popup with a name field and the
//! list of the item's kinds (`CreateFileFromTemplateDialog`), and what a
//! kind creates.

use std::path::PathBuf;

use gpui::{AppContext as _, Context, Window};
use project::fs_ops::{self, NewKind};
use ui::icons::IconName;
use ui::menu::MenuItem;

use super::file_ops::OpError;
use super::name_popup::{Message, NamePopup};
use super::{PanelEvent, ProjectPanel};

/// A row a product adds to *New*.
#[derive(Debug, Clone)]
pub struct NewItem {
    /// The row ("Python File").
    pub label: &'static str,
    pub icon: IconName,
    /// The popup's title ("New Python File").
    pub title: &'static str,
    /// The kinds listed under the name field; the first one is selected.
    pub kinds: Vec<NewItemKind>,
}

/// One kind of a [`NewItem`] ("Python unit test").
#[derive(Debug, Clone)]
pub struct NewItemKind {
    pub label: &'static str,
    pub icon: IconName,
    pub template: NewTemplate,
}

/// What a kind creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewTemplate {
    /// A file with `text`; `extension` is added to a name without one.
    File {
        extension: &'static str,
        text: &'static str,
    },
    /// Folders named by a dotted path (`a.b`), each with a file `marker`
    /// holding `text` (a package and its `__init__.py`).
    Package {
        marker: &'static str,
        text: &'static str,
    },
}

ui::ui_commands! {
    COMMANDS, scope = "ProjectTree";
    NewFromTemplate {
        item: String,
        kind: Option<String>,
        name: Option<String>,
        directory: Option<String>
    } => "project.tree.new-item", "Create New Item",
        "Creates one of the product's kinds of New (`item` is the row, for example `Python File`) in the selected folder (or the folder of the selected file, or `directory`): asks in its popup, or takes `name` and `kind` (a row of the popup's list, the first by default).",
        r#"{"item": "Python File", "kind": "Python unit test", "name": "test_app"}"#;
}

impl ProjectPanel {
    /// The product's rows of *New*, after *Directory*.
    pub fn set_new_items(&mut self, items: Vec<NewItem>) {
        self.ops.new_items = items;
    }

    /// The product's rows of *New* for the folder `dir` (relative).
    pub(super) fn product_new_items(&self, dir: Option<String>) -> Vec<MenuItem> {
        self.ops
            .new_items
            .iter()
            .map(|item| {
                MenuItem::action(
                    item.label,
                    NewFromTemplate {
                        item: item.label.to_owned(),
                        kind: None,
                        name: None,
                        directory: dir.clone(),
                    },
                )
                .icon(item.icon)
            })
            .collect()
    }

    pub(super) fn new_from_template(
        &mut self,
        a: &NewFromTemplate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.ops.new_items.iter().position(|i| i.label == a.item) else {
            return;
        };
        if self.filter.is_none() {
            return;
        }
        let Some(dir) = self.target_dir(a.directory.as_deref()) else {
            return;
        };
        let item = self.ops.new_items[index].clone();
        let kind = match &a.kind {
            Some(k) => match item.kinds.iter().position(|kind| kind.label == k) {
                Some(kind) => kind,
                None => return,
            },
            None => 0,
        };
        if let Some(name) = &a.name {
            self.create_from_template(dir, index, kind, name.clone(), window, cx);
            return;
        }
        let kinds = item
            .kinds
            .iter()
            .map(|k| (k.label.into(), k.icon))
            .collect();
        let popup = cx.new(|cx| NamePopup::new_item_with_kinds(item.title, kinds, window, cx));
        self.open_popup(
            popup,
            super::file_ops::Purpose::Template { dir, item: index },
            window,
            cx,
        );
    }

    /// The popup of a package checks the path as it is typed.
    pub(super) fn check_new_package(
        &mut self,
        dir: PathBuf,
        text: String,
        popup: gpui::Entity<NamePopup>,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(filter)) = (self.root(), self.filter.clone()) else {
            return;
        };
        self.ops.check += 1;
        let check = self.ops.check;
        let task = cx.background_spawn(async move {
            if text.is_empty() {
                return None;
            }
            match fs_ops::check_new_package(&dir, &text, &root, &filter) {
                Err(e) => Some(Message {
                    text: e.to_string().into(),
                    warning: false,
                }),
                Ok(_) => fs_ops::new_package_warning(&text, &filter).map(|w| Message {
                    text: w.to_string().into(),
                    warning: true,
                }),
            }
        });
        cx.spawn(async move |this, cx| {
            let message = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.ops.check == check {
                    popup.update(cx, |p, cx| p.set_message(message, cx));
                }
            });
        })
        .detach();
    }

    /// The template of the item's kind is a package.
    pub(super) fn is_package(&self, item: usize) -> bool {
        self.ops.new_items.get(item).is_some_and(|i| {
            i.kinds
                .iter()
                .any(|k| matches!(k.template, NewTemplate::Package { .. }))
        })
    }

    /// Creates a kind of a product item in the background: a file opens in
    /// the editor, a package is selected in the tree.
    pub(super) fn create_from_template(
        &mut self,
        dir: PathBuf,
        item: usize,
        kind: usize,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(filter)) = (self.root(), self.filter.clone()) else {
            return;
        };
        let Some(template) = self
            .ops
            .new_items
            .get(item)
            .and_then(|i| i.kinds.get(kind))
            .map(|k| k.template)
        else {
            return;
        };
        let generation = self.generation;
        self.loading += 1;
        let task = cx.background_spawn(async move {
            let name_error = |e: fs_ops::NameError| OpError::Name(e.to_string());
            match template {
                NewTemplate::File { extension, text } => {
                    let name = fs_ops::with_extension(&name, extension);
                    let target = fs_ops::check_new(&dir, &name, NewKind::File, &root, &filter)
                        .map_err(name_error)?;
                    let io = |e: std::io::Error| OpError::Io {
                        title: "Cannot Create File",
                        message: format!("Cannot create file '{}': {e}", target.path.display()),
                    };
                    let changes = fs_ops::create(&target).map_err(io)?;
                    fs_ops::write_template(&target.path, text).map_err(io)?;
                    Ok((target.path, false, changes))
                }
                NewTemplate::Package { marker, text } => {
                    let target = fs_ops::check_new_package(&dir, &name, &root, &filter)
                        .map_err(name_error)?;
                    let changes =
                        fs_ops::create_package(&target, &dir, marker, text).map_err(|e| {
                            OpError::Io {
                                title: "Create Package Error",
                                message: e.to_string(),
                            }
                        })?;
                    Ok((target.path, true, changes))
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((path, is_dir, changes)) => {
                        this.close_popup(window, cx);
                        this.apply_changes(&changes, cx);
                        this.reveal(&path, cx);
                        if is_dir {
                            window.focus(&this.focus);
                        } else {
                            cx.emit(PanelEvent::OpenFile(path));
                        }
                    }
                    Err(error) => this.operation_failed(error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
