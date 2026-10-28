//! The database module's interface (ADR 0021, 0022): the Database tool
//! window of the right stripe ([`explorer`]), the Data Sources dialog
//! ([`dialog`]) and the `db.explorer.*` commands, contributed to a product
//! by [`module`]. Data sources are kept by the product's
//! [`DataSourceStore`]: this crate never sees a driver, `db-service` or an
//! async runtime, and passwords only pass through it as [`db_core::Secret`].

pub mod console;
pub mod data_views;
pub mod dialog;
pub mod explorer;
pub mod fixture;
pub mod grid;
mod live;
mod password;
pub mod prompt;
pub mod query_settings;
mod settings_page;
pub mod sql;
pub mod store;
pub mod transfer;
pub mod tree;
pub mod url;

use std::sync::Arc;

use gpui::{App, Global};
use ui::icons::IconSvg;
use workbench::tool_windows::{Area, ToolWindowHook, ToolWindowSpec};
use workbench::workbench::ToolWindow;
use workbench::{Init, KeymapFiles, Module};

pub use store::{DataSource, DataSourceStore};

/// The tool window's id: its stripe button is `stripe-database`.
pub const TOOL_WINDOW: ToolWindow = ToolWindow("database");

/// The module's key bindings: its keymap folders, joined.
pub const KEYMAPS: KeymapFiles = KeymapFiles {
    macos: concat!(
        include_str!("../keymaps/macos/db.toml"),
        include_str!("../keymaps/macos/grid.toml"),
        include_str!("../keymaps/macos/console.toml"),
        include_str!("../keymaps/macos/values.toml")
    ),
    default: concat!(
        include_str!("../keymaps/default/db.toml"),
        include_str!("../keymaps/default/grid.toml"),
        include_str!("../keymaps/default/console.toml"),
        include_str!("../keymaps/default/values.toml")
    ),
};

/// Icons of the module (drawn for it; the reference's database plugin is
/// closed, so none of its icons are used).
pub mod icons {
    use ui::icons::IconName;

    pub const TOOL_WINDOW: IconName = IconName("db/database@20x20");
    pub const DATA_SOURCE: IconName = IconName("db/dataSource");
    pub const POSTGRES: IconName = IconName("db/postgres");
    pub const ORACLE: IconName = IconName("db/oracle");
    pub const ORACLE_CLIENT: IconName = IconName("db/oracleClient");
    pub const SQLITE: IconName = IconName("db/sqlite");
    pub const PROPERTIES: IconName = IconName("db/properties");
    pub const CONSOLE: IconName = IconName("db/console");
    pub const TABLE: IconName = IconName("db/table");
    pub const DISCONNECT: IconName = IconName("db/disconnect");
    pub const JUMP: IconName = IconName("db/jump");
    pub const IMPORT: IconName = IconName("db/import");
    pub const URL: IconName = IconName("db/url");
    pub const FILE: IconName = IconName("db/file");
    pub const DRIVER: IconName = IconName("db/driver");
    // Nodes of the object tree.
    pub const DATABASE_NODE: IconName = IconName("db/databaseNode");
    pub const SCHEMA: IconName = IconName("db/schema");
    pub const FOLDER: IconName = IconName("db/folder");
    pub const VIEW: IconName = IconName("db/view");
    pub const MATERIALIZED_VIEW: IconName = IconName("db/materializedView");
    pub const COLUMN: IconName = IconName("db/column");
    pub const COLUMN_KEY: IconName = IconName("db/columnKey");
    pub const COLUMN_NOT_NULL: IconName = IconName("db/columnNotNull");
    pub const KEY: IconName = IconName("db/key");
    pub const FOREIGN_KEY: IconName = IconName("db/foreignKey");
    pub const INDEX: IconName = IconName("db/index");
    pub const UNIQUE_INDEX: IconName = IconName("db/uniqueIndex");
    pub const CHECK: IconName = IconName("db/check");
    pub const TRIGGER: IconName = IconName("db/trigger");
    pub const ROUTINE: IconName = IconName("db/routine");
    pub const PROCEDURE: IconName = IconName("db/procedure");
    pub const SEQUENCE: IconName = IconName("db/sequence");
    pub const OBJECT_TYPE: IconName = IconName("db/objectType");
    pub const PACKAGE: IconName = IconName("db/package");
    pub const SYNONYM: IconName = IconName("db/synonym");
    pub const ERROR: IconName = IconName("db/error");
    // The data grid.
    pub const COLUMN_FOREIGN: IconName = IconName("db/columnForeign");
    pub const SORTABLE: IconName = IconName("db/sortable");
    pub const ORDER_BY: IconName = IconName("db/orderBy");
    pub const FIRST_PAGE: IconName = IconName("db/firstPage");
    pub const LAST_PAGE: IconName = IconName("db/lastPage");
    pub const STOP_DISABLED: IconName = IconName("db/stopDisabled");
    // The marks of an executed statement in the console's gutter.
    pub const STATEMENT_OK: IconName = IconName("db/statementOk");
    // The Services tool window.
    pub const SERVICES: IconName = IconName("db/services@20x20");
    pub const EXPORT_DATA: IconName = IconName("db/exportData");
    pub const IMPORT_DATA: IconName = IconName("db/importData");

    /// The icon of a backend's data sources.
    pub fn backend(backend: db_core::Backend) -> IconName {
        match backend {
            db_core::Backend::Postgres => POSTGRES,
            db_core::Backend::OracleThin => ORACLE,
            db_core::Backend::OracleOdpi => ORACLE_CLIENT,
            db_core::Backend::Sqlite => SQLITE,
        }
    }
}

macro_rules! icon {
    ($name:literal, $file:literal) => {
        IconSvg {
            name: concat!("db/", $name),
            light: include_bytes!(concat!("../assets/icons/", $file, ".svg")),
            dark: Some(include_bytes!(concat!(
                "../assets/icons/",
                $file,
                "_dark.svg"
            ))),
        }
    };
}

pub const ICONS: &[IconSvg] = &[
    icon!("database@20x20", "database@20x20"),
    icon!("dataSource", "dataSource"),
    icon!("postgres", "postgres"),
    icon!("oracle", "oracle"),
    icon!("oracleClient", "oracleClient"),
    icon!("sqlite", "sqlite"),
    icon!("properties", "properties"),
    icon!("console", "console"),
    icon!("table", "table"),
    icon!("disconnect", "disconnect"),
    icon!("jump", "jump"),
    icon!("import", "import"),
    icon!("url", "url"),
    icon!("file", "file"),
    icon!("driver", "driver"),
    icon!("databaseNode", "databaseNode"),
    icon!("schema", "schema"),
    icon!("folder", "folder"),
    icon!("view", "view"),
    icon!("materializedView", "materializedView"),
    icon!("column", "column"),
    icon!("columnKey", "columnKey"),
    icon!("columnNotNull", "columnNotNull"),
    icon!("key", "key"),
    icon!("foreignKey", "foreignKey"),
    icon!("index", "index"),
    icon!("uniqueIndex", "uniqueIndex"),
    icon!("check", "check"),
    icon!("statementOk", "statementOk"),
    icon!("trigger", "trigger"),
    icon!("routine", "routine"),
    icon!("procedure", "procedure"),
    icon!("sequence", "sequence"),
    icon!("objectType", "objectType"),
    icon!("package", "package"),
    icon!("synonym", "synonym"),
    icon!("error", "error"),
    icon!("columnForeign", "columnForeign"),
    icon!("sortable", "sortable"),
    icon!("orderBy", "orderBy"),
    icon!("firstPage", "firstPage"),
    icon!("lastPage", "lastPage"),
    icon!("stopDisabled", "stopDisabled"),
    icon!("services@20x20", "services@20x20"),
    icon!("exportData", "exportData"),
    icon!("importData", "importData"),
];

/// The product's store, for the module's views.
pub(crate) struct Store(pub Arc<dyn DataSourceStore>);

impl Global for Store {}

pub(crate) fn store(cx: &App) -> Arc<dyn DataSourceStore> {
    cx.global::<Store>().0.clone()
}

/// What the product gives the module.
#[derive(Clone)]
pub struct DbUiConfig {
    pub store: Arc<dyn DataSourceStore>,
}

/// The database module for `Product::with_module`.
pub fn module(config: DbUiConfig) -> Module {
    let store = config.store;
    let hub = Arc::new(sql::Hub::new(store.clone()));
    let init_hub = hub.clone();
    Module {
        name: "database",
        icons: ICONS,
        file_types: vec![sql::file_type(hub)],
        status_widgets: vec![grid::status_widget()],
        editor_headers: vec![sql::widget::editor_header()],
        settings_pages: settings_page::pages(),
        init: Some(Init::new(move |cx| {
            cx.set_global(Store(store.clone()));
            settings_ui::store::register::<query_settings::QueryExecutionSettings>(cx);
            sql::widget::init(cx, init_hub.clone());
            explorer::init(cx);
            dialog::init(cx);
            prompt::init(cx);
            grid::init(cx);
            console::init(cx);
            transfer::init(cx);
        })),
        keymaps: Some(KEYMAPS),
        settings_pages: data_views::pages(),
        tool_windows: vec![
            ToolWindowHook {
                spec: ToolWindowSpec {
                    id: TOOL_WINDOW,
                    area: Area::Right,
                    // After Notifications (0), room for the AI tool windows.
                    order: 20,
                    icon: icons::TOOL_WINDOW,
                    title: "Database",
                    command: None,
                    registered: true,
                },
                create: std::rc::Rc::new(explorer::create),
                width: Some(explorer::WIDTH),
            },
            console::services::hook(),
        ],
        ..Module::default()
    }
}

/// The plain name of a backend, as menus and the driver list show it.
pub fn backend_title(backend: db_core::Backend) -> &'static str {
    match backend {
        db_core::Backend::Postgres => "PostgreSQL",
        db_core::Backend::OracleThin => "Oracle",
        db_core::Backend::OracleOdpi => "Oracle (Instant Client)",
        db_core::Backend::Sqlite => "SQLite",
    }
}

/// The backends in the order menus list them.
pub const BACKENDS: [db_core::Backend; 4] = [
    db_core::Backend::Postgres,
    db_core::Backend::OracleThin,
    db_core::Backend::OracleOdpi,
    db_core::Backend::Sqlite,
];
