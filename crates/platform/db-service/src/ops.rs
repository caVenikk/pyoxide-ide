//! The `db.*` operations as typed async functions and their registration in
//! the command registry. The CLI calls the functions directly; the control
//! channel and the IDE go through the registry. Same code either way.

use std::sync::Arc;
use std::time::{Duration, Instant};

use commands::{CommandError, CommandMeta, Registry, typed};
use db_core::schema::{
    self, ColumnInfo, ConstraintInfo, IndexInfo, ObjectInfo, ObjectKind, SchemaInfo,
};
use db_core::{Backend, Error, ErrorKind, ExecOptions, Outcome, ResultSet, Secret, Value, collect};
use serde::{Deserialize, Serialize};

use crate::diagnose::{self, DiagnoseOptions, Report};
use crate::profiles::Profile;
use crate::secrets::{SECRET_SERVICE, SecretKind, SecretSource};
use crate::service::{DbService, Opened, Target, millis};

// ---------- arguments and results ----------

#[derive(Debug, Clone, Serialize)]
pub struct BackendInfo {
    pub backend: Backend,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileSummary {
    pub name: String,
    pub backend: Backend,
    pub target: String,
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NameArgs {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveProfileArgs {
    pub name: String,
    pub profile: Profile,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecretArgs {
    pub profile: String,
    #[serde(default = "default_kind")]
    pub kind: SecretKind,
}

fn default_kind() -> SecretKind {
    SecretKind::Password
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretStatus {
    pub password: SecretSource,
    pub wallet_password: SecretSource,
    pub store: String,
    pub store_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TargetArgs {
    #[serde(flatten)]
    pub target: Target,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub backend: Backend,
    pub client: String,
    pub server_version: String,
    pub password_source: SecretSource,
    pub connect_ms: f64,
    pub ping_ms: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryArgs {
    #[serde(flatten)]
    pub target: Target,
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Value>,
    #[serde(default = "default_max_rows")]
    pub max_rows: usize,
    /// Overrides the profile's `query_timeout_secs`.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn default_max_rows() -> usize {
    500
}

#[derive(Debug, Clone, Serialize)]
pub struct Timing {
    pub connect_ms: f64,
    /// Until the statement returned (and, for queries, the first page arrived).
    pub first_page_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryOutput {
    pub backend: Backend,
    pub result: ResultSet,
    pub timing: Timing,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObjectsArgs {
    #[serde(flatten)]
    pub target: Target,
    /// Defaults to the current schema.
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub kinds: Vec<ObjectKind>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TableArgs {
    #[serde(flatten)]
    pub target: Target,
    #[serde(default)]
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableInfo {
    pub schema: String,
    pub table: String,
    pub columns: Vec<ColumnInfo>,
    /// `None` when the backend could not provide them (see `errors`).
    pub indexes: Option<Vec<IndexInfo>>,
    pub constraints: Option<Vec<ConstraintInfo>>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceArgs {
    #[serde(flatten)]
    pub target: Target,
    #[serde(default)]
    pub schema: Option<String>,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnoseArgs {
    #[serde(flatten)]
    pub target: Target,
    #[serde(flatten)]
    pub options: DiagnoseOptions,
}

// ---------- operations ----------

pub fn backends() -> Vec<BackendInfo> {
    Backend::ALL
        .into_iter()
        .map(|b| match crate::driver(b).availability() {
            Ok(detail) => BackendInfo {
                backend: b,
                available: true,
                detail,
            },
            Err(e) => BackendInfo {
                backend: b,
                available: false,
                detail: e.message,
            },
        })
        .collect()
}

pub fn profile_list(svc: &DbService) -> Result<Vec<ProfileSummary>, Error> {
    Ok(svc
        .profiles()?
        .profiles
        .into_iter()
        .map(|(name, p)| ProfileSummary {
            target: p.target(),
            user: p.user.clone(),
            backend: p.backend,
            name,
        })
        .collect())
}

pub fn secret_status(svc: &DbService, profile: &str) -> Result<SecretStatus, Error> {
    let p = svc.profiles()?.get(profile)?.clone();
    // Report without prompting.
    let quiet = DbService::new(
        svc.profiles_path.clone(),
        svc.store.clone(),
        None,
        svc.env.clone(),
    );
    let s = quiet.secrets_for(profile, &p);
    Ok(SecretStatus {
        password: s.password_source,
        wallet_password: s.wallet_source,
        store: svc.store.name().to_owned(),
        store_error: s.store_error,
    })
}

/// Ask the front-end for a secret and save it in the secret store.
pub fn secret_set(svc: &DbService, profile: &str, kind: SecretKind) -> Result<(), Error> {
    svc.profiles()?.get(profile)?;
    let prompter = svc
        .prompter
        .as_ref()
        .ok_or_else(|| Error::config("no interactive prompt available to read the secret"))?;
    let what = match kind {
        SecretKind::Password => format!("Password for profile {profile}: "),
        SecretKind::WalletPassword => format!("Wallet password for profile {profile}: "),
    };
    let secret: Secret = prompter
        .secret(&what)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::config("no secret entered"))?;
    svc.store
        .set(SECRET_SERVICE, &kind.account(profile), &secret)
        .map_err(store_error)
}

pub fn secret_delete(svc: &DbService, profile: &str, kind: SecretKind) -> Result<bool, Error> {
    svc.store
        .delete(SECRET_SERVICE, &kind.account(profile))
        .map_err(store_error)
}

fn store_error(e: secrets::SecretError) -> Error {
    Error::new(ErrorKind::Unavailable, e.to_string())
}

pub async fn connection_test(svc: &DbService, target: &Target) -> Result<TestResult, Error> {
    let opened = svc.open(target).await?;
    let started = Instant::now();
    opened.conn.ping().await?;
    let ping_ms = millis(started);
    let server_version = opened.conn.server_version().await.unwrap_or_default();
    let client = crate::driver(opened.profile.backend)
        .availability()
        .unwrap_or_default();
    opened.conn.close().await.ok();
    Ok(TestResult {
        backend: opened.profile.backend,
        client,
        server_version,
        password_source: opened.password_source,
        connect_ms: opened.connect_ms,
        ping_ms,
    })
}

/// Test Connection of the IDE's data source dialog: `profile` as edited,
/// saved or not, under the name `name`, with the password typed into the
/// dialog (`Some`) or the one found the usual way (`None`: environment,
/// secret store). Nothing is written.
pub async fn test_profile(
    svc: &DbService,
    name: &str,
    profile: &Profile,
    password: Option<Secret>,
) -> Result<TestResult, Error> {
    let opened = open_profile(svc, name, profile, password).await?;
    let started = Instant::now();
    let pinged = opened.conn.ping().await;
    let ping_ms = millis(started);
    let server_version = opened.conn.server_version().await.unwrap_or_default();
    opened.conn.close().await.ok();
    pinged?;
    Ok(TestResult {
        backend: profile.backend,
        client: crate::driver(profile.backend)
            .availability()
            .unwrap_or_default(),
        server_version,
        password_source: opened.password_source,
        connect_ms: opened.connect_ms,
        ping_ms,
    })
}

/// The schemas of `profile` as edited in the IDE, connecting as
/// [`test_profile`] does.
pub async fn profile_schemas(
    svc: &DbService,
    name: &str,
    profile: &Profile,
    password: Option<Secret>,
) -> Result<Vec<SchemaInfo>, Error> {
    let opened = open_profile(svc, name, profile, password).await?;
    let r = schema::introspector(opened.conn.as_ref()).schemas().await;
    opened.conn.close().await.ok();
    r
}

/// Opens `profile` (saved or not) under the name `name` with the given
/// password, else the one found the usual way.
pub async fn open_profile(
    svc: &DbService,
    name: &str,
    profile: &Profile,
    password: Option<Secret>,
) -> Result<Opened, Error> {
    profile.validate()?;
    let (params, password_source) = match password {
        Some(password) => {
            let wallet = svc.secrets_for(name, profile).wallet_password;
            (profile.to_params(password, wallet)?, SecretSource::Prompt)
        }
        None => svc.params_for(name, profile)?,
    };
    let started = Instant::now();
    let conn = crate::driver(profile.backend).connect(&params).await?;
    Ok(Opened {
        conn,
        name: name.to_owned(),
        profile: profile.clone(),
        password_source,
        connect_ms: millis(started),
    })
}

pub async fn query(svc: &DbService, args: &QueryArgs) -> Result<QueryOutput, Error> {
    let Opened {
        conn,
        profile,
        connect_ms,
        ..
    } = svc.open(&args.target).await?;
    let timeout = args
        .timeout_secs
        .or(profile.query_timeout_secs)
        .filter(|s| *s > 0)
        .map(Duration::from_secs);
    let opts = ExecOptions {
        timeout,
        fetch_size: args.max_rows.clamp(1, 500),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let outcome = conn.execute(&args.sql, &args.params, &opts).await;
    let result = match outcome {
        Ok(Outcome::Affected(n)) => Ok((
            ResultSet {
                rows_affected: n,
                ..ResultSet::default()
            },
            millis(started),
        )),
        Ok(Outcome::Rows(mut cursor)) => {
            // Time the first page separately, then collect the rest.
            let first = cursor
                .next_page(opts.fetch_size.min(args.max_rows.max(1)))
                .await;
            let first_ms = millis(started);
            match first {
                Ok(page) => {
                    let exhausted = cursor.is_exhausted() || page.is_empty();
                    let columns = cursor.columns().to_vec();
                    let mut rs = if exhausted || page.len() >= args.max_rows {
                        let truncated = !exhausted;
                        cursor.close().await.ok();
                        ResultSet {
                            columns,
                            rows: Vec::new(),
                            rows_affected: 0,
                            truncated,
                        }
                    } else {
                        collect(cursor, args.max_rows - page.len(), opts.fetch_size).await?
                    };
                    let mut rows = page;
                    rows.append(&mut rs.rows);
                    rows.truncate(args.max_rows);
                    rs.rows = rows;
                    Ok((rs, first_ms))
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    let total_ms = millis(started);
    conn.close().await.ok();
    let (result, first_page_ms) = result?;
    Ok(QueryOutput {
        backend: profile.backend,
        result,
        timing: Timing {
            connect_ms,
            first_page_ms,
            total_ms,
        },
    })
}

pub async fn schema_list(svc: &DbService, target: &Target) -> Result<Vec<SchemaInfo>, Error> {
    let opened = svc.open(target).await?;
    let r = schema::introspector(opened.conn.as_ref()).schemas().await;
    opened.conn.close().await.ok();
    r
}

pub async fn schema_objects(svc: &DbService, args: &ObjectsArgs) -> Result<Vec<ObjectInfo>, Error> {
    let opened = svc.open(&args.target).await?;
    let intro = schema::introspector(opened.conn.as_ref());
    let r = async {
        let schema = match &args.schema {
            Some(s) => s.clone(),
            None => intro.current_schema().await?,
        };
        intro.objects(&schema, &args.kinds).await
    }
    .await;
    drop(intro);
    opened.conn.close().await.ok();
    r
}

pub async fn schema_table(svc: &DbService, args: &TableArgs) -> Result<TableInfo, Error> {
    let opened = svc.open(&args.target).await?;
    let intro = schema::introspector(opened.conn.as_ref());
    let r = async {
        let schema = match &args.schema {
            Some(s) => s.clone(),
            None => intro.current_schema().await?,
        };
        let columns = intro.columns(&schema, &args.table).await?;
        if columns.is_empty() {
            return Err(Error::query(format!(
                "table {schema}.{} not found or not visible",
                args.table
            )));
        }
        let mut errors = Vec::new();
        let indexes = intro
            .indexes(&schema, &args.table)
            .await
            .map_err(|e| errors.push(format!("indexes: {}", e.message)))
            .ok();
        let constraints = intro
            .constraints(&schema, &args.table)
            .await
            .map_err(|e| errors.push(format!("constraints: {}", e.message)))
            .ok();
        Ok(TableInfo {
            schema,
            table: args.table.clone(),
            columns,
            indexes,
            constraints,
            errors,
        })
    }
    .await;
    drop(intro);
    opened.conn.close().await.ok();
    r
}

pub async fn schema_source(svc: &DbService, args: &SourceArgs) -> Result<Option<String>, Error> {
    let opened = svc.open(&args.target).await?;
    let intro = schema::introspector(opened.conn.as_ref());
    let r = async {
        let schema = match &args.schema {
            Some(s) => s.clone(),
            None => intro.current_schema().await?,
        };
        intro.source(&schema, &args.name, args.kind).await
    }
    .await;
    drop(intro);
    opened.conn.close().await.ok();
    r
}

pub async fn diagnose(svc: &DbService, args: &DiagnoseArgs) -> Report {
    diagnose::diagnose(svc, &args.target, &args.options).await
}

// ---------- registry ----------

/// Map a database error to a command error with a stable code (`db.auth`).
pub fn command_error(e: Error) -> CommandError {
    let kind = serde_json::to_value(e.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "internal".into());
    let message = match &e.code {
        Some(code) if !e.message.contains(code.as_str()) => format!("[{code}] {}", e.message),
        _ => e.message,
    };
    CommandError::new(format!("db.{kind}"), message)
}

macro_rules! meta {
    ($id:literal, $title:literal, $summary:literal, $example:literal) => {
        CommandMeta {
            id: $id,
            title: $title,
            summary: $summary,
            args_example: $example,
        }
    };
}

#[derive(Deserialize)]
struct NoArgs {}

/// Register every `db.*` command.
pub fn register(registry: &mut Registry, svc: Arc<DbService>) {
    registry.register(typed(
        meta!(
            "db.backends",
            "Database Backends",
            "Lists database backends and whether their client side is available.",
            "{}"
        ),
        |_: NoArgs| async move { Ok::<_, CommandError>(backends()) },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.profile.list",
            "List Connection Profiles",
            "Lists saved connection profiles.",
            "{}"
        ),
        move |_: NoArgs| {
            let s = s.clone();
            async move { profile_list(&s).map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.profile.get",
            "Show Connection Profile",
            "Returns one saved profile (never contains secrets).",
            r#"{"name": "local-pg"}"#
        ),
        move |a: NameArgs| {
            let s = s.clone();
            async move {
                let file = s.profiles().map_err(command_error)?;
                file.get(&a.name).cloned().map_err(command_error)
            }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.profile.save",
            "Save Connection Profile",
            "Creates or replaces a profile.",
            r#"{"name": "local-pg", "profile": {"backend": "postgres", "host": "localhost", "port": 55432, "database": "pyoxide", "user": "pyoxide"}}"#
        ),
        move |a: SaveProfileArgs| {
            let s = s.clone();
            async move { s.save_profile(&a.name, a.profile).map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.profile.remove",
            "Remove Connection Profile",
            "Deletes a profile (its stored secrets stay until removed).",
            r#"{"name": "local-pg"}"#
        ),
        move |a: NameArgs| {
            let s = s.clone();
            async move { s.remove_profile(&a.name).map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.secret.status",
            "Secret Status",
            "Shows where a profile's password would come from, never the value.",
            r#"{"profile": "local-pg"}"#
        ),
        move |a: SecretArgs| {
            let s = s.clone();
            async move { secret_status(&s, &a.profile).map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.secret.set",
            "Store Password",
            "Asks for a password through the front-end and stores it in the secret store.",
            r#"{"profile": "local-pg", "kind": "password"}"#
        ),
        move |a: SecretArgs| {
            let s = s.clone();
            async move { secret_set(&s, &a.profile, a.kind).map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.secret.delete",
            "Forget Password",
            "Removes a stored password.",
            r#"{"profile": "local-pg", "kind": "password"}"#
        ),
        move |a: SecretArgs| {
            let s = s.clone();
            async move { secret_delete(&s, &a.profile, a.kind).map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.connection.test",
            "Test Connection",
            "Connects, pings and reports the server version.",
            r#"{"profile": "local-pg"}"#
        ),
        move |a: TargetArgs| {
            let s = s.clone();
            async move { connection_test(&s, &a.target).await.map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.query",
            "Run SQL",
            "Runs one statement and returns up to max_rows typed rows.",
            r#"{"profile": "local-pg", "sql": "select 1", "max_rows": 500}"#
        ),
        move |a: QueryArgs| {
            let s = s.clone();
            async move { query(&s, &a).await.map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.schema.list",
            "List Schemas",
            "Lists schemas visible to the session.",
            r#"{"profile": "local-pg"}"#
        ),
        move |a: TargetArgs| {
            let s = s.clone();
            async move { schema_list(&s, &a.target).await.map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.schema.objects",
            "List Schema Objects",
            "Lists tables, views, routines and other objects of a schema.",
            r#"{"profile": "local-pg", "schema": "public", "kinds": ["table", "view"]}"#
        ),
        move |a: ObjectsArgs| {
            let s = s.clone();
            async move { schema_objects(&s, &a).await.map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.schema.table",
            "Describe Table",
            "Columns, indexes and constraints of a table.",
            r#"{"profile": "local-pg", "table": "orders"}"#
        ),
        move |a: TableArgs| {
            let s = s.clone();
            async move { schema_table(&s, &a).await.map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.schema.source",
            "Show Object Source",
            "Source or DDL of a view, routine, package or trigger.",
            r#"{"profile": "local-pg", "name": "active_orders", "kind": "view"}"#
        ),
        move |a: SourceArgs| {
            let s = s.clone();
            async move { schema_source(&s, &a).await.map_err(command_error) }
        },
    ));
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "db.import.pycharm",
            "Import PyCharm Data Sources",
            "Creates profiles from a PyCharm project's data sources and moves their passwords into the secret store; dry_run reports only.",
            r#"{"project": "/path/to/project", "dry_run": true, "allow_os_store": false}"#
        ),
        move |a: crate::import::PycharmImportArgs| {
            let s = s.clone();
            async move {
                tokio::task::spawn_blocking(move || crate::import::import_pycharm(&s, &a))
                    .await
                    .map_err(|e| CommandError::new("internal", e.to_string()))?
                    .map_err(command_error)
            }
        },
    ));
    let s = svc;
    registry.register(typed(
        meta!("db.diagnose", "Diagnose Connection", "Checks a connection step by step and explains the first failure; never prints secrets.", r#"{"profile": "local-pg", "deep": false, "share": true, "try_all": false}"#),
        move |a: DiagnoseArgs| {
            let s = s.clone();
            async move { Ok::<_, CommandError>(diagnose(&s, &a).await) }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrets::MemoryStore;
    use serde_json::json;

    fn registry(dir: &std::path::Path) -> Registry {
        let svc = Arc::new(DbService::new(
            dir.join("p.toml"),
            Arc::new(MemoryStore::default()),
            None,
            Arc::new(|_| None),
        ));
        let mut r = Registry::new();
        register(&mut r, svc);
        r
    }

    #[test]
    fn ids_are_stable() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<&str> = registry(dir.path()).list().iter().map(|m| m.id).collect();
        for id in [
            "db.backends",
            "db.query",
            "db.diagnose",
            "db.profile.save",
            "db.secret.set",
        ] {
            assert!(ids.contains(&id), "{id}");
        }
        assert!(ids.iter().all(|id| id.starts_with("db.")));
    }

    #[tokio::test]
    async fn profile_round_trip_through_registry() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        r.invoke(
            "db.profile.save",
            json!({"name": "scratch", "profile": {"backend": "sqlite", "path": ":memory:"}}),
        )
        .await
        .unwrap();
        let list = r.invoke("db.profile.list", json!({})).await.unwrap();
        assert_eq!(list[0]["name"], "scratch");
        let e = r
            .invoke("db.profile.get", json!({"name": "missing"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "db.config");
        let e = r
            .invoke("db.secret.set", json!({"profile": "scratch"}))
            .await
            .unwrap_err();
        assert!(e.message.contains("prompt"), "{e}");
    }

    #[test]
    fn error_codes() {
        let e = command_error(db_core::oracle::error_from_message(
            "ORA-01017: invalid",
            ErrorKind::Connect,
        ));
        assert_eq!(e.code, "db.auth");
        let e = command_error(Error::new(ErrorKind::Auth, "bad password").with_code("28P01"));
        assert_eq!(e.message, "[28P01] bad password");
    }
}
