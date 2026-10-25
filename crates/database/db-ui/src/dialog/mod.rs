//! The Data Sources dialog (the reference's "Data Sources and Drivers"
//! window, Data Sources page): the project's data sources on the left,
//! the selected one's name and tabs on the right (General, Options,
//! SSH/SSL, Schemas), Test Connection under the tabs and Cancel, Apply, OK
//! at the bottom.
//!
//! Nothing is written until Apply or OK: the dialog edits copies of the
//! profiles, a typed password stays in its field, and the store writes the
//! profiles and the passwords together. Test Connection and the schema
//! list connect through the store off the UI thread; closing the dialog or
//! starting another test cancels a running one.

mod view;

use std::path::PathBuf;

use db_core::profile::{DEFAULT_AUTO_DISCONNECT_SECS, DEFAULT_KEEP_ALIVE_SECS};
use db_core::schema::{Level, SchemaFilter, SchemaInfo, Scope};
use db_core::{Backend, Dialect, PasswordSave, Profile, Secret, TlsMode, TlsProfile, TxMode};
use editor::Editor;
use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, Global,
    Pixels, Subscription, Task, WeakEntity, Window, px,
};
use serde_json::{Value as Json, json};
use ui::form::MenuPopup;
use ui::menu::MenuItem;

use crate::password::{PasswordEdited, PasswordField};
use crate::store::{DataSource, Failure, PasswordChange, SaveRequest, ServerInfo};
use crate::{BACKENDS, backend_title, icons};

ui::ui_commands! {
    COMMANDS, scope = "DataSourcesDialog";
    /// Set a field.
    DialogSet { field: String, value: String } => "db.data-sources.set", "Set Field",
        "Sets a field of the selected data source in the Data Sources dialog. Text fields: `name`, `comment`, `host`, `port`, `user`, `database`, `service`, `dsn`, `path`, `url`, `time-zone`, `keep-alive-secs`, `auto-disconnect-secs`, `startup-script`, `refresh-minutes`, `ca-file`, `client-cert`, `client-key`, `wallet-dir`, `schema-pattern`; `password` (typed, never echoed). Choices: `driver` (a backend id), `connection-type` (`default`, `url-only`, `tns`), `auth` (`user-password`, `none`), `save` (`forever`, `until-restart`, `never`), `service-kind` (`service`, `sid`), `tx-mode` (`auto`, `manual`), `session` (`dedicated`, `single`), `level` (`auto`, `names`, `structure`, `full`), `ssl-mode` (`prefer`, `require`, `verify-ca`, `verify-full`). Check boxes (`true`, `false`): `read-only`, `keep-alive`, `auto-disconnect`, `auto-sync`, `use-ssl`, `show-system`, `all-schemas`, `current-schema`, `schema:<name>`.",
        r#"{"field": "host", "value": "localhost"}"#;
    /// Open a list.
    DialogOpenList { list: String } => "db.data-sources.open-list", "Open List",
        "Opens a list of the dialog: `driver`, `auth`, `save`, `service-kind`, `tx-mode`, `session`, `level`, `ssl-mode`, or `add` (the menu of the \"+\" button).",
        r#"{"list": "save"}"#;
    /// Select a data source.
    DialogSelect { name: String } => "db.data-sources.select", "Select Data Source",
        "Selects the data source `name` in the dialog's list.",
        r#"{"name": "pyoxide-local"}"#;
    /// Show a tab.
    DialogTab { tab: String } => "db.data-sources.tab", "Select Tab",
        "Shows a tab of the selected data source: `general`, `options`, `ssh-ssl`, `schemas`.",
        r#"{"tab": "options"}"#;
    /// Add a data source.
    DialogAdd { backend: String } => "db.data-sources.add", "Add Data Source",
        "Adds a data source of `backend` (`postgres`, `oracle-thin`, `oracle-odpi`, `sqlite`) to the dialog's list and selects it.",
        r#"{"backend": "postgres"}"#;
    /// Remove the selected data source.
    DialogRemove => "db.data-sources.remove", "Remove",
        "Removes the selected data source from the list; Apply or OK deletes it with its kept passwords.";
    /// Duplicate the selected data source.
    DialogDuplicate => "db.data-sources.duplicate", "Duplicate",
        "Adds a copy of the selected data source (without its password).";
    /// Test the connection.
    DialogTest => "db.data-sources.test-connection", "Test Connection",
        "Connects to the selected data source as edited (with the typed password, else the kept one; asks for it when it is not kept) and shows the server's version or the error. Writes nothing. Again while it runs: cancels it.";
    /// Show the result of the last test.
    DialogShowResult => "db.data-sources.show-result", "Show Test Result",
        "Shows the details of the last Test Connection again.";
    /// Copy the result of the last test.
    DialogCopyResult => "db.data-sources.copy-result", "Copy Test Result",
        "Copies the details of the last Test Connection.";
    /// Close the failure message, focus the password.
    DialogChangeCredentials => "db.data-sources.change-credentials", "Change Credentials",
        "Closes the message of a refused login and puts the focus into the password field.";
    /// Close a message.
    DialogDismiss => "db.data-sources.dismiss", "Close Message",
        "Closes the dialog's message, the test result or the error banner.";
    /// Read the schemas.
    DialogLoadSchemas => "db.data-sources.load-schemas", "Refresh Schemas",
        "Reads the schemas of the selected data source for the Schemas tab.";
    /// Add a comment.
    DialogAddComment => "db.data-sources.add-comment", "Add Comment",
        "Shows the comment field of the selected data source.";
    /// Apply.
    DialogApply => "db.data-sources.apply", "Apply",
        "Saves the data sources and their passwords; the dialog stays open.";
    /// OK.
    DialogOk => "db.data-sources.ok", "OK",
        "Saves the data sources and their passwords and closes the dialog.";
    /// Cancel.
    DialogCancel => "db.data-sources.cancel", "Cancel",
        "Closes the dialog without saving.";
}

/// Texts of the dialog: plain names of fields as the reference has them,
/// our own wording for messages.
pub const TITLE: &str = "Data Sources";
/// The password field's placeholder.
const HIDDEN: &str = "<hidden>";

/// How the dialog opens.
#[derive(Debug, Clone)]
pub enum Initial {
    /// With a new data source of the backend.
    New(Backend),
    /// With a new PostgreSQL data source edited as a URL.
    Url,
    /// At this data source.
    Select(String),
}

/// The tabs of a data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Options,
    SshSsl,
    Schemas,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::General, Tab::Options, Tab::SshSsl, Tab::Schemas];

    pub fn id(self) -> &'static str {
        match self {
            Tab::General => "general",
            Tab::Options => "options",
            Tab::SshSsl => "ssh-ssl",
            Tab::Schemas => "schemas",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Options => "Options",
            Tab::SshSsl => "SSH/SSL",
            Tab::Schemas => "Schemas",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// The text fields of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FieldId {
    Name,
    Comment,
    Host,
    Port,
    User,
    Database,
    Service,
    Dsn,
    Path,
    Url,
    TimeZone,
    KeepAlive,
    AutoDisconnect,
    StartupScript,
    RefreshMinutes,
    CaFile,
    ClientCert,
    ClientKey,
    WalletDir,
    SchemaPattern,
}

impl FieldId {
    const ALL: [FieldId; 20] = [
        FieldId::Name,
        FieldId::Comment,
        FieldId::Host,
        FieldId::Port,
        FieldId::User,
        FieldId::Database,
        FieldId::Service,
        FieldId::Dsn,
        FieldId::Path,
        FieldId::Url,
        FieldId::TimeZone,
        FieldId::KeepAlive,
        FieldId::AutoDisconnect,
        FieldId::StartupScript,
        FieldId::RefreshMinutes,
        FieldId::CaFile,
        FieldId::ClientCert,
        FieldId::ClientKey,
        FieldId::WalletDir,
        FieldId::SchemaPattern,
    ];

    fn id(self) -> &'static str {
        match self {
            FieldId::Name => "name",
            FieldId::Comment => "comment",
            FieldId::Host => "host",
            FieldId::Port => "port",
            FieldId::User => "user",
            FieldId::Database => "database",
            FieldId::Service => "service",
            FieldId::Dsn => "dsn",
            FieldId::Path => "path",
            FieldId::Url => "url",
            FieldId::TimeZone => "time-zone",
            FieldId::KeepAlive => "keep-alive-secs",
            FieldId::AutoDisconnect => "auto-disconnect-secs",
            FieldId::StartupScript => "startup-script",
            FieldId::RefreshMinutes => "refresh-minutes",
            FieldId::CaFile => "ca-file",
            FieldId::ClientCert => "client-cert",
            FieldId::ClientKey => "client-key",
            FieldId::WalletDir => "wallet-dir",
            FieldId::SchemaPattern => "schema-pattern",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    /// Paths and statements are edited in the editor font.
    fn ui_font(self) -> bool {
        !matches!(
            self,
            FieldId::StartupScript
                | FieldId::Path
                | FieldId::CaFile
                | FieldId::ClientCert
                | FieldId::ClientKey
                | FieldId::WalletDir
        )
    }
}

/// The lists of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ListId {
    Driver,
    Auth,
    Save,
    ServiceKind,
    TxMode,
    Session,
    Level,
    SslMode,
    Add,
}

impl ListId {
    const ALL: [ListId; 9] = [
        ListId::Driver,
        ListId::Auth,
        ListId::Save,
        ListId::ServiceKind,
        ListId::TxMode,
        ListId::Session,
        ListId::Level,
        ListId::SslMode,
        ListId::Add,
    ];

    fn id(self) -> &'static str {
        match self {
            ListId::Driver => "driver",
            ListId::Auth => "auth",
            ListId::Save => "save",
            ListId::ServiceKind => "service-kind",
            ListId::TxMode => "tx-mode",
            ListId::Session => "session",
            ListId::Level => "level",
            ListId::SslMode => "ssl-mode",
            ListId::Add => "add",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.id() == id)
    }
}

/// The values of a list: id and label.
pub(crate) fn choices(list: ListId) -> Vec<(&'static str, &'static str)> {
    match list {
        ListId::Driver => BACKENDS
            .iter()
            .map(|b| (b.id(), backend_title(*b)))
            .collect(),
        ListId::Auth => vec![("user-password", "User & Password"), ("none", "No auth")],
        ListId::Save => vec![
            ("never", "Never"),
            ("until-restart", "Until restart"),
            ("forever", "Forever"),
        ],
        ListId::ServiceKind => vec![("service", "Service name"), ("sid", "SID")],
        ListId::TxMode => vec![("auto", "Auto"), ("manual", "Manual")],
        ListId::Session => vec![
            ("dedicated", "Dedicated sessions"),
            ("single", "Single session"),
        ],
        ListId::Level => vec![
            ("auto", "Auto select"),
            ("names", "Names only"),
            ("structure", "Structure"),
            ("full", "Everything"),
        ],
        ListId::SslMode => vec![
            ("prefer", "Prefer"),
            ("require", "Require"),
            ("verify-ca", "Verify CA"),
            ("verify-full", "Full verification"),
        ],
        ListId::Add => BACKENDS
            .iter()
            .map(|b| (b.id(), backend_title(*b)))
            .collect(),
    }
}

/// What happens to the typed password of an item.
#[derive(Debug, Clone, Default)]
enum PasswordEdit {
    #[default]
    Keep,
    Set(Secret),
}

/// A data source of the dialog's list.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    /// Its name in the store; `None` for one added in the dialog.
    original: Option<String>,
    source: DataSource,
    password: PasswordEdit,
    /// The store keeps a password for it.
    has_kept: bool,
    /// The comment field shows.
    comment_shown: bool,
    /// The schemas as last read, for the Schemas tab.
    schemas: Option<Vec<SchemaInfo>>,
}

impl Item {
    fn profile(&self) -> &Profile {
        &self.source.profile
    }

    fn gui(&self) -> db_core::GuiOptions {
        self.source.profile.gui_options()
    }

    fn set_gui(&mut self, f: impl FnOnce(&mut db_core::GuiOptions)) {
        let mut gui = self.gui();
        f(&mut gui);
        self.source.profile.gui = (!gui.is_default()).then_some(gui);
    }
}

/// A test of the connection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TestState {
    Idle,
    Running,
    Ok(ServerInfo),
    Failed(Failure),
}

/// The message over the dialog after a failed test (`Connecting to …`).
#[derive(Debug, Clone)]
pub(crate) struct FailureMessage {
    pub name: String,
    pub failure: Failure,
}

/// The open dialog, for `db.state`.
#[derive(Default)]
struct OpenDialog(Option<WeakEntity<DataSourcesDialog>>);

impl Global for OpenDialog {}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// The open dialog's state (never a password), `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenDialog>()
        .and_then(|d| d.0.as_ref()?.upgrade())
        .map(|d| d.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

/// Opens the dialog over `window` (the main window of `root`), with the
/// project's data sources as last read.
pub(crate) fn open(
    root: PathBuf,
    sources: Vec<DataSource>,
    initial: Initial,
    window: &mut Window,
    cx: &mut App,
) {
    let options = ui::dialog_window::DialogOptions {
        title: view::WINDOW_TITLE.into(),
        size: gpui::size(
            px(view::WIDTH),
            px(view::HEIGHT + ui::dialog_window::title_height(true)),
        ),
        min_size: Some(gpui::size(px(view::MIN_SIZE.0), px(view::MIN_SIZE.1))),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let dialog = cx.new(|cx| DataSourcesDialog::new(root, sources, initial, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        cx.set_global(OpenDialog(Some(dialog.downgrade())));
    }
}

/// A failure as one line of text: the code, the message, the hint.
pub(crate) fn failure_text(f: &Failure) -> String {
    let mut text = match &f.code {
        Some(code) if !f.message.contains(code.as_str()) => format!("[{code}] {}", f.message),
        _ => f.message.clone(),
    };
    if let Some(hint) = &f.hint {
        text.push_str(&format!("\n{}", capitalize(hint)));
        if let Some(next) = &f.next {
            text.push_str(&format!(": {next}"));
        }
        text.push('.');
    }
    text
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A new data source of `backend` with the reference's defaults.
fn new_profile(backend: Backend) -> Profile {
    let mut p = Profile::new(backend);
    match backend.dialect() {
        Dialect::Postgres => {
            p.host = "localhost".into();
            p.port = backend.default_port();
            p.database = Some("postgres".into());
        }
        Dialect::Oracle => {
            p.host = "localhost".into();
            p.port = backend.default_port();
            p.service = Some("FREEPDB1".into());
        }
        Dialect::Sqlite => {
            p.gui = Some(db_core::GuiOptions {
                auth: db_core::profile::AuthMethod::None,
                ..Default::default()
            });
        }
    }
    p
}

/// `user@host`, `backend@host`, or the file name, made unique among
/// `taken` and valid as a profile name.
fn default_name(profile: &Profile, taken: &[String]) -> String {
    let base = match profile.dialect() {
        Dialect::Sqlite => profile
            .path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sqlite".into()),
        _ => {
            let who = match profile.backend {
                Backend::Postgres => "postgres",
                _ => "oracle",
            };
            let host = if profile.host.is_empty() {
                "localhost"
            } else {
                profile.host.as_str()
            };
            format!("{who}@{host}")
        }
    };
    let base: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|n| !taken.contains(n))
        .expect("a free name")
}

/// An open list or menu of the dialog.
struct OpenList {
    list: ListId,
    popup: MenuPopup,
}

pub struct DataSourcesDialog {
    root: PathBuf,
    items: Vec<Item>,
    selected: Option<usize>,
    removed: Vec<String>,
    tab: Tab,
    fields: Vec<(FieldId, Entity<Editor>)>,
    password: Entity<PasswordField>,
    /// The URL field's text does not describe a connection.
    url_error: Option<String>,
    /// The port field's text is not a port.
    port_error: bool,
    /// The last error of Apply or OK.
    save_error: Option<String>,
    test: TestState,
    /// The details of the last test show (the popup above Test
    /// Connection).
    result_shown: bool,
    failure: Option<FailureMessage>,
    /// An error reading the schemas, shown above Test Connection.
    banner: Option<String>,
    schemas_loading: bool,
    list: Option<OpenList>,
    /// Where each combo is, for its list.
    anchors: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<ListId, Bounds<Pixels>>>>,
    focus: FocusHandle,
    filling: bool,
    test_task: Task<()>,
    schema_task: Task<()>,
    save_task: Task<()>,
    kept_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for DataSourcesDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DataSourcesDialog {
    fn new(
        root: PathBuf,
        sources: Vec<DataSource>,
        initial: Initial,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let fields: Vec<(FieldId, Entity<Editor>)> = FieldId::ALL
            .into_iter()
            .map(|id| {
                let editor = editor::field::new("", None, id.ui_font(), window, cx);
                subscriptions.push(cx.subscribe(
                    &editor,
                    move |this, _, e: &editor::EditorEvent, cx| {
                        if matches!(e, editor::EditorEvent::Edited(_)) {
                            this.field_edited(id, cx);
                        }
                    },
                ));
                (id, editor)
            })
            .collect();
        let password = cx.new(|cx| PasswordField::new("data-source-password", cx));
        subscriptions.push(cx.subscribe(&password, |this, _, _: &PasswordEdited, cx| {
            this.password_edited(cx)
        }));
        let items = sources
            .into_iter()
            .map(|source| Item {
                original: Some(source.name.clone()),
                has_kept: false,
                source,
                password: PasswordEdit::Keep,
                comment_shown: false,
                schemas: None,
            })
            .collect();
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut this = Self {
            root,
            items,
            selected: None,
            removed: Vec::new(),
            tab: Tab::General,
            fields,
            password,
            url_error: None,
            port_error: false,
            save_error: None,
            test: TestState::Idle,
            result_shown: false,
            failure: None,
            banner: None,
            schemas_loading: false,
            list: None,
            anchors: Default::default(),
            focus,
            filling: false,
            test_task: Task::ready(()),
            schema_task: Task::ready(()),
            save_task: Task::ready(()),
            kept_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        match initial {
            Initial::New(backend) => this.add(backend, window, cx),
            Initial::Url => {
                this.add(Backend::Postgres, window, cx);
                this.set_choice(ListId::Driver, "postgres", cx);
                if let Some(item) = this.item_mut() {
                    item.set_gui(|g| g.url_only = true);
                }
                this.fill(cx);
            }
            Initial::Select(name) => {
                let at = this.items.iter().position(|i| i.source.name == name);
                this.select(at.or(Some(0)).filter(|_| !this.items.is_empty()), cx);
            }
        }
        this.read_kept_passwords(cx);
        this
    }

    /// Which data sources have a kept password (the credential store may
    /// block: read off the UI thread).
    fn read_kept_passwords(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .items
            .iter()
            .filter_map(|i| i.original.clone())
            .collect();
        let store = crate::store(cx);
        let root = self.root.clone();
        let read = cx.background_spawn(async move {
            names
                .into_iter()
                .filter(|n| store.has_password(&root, n))
                .collect::<Vec<_>>()
        });
        self.kept_task = cx.spawn(async move |this, cx| {
            let kept = read.await;
            let _ = this.update(cx, |this, cx| {
                for item in &mut this.items {
                    item.has_kept = item.original.as_ref().is_some_and(|o| kept.contains(o));
                }
                cx.notify();
            });
        });
    }

    // ---- the selected item ------------------------------------------------------

    pub(crate) fn item(&self) -> Option<&Item> {
        self.items.get(self.selected?)
    }

    fn item_mut(&mut self) -> Option<&mut Item> {
        self.items.get_mut(self.selected?)
    }

    pub(crate) fn editor(&self, id: FieldId) -> &Entity<Editor> {
        &self
            .fields
            .iter()
            .find(|(f, _)| *f == id)
            .expect("every field has an editor")
            .1
    }

    fn text(&self, id: FieldId, cx: &App) -> String {
        self.editor(id).read(cx).text()
    }

    fn set_text(&self, id: FieldId, text: &str, cx: &mut App) {
        self.editor(id).update(cx, |e, cx| {
            e.set_text_selected(text, cx);
            let end = e.text().chars().count();
            e.set_selections(text::SelectionSet::caret(end), cx);
        });
    }

    /// Fills the fields from the selected item.
    fn fill(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.item().cloned() else {
            return;
        };
        self.filling = true;
        let p = item.profile();
        let gui = item.gui();
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        let num = |v: Option<u64>, default: u64| v.unwrap_or(default).to_string();
        let tls = p.tls.clone().unwrap_or_default();
        let scope = gui.scope.clone().unwrap_or_else(Scope::current);
        let pattern = match &scope.filter {
            SchemaFilter::Patterns(p) => p.join(", "),
            _ => String::new(),
        };
        let texts = [
            (FieldId::Name, item.source.name.clone()),
            (FieldId::Comment, opt(&gui.comment)),
            (FieldId::Host, p.host.clone()),
            (
                FieldId::Port,
                p.port.map(|p| p.to_string()).unwrap_or_default(),
            ),
            (FieldId::User, p.user.clone()),
            (FieldId::Database, opt(&p.database)),
            (
                FieldId::Service,
                p.sid
                    .clone()
                    .or_else(|| p.service.clone())
                    .unwrap_or_default(),
            ),
            (FieldId::Dsn, opt(&p.dsn)),
            (FieldId::Path, opt(&p.path)),
            (FieldId::Url, crate::url::format(p)),
            (FieldId::TimeZone, opt(&gui.time_zone)),
            (
                FieldId::KeepAlive,
                num(gui.keep_alive_secs, DEFAULT_KEEP_ALIVE_SECS),
            ),
            (
                FieldId::AutoDisconnect,
                num(gui.auto_disconnect_secs, DEFAULT_AUTO_DISCONNECT_SECS),
            ),
            (FieldId::StartupScript, opt(&gui.startup_script)),
            (
                FieldId::RefreshMinutes,
                gui.refresh_minutes
                    .map(|m| m.to_string())
                    .unwrap_or_default(),
            ),
            (FieldId::CaFile, opt(&tls.ca_file)),
            (FieldId::ClientCert, opt(&tls.client_cert)),
            (FieldId::ClientKey, opt(&tls.client_key)),
            (FieldId::WalletDir, opt(&tls.wallet_dir)),
            (FieldId::SchemaPattern, pattern),
        ];
        for (id, text) in texts {
            self.set_text(id, &text, cx);
        }
        // As the reference: `<hidden>` whether a password is kept or not
        // (what is kept is not shown).
        let placeholder = HIDDEN;
        let typed = match &item.password {
            PasswordEdit::Set(secret) => Some(secret.clone()),
            PasswordEdit::Keep => None,
        };
        self.password.update(cx, |f, cx| {
            f.reset(placeholder, cx);
            if let Some(secret) = typed {
                f.set(secret.expose(), cx);
            }
        });
        self.url_error = None;
        self.port_error = false;
        self.filling = false;
        cx.notify();
    }

    fn select(&mut self, at: Option<usize>, cx: &mut Context<Self>) {
        self.close_list(cx);
        self.selected = at;
        self.test = TestState::Idle;
        self.result_shown = false;
        self.failure = None;
        self.banner = None;
        self.test_task = Task::ready(());
        self.fill(cx);
        if self.tab == Tab::Schemas {
            self.ensure_schemas(cx);
        }
        cx.notify();
    }

    fn taken_names(&self, except: Option<usize>) -> Vec<String> {
        self.items
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != except)
            .map(|(_, i)| i.source.name.clone())
            .collect()
    }

    fn add(&mut self, backend: Backend, _: &mut Window, cx: &mut Context<Self>) {
        let profile = new_profile(backend);
        let name = default_name(&profile, &self.taken_names(None));
        self.items.push(Item {
            original: None,
            source: DataSource { name, profile },
            password: PasswordEdit::Keep,
            has_kept: false,
            comment_shown: false,
            schemas: None,
        });
        self.tab = Tab::General;
        self.select(Some(self.items.len() - 1), cx);
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.selected else {
            return;
        };
        let item = self.items.remove(at);
        if let Some(original) = item.original {
            self.removed.push(original);
        }
        let next = if self.items.is_empty() {
            None
        } else {
            Some(at.min(self.items.len() - 1))
        };
        self.select(next, cx);
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.item().cloned() else {
            return;
        };
        let name = default_name(&item.source.profile, &self.taken_names(None));
        self.items.push(Item {
            original: None,
            source: DataSource {
                name,
                profile: item.source.profile,
            },
            password: PasswordEdit::Keep,
            has_kept: false,
            comment_shown: item.comment_shown,
            schemas: None,
        });
        self.select(Some(self.items.len() - 1), cx);
    }

    // ---- editing ----------------------------------------------------------------

    fn field_edited(&mut self, id: FieldId, cx: &mut Context<Self>) {
        if self.filling {
            return;
        }
        let text = self.text(id, cx);
        let trimmed = text.trim().to_owned();
        let some = |t: &str| (!t.is_empty()).then(|| t.to_owned());
        let num = |t: &str| t.parse::<u64>().ok();
        let Some(item) = self.item_mut() else {
            return;
        };
        let mut connection_changed = true;
        match id {
            FieldId::Name => {
                item.source.name = trimmed;
                connection_changed = false;
            }
            FieldId::Comment => {
                item.set_gui(|g| g.comment = some(&text));
                connection_changed = false;
            }
            FieldId::Host => item.source.profile.host = trimmed,
            FieldId::Port => {
                let port = trimmed.parse::<u16>().ok();
                item.source.profile.port = port;
                self.port_error = !trimmed.is_empty() && port.is_none();
            }
            FieldId::User => item.source.profile.user = trimmed,
            FieldId::Database => item.source.profile.database = some(&trimmed),
            FieldId::Service => {
                let p = &mut item.source.profile;
                if p.sid.is_some() {
                    p.sid = Some(trimmed);
                } else {
                    p.service = some(&trimmed);
                }
            }
            FieldId::Dsn => item.source.profile.dsn = some(&trimmed),
            FieldId::Path => item.source.profile.path = some(&trimmed),
            FieldId::Url => {
                match crate::url::apply(&mut item.source.profile, &text) {
                    Ok(()) => {
                        self.url_error = None;
                        self.fill_connection_fields(cx);
                    }
                    Err(e) => self.url_error = Some(e),
                }
                cx.notify();
                return;
            }
            FieldId::TimeZone => item.set_gui(|g| g.time_zone = some(&trimmed)),
            FieldId::KeepAlive => {
                let on = item.gui().keep_alive_secs.is_some();
                if on {
                    item.set_gui(|g| g.keep_alive_secs = num(&trimmed));
                }
            }
            FieldId::AutoDisconnect => {
                let on = item.gui().auto_disconnect_secs.is_some();
                if on {
                    item.set_gui(|g| g.auto_disconnect_secs = num(&trimmed));
                }
            }
            FieldId::StartupScript => item.set_gui(|g| g.startup_script = some(&text)),
            FieldId::RefreshMinutes => item.set_gui(|g| g.refresh_minutes = num(&trimmed)),
            FieldId::CaFile => tls_mut(item).ca_file = some(&trimmed),
            FieldId::ClientCert => tls_mut(item).client_cert = some(&trimmed),
            FieldId::ClientKey => tls_mut(item).client_key = some(&trimmed),
            FieldId::WalletDir => tls_mut(item).wallet_dir = some(&trimmed),
            FieldId::SchemaPattern => {
                self.update_scope(cx);
                return;
            }
        }
        if connection_changed {
            let url = self.item().map(|i| crate::url::format(i.profile()));
            if let Some(url) = url {
                self.filling = true;
                self.set_text(FieldId::Url, &url, cx);
                self.filling = false;
                self.url_error = None;
            }
        }
        cx.notify();
    }

    /// The connection fields again after the URL set them.
    fn fill_connection_fields(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.item().map(|i| i.profile().clone()) else {
            return;
        };
        self.filling = true;
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        for (id, text) in [
            (FieldId::Host, p.host.clone()),
            (
                FieldId::Port,
                p.port.map(|p| p.to_string()).unwrap_or_default(),
            ),
            (FieldId::User, p.user.clone()),
            (FieldId::Database, opt(&p.database)),
            (
                FieldId::Service,
                p.sid
                    .clone()
                    .or_else(|| p.service.clone())
                    .unwrap_or_default(),
            ),
            (FieldId::Dsn, opt(&p.dsn)),
            (FieldId::Path, opt(&p.path)),
        ] {
            self.set_text(id, &text, cx);
        }
        self.filling = false;
    }

    fn password_edited(&mut self, cx: &mut Context<Self>) {
        let secret = self.password.read(cx).secret();
        if let (Some(item), Some(secret)) = (self.item_mut(), secret) {
            item.password = PasswordEdit::Set(secret);
        }
        cx.notify();
    }

    /// The introspection scope from the Schemas tab's check boxes and
    /// pattern.
    fn update_scope(&mut self, cx: &mut Context<Self>) {
        let pattern = self.text(FieldId::SchemaPattern, cx);
        let Some(item) = self.item_mut() else {
            return;
        };
        let old = item.gui().scope.unwrap_or_else(Scope::current);
        let patterns: Vec<String> = pattern
            .split(',')
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();
        let filter = if !patterns.is_empty() {
            SchemaFilter::Patterns(patterns)
        } else if matches!(old.filter, SchemaFilter::Patterns(_)) {
            SchemaFilter::Current
        } else {
            old.filter.clone()
        };
        let scope = Scope {
            filter,
            system: old.system,
        };
        item.set_gui(|g| g.scope = (scope != Scope::current()).then_some(scope));
        cx.notify();
    }

    /// Sets a check box of the Schemas tab.
    fn set_schema_check(&mut self, what: &str, on: bool, cx: &mut Context<Self>) {
        let Some(item) = self.item_mut() else {
            return;
        };
        let current_name = item
            .schemas
            .as_ref()
            .and_then(|s| s.iter().find(|s| s.is_current))
            .map(|s| s.name.clone());
        let mut scope = item.gui().scope.unwrap_or_else(Scope::current);
        match what {
            "all-schemas" => {
                scope.filter = if on {
                    SchemaFilter::All
                } else {
                    SchemaFilter::Current
                };
            }
            "current-schema" => {
                scope.filter = match (&scope.filter, on) {
                    (SchemaFilter::List(names), true) if !names.is_empty() => {
                        let mut names = names.clone();
                        if let Some(c) = &current_name
                            && !names.contains(c)
                        {
                            names.push(c.clone());
                        }
                        SchemaFilter::List(names)
                    }
                    (_, true) => SchemaFilter::Current,
                    (SchemaFilter::Current, false) => SchemaFilter::List(Vec::new()),
                    (other, false) => other.clone(),
                };
            }
            "show-system" => scope.system = on,
            name => {
                let mut names = match &scope.filter {
                    SchemaFilter::List(names) => names.clone(),
                    SchemaFilter::Current => current_name.iter().cloned().collect(),
                    _ => Vec::new(),
                };
                names.retain(|n| n != name);
                if on {
                    names.push(name.to_owned());
                }
                names.sort();
                scope.filter = if names.len() == 1 && current_name.as_deref() == Some(&names[0]) {
                    SchemaFilter::Current
                } else {
                    SchemaFilter::List(names)
                };
            }
        }
        item.set_gui(|g| g.scope = (scope != Scope::current()).then_some(scope));
        cx.notify();
    }

    /// Sets a choice of a list.
    fn set_choice(&mut self, list: ListId, value: &str, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.item_mut() else {
            return false;
        };
        match list {
            ListId::Driver => {
                let Some(backend) = Backend::from_id(value) else {
                    return false;
                };
                let old = item.source.profile.backend;
                if old.dialect() != backend.dialect() {
                    let mut fresh = new_profile(backend);
                    fresh.user = item.source.profile.user.clone();
                    fresh.read_only = item.source.profile.read_only;
                    item.source.profile = fresh;
                } else {
                    item.source.profile.backend = backend;
                    if backend != Backend::OracleOdpi && old == Backend::OracleOdpi {
                        item.source.profile.dsn = None;
                    }
                }
                self.fill(cx);
            }
            ListId::Auth => {
                let auth = match value {
                    "none" => db_core::profile::AuthMethod::None,
                    "user-password" => db_core::profile::AuthMethod::UserPassword,
                    _ => return false,
                };
                item.set_gui(|g| g.auth = auth);
            }
            ListId::Save => {
                let Some(save) = PasswordSave::from_id(value) else {
                    return false;
                };
                item.set_gui(|g| g.password_save = save);
            }
            ListId::ServiceKind => {
                let p = &mut item.source.profile;
                let value_now = p.sid.take().or_else(|| p.service.take());
                match value {
                    "sid" => p.sid = Some(value_now.unwrap_or_default()),
                    "service" => p.service = value_now.filter(|v| !v.is_empty()),
                    _ => return false,
                }
                let url = crate::url::format(p);
                self.filling = true;
                self.set_text(FieldId::Url, &url, cx);
                self.filling = false;
            }
            ListId::TxMode => {
                let mode = match value {
                    "auto" => None,
                    "manual" => Some(TxMode::Manual),
                    _ => return false,
                };
                item.set_gui(|g| g.tx_mode = mode);
            }
            ListId::Session => match value {
                "dedicated" | "single" => {
                    let single = value == "single";
                    item.set_gui(|g| g.single_session = single);
                }
                _ => return false,
            },
            ListId::Level => {
                let level = match value {
                    "auto" => None,
                    "names" => Some(Level::Names),
                    "structure" => Some(Level::Structure),
                    "full" => Some(Level::Full),
                    _ => return false,
                };
                item.set_gui(|g| g.level = level);
            }
            ListId::SslMode => {
                let mode = match value {
                    "prefer" => TlsMode::Prefer,
                    "require" => TlsMode::Require,
                    "verify-ca" => TlsMode::VerifyCa,
                    "verify-full" => TlsMode::VerifyFull,
                    _ => return false,
                };
                tls_mut(item).mode = mode;
            }
            ListId::Add => {
                let Some(backend) = Backend::from_id(value) else {
                    return false;
                };
                self.close_list(cx);
                self.items.push(Item {
                    original: None,
                    source: DataSource {
                        name: String::new(),
                        profile: new_profile(backend),
                    },
                    password: PasswordEdit::Keep,
                    has_kept: false,
                    comment_shown: false,
                    schemas: None,
                });
                let at = self.items.len() - 1;
                let name =
                    default_name(&self.items[at].source.profile, &self.taken_names(Some(at)));
                self.items[at].source.name = name;
                self.tab = Tab::General;
                self.select(Some(at), cx);
            }
        }
        cx.notify();
        true
    }

    /// Sets a check box.
    fn set_check(&mut self, field: &str, on: bool, cx: &mut Context<Self>) -> bool {
        if field.starts_with("schema:")
            || matches!(field, "all-schemas" | "current-schema" | "show-system")
        {
            let what = field.strip_prefix("schema:").unwrap_or(field).to_owned();
            self.set_schema_check(&what, on, cx);
            return true;
        }
        let keep_alive = self.text(FieldId::KeepAlive, cx).trim().parse::<u64>().ok();
        let auto_disconnect = self
            .text(FieldId::AutoDisconnect, cx)
            .trim()
            .parse::<u64>()
            .ok();
        let Some(item) = self.item_mut() else {
            return false;
        };
        match field {
            "read-only" => item.source.profile.read_only = on,
            "keep-alive" => item.set_gui(|g| {
                g.keep_alive_secs = on.then(|| keep_alive.unwrap_or(DEFAULT_KEEP_ALIVE_SECS))
            }),
            "auto-disconnect" => item.set_gui(|g| {
                g.auto_disconnect_secs =
                    on.then(|| auto_disconnect.unwrap_or(DEFAULT_AUTO_DISCONNECT_SECS))
            }),
            "auto-sync" => item.set_gui(|g| g.no_auto_sync = !on),
            "use-ssl" => {
                if on {
                    let tls = tls_mut(item);
                    if tls.mode == TlsMode::Disable {
                        tls.mode = TlsMode::Require;
                    }
                } else if let Some(tls) = &mut item.source.profile.tls {
                    tls.mode = TlsMode::Disable;
                    if *tls == TlsProfile::default() {
                        item.source.profile.tls = None;
                    }
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn set_connection_type(&mut self, value: &str, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.item_mut() else {
            return false;
        };
        match value {
            "default" => {
                item.set_gui(|g| g.url_only = false);
                if item.source.profile.dsn.is_some() {
                    item.source.profile.dsn = None;
                    if item.source.profile.service.is_none() && item.source.profile.sid.is_none() {
                        item.source.profile.service = Some(String::new());
                    }
                }
            }
            "url-only" => item.set_gui(|g| g.url_only = true),
            "tns" if item.source.profile.dialect() == Dialect::Oracle => {
                item.set_gui(|g| g.url_only = false);
                let p = &mut item.source.profile;
                if p.dsn.is_none() {
                    p.dsn = Some(String::new());
                }
                p.service = None;
                p.sid = None;
            }
            _ => return false,
        }
        self.fill(cx);
        true
    }

    fn on_set(&mut self, a: &DialogSet, window: &mut Window, cx: &mut Context<Self>) {
        let (field, value) = (a.field.as_str(), a.value.as_str());
        if field == "password" {
            self.password.update(cx, |f, cx| f.set(value, cx));
            return;
        }
        if field == "connection-type" {
            self.set_connection_type(value, cx);
            return;
        }
        if field == "tab" {
            if let Some(tab) = Tab::from_id(value) {
                self.show_tab(tab, cx);
            }
            return;
        }
        if let Some(id) = FieldId::from_id(field) {
            self.set_text(id, value, cx);
            // Filled as typed: the field's own edit event updates the item.
            self.field_edited(id, cx);
            return;
        }
        if let Some(list) = ListId::from_id(field) {
            self.set_choice(list, value, cx);
            return;
        }
        let on = match value {
            "true" => true,
            "false" => false,
            _ => return,
        };
        self.set_check(field, on, cx);
        let _ = window;
    }

    // ---- tabs and schemas ---------------------------------------------------------

    fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.result_shown = false;
        if tab == Tab::Schemas {
            self.ensure_schemas(cx);
        }
        cx.notify();
    }

    fn ensure_schemas(&mut self, cx: &mut Context<Self>) {
        if self.item().is_some_and(|i| i.schemas.is_none()) && !self.schemas_loading {
            self.load_schemas(cx);
        }
    }

    fn load_schemas(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.item().cloned() else {
            return;
        };
        if item.profile().validate().is_err() {
            return;
        }
        let at = self.selected;
        let password = self.password_to_try(&item);
        let read = crate::store(cx).schemas(&self.root, item.source.clone(), password);
        let run = cx.background_spawn(read);
        self.schemas_loading = true;
        self.banner = None;
        self.schema_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                this.schemas_loading = false;
                if this.selected != at {
                    return;
                }
                match result {
                    Ok(schemas) => {
                        if let Some(item) = this.item_mut() {
                            item.schemas = Some(schemas);
                        }
                    }
                    Err(f) => {
                        if let Some(item) = this.item_mut() {
                            item.schemas = Some(Vec::new());
                        }
                        this.banner = Some(failure_text(&f).replace('\n', " "));
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    // ---- testing ----------------------------------------------------------------

    /// The password a connection of `item` uses: the typed one, else the
    /// kept one (`None`).
    fn password_to_try(&self, item: &Item) -> Option<Secret> {
        match &item.password {
            PasswordEdit::Set(secret) => Some(secret.clone()),
            PasswordEdit::Keep => None,
        }
    }

    fn start_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.test == TestState::Running {
            // Again while it runs: cancel.
            self.test_task = Task::ready(());
            self.test = TestState::Idle;
            cx.notify();
            return;
        }
        let Some(item) = self.item().cloned() else {
            return;
        };
        if let Err(e) = item.profile().validate() {
            self.test = TestState::Failed(Failure {
                message: e.message,
                ..Failure::default()
            });
            self.result_shown = true;
            cx.notify();
            return;
        }
        let password = self.password_to_try(&item);
        let asks = password.is_none()
            && !item.has_kept
            && item.profile().needs_password()
            && item.gui().password_save == PasswordSave::Never;
        if asks {
            let this = cx.entity().downgrade();
            let (name, user) = (item.source.name.clone(), item.profile().user.clone());
            crate::prompt::ask(&name, &user, window, cx, move |secret, cx| {
                if let (Some(secret), Some(this)) = (secret, this.upgrade()) {
                    this.update(cx, |this, cx| this.run_test(item, Some(secret), cx));
                }
            });
            return;
        }
        self.run_test(item, password, cx);
    }

    fn run_test(&mut self, item: Item, password: Option<Secret>, cx: &mut Context<Self>) {
        let at = self.selected;
        let name = item.source.name.clone();
        let test = crate::store(cx).test(&self.root, item.source, password);
        let run = cx.background_spawn(test);
        self.test = TestState::Running;
        self.result_shown = false;
        self.failure = None;
        self.test_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                if this.selected != at {
                    return;
                }
                match result {
                    Ok(info) => {
                        this.test = TestState::Ok(info);
                        this.result_shown = true;
                    }
                    Err(failure) => {
                        this.failure = Some(FailureMessage {
                            name,
                            failure: failure.clone(),
                        });
                        this.test = TestState::Failed(failure);
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// The details of the last test, as copied.
    pub(crate) fn result_text(&self) -> Option<String> {
        let tls = self
            .item()
            .map(|i| i.profile().tls_mode())
            .unwrap_or_default();
        match &self.test {
            TestState::Ok(info) => Some(format!(
                "Server: {} {}\nDriver: {}\nConnected in {:.0} ms, ping {:.0} ms\nTLS: {}",
                info.product,
                info.version,
                info.driver,
                info.connect_ms,
                info.ping_ms,
                tls_text(tls)
            )),
            TestState::Failed(f) => Some(failure_text(f)),
            _ => None,
        }
    }

    fn copy_result(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.result_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    // ---- lists ------------------------------------------------------------------

    pub(crate) fn current_choice(&self, list: ListId) -> Option<&'static str> {
        let item = self.item()?;
        let gui = item.gui();
        let p = item.profile();
        Some(match list {
            ListId::Driver => p.backend.id(),
            ListId::Auth => match gui.auth {
                db_core::profile::AuthMethod::UserPassword => "user-password",
                db_core::profile::AuthMethod::None => "none",
            },
            ListId::Save => gui.password_save.id(),
            ListId::ServiceKind => {
                if p.sid.is_some() {
                    "sid"
                } else {
                    "service"
                }
            }
            ListId::TxMode => match gui.tx_mode {
                Some(TxMode::Manual) => "manual",
                _ => "auto",
            },
            ListId::Session => {
                if gui.single_session {
                    "single"
                } else {
                    "dedicated"
                }
            }
            ListId::Level => match gui.level {
                None => "auto",
                Some(Level::Names) => "names",
                Some(Level::Structure) => "structure",
                Some(Level::Full) => "full",
            },
            ListId::SslMode => match p.tls_mode() {
                TlsMode::Disable | TlsMode::Require => "require",
                TlsMode::Prefer => "prefer",
                TlsMode::VerifyCa => "verify-ca",
                TlsMode::VerifyFull => "verify-full",
            },
            ListId::Add => return None,
        })
    }

    pub(crate) fn choice_label(&self, list: ListId) -> &'static str {
        let current = self.current_choice(list);
        choices(list)
            .into_iter()
            .find(|(id, _)| Some(*id) == current)
            .map(|(_, label)| label)
            .unwrap_or("")
    }

    fn open_list(&mut self, list: ListId, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.as_ref().is_some_and(|l| l.list == list) {
            self.close_list(cx);
            return;
        }
        self.close_list(cx);
        let current = self.current_choice(list);
        let items: Vec<MenuItem> = choices(list)
            .into_iter()
            .map(|(id, label)| {
                let item = MenuItem::action(
                    label,
                    DialogSet {
                        field: list.id().to_owned(),
                        value: id.to_owned(),
                    },
                );
                let item = match list {
                    ListId::Driver | ListId::Add => {
                        item.icon(icons::backend(Backend::from_id(id).expect("a backend")))
                    }
                    _ => item,
                };
                item.bold(Some(id) == current && list != ListId::Add)
            })
            .collect();
        let bounds = self.anchors.borrow().get(&list).copied();
        let position = match bounds {
            Some(b) => gpui::point(b.left(), b.bottom() + px(2.0)),
            None => gpui::point(px(40.0), px(80.0)),
        };
        let activate = !workbench::env::background();
        let popup = MenuPopup::open(
            items,
            None,
            position,
            &self.focus,
            activate,
            window,
            cx,
            |this, cx| this.close_list(cx),
        );
        if let Some(popup) = popup {
            self.list = Some(OpenList { list, popup });
        }
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.list.take() {
            open.popup.close(cx);
        }
        cx.notify();
    }

    // ---- saving -----------------------------------------------------------------

    /// What is wrong with the list, if anything: the first item's problem.
    fn problem(&self) -> Option<(usize, String)> {
        for (i, item) in self.items.iter().enumerate() {
            let name = &item.source.name;
            if let Err(e) = db_core::profile::validate_name(name) {
                return Some((i, e.message));
            }
            if self.items.iter().filter(|o| &o.source.name == name).count() > 1 {
                return Some((
                    i,
                    format!("Another data source is named \u{201c}{name}\u{201d}"),
                ));
            }
            if let Err(e) = item.profile().validate() {
                return Some((i, format!("{name}: {}", e.message)));
            }
        }
        None
    }

    fn request(&self) -> SaveRequest {
        let mut request = SaveRequest {
            sources: self.items.iter().map(|i| i.source.clone()).collect(),
            removed: self.removed.clone(),
            ..SaveRequest::default()
        };
        for item in &self.items {
            let name = item.source.name.clone();
            if let Some(original) = &item.original
                && original != &name
            {
                request.renamed.push((original.clone(), name.clone()));
            }
            let never = item.gui().password_save == PasswordSave::Never;
            let change = match &item.password {
                PasswordEdit::Set(secret) if never || secret.is_empty() => PasswordChange::Clear,
                PasswordEdit::Set(secret) => PasswordChange::Set(secret.clone()),
                PasswordEdit::Keep if never && item.has_kept => PasswordChange::Clear,
                PasswordEdit::Keep => PasswordChange::Keep,
            };
            request.passwords.push((name, change));
        }
        request
    }

    fn save(&mut self, close: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        if let Some((at, problem)) = self.problem() {
            self.save_error = Some(problem);
            if self.selected != Some(at) {
                self.select(Some(at), cx);
            }
            cx.notify();
            return;
        }
        self.save_error = None;
        let request = self.request();
        let store = crate::store(cx);
        let root = self.root.clone();
        let write = cx.background_spawn(async move { store.save(&root, request) });
        let handle = window.window_handle();
        self.save_task = cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.saved(cx);
                        crate::explorer::reload(&this.root.clone(), cx);
                        if close {
                            this.test_task = Task::ready(());
                            this.forget(cx);
                            ui::dialog_window::close(handle, cx);
                        }
                    }
                    Err(e) => this.save_error = Some(e),
                }
                cx.notify();
            });
        });
    }

    /// After a save: the store's names and passwords are the dialog's.
    fn saved(&mut self, cx: &mut Context<Self>) {
        for item in &mut self.items {
            item.original = Some(item.source.name.clone());
            item.password = PasswordEdit::Keep;
        }
        self.removed.clear();
        self.fill(cx);
        self.read_kept_passwords(cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        self.test_task = Task::ready(());
        self.schema_task = Task::ready(());
        self.forget(cx);
        // From its own handlers the window is being updated: it removes
        // itself.
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    /// No longer the open dialog of `db.state`.
    fn forget(&self, cx: &mut Context<Self>) {
        if cx
            .try_global::<OpenDialog>()
            .and_then(|d| d.0.as_ref())
            .is_some_and(|d| d.entity_id() == cx.entity_id())
        {
            cx.set_global(OpenDialog(None));
        }
    }

    // ---- state ------------------------------------------------------------------

    pub fn state_json(&self, cx: &App) -> Json {
        let item = self.item().map(|item| {
            let p = item.profile();
            json!({
                "name": item.source.name,
                "new": item.original.is_none(),
                "backend": p.backend.id(),
                "profile": serde_json::to_value(p).unwrap_or(Json::Null),
                "url": self.text(FieldId::Url, cx),
                "url_error": self.url_error,
                "port_error": self.port_error,
                "password": self.password.read(cx).state_json(),
                "has_kept_password": item.has_kept,
                "comment_shown": item.comment_shown,
                "schemas": item.schemas.as_ref().map(|s| s.iter().map(|s| json!({
                    "name": s.name, "system": s.is_system, "current": s.is_current,
                })).collect::<Vec<_>>()),
            })
        });
        let test = match &self.test {
            TestState::Idle => json!({"state": "idle"}),
            TestState::Running => json!({"state": "running"}),
            TestState::Ok(info) => json!({
                "state": "ok", "product": info.product, "version": info.version,
                "driver": info.driver,
            }),
            TestState::Failed(f) => json!({
                "state": "failed", "message": f.message, "code": f.code,
                "hint": f.hint, "auth": f.auth,
            }),
        };
        json!({
            "items": self.items.iter().map(|i| i.source.name.clone()).collect::<Vec<_>>(),
            "selected": item.unwrap_or(Json::Null),
            "removed": self.removed,
            "tab": self.tab.id(),
            "test": test,
            "result_shown": self.result_shown,
            "result": self.result_text(),
            "failure": self.failure.as_ref().map(|f| json!({
                "name": f.name, "text": failure_text(&f.failure), "auth": f.failure.auth,
            })),
            "banner": self.banner,
            "schemas_loading": self.schemas_loading,
            "save_error": self.save_error,
            "problem": self.problem().map(|(_, p)| p),
            "list": self.list.as_ref().map(|l| json!({
                "list": l.list.id(),
                "menu": l.popup.menu.read(cx).state_json(),
            })),
        })
    }
}

fn tls_mut(item: &mut Item) -> &mut TlsProfile {
    item.source
        .profile
        .tls
        .get_or_insert_with(TlsProfile::default)
}

pub(crate) fn tls_text(mode: TlsMode) -> &'static str {
    match mode {
        TlsMode::Disable => "off",
        TlsMode::Prefer => "if the server offers it",
        TlsMode::Require => "required",
        TlsMode::VerifyCa => "required, certificate verified",
        TlsMode::VerifyFull => "required, certificate and host verified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_are_unique_and_valid() {
        let p = new_profile(Backend::Postgres);
        assert_eq!(default_name(&p, &[]), "postgres@localhost");
        let taken = vec!["postgres@localhost".to_owned()];
        assert_eq!(default_name(&p, &taken), "postgres@localhost-2");
        let mut s = new_profile(Backend::Sqlite);
        s.path = Some("/data/my file.db".into());
        assert_eq!(default_name(&s, &[]), "my_file");
        for name in ["postgres@localhost-2", "my_file"] {
            assert!(db_core::profile::validate_name(name).is_ok());
        }
    }

    #[test]
    fn failures_read_as_one_message_with_the_hint() {
        let f = Failure {
            message: "FATAL: password authentication failed for user \"pyoxide\"".into(),
            code: Some("28P01".into()),
            hint: Some("the server refused the user name or password".into()),
            next: Some("check them".into()),
            auth: true,
        };
        assert_eq!(
            failure_text(&f),
            "[28P01] FATAL: password authentication failed for user \"pyoxide\"\n\
             The server refused the user name or password: check them."
        );
    }

    #[test]
    fn sqlite_needs_no_password() {
        assert!(!new_profile(Backend::Sqlite).needs_password());
        assert!(new_profile(Backend::Postgres).needs_password());
    }
}
