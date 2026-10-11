//! `pyoxide-db`: the command-line front-end of the Pyoxide database engine.
//! Every sub-command calls the same functions as the `db.*` commands.

use std::io::{BufRead, IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use commands::Registry;
use db_core::schema::ObjectKind;
use db_core::{Backend, Column, Error, ResultSet, Secret, TlsMode, Value, ValueKind, render_table};
use db_service::diagnose::{DiagnoseOptions, Status};
use db_service::ops;
use db_service::profiles::{self, Profile, TlsProfile};
use db_service::secrets::SecretKind;
use db_service::{DbService, Target};
use secrets::config::{KeePassConfig, MasterPasswordSource, StoreKind};
use secrets::{KeyringStore, LazyStore, Prompter, SecretStore, SecretsConfig};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "pyoxide-db",
    version,
    about = "Database client of Pyoxide IDE: Oracle, PostgreSQL, SQLite",
    after_help = "Passwords are never accepted as arguments: they come from PYOXIDE_DB_PASSWORD, \
                  the profile's password_env, the OS credential store (`secret set`) or a prompt."
)]
struct Cli {
    /// Profiles file (default: $PYOXIDE_DB_PROFILES or the user config directory).
    #[arg(long, global = true, value_name = "PATH")]
    profiles: Option<PathBuf>,
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(flatten)]
    conn: ConnArgs,
    #[command(subcommand)]
    cmd: Cmd,
}

/// How to connect: a saved profile, or inline settings.
#[derive(Args, Clone, Default)]
struct ConnArgs {
    /// Saved profile name.
    #[arg(short = 'p', long, global = true)]
    profile: Option<String>,
    /// oracle-thin | oracle-odpi | postgres | sqlite (inferred when omitted).
    #[arg(long, global = true, value_parser = parse_backend)]
    backend: Option<Backend>,
    #[arg(long, global = true, env = "PYOXIDE_DB_HOST")]
    host: Option<String>,
    #[arg(long, global = true, env = "PYOXIDE_DB_PORT")]
    port: Option<u16>,
    /// Oracle service name.
    #[arg(long, global = true, env = "PYOXIDE_DB_SERVICE")]
    service: Option<String>,
    /// Oracle SID.
    #[arg(long, global = true)]
    sid: Option<String>,
    /// Oracle connect string (TNS alias, Easy Connect, descriptor).
    #[arg(long, global = true)]
    dsn: Option<String>,
    /// PostgreSQL database.
    #[arg(long, global = true)]
    database: Option<String>,
    /// SQLite file or :memory:.
    #[arg(long, global = true)]
    path: Option<String>,
    #[arg(long, global = true, env = "PYOXIDE_DB_USER")]
    user: Option<String>,
    /// TLS mode: disable | prefer | require | verify-ca | verify-full.
    #[arg(long, global = true, value_parser = parse_tls)]
    tls: Option<TlsMode>,
    #[arg(long, global = true)]
    ca_file: Option<String>,
    #[arg(long, global = true)]
    wallet_dir: Option<String>,
    #[arg(long, global = true)]
    server_name: Option<String>,
    #[arg(long, global = true, value_name = "SECS")]
    connect_timeout: Option<u64>,
    /// SQLite: create the file if missing.
    #[arg(long, global = true)]
    create: bool,
    #[arg(long, global = true)]
    read_only: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// List backends and whether they can be used here.
    Backends,
    /// Manage saved connection profiles.
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// Manage stored passwords (values are never printed).
    #[command(subcommand)]
    Secret(SecretCmd),
    /// Connect, ping and show the server version.
    #[command(alias = "ping")]
    Test,
    /// Run one SQL statement.
    Query {
        sql: String,
        #[arg(long, default_value_t = 500)]
        max_rows: usize,
        /// Statement timeout in seconds (0: none).
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
        /// Print timings to stderr.
        #[arg(long)]
        stats: bool,
    },
    /// List schemas.
    Schemas,
    /// List objects of a schema (default: the current schema).
    Objects {
        #[arg(long)]
        schema: Option<String>,
        /// table, view, materialized_view, sequence, index, trigger, procedure, function, package, type, synonym.
        #[arg(long = "kind", value_parser = parse_kind)]
        kinds: Vec<ObjectKind>,
    },
    /// List tables of a schema.
    Tables {
        #[arg(long)]
        schema: Option<String>,
    },
    /// Columns, indexes and constraints of a table: `describe TABLE` or `describe SCHEMA TABLE`.
    Describe {
        #[arg(num_args = 1..=2, required = true)]
        names: Vec<String>,
        #[arg(long)]
        schema: Option<String>,
    },
    /// Source or DDL of a view, routine, package or trigger.
    Source {
        name: String,
        #[arg(long, value_parser = parse_kind)]
        kind: ObjectKind,
        #[arg(long)]
        schema: Option<String>,
    },
    /// Check the connection step by step and explain the first failure.
    Diagnose {
        /// Mask host names, addresses, user, service and paths for sharing.
        #[arg(long)]
        share: bool,
        /// Also check paging and statement timeouts.
        #[arg(long)]
        deep: bool,
        /// Oracle: also try the other Oracle backend.
        #[arg(long)]
        try_all: bool,
    },
    /// Choose where passwords are kept: OS credential store, KeePass, memory.
    #[command(subcommand, name = "secret-store")]
    SecretStore(SecretStoreCmd),
    /// Import connections from another IDE.
    #[command(subcommand)]
    Import(ImportCmd),
    /// List all registered commands.
    Commands,
    /// Invoke a registered command with JSON arguments (from the argument or stdin).
    Call { id: String, args: Option<String> },
}

#[derive(Subcommand)]
enum ProfileCmd {
    List,
    Show {
        name: String,
    },
    /// Create or replace a profile from the connection flags.
    Add {
        name: String,
        /// Environment variable that holds the password.
        #[arg(long)]
        password_env: Option<String>,
        #[arg(long, value_name = "SECS")]
        query_timeout: Option<u64>,
    },
    Remove {
        name: String,
    },
}

#[derive(Subcommand)]
enum SecretStoreCmd {
    /// Show the configured store.
    Show,
    /// Switch the store. For KeePass the master password is asked once and
    /// kept in the OS credential store unless --master says otherwise.
    Use {
        /// keychain | keepass | memory
        kind: String,
        /// KeePass database file (.kdbx).
        #[arg(long)]
        path: Option<String>,
        /// KeePass key file.
        #[arg(long)]
        key_file: Option<String>,
        /// Where the KeePass master password comes from: keychain | env | prompt.
        #[arg(long, default_value = "keychain")]
        master: String,
        /// Create the KeePass database if it does not exist.
        #[arg(long)]
        create: bool,
    },
}

#[derive(Subcommand)]
enum ImportCmd {
    /// Data sources of a PyCharm project, with passwords from PyCharm's
    /// credential store (its KeePass file, or the OS store with --allow-keychain).
    Pycharm {
        /// Project directory (or its .idea directory).
        project: PathBuf,
        /// Show what would be imported; write nothing, print no secrets.
        #[arg(long)]
        dry_run: bool,
        /// PyCharm configuration directory (default: the newest found).
        #[arg(long)]
        ide_config: Option<PathBuf>,
        /// Read passwords PyCharm keeps in the OS credential store (the OS may ask).
        #[arg(long)]
        allow_keychain: bool,
        /// Subsystem part of PyCharm's credential names (default DB).
        #[arg(long)]
        naming_subsystem: Option<String>,
        /// Key part of PyCharm's credential names: uuid (default) or name.
        #[arg(long, default_value = "uuid")]
        naming_key: String,
        /// Replace profiles that already exist.
        #[arg(long)]
        overwrite: bool,
        /// Import profiles only.
        #[arg(long)]
        no_passwords: bool,
    },
}

#[derive(Subcommand)]
enum SecretCmd {
    /// Store a password in the OS credential store (prompt, or one line from stdin with --stdin).
    Set {
        profile: String,
        #[arg(long)]
        wallet: bool,
        #[arg(long)]
        stdin: bool,
    },
    Delete {
        profile: String,
        #[arg(long)]
        wallet: bool,
    },
    /// Where the password would come from.
    Status { profile: String },
}

fn parse_backend(s: &str) -> Result<Backend, String> {
    Backend::from_id(s).ok_or_else(|| {
        let ids: Vec<&str> = Backend::ALL.iter().map(|b| b.id()).collect();
        format!("unknown backend {s:?}, expected one of {}", ids.join(", "))
    })
}

fn parse_tls(s: &str) -> Result<TlsMode, String> {
    serde_json::from_value(serde_json::Value::String(s.into()))
        .map_err(|_| "expected disable, prefer, require, verify-ca or verify-full".to_string())
}

fn parse_kind(s: &str) -> Result<ObjectKind, String> {
    serde_json::from_value(serde_json::Value::String(s.replace('-', "_")))
        .map_err(|_| format!("unknown object kind {s:?}"))
}

/// Asks on the terminal without echo; `None` when stdin is not a terminal.
struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn secret(&self, prompt: &str) -> Option<Secret> {
        if !std::io::stdin().is_terminal() {
            return None;
        }
        rpassword::prompt_password(prompt).ok().map(Secret::new)
    }
}

/// Secret from one line of stdin, for `secret set --stdin`.
struct StdinPrompter;

impl Prompter for StdinPrompter {
    fn secret(&self, _: &str) -> Option<Secret> {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok()?;
        Some(Secret::new(line.trim_end_matches(['\r', '\n'])))
    }
}

impl ConnArgs {
    fn is_inline(&self) -> bool {
        self.backend.is_some()
            || self.host.is_some()
            || self.path.is_some()
            || self.dsn.is_some()
            || self.service.is_some()
            || self.sid.is_some()
            || self.database.is_some()
    }

    fn backend(&self) -> Backend {
        self.backend.unwrap_or(if self.path.is_some() {
            Backend::Sqlite
        } else if self.database.is_some() && self.service.is_none() && self.sid.is_none() {
            Backend::Postgres
        } else {
            Backend::OracleThin
        })
    }

    fn to_profile(&self) -> Profile {
        let mut p = Profile::new(self.backend());
        // Network backends default to the local machine.
        p.host = match (&self.host, p.backend, &self.dsn) {
            (Some(h), _, _) => h.clone(),
            (None, Backend::Sqlite, _) | (None, _, Some(_)) => String::new(),
            (None, _, None) => "localhost".into(),
        };
        p.port = self.port;
        p.service = self.service.clone();
        p.sid = self.sid.clone();
        p.dsn = self.dsn.clone();
        p.database = self.database.clone();
        p.path = self.path.clone();
        p.user = self.user.clone().unwrap_or_default();
        p.create = self.create;
        p.read_only = self.read_only;
        p.connect_timeout_secs = self.connect_timeout;
        let tls = TlsProfile {
            mode: self.tls.unwrap_or_default(),
            ca_file: self.ca_file.clone(),
            wallet_dir: self.wallet_dir.clone(),
            server_name: self.server_name.clone(),
            ..TlsProfile::default()
        };
        if tls != TlsProfile::default() {
            p.tls = Some(tls);
        }
        p
    }

    fn target(&self) -> Result<Target, Error> {
        match (&self.profile, self.is_inline()) {
            (Some(name), false) => Ok(Target::Profile(name.clone())),
            (Some(_), true) => Err(Error::config(
                "use either --profile or inline connection flags, not both",
            )),
            (None, true) => Ok(Target::Connection(Box::new(self.to_profile()))),
            (None, false) => Err(Error::config(
                "no connection: pass --profile NAME or connection flags (--backend, --host, --path, ...)",
            )),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            match &e.code {
                Some(code) if !e.message.contains(code.as_str()) => {
                    eprintln!("error: [{code}] {}", e.message)
                }
                _ => eprintln!("error: {}", e.message),
            }
            ExitCode::FAILURE
        }
    }
}

fn service(cli: &Cli, prompter: Option<Arc<dyn Prompter>>) -> DbService {
    let env = secrets::process_env();
    DbService::new(
        cli.profiles.clone().unwrap_or_else(profiles::default_path),
        configured_store(prompter.clone(), env.clone()),
        prompter,
        env,
    )
}

/// The store chosen in the secret settings, opened on first use.
fn configured_store(
    prompter: Option<Arc<dyn Prompter>>,
    env: secrets::EnvLookup,
) -> Arc<dyn SecretStore> {
    let path = secrets::config::config_path(&env);
    let config = match SecretsConfig::load(&path) {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            return Arc::new(LazyStore::new("secret settings", move || {
                Err(secrets::SecretError::new(
                    secrets::SecretErrorKind::Format,
                    "secret settings",
                    msg,
                ))
            }));
        }
    };
    let name = match config.store {
        StoreKind::Keychain => KeyringStore.name().to_owned(),
        StoreKind::Memory => "memory (not saved)".to_owned(),
        StoreKind::KeePass => format!(
            "KeePass database {}",
            config
                .keepass
                .as_ref()
                .map(|k| k.path.as_str())
                .unwrap_or("?")
        ),
    };
    Arc::new(LazyStore::new(name, move || {
        secrets::open_store(&config, prompter.as_deref(), &env)
    }))
}

fn print_json<T: Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn text_table(names: &[&str], rows: Vec<Vec<String>>) -> String {
    render_table(&ResultSet {
        columns: names
            .iter()
            .map(|n| Column {
                name: (*n).into(),
                type_name: "TEXT".into(),
                kind: ValueKind::Text,
                nullable: true,
            })
            .collect(),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(Value::Text).collect())
            .collect(),
        ..Default::default()
    })
}

async fn run(cli: Cli) -> Result<ExitCode, Error> {
    let prompter: Arc<dyn Prompter> = Arc::new(TerminalPrompter);
    let svc = service(&cli, Some(prompter));
    let json = cli.json;
    match &cli.cmd {
        Cmd::Backends => {
            let list = ops::backends();
            if json {
                print_json(&list);
            } else {
                let rows = list
                    .iter()
                    .map(|b| {
                        vec![
                            b.backend.to_string(),
                            if b.available { "yes" } else { "no" }.into(),
                            b.detail.clone(),
                        ]
                    })
                    .collect();
                println!("{}", text_table(&["BACKEND", "AVAILABLE", "DETAIL"], rows));
            }
        }
        Cmd::Profile(cmd) => profile_cmd(&cli, &svc, cmd)?,
        Cmd::Secret(cmd) => secret_cmd(&cli, cmd)?,
        Cmd::Test => {
            let r = ops::connection_test(&svc, &cli.conn.target()?).await?;
            if json {
                print_json(&r);
            } else {
                println!(
                    "OK: {}\nbackend {} ({}), password from {}, connect {} ms, ping {} ms",
                    r.server_version,
                    r.backend,
                    r.client,
                    r.password_source,
                    r.connect_ms,
                    r.ping_ms
                );
            }
        }
        Cmd::Query {
            sql,
            max_rows,
            timeout,
            stats,
        } => {
            let args = ops::QueryArgs {
                target: cli.conn.target()?,
                sql: sql.clone(),
                params: Vec::new(),
                max_rows: *max_rows,
                timeout_secs: *timeout,
            };
            let out = ops::query(&svc, &args).await?;
            if json {
                print_json(&out);
            } else {
                println!("[{}]", out.backend);
                println!("{}", render_table(&out.result));
            }
            if *stats {
                let t = &out.timing;
                eprintln!(
                    "connect {} ms, first page {} ms, total {} ms, {} row(s)",
                    t.connect_ms,
                    t.first_page_ms,
                    t.total_ms,
                    out.result.rows.len()
                );
            }
        }
        Cmd::Schemas => {
            let list = ops::schema_list(&svc, &cli.conn.target()?).await?;
            if json {
                print_json(&list);
            } else {
                let rows = list
                    .iter()
                    .map(|s| {
                        let mut flags = Vec::new();
                        if s.is_current {
                            flags.push("current");
                        }
                        if s.is_system {
                            flags.push("system");
                        }
                        vec![s.name.clone(), flags.join(", ")]
                    })
                    .collect();
                println!("{}", text_table(&["SCHEMA", ""], rows));
            }
        }
        Cmd::Objects { schema, kinds } => {
            objects(&svc, &cli, schema.clone(), kinds.clone()).await?;
        }
        Cmd::Tables { schema } => {
            objects(&svc, &cli, schema.clone(), vec![ObjectKind::Table]).await?;
        }
        Cmd::Describe { names, schema } => {
            let (schema, table) = match names.as_slice() {
                [s, t] => (Some(s.clone()), t.clone()),
                [t] => (schema.clone(), t.clone()),
                _ => unreachable!("clap enforces 1..=2"),
            };
            let args = ops::TableArgs {
                target: cli.conn.target()?,
                schema,
                table,
            };
            let info = ops::schema_table(&svc, &args).await?;
            if json {
                print_json(&info);
            } else {
                describe_text(&info);
            }
        }
        Cmd::Source { name, kind, schema } => {
            let args = ops::SourceArgs {
                target: cli.conn.target()?,
                schema: schema.clone(),
                name: name.clone(),
                kind: *kind,
            };
            let src = ops::schema_source(&svc, &args).await?;
            if json {
                print_json(&src);
            } else {
                println!("{}", src.as_deref().unwrap_or("(no source available)"));
            }
        }
        Cmd::Diagnose {
            share,
            deep,
            try_all,
        } => {
            let args = ops::DiagnoseArgs {
                target: cli.conn.target()?,
                options: DiagnoseOptions {
                    share: *share,
                    deep: *deep,
                    try_all: *try_all,
                },
            };
            let report = ops::diagnose(&svc, &args).await;
            if json {
                print_json(&report);
            } else {
                println!("{}", db_service::diagnose::render(&report));
            }
            if report.result == Status::Fail {
                return Ok(ExitCode::FAILURE);
            }
        }
        Cmd::SecretStore(cmd) => secret_store_cmd(cmd)?,
        Cmd::Import(ImportCmd::Pycharm {
            project,
            dry_run,
            ide_config,
            allow_keychain,
            naming_subsystem,
            naming_key,
            overwrite,
            no_passwords,
        }) => {
            let args = db_service::import::PycharmImportArgs {
                project: project.clone(),
                ide_config: ide_config.clone(),
                allow_os_store: *allow_keychain,
                naming_subsystem: naming_subsystem.clone(),
                naming_by_name: match naming_key.as_str() {
                    "uuid" => false,
                    "name" => true,
                    other => {
                        return Err(Error::config(format!(
                            "--naming-key must be uuid or name, got {other:?}"
                        )));
                    }
                },
                dry_run: *dry_run,
                overwrite: *overwrite,
                no_passwords: *no_passwords,
            };
            let report = db_service::import::import_pycharm(&svc, &args)?;
            if json {
                print_json(&report);
            } else {
                println!("{}", db_service::import::render(&report));
            }
        }
        Cmd::Commands => {
            let registry = registry(svc);
            if json {
                print_json(&registry.list());
            } else {
                let rows = registry
                    .list()
                    .iter()
                    .map(|m| {
                        vec![
                            m.id.to_owned(),
                            m.summary.to_owned(),
                            m.args_example.to_owned(),
                        ]
                    })
                    .collect();
                println!(
                    "{}",
                    text_table(&["COMMAND", "SUMMARY", "EXAMPLE ARGS"], rows)
                );
            }
        }
        Cmd::Call { id, args } => {
            let raw = match args {
                Some(a) => a.clone(),
                None => {
                    let mut s = String::new();
                    std::io::stdin()
                        .read_to_string(&mut s)
                        .map_err(|e| Error::config(format!("reading stdin: {e}")))?;
                    s
                }
            };
            let value: serde_json::Value = if raw.trim().is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_str(&raw)
                    .map_err(|e| Error::config(format!("arguments are not JSON: {e}")))?
            };
            match registry(svc).invoke(id, value).await {
                Ok(v) => print_json(&v),
                Err(e) => {
                    print_json(&e);
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn registry(svc: DbService) -> Registry {
    let mut r = Registry::new();
    ops::register(&mut r, Arc::new(svc));
    r
}

async fn objects(
    svc: &DbService,
    cli: &Cli,
    schema: Option<String>,
    kinds: Vec<ObjectKind>,
) -> Result<(), Error> {
    let args = ops::ObjectsArgs {
        target: cli.conn.target()?,
        schema,
        kinds,
    };
    let list = ops::schema_objects(svc, &args).await?;
    if cli.json {
        print_json(&list);
    } else {
        let kind = |k: ObjectKind| {
            serde_json::to_value(k)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        };
        let rows = list
            .iter()
            .map(|o| {
                vec![
                    o.schema.clone(),
                    o.name.clone(),
                    kind(o.kind),
                    o.parent.clone().unwrap_or_default(),
                    o.comment.clone().unwrap_or_default(),
                ]
            })
            .collect();
        println!(
            "{}",
            text_table(&["SCHEMA", "NAME", "KIND", "PARENT", "COMMENT"], rows)
        );
    }
    Ok(())
}

fn describe_text(info: &ops::TableInfo) {
    println!("{}.{}", info.schema, info.table);
    let rows = info
        .columns
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.data_type.clone(),
                if c.nullable { "" } else { "NOT NULL" }.into(),
                c.default.clone().unwrap_or_default(),
                if c.auto_increment { "auto" } else { "" }.into(),
                c.comment.clone().unwrap_or_default(),
            ]
        })
        .collect();
    println!(
        "{}",
        text_table(&["COLUMN", "TYPE", "NULL", "DEFAULT", "", "COMMENT"], rows)
    );
    if let Some(indexes) = &info.indexes {
        let rows = indexes
            .iter()
            .map(|i| {
                vec![
                    i.name.clone(),
                    if i.unique { "unique" } else { "" }.into(),
                    i.columns.join(", "),
                    i.method.clone().unwrap_or_default(),
                ]
            })
            .collect();
        println!(
            "\n{}",
            text_table(&["INDEX", "", "COLUMNS", "METHOD"], rows)
        );
    }
    if let Some(constraints) = &info.constraints {
        let rows = constraints
            .iter()
            .map(|c| {
                let target = match (&c.ref_table, &c.check) {
                    (Some(t), _) => format!(
                        "-> {}{t}({}){}",
                        c.ref_schema
                            .as_ref()
                            .map(|s| format!("{s}."))
                            .unwrap_or_default(),
                        c.ref_columns.join(", "),
                        c.on_delete
                            .as_ref()
                            .map(|d| format!(" on delete {d}"))
                            .unwrap_or_default()
                    ),
                    (None, Some(check)) => check.clone(),
                    _ => String::new(),
                };
                vec![
                    c.name.clone(),
                    format!("{:?}", c.kind),
                    c.columns.join(", "),
                    target,
                ]
            })
            .collect();
        println!(
            "\n{}",
            text_table(&["CONSTRAINT", "KIND", "COLUMNS", "DETAIL"], rows)
        );
    }
    for e in &info.errors {
        eprintln!("note: {e}");
    }
}

fn profile_cmd(cli: &Cli, svc: &DbService, cmd: &ProfileCmd) -> Result<(), Error> {
    match cmd {
        ProfileCmd::List => {
            let list = ops::profile_list(svc)?;
            if cli.json {
                print_json(&list);
            } else if list.is_empty() {
                println!("no profiles in {}", svc.profiles_path.display());
            } else {
                let rows = list
                    .iter()
                    .map(|p| {
                        vec![
                            p.name.clone(),
                            p.backend.to_string(),
                            p.target.clone(),
                            p.user.clone(),
                        ]
                    })
                    .collect();
                println!(
                    "{}",
                    text_table(&["PROFILE", "BACKEND", "TARGET", "USER"], rows)
                );
            }
        }
        ProfileCmd::Show { name } => {
            let file = svc.profiles()?;
            let p = file.get(name)?;
            if cli.json {
                print_json(p);
            } else {
                let mut one = profiles::ProfilesFile::default();
                one.profiles.insert(name.clone(), p.clone());
                print!("{}", one.to_toml()?);
            }
        }
        ProfileCmd::Add {
            name,
            password_env,
            query_timeout,
        } => {
            if cli.conn.profile.is_some() {
                return Err(Error::config(
                    "`profile add` takes the connection flags, not --profile",
                ));
            }
            let mut p = cli.conn.to_profile();
            p.password_env = password_env.clone();
            p.query_timeout_secs = *query_timeout;
            svc.save_profile(name, p)?;
            println!("saved profile {name} in {}", svc.profiles_path.display());
            println!("store its password with: pyoxide-db secret set {name}");
        }
        ProfileCmd::Remove { name } => {
            if svc.remove_profile(name)? {
                println!("removed profile {name}");
            } else {
                return Err(Error::config(format!("no profile named {name:?}")));
            }
        }
    }
    Ok(())
}

fn secret_store_cmd(cmd: &SecretStoreCmd) -> Result<(), Error> {
    let env = secrets::process_env();
    let path = secrets::config::config_path(&env);
    let cfg_err = |e: secrets::SecretError| Error::config(e.to_string());
    match cmd {
        SecretStoreCmd::Show => {
            let config = SecretsConfig::load(&path).map_err(cfg_err)?;
            println!("settings: {}", path.display());
            print!("{}", config.to_toml().map_err(cfg_err)?);
        }
        SecretStoreCmd::Use {
            kind,
            path: kp_path,
            key_file,
            master,
            create,
        } => {
            let mut config = SecretsConfig::load(&path).map_err(cfg_err)?;
            config.store = serde_json::from_value(serde_json::Value::String(kind.clone()))
                .map_err(|_| Error::config("store must be keychain, keepass or memory"))?;
            if config.store == StoreKind::KeePass {
                let file = kp_path
                    .clone()
                    .or_else(|| config.keepass.as_ref().map(|k| k.path.clone()))
                    .ok_or_else(|| Error::config("keepass needs --path to a .kdbx file"))?;
                let mut kp = KeePassConfig::new(file);
                kp.key_file = key_file.clone();
                kp.create = *create;
                kp.master_password =
                    serde_json::from_value(serde_json::Value::String(master.clone()))
                        .map_err(|_| Error::config("--master must be keychain, env or prompt"))?;
                if kp.master_password == MasterPasswordSource::Keychain {
                    let account = kp.master_password_account();
                    let known = KeyringStore
                        .get(secrets::config::KEEPASS_MASTER_SERVICE, &account)
                        .ok()
                        .flatten()
                        .is_some();
                    if !known {
                        let master = TerminalPrompter
                            .secret(&format!(
                                "KeePass master password ({}): ",
                                kp.db_path().display()
                            ))
                            .ok_or_else(|| Error::config("no master password entered"))?;
                        KeyringStore
                            .set(secrets::config::KEEPASS_MASTER_SERVICE, &account, &master)
                            .map_err(|e| Error::config(e.to_string()))?;
                    }
                }
                config.keepass = Some(kp);
            }
            // Open it once so mistakes show up now, not at the next connect.
            let prompter = TerminalPrompter;
            secrets::open_store(&config, Some(&prompter), &env).map_err(cfg_err)?;
            config.save(&path).map_err(cfg_err)?;
            println!("secret store: {kind} (settings in {})", path.display());
        }
    }
    Ok(())
}

fn secret_cmd(cli: &Cli, cmd: &SecretCmd) -> Result<(), Error> {
    let kind = |wallet: bool| {
        if wallet {
            SecretKind::WalletPassword
        } else {
            SecretKind::Password
        }
    };
    match cmd {
        SecretCmd::Set {
            profile,
            wallet,
            stdin,
        } => {
            let prompter: Arc<dyn Prompter> = if *stdin {
                Arc::new(StdinPrompter)
            } else {
                Arc::new(TerminalPrompter)
            };
            let svc = service(cli, Some(prompter));
            ops::secret_set(&svc, profile, kind(*wallet))?;
            println!("stored in {}", svc.store.name());
        }
        SecretCmd::Delete { profile, wallet } => {
            let svc = service(cli, None);
            if ops::secret_delete(&svc, profile, kind(*wallet))? {
                println!("deleted");
            } else {
                println!("nothing stored");
            }
        }
        SecretCmd::Status { profile } => {
            let svc = service(cli, None);
            let s = ops::secret_status(&svc, profile)?;
            if cli.json {
                print_json(&s);
            } else {
                println!("password: {}", s.password);
                println!("wallet password: {}", s.wallet_password);
                if let Some(e) = s.store_error {
                    println!("secret store problem: {e}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn infers_backend() {
        let mut c = ConnArgs {
            path: Some(":memory:".into()),
            ..Default::default()
        };
        assert_eq!(c.backend(), Backend::Sqlite);
        c.path = None;
        c.database = Some("app".into());
        assert_eq!(c.backend(), Backend::Postgres);
        c.service = Some("ORCL".into());
        assert_eq!(c.backend(), Backend::OracleThin);
    }

    #[test]
    fn legacy_flags_still_parse() {
        let cli =
            Cli::try_parse_from(["pyoxide-db", "--service", "FREEPDB1", "--user", "u", "ping"])
                .unwrap();
        assert!(matches!(cli.cmd, Cmd::Test));
        assert!(matches!(cli.conn.target().unwrap(), Target::Connection(_)));
        let cli =
            Cli::try_parse_from(["pyoxide-db", "describe", "HR", "EMPLOYEES", "-p", "x"]).unwrap();
        assert!(matches!(cli.conn.target().unwrap(), Target::Profile(_)));
    }
}
