//! The Run/Debug Configurations dialog (`EditConfigurationsDialog`,
//! `RunConfigurable`), a window of its own (`ui::dialog_window`): the
//! configurations as a tree (type, folder, configuration) in the list
//! order, a toolbar (Add New Configuration, Remove Configuration, Copy
//! Configuration, Save Configuration, Create New Folder, Sort
//! Configurations), the selected configuration's name, "Store as project
//! file", its type's form with "Modify options" as the reference's
//! fragmented editor draws it (the head fields without labels, the
//! labelled rows, the options that are on as tags), and Run, Cancel, Apply
//! and OK. It edits a copy of the configurations; Apply and OK hand the
//! copy back. A configuration of a type Pyoxide does not support is shown
//! read-only.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use editor::Editor;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, IntoElement, ModifiersChangedEvent, Pixels, Point, Render,
    SharedString, Subscription, Task, Window, canvas, div, prelude::*, px,
};
use run_config::form::{self, Field, FieldKind};
use run_config::{
    ConfigKey, RunConfiguration, RunConfigurations, Storage, TypeEditor, Types, paths,
};
use serde_json::{Value, json};
use themes::{Metric, UiRole};
use ui::components::{checkbox, input_frame};
use ui::icons::{Icon, IconName};
use ui::menu::{ChosenEvent, ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::FieldText;
use super::configurations::RunConfigs;
use super::widget::TEMPORARY_ICON_ALPHA;
use crate::dialog::{Dialog, DialogButton, DialogEvent};

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
    /// Apply, close and run.
    DialogRun => "run.configurations.dialog.run", "Run",
        "Saves the dialog's changes, closes it and runs the selected configuration.";
    /// Expand the selected node.
    DialogExpand => "run.configurations.dialog.expand", "Expand",
        "Expands the selected type or folder of the dialog's tree, or selects its first child.";
    /// Collapse the selected node.
    DialogCollapse => "run.configurations.dialog.collapse", "Collapse",
        "Collapses the selected type or folder of the dialog's tree, or selects its parent.";
    /// Remove the focused option tag.
    DialogRemoveTag => "run.configurations.dialog.remove-tag", "Remove Option",
        "Turns off the option of the focused tag under the form.";
    /// Choose a file for a path field.
    DialogBrowse { field: String } => "run.configurations.dialog.browse", "Browse",
        "Chooses a file or folder for a path field of the selected configuration.",
        r#"{"field": "script"}"#;
}

/// Key contexts of the dialog and its tree.
pub const KEY_CONTEXT: &str = "RunConfigurationsDialog";
pub const TREE_CONTEXT: &str = "RunConfigurationsTree";
/// Key context of an option tag under the form.
pub const TAG_CONTEXT: &str = "RunConfigurationsTag";

/// Ids of the fields every configuration has.
const NAME: &str = "name";
const STORE: &str = "store-as-project-file";
const SHARED_ENV: &str = "shared-env";
const FOLDER_NAME: &str = "folder-name";

/// The window's title, its content size and the smallest size it can be
/// resized to.
pub const TITLE: &str = "Run/Debug Configurations";
pub const WIDTH: f32 = 800.0;
pub const HEIGHT: f32 = 652.0;
pub const MIN_WIDTH: f32 = 640.0;
pub const MIN_HEIGHT: f32 = 420.0;
/// The tree: its width, rows, the margin of the selection and the indent
/// of a level (`Tree.leftChildIndent` + `Tree.rightChildIndent`).
const TREE_WIDTH: f32 = 214.0;
const ROW: f32 = 24.0;
const TREE_MARGIN: f32 = 12.0;
const INDENT: f32 = 19.0;
/// The form: its insets, the label column of its rows, field height and
/// the space between fields.
const FORM_LEFT: f32 = 17.0;
const FORM_RIGHT: f32 = 18.0;
const FORM_TOP: f32 = 18.0;
const NAME_LABEL: f32 = 59.0;
const NAME_FIELD: f32 = 280.0;
const LABEL_WIDTH: f32 = 153.0;
const FIELD: f32 = 28.0;
const FIELD_GAP: f32 = 12.0;
const CHOICE_WIDTH: f32 = 87.0;
/// The largest a popup of the dialog opens before it fits its rows.
const POPUP_MAX: (f32, f32) = (640.0, 900.0);
/// The buttons under the form.
const BUTTON: f32 = 71.0;
const BUTTON_HEIGHT: f32 = 27.0;

/// A node of the tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Node {
    Type(String),
    Folder(String, String),
    Config(ConfigKey),
}

/// What the dialog asks of the workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditDialogEvent {
    /// The dialog is done (OK, Cancel, its window's close button).
    Closed,
    /// Run was chosen: the changes are applied; the dialog closes and the
    /// configuration runs.
    Run(ConfigKey),
}

/// A field of the form and its text field.
struct FieldView {
    field: Field,
    editor: Option<Entity<Editor>>,
}

/// An open popup of the dialog (types, a choice, Modify options).
struct OpenMenu {
    menu: Entity<ContextMenu>,
    /// The popup's window (`ui::dialog_window::open_popup`).
    window: gpui::AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
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
    /// Where the Add, Modify options and Run popups open.
    add_anchor: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    options_anchor: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    run_anchor: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    choice_anchor: std::rc::Rc<std::cell::RefCell<HashMap<String, Bounds<Pixels>>>>,
    /// A question of the dialog ("Store as project file" with personal
    /// values) and its subscription.
    prompt: Option<(Entity<Dialog>, Subscription)>,
    /// What the product tells about fields of the selected configuration
    /// (its interpreter), by field id, and the lookup.
    described: HashMap<String, FieldText>,
    describable: HashSet<String>,
    describing: Option<Task<()>>,
    /// Types and folders whose children are hidden.
    collapsed: HashSet<Node>,
    /// Alt is held: the head fields show their labels.
    hints: bool,
    /// The focus of each option's tag, by option id.
    tag_focus: HashMap<String, FocusHandle>,
    /// The project folder: path fields show their paths expanded from it.
    root: Option<std::path::PathBuf>,
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
        let (list, types, editors, root) = {
            let c = configs.read(cx);
            (
                c.list().clone(),
                c.types().clone(),
                c.editors().to_vec(),
                c.root().map(|r| r.to_owned()),
            )
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
            run_anchor: Default::default(),
            choice_anchor: Default::default(),
            prompt: None,
            described: HashMap::new(),
            describable: HashSet::new(),
            describing: None,
            collapsed: HashSet::new(),
            hints: false,
            tag_focus: HashMap::new(),
            root,
            _subscriptions: Vec::new(),
        };
        // The window's close button cancels; the workbench then closes it.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(this) = this.upgrade() {
                this.update(cx, |_, cx| cx.emit(EditDialogEvent::Closed));
            }
            false
        });
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
        // The tree opens collapsed but for the way to the selected
        // configuration (`RunConfigurable.selectConfigurableOnShow`:
        // `TreeUtil.selectInTree` makes the node visible).
        dialog.collapsed = dialog
            .nodes()
            .into_iter()
            .map(|(n, _)| n)
            .filter(Self::is_parent)
            .collect();
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

    /// The nodes shown: [`EditDialog::nodes`] without the children of
    /// collapsed types and folders.
    fn visible_nodes(&self) -> Vec<(Node, usize)> {
        let mut hidden_below: Option<usize> = None;
        self.nodes()
            .into_iter()
            .filter(|(node, depth)| {
                if let Some(d) = hidden_below {
                    if *depth > d {
                        return false;
                    }
                    hidden_below = None;
                }
                if self.collapsed.contains(node) {
                    hidden_below = Some(*depth);
                }
                true
            })
            .collect()
    }

    /// A type or folder node (it has children).
    fn is_parent(node: &Node) -> bool {
        matches!(node, Node::Type(_) | Node::Folder(..))
    }

    /// Expands a collapsed node, or selects the first child of an expanded
    /// one (`TreeUtil.expandOrSelectFirstChild`, Right).
    fn expand(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.selected.clone() else {
            return;
        };
        if !Self::is_parent(&node) {
            return;
        }
        if self.collapsed.remove(&node) {
            cx.notify();
        } else {
            self.step(1, window, cx);
        }
    }

    /// Collapses an expanded node, or selects the parent (Left).
    fn collapse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.selected.clone() else {
            return;
        };
        if Self::is_parent(&node) && !self.collapsed.contains(&node) {
            self.collapsed.insert(node);
            cx.notify();
            return;
        }
        let nodes = self.visible_nodes();
        let Some(at) = nodes.iter().position(|(n, _)| *n == node) else {
            return;
        };
        let depth = nodes[at].1;
        if let Some((parent, _)) = nodes[..at].iter().rev().find(|(_, d)| *d < depth) {
            self.select_node(Some(parent.clone()), window, cx);
        }
    }

    /// The chevron of a type or folder: expands or collapses it.
    fn toggle_node(&mut self, node: Node, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&node) {
            self.collapsed.insert(node);
        }
        cx.notify();
    }

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
        let nodes = self.visible_nodes();
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
        ui_font: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let placeholder = placeholder.map(|p| SharedString::from(p.to_owned()));
        cx.new(|cx| {
            let mut e = Editor::new(text::Buffer::new(text), window, cx);
            e.set_single_line(true);
            e.set_ui_font(ui_font);
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
                            "Shared variables:",
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
        for field in fields.iter_mut() {
            if field.kind == FieldKind::Path {
                field.value = self.shown_paths(&field.value);
                // A variable that is expanded needs no note.
                if !field.value.starts_with('$') {
                    field.note = None;
                }
            }
        }
        self.reveal();
        for field in fields {
            let editor = match field.kind {
                FieldKind::Text | FieldKind::Path | FieldKind::Env => Some(Self::text_field(
                    &field.value,
                    field.placeholder.as_deref(),
                    Self::ui_font(&field),
                    window,
                    cx,
                )),
                FieldKind::Choice(_) | FieldKind::Flag => None,
            };
            self.fields.push(FieldView { field, editor });
        }
        // Every option that can be a tag can take the focus.
        if let Some(config) = self.selected_config().cloned()
            && let Some(editor) = self.editor_of(&config.type_id)
        {
            let shown = self.shown.get(&config.key()).cloned().unwrap_or_default();
            for option in editor.options(&config, &shown) {
                if option.tag && !self.tag_focus.contains_key(&option.id) {
                    self.tag_focus.insert(option.id.clone(), cx.focus_handle());
                }
            }
        }
        self.describe(cx);
    }

    /// The focus of an option's tag (tests and agents focus a tag with it).
    pub fn tag_focus(&self, option: &str) -> Option<FocusHandle> {
        self.tag_focus.get(option).cloned()
    }

    /// Expands the types and folders above the selected node.
    fn reveal(&mut self) {
        match &self.selected {
            Some(Node::Config(key)) => {
                self.collapsed.remove(&Node::Type(key.type_id.clone()));
                if let Some(folder) = self.list.get(key).and_then(|c| c.folder.clone()) {
                    self.collapsed
                        .remove(&Node::Folder(key.type_id.clone(), folder));
                }
            }
            Some(Node::Folder(type_id, _)) => {
                self.collapsed.remove(&Node::Type(type_id.clone()));
            }
            _ => {}
        }
    }

    /// Paths as a path field shows them: expanded, as the reference loads
    /// them (`PathMacroManager` expands `$PROJECT_DIR$`, `$MODULE_DIR$`
    /// and `$USER_HOME$`); other variables stay as written. A list is
    /// separated by `;`.
    fn shown_paths(&self, stored: &str) -> String {
        let Some(root) = self.root.as_deref() else {
            return stored.to_owned();
        };
        stored
            .split(';')
            .filter_map(|p| paths::resolve(p, root))
            .map(|p| paths::shown(&p))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Paths typed into a path field in their stored form: relative to the
    /// project inside it (ADR 0012), else as typed.
    fn stored_paths(&self, shown: &str) -> String {
        let Some(root) = self.root.as_deref() else {
            return shown.to_owned();
        };
        shown
            .split(';')
            .map(|p| paths::from_reference(p, root))
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Text fields in the editor's font: parameters and options typed as a
    /// command line, environment variables (`RawCommandLineEditor`,
    /// `EnvironmentVariablesTextFieldWithBrowseButton`).
    fn ui_font(field: &Field) -> bool {
        !(field.kind == FieldKind::Env || field.head && field.kind == FieldKind::Text)
    }

    /// Asks the product about the selected configuration's fields it can
    /// tell more of (its interpreter).
    fn describe(&mut self, cx: &mut Context<Self>) {
        self.described.clear();
        self.describable.clear();
        self.describing = None;
        let Some(config) = self.selected_config().cloned() else {
            return;
        };
        let ids: Vec<String> = self.fields.iter().map(|f| f.field.id.clone()).collect();
        let configs = self.configs.clone();
        let tasks: Vec<(String, Task<Option<FieldText>>)> = ids
            .into_iter()
            .filter_map(|id| {
                let task = configs.update(cx, |c, cx| c.describe_field(&config, &id, cx))?;
                Some((id, task))
            })
            .collect();
        if tasks.is_empty() {
            return;
        }
        self.describable = tasks.iter().map(|(id, _)| id.clone()).collect();
        let key = config.key();
        self.describing = Some(cx.spawn(async move |this, cx| {
            for (id, task) in tasks {
                let Some(text) = task.await else {
                    continue;
                };
                let _ = this.update(cx, |this, cx| {
                    if this.selected == Some(Node::Config(key.clone())) {
                        this.described.insert(id, text);
                        cx.notify();
                    }
                });
            }
        }));
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
        let is_path = self
            .fields
            .iter()
            .any(|f| f.field.id == id && f.field.kind == FieldKind::Path);
        let stored = if is_path {
            self.stored_paths(value)
        } else {
            value.to_owned()
        };
        let value = stored.as_str();
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
    fn set_store(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
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
                self.ask_store_values(&personal, window, cx);
                return;
            }
        }
        self.move_storage(on, false, cx);
    }

    /// "Store as project file" for a configuration with personal values:
    /// the values stay personal unless the person chooses otherwise.
    fn ask_store_values(&mut self, names: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let list = names.join(", ");
        let dialog = cx.new(|cx| {
            Dialog::new(
                "Store as Project File",
                format!(
                    "These variables have personal values: {list}.\nThe project file lists their names; their values stay on this machine unless they are stored too."
                ),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "store-values",
                        label: "Store Values Too".into(),
                    },
                    DialogButton {
                        id: "keep-personal",
                        label: "Keep Values Personal".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::QUESTION_DIALOG)
        });
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            let DialogEvent::Closed { button, .. } = event;
            this.prompt = None;
            window.focus(&this.tree_focus);
            if let Some(choice @ ("store-values" | "keep-personal")) = button {
                this.store_as_project_file(*choice == "store-values", window, cx);
            }
            cx.notify();
        });
        window.focus(&dialog.focus_handle(cx));
        self.prompt = Some((dialog, subscription));
        cx.notify();
    }

    /// The answer to the question of [`EditDialog::ask_store_values`]: the
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
        let nodes = self.visible_nodes();
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
        let nodes = self.visible_nodes();
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
            self.set_store(action.value == "true", window, cx);
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
        // A group's rows together, in the order the groups first come.
        let mut options = editor.options(&config, &shown);
        let mut groups: Vec<String> = Vec::new();
        for o in &options {
            if !groups.contains(&o.group) {
                groups.push(o.group.clone());
            }
        }
        options.sort_by_key(|o| groups.iter().position(|g| *g == o.group));
        for option in options {
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
            )
            .enabled(option.enabled);
            items.push(if option.checked {
                item.icon(IconName::CHECKED)
            } else {
                item
            });
        }
        // Under the link, its left edge at the link's (`showPopup` below
        // the component).
        let at = self.options_anchor.get().bottom_left() + gpui::point(px(-8.0), px(6.0));
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
        self.close_menu(cx);
        // The shortcuts of the rows where their actions run: here.
        let mut items = items;
        ui::menu::fill_shortcuts(&mut items, &self.focus, window);
        let options = PopupOptions {
            title,
            speed_search: true,
            own_window: true,
            ..PopupOptions::default()
        };
        let mut built = None;
        let popup = ui::dialog_window::open_popup(
            window,
            position,
            gpui::size(px(POPUP_MAX.0), px(POPUP_MAX.1)),
            !crate::env::background(),
            cx,
            |window, cx| {
                let menu = ContextMenu::popup(items, options, window, cx);
                built = Some(menu.clone());
                menu.into()
            },
        );
        let (Some(popup), Some(menu)) = (popup, built) else {
            return;
        };
        let subscriptions = vec![
            // The chosen row's action runs in the dialog.
            cx.subscribe_in(&menu, window, |_, _, chosen: &ChosenEvent, window, cx| {
                window.dispatch_action(chosen.0.boxed_clone(), cx);
            }),
            cx.subscribe(&menu, |this, _, _: &gpui::DismissEvent, cx| {
                this.close_menu(cx);
            }),
        ];
        self.menu = Some(OpenMenu {
            menu,
            window: popup.into(),
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Closes the open popup's window.
    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.menu.take() {
            ui::dialog_window::close(open.window, cx);
            cx.notify();
        }
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

    fn on_run(&mut self, _: &DialogRun, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self
            .selected_config()
            .filter(|c| !c.is_unsupported())
            .map(RunConfiguration::key)
        else {
            return;
        };
        self.apply(cx);
        cx.emit(EditDialogEvent::Run(key));
    }

    fn on_expand(&mut self, _: &DialogExpand, window: &mut Window, cx: &mut Context<Self>) {
        self.expand(window, cx);
    }

    fn on_collapse(&mut self, _: &DialogCollapse, window: &mut Window, cx: &mut Context<Self>) {
        self.collapse(window, cx);
    }

    /// The Browse button of a path field: a file (a folder for a working
    /// directory) chosen in the system's dialog.
    fn on_browse(&mut self, action: &DialogBrowse, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        let id = action.field.clone();
        let directories = id.contains("directory");
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: None,
        });
        let root = self.configs.read(cx).root().map(|r| r.to_owned());
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            // Inside the project the path is kept relative to it.
            let value = root
                .as_deref()
                .and_then(|r| path.strip_prefix(r).ok())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_value(&id, &value, cx);
                this.rebuild(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The Run split button's arrow: the executors.
    fn open_executors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![
            MenuItem::action("Run", DialogRun).icon(IconName::RUN),
            MenuItem::unavailable("Debug").icon(IconName::DEBUG),
        ];
        let at = self.run_anchor.get().bottom_left();
        self.open_menu(items, None, at, window, cx);
    }

    /// The dialog for `run.state`.
    pub fn state_json(&self, cx: &App) -> Value {
        let node_json = |n: &Node| match n {
            Node::Type(t) => json!({"kind": "type", "type": t}),
            Node::Folder(t, f) => json!({"kind": "folder", "type": t, "folder": f}),
            Node::Config(k) => json!({"kind": "configuration", "type": k.type_id, "name": k.name}),
        };
        let tree: Vec<Value> = self
            .visible_nodes()
            .iter()
            .map(|(n, depth)| {
                let mut v = node_json(n);
                v["depth"] = json!(depth);
                if Self::is_parent(n) {
                    v["expanded"] = json!(!self.collapsed.contains(n));
                }
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
                    "head": f.field.head,
                    "described": self.described.get(&f.field.id).map(|d| json!({"text": d.text, "detail": d.detail})),
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
                        .map(|o| json!({"group": o.group, "id": o.id, "label": o.label, "checked": o.checked, "tag": o.tag}))
                        .collect(),
                )
            })
            .unwrap_or_default();
        json!({
            "title": TITLE,
            "tree": tree,
            "selected": self.selected.as_ref().map(node_json),
            "store_as_project_file": config.map(|c| c.is_shared()),
            "read_only": config.map(|c| c.is_unsupported()),
            "fields": fields,
            "options": options,
            "error": self.error,
            "modified": self.modified,
            "menu": self.menu.as_ref().map(|m| m.menu.read(cx).state_json()),
            "prompt": self.prompt.as_ref().map(|(d, _)| d.read(cx).state_json()),
        })
    }
}

/// Colours of the dialog, read once per frame.
#[derive(Clone, Copy)]
struct Colors {
    surface: gpui::Hsla,
    raised: gpui::Hsla,
    text: gpui::Hsla,
    muted: gpui::Hsla,
    disabled: gpui::Hsla,
    border: gpui::Hsla,
    input: gpui::Hsla,
    divider: gpui::Hsla,
    selection: gpui::Hsla,
    accent: gpui::Hsla,
    on_accent: gpui::Hsla,
    link: gpui::Hsla,
    hover: gpui::Hsla,
    danger: gpui::Hsla,
    focus: gpui::Hsla,
    radius: Pixels,
    field_radius: Pixels,
}

impl Colors {
    fn of(cx: &App) -> Self {
        let theme = cx.theme();
        let radius = theme.metric(Metric::CornerRadius);
        Self {
            surface: theme.color(UiRole::Surface),
            raised: theme.color(UiRole::SurfaceRaised),
            text: theme.color(UiRole::Text),
            muted: theme.color(UiRole::TextMuted),
            disabled: theme.color(UiRole::TextDisabled),
            border: theme.color(UiRole::Border),
            input: theme.color(UiRole::BorderInput),
            divider: theme.color(UiRole::Divider),
            selection: theme.color(UiRole::Selection),
            accent: theme.color(UiRole::Accent),
            on_accent: theme.color(UiRole::OnAccent),
            link: theme.color(UiRole::TextLink),
            hover: theme.color(UiRole::Hover),
            danger: theme.color(UiRole::Danger),
            focus: theme.color(UiRole::Focus),
            radius,
            field_radius: radius * 1.5,
        }
    }
}

/// A small button inside a field (Insert Macros, Browse, Expand); without
/// an action it is drawn disabled.
fn field_button(
    id: SharedString,
    icon: IconName,
    tooltip: &'static str,
    action: Option<Box<dyn gpui::Action>>,
) -> AnyElement {
    let enabled = action.is_some();
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(16.0))
        .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
        .child(Icon::new(icon))
        .tooltip(ui::components::Tooltip::text(tooltip, None))
        .when_some(action, |d, action| {
            d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        })
        .into_any_element()
}

/// A combo box: filled, with the arrow at its end (`ComboBox` in the New
/// UI's dialogs).
fn combo(id: SharedString, c: Colors, content: Vec<AnyElement>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(FIELD))
        .px(px(8.0))
        .rounded(c.field_radius)
        .border_1()
        .border_color(c.input)
        .bg(c.raised)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .children(content),
        )
        .child(div().flex_none().child(Icon::new(IconName::CHEVRON_DOWN)))
}

impl EditDialog {
    /// The left side: the toolbar, the tree and the templates link.
    fn render_tree(&self, c: Colors, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
                .size(px(22.0))
                .rounded(c.radius)
                .when(enabled, |d| d.hover(move |s| s.bg(c.hover)))
                .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
                .child(Icon::new(icon))
                .tooltip(ui::components::Tooltip::text(tooltip, None))
                .when(enabled, |d| {
                    d.on_click(move |_, window, cx| {
                        window.dispatch_action(action.boxed_clone(), cx)
                    })
                })
        };
        let config = self.selected_config();
        let is_config = config.is_some();
        let temporary = config.is_some_and(|c| c.temporary);
        // The toolbar (`RunConfigurable.createLeftPanel`); Save shows for a
        // temporary configuration.
        let toolbar = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(2.0))
            .h(px(ROW))
            .border_b_1()
            .border_color(c.border)
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
            .when(temporary, |d| {
                d.child(tool(
                    "run-dialog-save",
                    IconName::SAVE,
                    "Save Configuration",
                    true,
                    Box::new(DialogSaveTemporary),
                ))
            })
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

        let nodes = self.visible_nodes();
        let tree_focused = self.tree_focus.is_focused(window);
        // Indent guides (`ui::tree_guides`): the chevrons sit at the row's
        // start, one indent per level.
        let guides = ui::tree_guides::TreeGuides {
            first: px(0.0),
            indent: px(INDENT),
            control: px(16.0),
        };
        let guide_color = cx.theme().color(UiRole::TreeIndentGuide);
        let rows: Vec<AnyElement> =
            nodes
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
                    let parent = Self::is_parent(node);
                    let expanded = !self.collapsed.contains(node);
                    let bold = matches!(node, Node::Type(_));
                    let toggle = node.clone();
                    let node = node.clone();
                    let selector = format!("run-dialog-node:{label}");
                    div()
                        .id(("run-dialog-node", i))
                        .debug_selector(move || selector.clone())
                        .flex()
                        .flex_none()
                        .flex_row()
                        .items_center()
                        .h(px(ROW))
                        .mx(px(TREE_MARGIN))
                        .pl(px(INDENT * *depth as f32))
                        .rounded(c.radius)
                        .when(selected, |d| d.bg(c.selection))
                        .when(selected && !tree_focused, |d| d.opacity(0.9))
                        .children(guides.row(*depth, parent && expanded, px(ROW), guide_color))
                        .child(
                            div()
                                .id(("run-dialog-chevron", i))
                                .flex_none()
                                .w(px(16.0))
                                .when(parent, |d| {
                                    d.child(Icon::new(if expanded {
                                        IconName::CHEVRON_DOWN
                                    } else {
                                        IconName::CHEVRON_RIGHT
                                    }))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_node(toggle.clone(), cx);
                                    }))
                                }),
                        )
                        .child(
                            div()
                                .flex_none()
                                .ml(px(5.0))
                                .when(alpha < 1.0, |d| d.opacity(alpha))
                                .child(Icon::new(icon)),
                        )
                        .child(
                            div()
                                .ml(px(3.0))
                                .whitespace_nowrap()
                                .text_color(c.text)
                                .when(bold, |d| d.font_weight(FontWeight::SEMIBOLD))
                                .child(label),
                        )
                        .children(note.map(|n| {
                            div()
                                .ml(px(6.0))
                                .whitespace_nowrap()
                                .text_color(c.muted)
                                .child(n)
                        }))
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
            .min_h(px(0.0))
            .overflow_y_scroll()
            .pt(px(5.0))
            .children(rows)
            .when(nodes.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(40.0))
                        .text_color(c.muted)
                        .child("No run configurations added.")
                        .child(
                            div()
                                .id("run-dialog-add-new")
                                .text_color(c.link)
                                .child("Add new\u{2026}")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(DialogAdd { r#type: None }), cx)
                                }),
                        ),
                )
            });
        // Templates are not editable yet.
        let templates = div()
            .flex_none()
            .pl(px(11.0))
            .pt(px(8.0))
            .pb(px(12.0))
            .text_color(c.link)
            .opacity(ui::components::DISABLED_ALPHA)
            .whitespace_nowrap()
            .child("Edit configuration templates\u{2026}");
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(TREE_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(c.border)
            .child(toolbar)
            .child(tree)
            .child(templates)
            .into_any_element()
    }

    /// A text field of the form with its buttons.
    fn render_text_field(
        &self,
        f: &FieldView,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = f.field.id.clone();
        let focused = editor.focus_handle(cx).is_focused(window);
        let selector = format!("run-dialog-field:{id}");
        let button_id = |name: &str| SharedString::from(format!("run-dialog-{name}-{id}"));
        let buttons: Vec<AnyElement> = match f.field.kind {
            FieldKind::Path => vec![
                field_button(button_id("macros"), IconName::MACROS, "Insert Macros", None),
                field_button(
                    button_id("browse"),
                    IconName::BROWSE,
                    "Browse",
                    Some(Box::new(DialogBrowse { field: id.clone() })),
                ),
            ],
            FieldKind::Env => vec![field_button(
                button_id("env"),
                IconName::MACROS,
                "Edit Environment Variables",
                None,
            )],
            _ if f.field.head => vec![
                field_button(button_id("macros"), IconName::MACROS, "Insert Macros", None),
                field_button(button_id("expand"), IconName::EXPAND, "Expand", None),
            ],
            _ => Vec::new(),
        };
        input_frame(focused, false, cx)
            .id(SharedString::from(format!("run-dialog-field-{id}")))
            .debug_selector(move || selector.clone())
            .key_context(KEY_CONTEXT)
            .h(px(FIELD))
            .pl(px(8.0))
            .pr(px(7.0))
            .gap(px(9.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(editor::field_line_height_for(
                        Self::ui_font(&f.field),
                        window,
                        cx,
                    ))
                    .overflow_hidden()
                    .child(editor.clone()),
            )
            .children(buttons)
            .into_any_element()
    }

    /// A choice field: a combo box that opens its values.
    fn render_choice(&self, f: &FieldView, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let FieldKind::Choice(choices) = &f.field.kind else {
            return div().into_any_element();
        };
        let id = f.field.id.clone();
        let label = choices
            .iter()
            .find(|(v, _)| *v == f.field.value)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| f.field.value.clone());
        let anchors = self.choice_anchor.clone();
        let anchor_id = id.clone();
        let open_id = id.clone();
        let selector = format!("run-dialog-choice:{id}");
        combo(
            SharedString::from(format!("run-dialog-choice-{id}")),
            c,
            vec![label.into_any_element()],
        )
        .debug_selector(move || selector.clone())
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

    /// A field the product describes (the interpreter): a combo box with
    /// the icon, the name and grey details.
    fn render_described(&self, f: &FieldView, c: Colors) -> AnyElement {
        let id = f.field.id.clone();
        let selector = format!("run-dialog-field:{id}");
        let content: Vec<AnyElement> = match self.described.get(&id) {
            Some(d) => {
                let mut out = Vec::new();
                if let Some(icon) = d.icon {
                    out.push(Icon::new(icon).into_any_element());
                }
                out.push(
                    div()
                        .text_color(c.text)
                        .child(d.text.clone())
                        .into_any_element(),
                );
                out.push(
                    div()
                        .text_color(c.muted)
                        .child(d.detail.clone())
                        .into_any_element(),
                );
                out
            }
            None => vec![
                div()
                    .text_color(c.muted)
                    .child(if f.field.value.is_empty() {
                        f.field.placeholder.clone().unwrap_or_default()
                    } else {
                        f.field.value.clone()
                    })
                    .into_any_element(),
            ],
        };
        // TODO(#75): choosing the interpreter here; until interpreters are
        // configured in the IDE the combo only shows the one that runs.
        combo(
            SharedString::from(format!("run-dialog-combo-{id}")),
            c,
            content,
        )
        .debug_selector(move || selector.clone())
        .tooltip(ui::components::Tooltip::text(
            "The interpreter is chosen in the interpreter widget",
            None,
        ))
        .into_any_element()
    }

    /// The form of the selected configuration (`RunConfigurationFragmentedEditor`):
    /// Name and "Store as project file", the Run header with Modify
    /// options, the head fields, the labelled rows and the tags.
    fn render_form(&self, c: Colors, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let config = self.selected_config().cloned();
        let field_input =
            |this: &Self, f: &FieldView, window: &mut Window, cx: &mut Context<Self>| {
                if this.describable.contains(&f.field.id) {
                    return this.render_described(f, c);
                }
                match (&f.field.kind, &f.editor) {
                    (FieldKind::Choice(_), _) => this.render_choice(f, c, cx),
                    (FieldKind::Flag, _) => {
                        let on = f.field.value == "true";
                        let field = f.field.id.clone();
                        let id = f.field.id.clone();
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
                    (_, Some(editor)) => this.render_text_field(f, editor, window, cx),
                    _ => div().into_any_element(),
                }
            };

        let Some(Node::Config(_)) = &self.selected else {
            // A folder: its name; a type or nothing: a hint.
            return match &self.selected {
                Some(Node::Folder(..)) => {
                    let rows: Vec<AnyElement> = self
                        .fields
                        .iter()
                        .map(|f| {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(LABEL_WIDTH))
                                        .whitespace_nowrap()
                                        .child(f.field.label.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(field_input(self, f, window, cx)),
                                )
                                .into_any_element()
                        })
                        .collect();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(FIELD_GAP))
                        .pt(px(FORM_TOP))
                        .pl(px(FORM_LEFT))
                        .pr(px(FORM_RIGHT))
                        .children(rows)
                        .child(
                            div()
                                .text_color(c.muted)
                                .child("Note: empty folders won't be saved"),
                        )
                        .into_any_element()
                }
                _ => div()
                    .pt(px(FORM_TOP))
                    .pl(px(FORM_LEFT))
                    .text_color(c.muted)
                    .child(if self.list.is_empty() {
                        "Add a configuration with the + button"
                    } else {
                        "Select a configuration"
                    })
                    .into_any_element(),
            };
        };
        let Some(config) = config else {
            return div().into_any_element();
        };

        // Name and "Store as project file".
        let name = self.fields.iter().find(|f| f.field.id == NAME);
        let shared = config.is_shared();
        let store = div()
            .id("run-dialog-store")
            .debug_selector(|| "run-dialog-store".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .child(checkbox("run-dialog-store-check", shared, cx))
            .child(div().whitespace_nowrap().child("Store as project file"))
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(DialogSet {
                        field: STORE.to_owned(),
                        value: (!shared).to_string(),
                    }),
                    cx,
                )
            });
        let name_row = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(FIELD))
            .child(
                div()
                    .flex_none()
                    .w(px(NAME_LABEL))
                    .whitespace_nowrap()
                    .child("Name:"),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(NAME_FIELD))
                    .children(name.map(|f| field_input(self, f, window, cx))),
            )
            .child(div().flex_1())
            .when(!config.is_unsupported(), |d| {
                d.child(store).child(
                    div()
                        .flex_none()
                        .ml(px(6.0))
                        .opacity(ui::components::DISABLED_ALPHA)
                        .child(Icon::new(IconName::INLINE_SETTINGS)),
                )
            });

        let body: AnyElement = if config.is_unsupported() {
            let type_name = self.types.name_of(&config).to_owned();
            let details = self.configs.read(cx).unsupported_details(&config);
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .mt(px(20.0))
                .child(format!("{type_name}: not supported yet"))
                .child(
                    div()
                        .text_color(c.muted)
                        .child("Kept as the reference IDE wrote it; it cannot run in Pyoxide yet."),
                )
                .children(details.into_iter().map(|(k, v)| {
                    div()
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .flex_none()
                                .w(px(LABEL_WIDTH))
                                .text_color(c.muted)
                                .child(k),
                        )
                        .child(v)
                }))
                .into_any_element()
        } else {
            // The Run header (`RunConfigurationFragmentedEditor`'s group
            // with "Modify options" and its shortcut).
            let header = div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(24.0))
                .mt(px(9.0))
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
                        .text_color(c.link)
                        // A link: the hand and the underline under the
                        // pointer (`DropDownLink`).
                        .cursor_pointer()
                        .group("run-dialog-modify-options")
                        .child(
                            div()
                                .group_hover("run-dialog-modify-options", |s| s.underline())
                                .child("Modify options"),
                        )
                        .child(Icon::new(IconName::CHEVRON_DOWN))
                        .child(anchor(self.options_anchor.clone()))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(DialogModifyOptions), cx)
                        }),
                )
                .child(div().flex_none().ml(px(12.0)).text_color(c.muted).children(
                    ui::keys::shortcut_in(&DialogModifyOptions, &self.focus, window),
                ));
            // The head: the module, then the head fields (a choice and the
            // field after it share a line).
            let head: Vec<&FieldView> = self
                .fields
                .iter()
                .filter(|f| f.field.head && f.field.id != NAME)
                .collect();
            let small = cx.theme().ui_size() - px(2.0);
            let hint = |label: &str| {
                let label = label.trim_end_matches(':').to_owned();
                div()
                    .text_size(small)
                    .text_color(c.muted)
                    .whitespace_nowrap()
                    .child(label)
            };
            let mut head_rows: Vec<AnyElement> = Vec::new();
            if !head.is_empty() {
                let module = self
                    .configs
                    .read(cx)
                    .root()
                    .and_then(|r| r.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                head_rows.push(
                    combo(
                        "run-dialog-module".into(),
                        c,
                        vec![
                            Icon::new(IconName::MODULE).into_any_element(),
                            div().child(module).into_any_element(),
                        ],
                    )
                    .debug_selector(|| "run-dialog-module".into())
                    .into_any_element(),
                );
            }
            let mut i = 0;
            while i < head.len() {
                let f = head[i];
                let paired = matches!(f.field.kind, FieldKind::Choice(_)) && i + 1 < head.len();
                let row = if paired {
                    let next = head[i + 1];
                    i += 2;
                    div()
                        .flex()
                        .flex_row()
                        .items_end()
                        .gap(px(7.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .flex_none()
                                .w(px(CHOICE_WIDTH))
                                .when(self.hints, |d| d.child(hint(&f.field.label)))
                                .child(field_input(self, f, window, cx)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .flex_1()
                                .min_w(px(0.0))
                                .when(self.hints, |d| d.child(hint(&next.field.label)))
                                .child(field_input(self, next, window, cx)),
                        )
                        .into_any_element()
                } else {
                    i += 1;
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .when(self.hints, |d| d.child(hint(&f.field.label)))
                        .child(field_input(self, f, window, cx))
                        .into_any_element()
                };
                head_rows.push(row);
            }
            // The labelled rows.
            let rows: Vec<AnyElement> = self
                .fields
                .iter()
                .filter(|f| !f.field.head && f.field.id != NAME)
                .map(|f| {
                    let note = f.field.note.clone().or_else(|| {
                        (f.field.kind == FieldKind::Env).then(|| {
                            "Separate variables with semicolon: VAR=value; VAR1=value1".to_owned()
                        })
                    });
                    let label = match f.field.kind {
                        FieldKind::Flag => None,
                        _ => Some(f.field.label.clone()),
                    };
                    let choice = matches!(f.field.kind, FieldKind::Choice(_));
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(LABEL_WIDTH))
                                        .whitespace_nowrap()
                                        .children(label),
                                )
                                .child(
                                    div()
                                        .when(choice, |d| d.flex_none().w(px(2.0 * CHOICE_WIDTH)))
                                        .when(!choice, |d| d.flex_1())
                                        .min_w(px(0.0))
                                        .child(field_input(self, f, window, cx)),
                                ),
                        )
                        .children(note.map(|n| {
                            div()
                                .pl(px(LABEL_WIDTH))
                                .pt(px(10.0))
                                .text_color(c.muted)
                                .child(n)
                        }))
                        .into_any_element()
                })
                .collect();
            // The options that are on, as tags with a remove button; the
            // Before Launch ones come first, as the reference's fragments.
            let mut on: Vec<form::OptionItem> = self
                .editor_of(&config.type_id)
                .map(|editor| {
                    let shown = self.shown.get(&config.key()).cloned().unwrap_or_default();
                    editor.options(&config, &shown)
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|o| o.tag && o.checked)
                .collect();
            on.sort_by_key(|o| o.group != "Before Launch");
            let tags: Vec<AnyElement> = on
                .into_iter()
                .enumerate()
                .map(|(i, o)| {
                    let selector = format!("run-dialog-tag:{}", o.label);
                    let option = o.id.clone();
                    let remove = option.clone();
                    let focus = self.tag_focus.get(&o.id).cloned();
                    let focused = focus.as_ref().is_some_and(|f| f.is_focused(window));
                    // "Remove option" with the key that removes a focused
                    // tag (`TagButton`).
                    let keys = focus.as_ref().and_then(|f| {
                        ui::keys::shortcut_in(&DialogRemoveTag, f, window).map(SharedString::from)
                    });
                    let dialog_focus = self.focus.clone();
                    let click_focus = focus.clone();
                    div()
                        .id(("run-dialog-tag", i))
                        .debug_selector(move || selector.clone())
                        .key_context(TAG_CONTEXT)
                        .when_some(focus.as_ref(), |d, f| d.track_focus(f))
                        .on_action(move |_: &DialogRemoveTag, window, cx| {
                            window.focus(&dialog_focus);
                            window.dispatch_action(
                                Box::new(DialogToggleOption {
                                    option: remove.clone(),
                                }),
                                cx,
                            )
                        })
                        .flex()
                        .flex_none()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(FIELD))
                        .pl(px(11.0))
                        .pr(px(8.0))
                        .rounded_full()
                        .bg(c.raised)
                        .border_1()
                        .border_color(if focused { c.focus } else { c.raised })
                        .child(div().whitespace_nowrap().child(o.label.clone()))
                        .child(
                            div()
                                .id(("run-dialog-tag-remove", i))
                                .flex_none()
                                .cursor_pointer()
                                .child(Icon::new(IconName::CLOSE_SMALL))
                                .tooltip(ui::components::Tooltip::text("Remove option", keys))
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    window.dispatch_action(
                                        Box::new(DialogToggleOption {
                                            option: option.clone(),
                                        }),
                                        cx,
                                    )
                                }),
                        )
                        .on_click(move |_, window, _| {
                            if let Some(f) = &click_focus {
                                window.focus(f);
                            }
                        })
                        .into_any_element()
                })
                .collect();
            let has_head = !head_rows.is_empty();
            div()
                .flex()
                .flex_col()
                .child(div().mt(px(20.0)).h(px(1.0)).bg(c.divider))
                .child(header)
                .when(has_head, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(FIELD_GAP))
                            .mt(px(11.0))
                            .children(head_rows),
                    )
                    .child(
                        div()
                            .mt(px(9.0))
                            .text_color(c.muted)
                            .child("Press \u{2325} for field hints"),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(FIELD_GAP))
                        .mt(px(if has_head { 14.0 } else { 11.0 }))
                        .children(rows),
                )
                .when(!tags.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap_x(px(8.0))
                            .gap_y(px(FIELD_GAP))
                            .mt(px(32.0))
                            .children(tags),
                    )
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .pt(px(FORM_TOP))
            .pl(px(FORM_LEFT))
            .pr(px(FORM_RIGHT))
            .pb(px(16.0))
            .child(name_row)
            .child(body)
            .into_any_element()
    }

    /// The buttons: help, the error, Run with its executors, Cancel,
    /// Apply and OK.
    fn render_footer(&self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let can_run = self
            .selected_config()
            .is_some_and(|config| !config.is_unsupported());
        let button = |id: &'static str,
                      label: &'static str,
                      default: bool,
                      enabled: bool,
                      action: Box<dyn gpui::Action>| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(BUTTON_HEIGHT))
                .min_w(px(BUTTON))
                .px(px(14.0))
                .rounded(c.field_radius)
                .map(|d| {
                    if default {
                        d.bg(c.accent).text_color(c.on_accent)
                    } else {
                        d.border_1()
                            .border_color(c.input)
                            .when(enabled, |d| d.hover(move |s| s.bg(c.hover)))
                    }
                })
                .when(!enabled, |d| d.text_color(c.disabled))
                .child(label)
                .when(enabled, |d| {
                    d.on_click(move |_, window, cx| {
                        window.dispatch_action(action.boxed_clone(), cx)
                    })
                })
        };
        // The Run split button (`DialogWrapper` with the executors).
        let run = div()
            .id("run-dialog-run")
            .debug_selector(|| "run-dialog-run".into())
            .relative()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(BUTTON_HEIGHT))
            .rounded(c.field_radius)
            .border_1()
            .border_color(c.input)
            .when(!can_run, |d| d.opacity(ui::components::DISABLED_ALPHA))
            .child(
                div()
                    .id("run-dialog-run-main")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .h_full()
                    .px(px(14.0))
                    .child(Icon::new(IconName::RUN))
                    .child("Run")
                    .when(can_run, |d| {
                        d.on_click(|_, window, cx| window.dispatch_action(Box::new(DialogRun), cx))
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h(px(BUTTON_HEIGHT - 10.0))
                    .bg(c.input),
            )
            .child(
                div()
                    .id("run-dialog-run-more")
                    .flex()
                    .items_center()
                    .h_full()
                    .px(px(7.0))
                    .child(Icon::new(IconName::CHEVRON_DOWN))
                    .when(can_run, |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_executors(window, cx)
                        }))
                    }),
            )
            .child(anchor(self.run_anchor.clone()));
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(BUTTON_HEIGHT + 25.0))
            .pl(px(15.0))
            .pr(px(15.0))
            .border_t_1()
            .border_color(c.border)
            .child(
                div()
                    .id("run-dialog-help")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.0))
                    .rounded_full()
                    .border_1()
                    .border_color(c.input)
                    .opacity(ui::components::DISABLED_ALPHA)
                    .child(Icon::new(IconName::HELP))
                    .tooltip(ui::components::Tooltip::text("Help", None)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .px(px(12.0))
                    .debug_selector(|| "run-dialog-error".into())
                    .text_color(c.danger)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .children(self.error.clone().map(|e| format!("Error: {e}"))),
            )
            .child(run)
            .child(div().w(px(19.0)))
            .child(button(
                "run-dialog-cancel",
                "Cancel",
                false,
                true,
                Box::new(DialogCancel),
            ))
            .child(div().w(px(13.0)))
            .child(button(
                "run-dialog-apply",
                "Apply",
                false,
                self.modified,
                Box::new(DialogApply),
            ))
            .child(div().w(px(13.0)))
            .child(button(
                "run-dialog-ok",
                "OK",
                true,
                true,
                Box::new(DialogOk),
            ))
            .into_any_element()
    }
}

impl Render for EditDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = Colors::of(cx);
        let family = cx.theme().ui_family();
        let size = cx.theme().ui_size();
        let left = self.render_tree(c, window, cx);
        let form = self.render_form(c, window, cx);
        let footer = self.render_footer(c, cx);
        let prompt = self.prompt.as_ref().map(|(d, _)| d.clone());
        div()
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
            .on_action(cx.listener(Self::on_run))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_collapse))
            .on_action(cx.listener(Self::on_browse))
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, cx| {
                let alt = e.modifiers.alt;
                if alt != this.hints {
                    this.hints = alt;
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(c.surface)
            .text_color(c.text)
            .font_family(family)
            .text_size(size)
            // The title bar in the theme's colours (macOS; elsewhere the
            // system's).
            .children(ui::dialog_window::title_bar(TITLE, true, cx))
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
                            .child(form),
                    ),
            )
            .child(footer)
            .children(prompt)
    }
}
