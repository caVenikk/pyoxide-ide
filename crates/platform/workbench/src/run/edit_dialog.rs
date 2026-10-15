//! The Run/Debug Configurations dialog (`EditConfigurationsDialog`,
//! `RunConfigurable`): the configurations as a tree (type, folder,
//! configuration) in the list order, a toolbar (Add New Configuration,
//! Remove Configuration, Copy Configuration, Save Configuration, Create
//! New Folder, Sort Configurations), the selected configuration's name,
//! "Store as project file", its type's form with "Modify options", and
//! Cancel, Apply and OK. It edits a copy of the configurations; Apply and
//! OK hand the copy back. A configuration of a type Pyoxide does not
//! support is shown read-only.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use editor::Editor;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, IntoElement, Pixels, Point, Render, SharedString,
    Subscription, Window, canvas, div, prelude::*, px,
};
use run_config::form::{self, Field, FieldKind};
use run_config::{ConfigKey, RunConfiguration, RunConfigurations, Storage, TypeEditor, Types};
use serde_json::{Value, json};
use themes::{Metric, UiRole};
use ui::components::{checkbox, input_frame, popup_shadow};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::configurations::RunConfigs;
use super::widget::TEMPORARY_ICON_ALPHA;

ui::ui_commands! {
    DIALOG_COMMANDS, scope = "RunConfigurationsDialog";
    /// Select a node of the tree.
    DialogSelect { name: Option<String>, r#type: Option<String>, folder: Option<String> }
        => "run.configurations.dialog.select", "Select",
        "Selects a configuration by name (and type id), a folder (type and folder), or a type in the Run/Debug Configurations dialog.",
        r#"{"name": "server", "type": "python"}"#;
    /// Select the next node.
    DialogNext => "run.configurations.dialog.next", "Next",
        "Selects the next node of the dialog's tree.";
    /// Select the previous node.
    DialogPrevious => "run.configurations.dialog.previous", "Previous",
        "Selects the previous node of the dialog's tree.";
    /// Add a configuration.
    DialogAdd { r#type: Option<String> } => "run.configurations.dialog.add", "Add New Configuration",
        "Adds a configuration of a type (type id) to the dialog, in the selected folder; without a type, opens the list of types.",
        r#"{"type": "python"}"#;
    /// Remove the selected configuration or folder.
    DialogRemove => "run.configurations.dialog.remove", "Remove Configuration",
        "Removes the selected configuration; a selected folder goes and its configurations stay.";
    /// Copy the selected configuration.
    DialogCopy => "run.configurations.dialog.copy", "Copy Configuration",
        "Copies the selected configuration under a new name.";
    /// Make the selected temporary configuration permanent.
    DialogSaveTemporary => "run.configurations.dialog.save-temporary", "Save Configuration",
        "Makes the selected temporary configuration permanent.";
    /// Create a folder.
    DialogNewFolder => "run.configurations.dialog.new-folder", "Create New Folder",
        "Creates a folder in the selected type and moves the selected configuration into it.";
    /// Sort alphabetically.
    DialogSort => "run.configurations.dialog.sort", "Sort Configurations",
        "Sorts the configurations alphabetically (drops the custom order).";
    /// Set a field.
    DialogSet { field: String, value: String } => "run.configurations.dialog.set", "Set Field",
        "Sets a field of the selected configuration (name, store-as-project-file, env, shared-env, a field of its type) or folder (folder-name).",
        r#"{"field": "script", "value": "app/main.py"}"#;
    /// Turn an option of "Modify options" on or off.
    DialogToggleOption { option: String } => "run.configurations.dialog.toggle-option", "Toggle Option",
        "Turns a row of the selected configuration's Modify options on or off.",
        r#"{"option": "interpreter-options"}"#;
    /// Open "Modify options".
    DialogModifyOptions => "run.configurations.dialog.modify-options", "Modify Options",
        "Opens the selected configuration's Modify options menu.";
    /// Apply the changes.
    DialogApply => "run.configurations.dialog.apply", "Apply",
        "Saves the dialog's changes and keeps it open.";
    /// Apply and close.
    DialogOk => "run.configurations.dialog.ok", "OK",
        "Saves the dialog's changes and closes it.";
    /// Close without saving.
    DialogCancel => "run.configurations.dialog.cancel", "Cancel",
        "Closes the dialog without saving its changes.";
}

/// Key contexts of the dialog and its tree.
pub const KEY_CONTEXT: &str = "RunConfigurationsDialog";
pub const TREE_CONTEXT: &str = "RunConfigurationsTree";

/// Ids of the fields every configuration has.
const NAME: &str = "name";
const STORE: &str = "store-as-project-file";
const SHARED_ENV: &str = "shared-env";
const FOLDER_NAME: &str = "folder-name";

/// Size of the dialog and of its parts.
const WIDTH: f32 = 880.0;
const HEIGHT: f32 = 580.0;
const TREE_WIDTH: f32 = 260.0;
const LABEL_WIDTH: f32 = 170.0;
const ROW: f32 = 24.0;
const INDENT: f32 = 18.0;

/// A node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Type(String),
    Folder(String, String),
    Config(ConfigKey),
}

/// What the dialog asks of the workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditDialogEvent {
    /// The dialog closed.
    Closed,
    /// "Store as project file" was turned on for a configuration with
    /// personal values: these variables keep their values personal unless
    /// the person chooses otherwise (`store_as_project_file`).
    AskStoreValues(Vec<String>),
}

/// A field of the form and its text field.
struct FieldView {
    field: Field,
    editor: Option<Entity<Editor>>,
}

/// An open popup of the dialog (types, a choice, Modify options).
struct OpenMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

/// The Run/Debug Configurations dialog.
pub struct EditDialog {
    focus: FocusHandle,
    tree_focus: FocusHandle,
    configs: Entity<RunConfigs>,
    list: RunConfigurations,
    types: Types,
    editors: Vec<Arc<dyn TypeEditor>>,
    selected: Option<Node>,
    /// Option fields turned on in this session, per configuration.
    shown: HashMap<ConfigKey, BTreeSet<String>>,
    /// Folders made in this session that hold no configuration (not
    /// saved: "empty folders won't be saved").
    empty_folders: Vec<(String, String)>,
    fields: Vec<FieldView>,
    error: Option<String>,
    modified: bool,
    menu: Option<OpenMenu>,
    /// Where the Add and Modify options popups open.
    add_anchor: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    options_anchor: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    choice_anchor: std::rc::Rc<std::cell::RefCell<HashMap<String, Bounds<Pixels>>>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditDialogEvent> for EditDialog {}

impl Focusable for EditDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Records an element's bounds when it is laid out.
fn anchor(cell: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

impl EditDialog {
    /// The dialog on the project's configurations, `select` selected (else
    /// the selected configuration, else the first).
    pub fn new(
        configs: Entity<RunConfigs>,
        select: Option<ConfigKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (list, types, editors) = {
            let c = configs.read(cx);
            (c.list().clone(), c.types().clone(), c.editors().to_vec())
        };
        let mut dialog = Self {
            focus: cx.focus_handle(),
            tree_focus: cx.focus_handle(),
            configs,
            list,
            types,
            editors,
            selected: None,
            shown: HashMap::new(),
            empty_folders: Vec::new(),
            fields: Vec::new(),
            error: None,
            modified: false,
            menu: None,
            add_anchor: Default::default(),
            options_anchor: Default::default(),
            choice_anchor: Default::default(),
            _subscriptions: Vec::new(),
        };
        let first = select
            .or_else(|| dialog.list.selected.clone())
            .filter(|k| dialog.list.contains(k))
            .map(Node::Config)
            .or_else(|| {
                dialog
                    .nodes()
                    .into_iter()
                    .find_map(|(n, _)| matches!(n, Node::Config(_)).then_some(n))
            });
        dialog.selected = first;
        dialog.rebuild(window, cx);
        window.focus(&dialog.tree_focus);
        dialog
    }

    /// Where the focus is in the tree.
    pub fn tree_focus(&self) -> FocusHandle {
        self.tree_focus.clone()
    }

    /// The text field of a field of the form.
    pub fn field_focus(&self, id: &str, cx: &App) -> Option<FocusHandle> {
        self.fields
            .iter()
            .find(|f| f.field.id == id)
            .and_then(|f| f.editor.as_ref())
            .map(|e| e.focus_handle(cx))
    }

    fn editor_of(&self, type_id: &str) -> Option<Arc<dyn TypeEditor>> {
        self.editors
            .iter()
            .find(|e| e.type_id() == type_id)
            .cloned()
    }

    // ---- the tree ------------------------------------------------------------

    /// The nodes in order with their depth: types in list order, their
    /// folders (in the order they first appear) with their configurations,
    /// then the configurations without a folder.
    fn nodes(&self) -> Vec<(Node, usize)> {
        let sorted = self.list.sorted(&self.types);
        let mut out = Vec::new();
        let mut i = 0;
        let mut seen_types: Vec<String> = Vec::new();
        while i < sorted.len() {
            let type_id = sorted[i].type_id.clone();
            let mut end = i;
            while end < sorted.len() && sorted[end].type_id == type_id {
                end += 1;
            }
            let group = &sorted[i..end];
            out.push((Node::Type(type_id.clone()), 0));
            seen_types.push(type_id.clone());
            let mut folders: Vec<String> = Vec::new();
            for c in group {
                if let Some(f) = &c.folder
                    && !folders.contains(f)
                {
                    folders.push(f.clone());
                }
            }
            for (t, f) in &self.empty_folders {
                if *t == type_id && !folders.contains(f) {
                    folders.push(f.clone());
                }
            }
            for f in folders {
                out.push((Node::Folder(type_id.clone(), f.clone()), 1));
                for c in group.iter().filter(|c| c.folder.as_ref() == Some(&f)) {
                    out.push((Node::Config(c.key()), 2));
                }
            }
            for c in group.iter().filter(|c| c.folder.is_none()) {
                out.push((Node::Config(c.key()), 1));
            }
            i = end;
        }
        // Empty folders of a type without configurations.
        for (t, f) in &self.empty_folders {
            if !seen_types.contains(t) {
                out.push((Node::Type(t.clone()), 0));
                seen_types.push(t.clone());
                out.push((Node::Folder(t.clone(), f.clone()), 1));
            }
        }
        out
    }

    fn type_name(&self, type_id: &str) -> String {
        if let Some(info) = self.types.get(type_id) {
            return info.name.to_owned();
        }
        self.list
            .all()
            .iter()
            .find(|c| c.type_id == type_id)
            .map(|c| self.types.name_of(c).to_owned())
            .unwrap_or_else(|| type_id.to_owned())
    }

    fn selected_config(&self) -> Option<&RunConfiguration> {
        match &self.selected {
            Some(Node::Config(key)) => self.list.get(key),
            _ => None,
        }
    }

    /// Selects a node: the form of the one before is kept first.
    pub fn select_node(&mut self, node: Option<Node>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        self.selected = node;
        self.rebuild(window, cx);
        cx.notify();
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.nodes();
        if nodes.is_empty() {
            return;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|s| nodes.iter().position(|(n, _)| n == s));
        let next = match at {
            Some(i) => (i as isize + delta).clamp(0, nodes.len() as isize - 1) as usize,
            None => 0,
        };
        self.select_node(Some(nodes[next].0.clone()), window, cx);
    }

    // ---- the form --------------------------------------------------------------

    fn text_field(
        text: &str,
        placeholder: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let placeholder = placeholder.map(|p| SharedString::from(p.to_owned()));
        cx.new(|cx| {
            let mut e = Editor::new(text::Buffer::new(text), window, cx);
            e.set_single_line(true);
            e.set_ui_font(true);
            e.set_transparent(true);
            e.set_placeholder(placeholder);
            e.set_config(editor::EditorConfig {
                brackets: Vec::new(),
                ..editor::EditorConfig::default()
            });
            e
        })
    }

    /// The fields of the selected node, with text fields for the ones
    /// typed in.
    fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fields.clear();
        self.error = None;
        let mut fields: Vec<Field> = Vec::new();
        match self.selected.clone() {
            Some(Node::Config(key)) => {
                let Some(config) = self.list.get(&key).cloned() else {
                    return;
                };
                fields.push(Field::new(
                    NAME,
                    "Name:",
                    FieldKind::Text,
                    config.name.clone(),
                ));
                if !config.is_unsupported()
                    && let Some(editor) = self.editor_of(&config.type_id)
                {
                    let shown = self.shown.get(&key).cloned().unwrap_or_default();
                    fields.extend(editor.fields(&config, &shown));
                    if config.is_shared() {
                        let names: Vec<&str> = config
                            .env
                            .iter()
                            .filter(|v| v.shared)
                            .map(|v| v.name.as_str())
                            .collect();
                        fields.push(Field::new(
                            SHARED_ENV,
                            "Variables with shared values:",
                            FieldKind::Text,
                            names.join(";"),
                        ));
                    }
                    self.error = editor.validate(&config);
                }
            }
            Some(Node::Folder(_, name)) => {
                fields.push(Field::new(
                    FOLDER_NAME,
                    "Folder name:",
                    FieldKind::Text,
                    name,
                ));
            }
            _ => {}
        }
        for field in fields {
            let editor = match field.kind {
                FieldKind::Text | FieldKind::Path | FieldKind::Env => Some(Self::text_field(
                    &field.value,
                    field.placeholder.as_deref(),
                    window,
                    cx,
                )),
                FieldKind::Choice(_) | FieldKind::Flag => None,
            };
            self.fields.push(FieldView { field, editor });
        }
    }

    /// Keeps what was typed into the fields in the configurations.
    fn commit(&mut self, cx: &mut Context<Self>) {
        let typed: Vec<(String, String)> = self
            .fields
            .iter()
            .filter_map(|f| {
                let text = f.editor.as_ref()?.read(cx).text();
                let text = text.trim_end_matches(['\r', '\n']).to_owned();
                (text != f.field.value).then(|| (f.field.id.clone(), text))
            })
            .collect();
        if typed.is_empty() {
            return;
        }
        for f in self.fields.iter_mut() {
            if let Some((_, text)) = typed.iter().find(|(id, _)| *id == f.field.id) {
                f.field.value = text.clone();
            }
        }
        for (id, value) in typed {
            self.set_value(&id, &value, cx);
        }
    }

    /// Sets a field of the selected node without rebuilding the form.
    fn set_value(&mut self, id: &str, value: &str, cx: &mut Context<Self>) -> bool {
        self.modified = true;
        match self.selected.clone() {
            Some(Node::Config(key)) => {
                if id == NAME {
                    return match self.list.rename(&key, value) {
                        Ok(new) => {
                            if let Some(shown) = self.shown.remove(&key) {
                                self.shown.insert(new.clone(), shown);
                            }
                            self.selected = Some(Node::Config(new));
                            self.error = None;
                            true
                        }
                        Err(e) => {
                            self.error = Some(e);
                            false
                        }
                    };
                }
                let Some(mut config) = self.list.get(&key).cloned() else {
                    return false;
                };
                let done = match id {
                    form::common::ENV => {
                        config.env = form::parse_env(value, &config.env);
                        true
                    }
                    SHARED_ENV => {
                        let names: Vec<&str> = value.split(';').map(str::trim).collect();
                        for v in config.env.iter_mut() {
                            v.shared = names.contains(&v.name.as_str());
                        }
                        true
                    }
                    _ => match self.editor_of(&config.type_id) {
                        Some(editor) => editor.set(&mut config, id, value),
                        None => false,
                    },
                };
                if done {
                    if let Some(editor) = self.editor_of(&config.type_id) {
                        self.error = editor.validate(&config);
                    }
                    self.list.replace(&key, config);
                }
                done
            }
            Some(Node::Folder(type_id, name)) if id == FOLDER_NAME => {
                let new = value.trim().to_owned();
                if new.is_empty() {
                    self.error = Some("Folder name shouldn't be empty".to_owned());
                    return false;
                }
                let keys: Vec<ConfigKey> = self
                    .list
                    .all()
                    .iter()
                    .filter(|c| c.type_id == type_id && c.folder.as_ref() == Some(&name))
                    .map(RunConfiguration::key)
                    .collect();
                for key in keys {
                    if let Some(c) = self.list.get_mut(&key) {
                        c.folder = Some(new.clone());
                    }
                }
                for (t, f) in self.empty_folders.iter_mut() {
                    if *t == type_id && *f == name {
                        *f = new.clone();
                    }
                }
                self.selected = Some(Node::Folder(type_id, new));
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// "Store as project file" on or off. Turning it on for a
    /// configuration with personal values asks first.
    fn set_store(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(config) = self.selected_config() else {
            return;
        };
        if on == config.is_shared() {
            return;
        }
        if on {
            let personal: Vec<String> = config
                .env
                .iter()
                .filter(|v| !v.shared && v.value.is_some())
                .map(|v| v.name.clone())
                .collect();
            if !personal.is_empty() {
                cx.emit(EditDialogEvent::AskStoreValues(personal));
                return;
            }
        }
        self.move_storage(on, false, cx);
    }

    /// The answer to [`EditDialogEvent::AskStoreValues`]: the
    /// configuration becomes shared; `share_values` stores the personal
    /// values in the project file too.
    pub fn store_as_project_file(
        &mut self,
        share_values: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_storage(true, share_values, cx);
        self.rebuild(window, cx);
        cx.notify();
    }

    fn move_storage(&mut self, shared: bool, share_values: bool, cx: &mut Context<Self>) {
        let Some(Node::Config(key)) = self.selected.clone() else {
            return;
        };
        let Some(config) = self.list.get_mut(&key) else {
            return;
        };
        config.storage = if shared {
            Storage::Shared
        } else {
            Storage::Personal
        };
        if shared {
            // A temporary configuration that is shared becomes permanent
            // (`RunManagerImpl.shareConfiguration`).
            config.temporary = false;
            if share_values {
                for v in config.env.iter_mut() {
                    v.shared = true;
                }
            }
        }
        self.list.recent_temporary.retain(|k| *k != key);
        self.modified = true;
        cx.notify();
    }

    // ---- actions -----------------------------------------------------------------

    fn find_key(&self, name: &str, type_id: Option<&str>) -> Option<ConfigKey> {
        let found: Vec<&RunConfiguration> = self
            .list
            .find_by_name(name)
            .filter(|c| type_id.is_none_or(|t| t == c.type_id))
            .collect();
        match found.as_slice() {
            [one] => Some(one.key()),
            _ => None,
        }
    }

    fn on_select(&mut self, action: &DialogSelect, window: &mut Window, cx: &mut Context<Self>) {
        let node = match (&action.name, &action.r#type, &action.folder) {
            (Some(name), ty, _) => self.find_key(name, ty.as_deref()).map(Node::Config),
            (None, Some(ty), Some(folder)) => Some(Node::Folder(ty.clone(), folder.clone())),
            (None, Some(ty), None) => Some(Node::Type(ty.clone())),
            _ => None,
        };
        match node {
            Some(node) => self.select_node(Some(node), window, cx),
            None => {
                self.error = Some("No such run configuration".to_owned());
                cx.notify();
            }
        }
    }

    fn on_next(&mut self, _: &DialogNext, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn on_previous(&mut self, _: &DialogPrevious, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn on_add(&mut self, action: &DialogAdd, window: &mut Window, cx: &mut Context<Self>) {
        let Some(type_id) = action.r#type.clone() else {
            // The list of types ("Add New Configuration").
            let types = self.configs.read(cx).creatable_types();
            let items: Vec<MenuItem> = types
                .iter()
                .map(|t| {
                    MenuItem::action(
                        t.name,
                        DialogAdd {
                            r#type: Some(t.id.to_owned()),
                        },
                    )
                    .icon(IconName(t.icon))
                })
                .collect();
            let at = self.add_anchor.get().bottom_left();
            self.open_menu(items, Some("Add New Configuration".into()), at, window, cx);
            return;
        };
        self.commit(cx);
        let Some(editor) = self.editor_of(&type_id) else {
            self.error = Some(format!("Cannot add a configuration of type {type_id}"));
            cx.notify();
            return;
        };
        let name = self.list.unique_name(&type_id, "Unnamed");
        let mut config = editor.create(&name, Storage::Personal);
        config.folder = match &self.selected {
            Some(Node::Folder(t, f)) if *t == type_id => Some(f.clone()),
            Some(Node::Config(k)) if k.type_id == type_id => {
                self.list.get(k).and_then(|c| c.folder.clone())
            }
            _ => None,
        };
        let key = self.list.add(config);
        self.modified = true;
        self.selected = Some(Node::Config(key));
        self.rebuild(window, cx);
        cx.notify();
    }

    fn on_remove(&mut self, _: &DialogRemove, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.nodes();
        let at = self
            .selected
            .as_ref()
            .and_then(|s| nodes.iter().position(|(n, _)| n == s));
        match self.selected.clone() {
            Some(Node::Config(key)) => {
                self.list.remove(&key);
                self.shown.remove(&key);
            }
            Some(Node::Folder(type_id, name)) => {
                let keys: Vec<ConfigKey> = self
                    .list
                    .all()
                    .iter()
                    .filter(|c| c.type_id == type_id && c.folder.as_ref() == Some(&name))
                    .map(RunConfiguration::key)
                    .collect();
                for key in keys {
                    if let Some(c) = self.list.get_mut(&key) {
                        c.folder = None;
                    }
                }
                self.empty_folders
                    .retain(|(t, f)| !(*t == type_id && *f == name));
            }
            _ => return,
        }
        self.modified = true;
        // The node that took its place is selected.
        let nodes = self.nodes();
        self.selected = at
            .map(|i| i.min(nodes.len().saturating_sub(1)))
            .and_then(|i| nodes.get(i).map(|(n, _)| n.clone()));
        self.rebuild(window, cx);
        cx.notify();
    }

    fn on_copy(&mut self, _: &DialogCopy, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        let Some(config) = self.selected_config().cloned() else {
            return;
        };
        let mut copy = config;
        copy.file = None;
        copy.temporary = false;
        copy.name_is_generated = false;
        let key = self.list.add(copy);
        self.modified = true;
        self.selected = Some(Node::Config(key));
        self.rebuild(window, cx);
        cx.notify();
    }

    fn on_save_temporary(
        &mut self,
        _: &DialogSaveTemporary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(Node::Config(key)) = self.selected.clone()
            && self.list.make_permanent(&key)
        {
            self.modified = true;
            self.rebuild(window, cx);
            cx.notify();
        }
    }

    fn on_new_folder(&mut self, _: &DialogNewFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        let type_id = match &self.selected {
            Some(Node::Type(t)) | Some(Node::Folder(t, _)) => t.clone(),
            Some(Node::Config(k)) => k.type_id.clone(),
            None => return,
        };
        let taken: Vec<String> = self
            .nodes()
            .into_iter()
            .filter_map(|(n, _)| match n {
                Node::Folder(t, f) if t == type_id => Some(f),
                _ => None,
            })
            .collect();
        let name = if taken.iter().any(|f| f == "New Folder") {
            (1..)
                .map(|i| format!("New Folder ({i})"))
                .find(|n| !taken.contains(n))
                .expect("a free name")
        } else {
            "New Folder".to_owned()
        };
        // "Move into New Folder" with a configuration selected.
        match self.selected.clone() {
            Some(Node::Config(key)) => {
                if let Some(c) = self.list.get_mut(&key) {
                    c.folder = Some(name.clone());
                }
            }
            _ => self.empty_folders.push((type_id.clone(), name.clone())),
        }
        self.modified = true;
        self.selected = Some(Node::Folder(type_id, name));
        self.rebuild(window, cx);
        cx.notify();
    }

    fn on_sort(&mut self, _: &DialogSort, _: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        self.list.order.clear();
        self.modified = true;
        cx.notify();
    }

    fn on_set(&mut self, action: &DialogSet, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        if action.field == STORE {
            self.set_store(action.value == "true", cx);
        } else {
            self.set_value(&action.field, &action.value, cx);
        }
        self.rebuild(window, cx);
        cx.notify();
    }

    fn on_toggle_option(
        &mut self,
        action: &DialogToggleOption,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit(cx);
        let Some(Node::Config(key)) = self.selected.clone() else {
            return;
        };
        let Some(mut config) = self.list.get(&key).cloned() else {
            return;
        };
        let Some(editor) = self.editor_of(&config.type_id) else {
            return;
        };
        let shown = self.shown.entry(key.clone()).or_default();
        editor.toggle(&mut config, &action.option, shown);
        self.list.replace(&key, config);
        self.modified = true;
        self.rebuild(window, cx);
        cx.notify();
    }

    fn on_modify_options(
        &mut self,
        _: &DialogModifyOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit(cx);
        let Some(config) = self.selected_config().cloned() else {
            return;
        };
        let Some(editor) = self.editor_of(&config.type_id) else {
            return;
        };
        let shown = self.shown.get(&config.key()).cloned().unwrap_or_default();
        let mut items: Vec<MenuItem> = Vec::new();
        let mut group = None;
        for option in editor.options(&config, &shown) {
            if group.as_ref() != Some(&option.group) {
                if option.group.is_empty() {
                    items.push(MenuItem::separator());
                } else {
                    items.push(MenuItem::header(option.group.clone()));
                }
                group = Some(option.group.clone());
            }
            let item = MenuItem::action(
                option.label.clone(),
                DialogToggleOption {
                    option: option.id.clone(),
                },
            );
            items.push(if option.checked {
                item.icon(IconName::CHECKED)
            } else {
                item
            });
        }
        let at = self.options_anchor.get().bottom_left();
        self.open_menu(items, Some("Add Run Options".into()), at, window, cx);
    }

    /// The popup of a choice field.
    fn open_choice(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        let Some(f) = self.fields.iter().find(|f| f.field.id == id) else {
            return;
        };
        let FieldKind::Choice(choices) = &f.field.kind else {
            return;
        };
        let items: Vec<MenuItem> = choices
            .iter()
            .map(|(value, label)| {
                MenuItem::action(
                    label.clone(),
                    DialogSet {
                        field: id.to_owned(),
                        value: value.clone(),
                    },
                )
            })
            .collect();
        let at = self
            .choice_anchor
            .borrow()
            .get(id)
            .map(|b| b.bottom_left())
            .unwrap_or_default();
        self.open_menu(items, None, at, window, cx);
    }

    fn open_menu(
        &mut self,
        items: Vec<MenuItem>,
        title: Option<SharedString>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                title,
                speed_search: true,
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &gpui::DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Hands the edited configurations back.
    fn apply(&mut self, cx: &mut Context<Self>) {
        self.commit(cx);
        let mut list = self.list.clone();
        // Folders without configurations are not kept.
        let selected_ok = list.selected.as_ref().is_none_or(|k| list.contains(k));
        if !selected_ok {
            list.selected = None;
        }
        self.configs.update(cx, |c, cx| c.apply_edits(list, cx));
        self.modified = false;
        cx.notify();
    }

    fn on_apply(&mut self, _: &DialogApply, _: &mut Window, cx: &mut Context<Self>) {
        self.apply(cx);
    }

    fn on_ok(&mut self, _: &DialogOk, _: &mut Window, cx: &mut Context<Self>) {
        self.apply(cx);
        cx.emit(EditDialogEvent::Closed);
    }

    fn on_cancel(&mut self, _: &DialogCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditDialogEvent::Closed);
    }

    /// The dialog for `run.state`.
    pub fn state_json(&self, cx: &App) -> Value {
        let node_json = |n: &Node| match n {
            Node::Type(t) => json!({"kind": "type", "type": t}),
            Node::Folder(t, f) => json!({"kind": "folder", "type": t, "folder": f}),
            Node::Config(k) => json!({"kind": "configuration", "type": k.type_id, "name": k.name}),
        };
        let tree: Vec<Value> = self
            .nodes()
            .iter()
            .map(|(n, depth)| {
                let mut v = node_json(n);
                v["depth"] = json!(depth);
                v["text"] = json!(match n {
                    Node::Type(t) => self.type_name(t),
                    Node::Folder(_, f) => f.clone(),
                    Node::Config(k) => k.name.clone(),
                });
                v
            })
            .collect();
        let config = self.selected_config();
        let fields: Vec<Value> = self
            .fields
            .iter()
            .map(|f| {
                let value = match &f.editor {
                    Some(e) => e.read(cx).text(),
                    None => f.field.value.clone(),
                };
                json!({
                    "id": f.field.id,
                    "label": f.field.label,
                    "kind": match &f.field.kind {
                        FieldKind::Text => "text",
                        FieldKind::Path => "path",
                        FieldKind::Choice(_) => "choice",
                        FieldKind::Flag => "flag",
                        FieldKind::Env => "env",
                    },
                    "value": value,
                    "note": f.field.note,
                })
            })
            .collect();
        let options: Vec<Value> = config
            .and_then(|c| {
                let editor = self.editor_of(&c.type_id)?;
                let shown = self.shown.get(&c.key()).cloned().unwrap_or_default();
                Some(
                    editor
                        .options(c, &shown)
                        .into_iter()
                        .map(|o| json!({"group": o.group, "id": o.id, "label": o.label, "checked": o.checked}))
                        .collect(),
                )
            })
            .unwrap_or_default();
        json!({
            "title": "Run/Debug Configurations",
            "tree": tree,
            "selected": self.selected.as_ref().map(node_json),
            "store_as_project_file": config.map(|c| c.is_shared()),
            "read_only": config.map(|c| c.is_unsupported()),
            "fields": fields,
            "options": options,
            "error": self.error,
            "modified": self.modified,
            "menu": self.menu.as_ref().map(|m| m.menu.read(cx).state_json()),
        })
    }
}

impl Render for EditDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let viewport = window.viewport_size();
        let width = px(WIDTH).min(viewport.width - px(40.0));
        let height = px(HEIGHT).min(viewport.height - px(40.0));
        let text = theme.color(UiRole::Text);
        let secondary = theme.color(UiRole::TextDisabled);
        let line = theme.color(UiRole::PopupBorder);
        let selection = theme.color(UiRole::Selection);
        let accent = theme.color(UiRole::Accent);
        let on_accent = theme.color(UiRole::OnAccent);
        let border = theme.color(UiRole::BorderInput);
        let hover = theme.color(UiRole::Hover);
        let radius = theme.metric(Metric::CornerRadius);

        // The toolbar over the tree.
        let tool = |id: &'static str,
                    icon: IconName,
                    tooltip: &'static str,
                    enabled: bool,
                    action: Box<dyn gpui::Action>| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(ROW))
                .rounded(radius)
                .when(enabled, |d| d.hover(move |s| s.bg(hover)))
                .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
                .child(Icon::new(icon))
                .tooltip(ui::components::Tooltip::text(tooltip, None))
                .when(enabled, |d| {
                    d.on_click(move |_, window, cx| {
                        window.dispatch_action(action.boxed_clone(), cx)
                    })
                })
        };
        let config = self.selected_config().cloned();
        let is_config = config.is_some();
        let toolbar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(6.0))
            .h(px(ROW + 8.0))
            .child(
                div()
                    .relative()
                    .child(tool(
                        "run-dialog-add",
                        IconName::ADD,
                        "Add New Configuration",
                        true,
                        Box::new(DialogAdd { r#type: None }),
                    ))
                    .child(anchor(self.add_anchor.clone())),
            )
            .child(tool(
                "run-dialog-remove",
                IconName::REMOVE,
                "Remove Configuration",
                self.selected
                    .as_ref()
                    .is_some_and(|n| !matches!(n, Node::Type(_))),
                Box::new(DialogRemove),
            ))
            .child(tool(
                "run-dialog-copy",
                IconName::COPY,
                "Copy Configuration",
                is_config,
                Box::new(DialogCopy),
            ))
            .child(tool(
                "run-dialog-save",
                IconName::SAVE,
                "Save Configuration",
                config.as_ref().is_some_and(|c| c.temporary),
                Box::new(DialogSaveTemporary),
            ))
            .child(tool(
                "run-dialog-folder",
                IconName::NEW_FOLDER,
                "Create New Folder",
                self.selected.is_some(),
                Box::new(DialogNewFolder),
            ))
            .child(tool(
                "run-dialog-sort",
                IconName::SORT,
                "Sort Configurations",
                !self.list.is_empty(),
                Box::new(DialogSort),
            ));

        // The tree.
        let nodes = self.nodes();
        let tree_focused = self.tree_focus.is_focused(window);
        let rows: Vec<AnyElement> = nodes
            .iter()
            .enumerate()
            .map(|(i, (node, depth))| {
                let selected = self.selected.as_ref() == Some(node);
                let (icon, label, alpha, note) = match node {
                    Node::Type(t) => {
                        let icon = self
                            .types
                            .get(t)
                            .map(|i| IconName(i.icon))
                            .unwrap_or(IconName::FILE_UNKNOWN);
                        (icon, self.type_name(t), 1.0, None)
                    }
                    Node::Folder(_, f) => (IconName::FOLDER, f.clone(), 1.0, None),
                    Node::Config(k) => {
                        let c = self.list.get(k);
                        let icon = c
                            .and_then(|c| self.types.icon_of(c))
                            .map(IconName)
                            .unwrap_or(IconName::FILE_UNKNOWN);
                        let alpha = if c.is_some_and(|c| c.temporary) {
                            TEMPORARY_ICON_ALPHA
                        } else {
                            1.0
                        };
                        let note = c
                            .filter(|c| c.is_unsupported())
                            .map(|_| super::dropdown::NOT_SUPPORTED);
                        (icon, k.name.clone(), alpha, note)
                    }
                };
                let node = node.clone();
                let selector = format!("run-dialog-node:{label}");
                div()
                    .id(("run-dialog-node", i))
                    .debug_selector(move || selector.clone())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .h(px(ROW))
                    .pl(px(8.0 + INDENT * *depth as f32))
                    .pr(px(8.0))
                    .mx(px(4.0))
                    .rounded(radius)
                    .when(selected, |d| d.bg(selection))
                    .when(selected && !tree_focused, |d| d.opacity(0.9))
                    .child(
                        div()
                            .flex_none()
                            .when(alpha < 1.0, |d| d.opacity(alpha))
                            .child(Icon::new(icon)),
                    )
                    .child(div().whitespace_nowrap().text_color(text).child(label))
                    .children(
                        note.map(|n| div().whitespace_nowrap().text_color(secondary).child(n)),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        window.focus(&this.tree_focus);
                        this.select_node(Some(node.clone()), window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let tree = div()
            .id("run-dialog-tree")
            .debug_selector(|| "run-dialog-tree".into())
            .key_context(TREE_CONTEXT)
            .track_focus(&self.tree_focus)
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .py(px(4.0))
            .children(rows)
            .when(nodes.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(40.0))
                        .text_color(secondary)
                        .child("No run configurations added.")
                        .child(
                            div()
                                .id("run-dialog-add-new")
                                .text_color(accent)
                                .child("Add new\u{2026}")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(DialogAdd { r#type: None }), cx)
                                }),
                        ),
                )
            });
        let left = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(TREE_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(line)
            .child(toolbar)
            .child(tree);

        // The form.
        let field_row = |label: Option<String>, input: AnyElement, note: Option<String>| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_none()
                                .w(px(LABEL_WIDTH))
                                .text_color(text)
                                .whitespace_nowrap()
                                .children(label),
                        )
                        .child(div().flex_1().min_w(px(0.0)).child(input)),
                )
                .children(note.map(|n| {
                    div()
                        .pl(px(LABEL_WIDTH + 8.0))
                        .text_color(secondary)
                        .child(n)
                }))
        };
        let mut form_rows: Vec<AnyElement> = Vec::new();
        let mut name_row: Option<AnyElement> = None;
        for f in &self.fields {
            let id = f.field.id.clone();
            let input: AnyElement = match (&f.field.kind, &f.editor) {
                (FieldKind::Text | FieldKind::Path | FieldKind::Env, Some(editor)) => {
                    let focused = editor.focus_handle(cx).is_focused(window);
                    let selector = format!("run-dialog-field:{id}");
                    input_frame(focused, false, cx)
                        .id(SharedString::from(format!("run-dialog-field-{id}")))
                        .debug_selector(move || selector.clone())
                        .key_context(KEY_CONTEXT)
                        .h(px(ROW + 4.0))
                        .pl(px(6.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .h(editor::field_line_height(window, cx))
                                .overflow_hidden()
                                .child(editor.clone()),
                        )
                        .into_any_element()
                }
                (FieldKind::Choice(choices), _) => {
                    let label = choices
                        .iter()
                        .find(|(v, _)| *v == f.field.value)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| f.field.value.clone());
                    let anchors = self.choice_anchor.clone();
                    let anchor_id = id.clone();
                    let open_id = id.clone();
                    let selector = format!("run-dialog-choice:{id}");
                    div()
                        .id(SharedString::from(format!("run-dialog-choice-{id}")))
                        .debug_selector(move || selector.clone())
                        .relative()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .h(px(ROW + 4.0))
                        .px(px(8.0))
                        .w(px(140.0))
                        .rounded(radius)
                        .border_1()
                        .border_color(border)
                        .hover(move |s| s.bg(hover))
                        .child(div().flex_1().whitespace_nowrap().child(label))
                        .child(Icon::new(IconName::CHEVRON_DOWN))
                        .child(
                            canvas(
                                move |bounds, _, _| {
                                    anchors.borrow_mut().insert(anchor_id.clone(), bounds);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.open_choice(&open_id, window, cx)
                        }))
                        .into_any_element()
                }
                (FieldKind::Flag, _) => {
                    let on = f.field.value == "true";
                    let field = id.clone();
                    div()
                        .id(SharedString::from(format!("run-dialog-flag-{id}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(checkbox(
                            SharedString::from(format!("run-dialog-check-{id}")),
                            on,
                            cx,
                        ))
                        .child(f.field.label.clone())
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(DialogSet {
                                    field: field.clone(),
                                    value: (!on).to_string(),
                                }),
                                cx,
                            )
                        })
                        .into_any_element()
                }
                _ => div().into_any_element(),
            };
            let label = match f.field.kind {
                FieldKind::Flag => None,
                _ => Some(f.field.label.clone()),
            };
            let note = f.field.note.clone().or_else(|| {
                (f.field.kind == FieldKind::Env)
                    .then(|| "Separate variables with semicolon: VAR=value; VAR1=value1".to_owned())
            });
            let row = field_row(label, input, note).into_any_element();
            if id == NAME {
                name_row = Some(row);
            } else {
                form_rows.push(row);
            }
        }
        let right: AnyElement = match (&self.selected, &config) {
            (Some(Node::Config(_)), Some(c)) => {
                let shared = c.is_shared();
                let store = div()
                    .id("run-dialog-store")
                    .debug_selector(|| "run-dialog-store".into())
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .child(checkbox("run-dialog-store-check", shared, cx))
                    .child("Store as project file")
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(DialogSet {
                                field: STORE.to_owned(),
                                value: (!shared).to_string(),
                            }),
                            cx,
                        )
                    });
                let header = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().flex_1().children(name_row))
                    .when(!c.is_unsupported(), |d| d.child(store));
                let body: AnyElement = if c.is_unsupported() {
                    let type_name = self.types.name_of(c).to_owned();
                    let details = self.configs.read(cx).unsupported_details(c);
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(format!("{type_name}: not supported yet"))
                        .child(div().text_color(secondary).child(
                            "Kept as the reference IDE wrote it; it cannot run in Pyoxide yet.",
                        ))
                        .children(details.into_iter().map(|(k, v)| {
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.0))
                                .child(div().w(px(LABEL_WIDTH)).text_color(secondary).child(k))
                                .child(v)
                        }))
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Run"),
                                )
                                .child(
                                    div()
                                        .id("run-dialog-modify-options")
                                        .debug_selector(|| "run-dialog-modify-options".into())
                                        .relative()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(4.0))
                                        .text_color(accent)
                                        .child("Modify options")
                                        .child(Icon::new(IconName::CHEVRON_DOWN))
                                        .child(anchor(self.options_anchor.clone()))
                                        .on_click(|_, window, cx| {
                                            window
                                                .dispatch_action(Box::new(DialogModifyOptions), cx)
                                        }),
                                ),
                        )
                        .children(form_rows)
                        .into_any_element()
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(header)
                    .child(body)
                    .into_any_element()
            }
            (Some(Node::Folder(..)), _) => div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .children(form_rows)
                .children(name_row)
                .child(
                    div()
                        .text_color(secondary)
                        .child("Note: empty folders won't be saved"),
                )
                .into_any_element(),
            _ => div()
                .text_color(secondary)
                .child(if self.list.is_empty() {
                    "Add a configuration with the + button"
                } else {
                    "Select a configuration"
                })
                .into_any_element(),
        };
        let button = |id: &'static str,
                      label: &'static str,
                      default: bool,
                      action: Box<dyn gpui::Action>| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(28.0))
                .min_w(px(72.0))
                .px(px(14.0))
                .rounded(radius)
                .map(|d| {
                    if default {
                        d.bg(accent).text_color(on_accent)
                    } else {
                        d.border_1()
                            .border_color(border)
                            .hover(move |s| s.bg(hover))
                    }
                })
                .child(label)
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(line)
            .child(
                div()
                    .flex_1()
                    .debug_selector(|| "run-dialog-error".into())
                    .text_color(theme.color(UiRole::Danger))
                    .children(self.error.clone().map(|e| format!("Error: {e}"))),
            )
            .child(button(
                "run-dialog-cancel",
                "Cancel",
                false,
                Box::new(DialogCancel),
            ))
            .child(button(
                "run-dialog-apply",
                "Apply",
                false,
                Box::new(DialogApply),
            ))
            .child(button("run-dialog-ok", "OK", true, Box::new(DialogOk)));
        let dialog = div()
            .id("run-dialog")
            .debug_selector(|| "run-dialog".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_select))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_add))
            .on_action(cx.listener(Self::on_remove))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_save_temporary))
            .on_action(cx.listener(Self::on_new_folder))
            .on_action(cx.listener(Self::on_sort))
            .on_action(cx.listener(Self::on_set))
            .on_action(cx.listener(Self::on_toggle_option))
            .on_action(cx.listener(Self::on_modify_options))
            .on_action(cx.listener(Self::on_apply))
            .on_action(cx.listener(Self::on_ok))
            .on_action(cx.listener(Self::on_cancel))
            .flex()
            .flex_col()
            .w(width)
            .h(height)
            .rounded(px(10.0))
            .border_1()
            .border_color(line)
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .text_color(text)
            .child(
                div()
                    .flex()
                    .justify_center()
                    .py(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .border_b_1()
                    .border_color(line)
                    .child("Run/Debug Configurations"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(left)
                    .child(
                        div()
                            .id("run-dialog-form")
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_y_scroll()
                            .p(px(16.0))
                            .child(right),
                    ),
            )
            .child(footer);
        let menu = self
            .menu
            .as_ref()
            .map(|m| ui::menu::overlay(&m.menu, m.position, window));
        div()
            .id("run-dialog-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .child(dialog)
            .children(menu)
    }
}
