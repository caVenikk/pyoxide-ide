//! Data sources of a PyCharm project.
//!
//! - `.idea/dataSources.xml`: shared settings (name, driver, JDBC URL,
//!   driver properties); often committed to version control.
//! - `.idea/dataSources.local.xml`: per-user settings (user name, how the
//!   password is stored, authentication method). Never holds passwords.
//! - `.idea/dataSources/<uuid>.xml`: the introspected schema model. It is
//!   read only if it carries connection settings, which current IDE
//!   versions do not write there.
//!
//! Both files list `data-source` elements with the same `uuid`; the local
//! values win where both have one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::jdbc::{BackendHint, ParsedUrl, UrlContext, is_secret_name, parse_jdbc_url};
use crate::ImportError;
use crate::xml::{self, Element};

pub const IDEA_DIR: &str = ".idea";
pub const DATA_SOURCES_FILE: &str = "dataSources.xml";
pub const DATA_SOURCES_LOCAL_FILE: &str = "dataSources.local.xml";
pub const DATA_SOURCES_DIR: &str = "dataSources";

/// One data source as configured in PyCharm. Plain data; mapping to a
/// connection profile is up to the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportedDataSource {
    pub uuid: String,
    pub name: String,
    /// The IDE's driver id, e.g. `postgresql`, `oracle`, `sqlite.xerial`.
    pub driver_ref: Option<String>,
    /// JDBC driver class.
    pub jdbc_driver: Option<String>,
    pub jdbc_url: Option<String>,
    pub user: Option<String>,
    pub read_only: bool,
    pub auto_commit: Option<bool>,
    /// Authentication method id, e.g. `no-auth`.
    pub auth_provider: Option<String>,
    /// How the IDE keeps the password, e.g. `master_key`.
    pub secret_storage: Option<String>,
    /// Driver properties (secret-looking ones are dropped).
    pub properties: BTreeMap<String, String>,
    /// Connection fields from the URL; `None` with a `reason` otherwise.
    pub parsed: Option<ParsedUrl>,
    pub reason: Option<String>,
    /// Data-source level remarks (values dropped, files merged).
    pub notes: Vec<String>,
}

impl ImportedDataSource {
    pub fn backend_hint(&self) -> Option<BackendHint> {
        self.parsed.as_ref().map(|p| p.backend_hint)
    }

    /// Whether a password is expected: not for SQLite, not when the
    /// authentication method is `no-auth`.
    pub fn needs_password(&self) -> bool {
        self.backend_hint() != Some(BackendHint::Sqlite)
            && self.auth_provider.as_deref() != Some("no-auth")
    }
}

/// The project directory for `path`, which may also point at `.idea` or
/// at a file inside it.
pub fn project_dir(path: &Path) -> PathBuf {
    for p in path.ancestors() {
        if p.file_name().is_some_and(|n| n == IDEA_DIR) {
            return p.parent().map(Path::to_path_buf).unwrap_or_default();
        }
    }
    path.to_path_buf()
}

fn read_optional(path: &Path) -> Result<Option<String>, ImportError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ImportError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Read the data sources of the project at `path` (the project directory
/// or its `.idea`). `$PROJECT_DIR$` expands to the project directory.
pub fn load_project(path: &Path) -> Result<Vec<ImportedDataSource>, ImportError> {
    let project = project_dir(path);
    let idea = project.join(IDEA_DIR);
    let shared_path = idea.join(DATA_SOURCES_FILE);
    let local_path = idea.join(DATA_SOURCES_LOCAL_FILE);
    let shared = read_optional(&shared_path)?;
    let local = read_optional(&local_path)?;
    if shared.is_none() && local.is_none() {
        return Err(ImportError::NotFound(format!(
            "no {DATA_SOURCES_FILE} or {DATA_SOURCES_LOCAL_FILE} in {}",
            idea.display()
        )));
    }
    let parse = |text: &Option<String>, path: &Path| -> Result<Option<Element>, ImportError> {
        text.as_deref()
            .map(|t| {
                xml::parse(t).map_err(|message| ImportError::Xml {
                    path: path.to_path_buf(),
                    message,
                })
            })
            .transpose()
    };
    let shared = parse(&shared, &shared_path)?;
    let local = parse(&local, &local_path)?;
    let mut sources = merge(shared.as_ref(), local.as_ref());
    for ds in &mut sources {
        merge_model_file(&idea, ds)?;
    }
    let home = dirs::home_dir();
    let ctx = UrlContext {
        project_dir: Some(&project),
        user_home: home.as_deref(),
    };
    for ds in &mut sources {
        finish(ds, &ctx);
    }
    Ok(sources)
}

/// Parse the two files from memory (either may be missing).
pub fn parse_data_sources(
    shared: Option<&str>,
    local: Option<&str>,
    ctx: &UrlContext<'_>,
) -> Result<Vec<ImportedDataSource>, ImportError> {
    let parse = |text: Option<&str>, file: &str| -> Result<Option<Element>, ImportError> {
        text.map(|t| {
            xml::parse(t).map_err(|message| ImportError::Xml {
                path: PathBuf::from(file),
                message,
            })
        })
        .transpose()
    };
    let shared = parse(shared, DATA_SOURCES_FILE)?;
    let local = parse(local, DATA_SOURCES_LOCAL_FILE)?;
    let mut sources = merge(shared.as_ref(), local.as_ref());
    for ds in &mut sources {
        finish(ds, ctx);
    }
    Ok(sources)
}

/// `data-source` elements of the component named `component` (any
/// component when none has that name).
fn data_sources<'a>(root: &'a Element, component: &str) -> Vec<&'a Element> {
    let named: Vec<&Element> = root
        .children_named("component")
        .filter(|c| c.attr("name") == Some(component))
        .flat_map(|c| c.children_named("data-source"))
        .collect();
    if !named.is_empty() {
        return named;
    }
    root.children_named("component")
        .flat_map(|c| c.children_named("data-source"))
        .collect()
}

fn flag(e: &Element, name: &str) -> Option<bool> {
    e.attr(name)
        .or_else(|| e.child_text(name))
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
}

fn text(e: Option<&Element>, name: &str) -> Option<String> {
    e.and_then(|e| e.child_text(name)).map(str::to_owned)
}

fn properties(e: Option<&Element>, out: &mut BTreeMap<String, String>, notes: &mut Vec<String>) {
    let Some(props) = e.and_then(|e| e.child("driver-properties")) else {
        return;
    };
    for p in props.children_named("property") {
        let Some(name) = p.attr("name").filter(|n| !n.is_empty()) else {
            continue;
        };
        if is_secret_name(name) {
            notes.push(format!(
                "driver property {name} holds a secret; it was not imported"
            ));
            continue;
        }
        out.insert(
            name.to_owned(),
            p.attr("value").unwrap_or_default().to_owned(),
        );
    }
}

fn combine(uuid: String, shared: Option<&Element>, local: Option<&Element>) -> ImportedDataSource {
    let pick = |name: &str| text(local, name).or_else(|| text(shared, name));
    let name = shared
        .and_then(|e| e.attr("name"))
        .or_else(|| local.and_then(|e| e.attr("name")))
        .unwrap_or_default()
        .to_owned();
    let mut notes = Vec::new();
    let mut props = BTreeMap::new();
    properties(shared, &mut props, &mut notes);
    properties(local, &mut props, &mut notes);
    let flag_of = |name: &str| {
        local
            .and_then(|e| flag(e, name))
            .or_else(|| shared.and_then(|e| flag(e, name)))
    };
    if shared.is_none() {
        notes.push(format!("defined only in {DATA_SOURCES_LOCAL_FILE}"));
    }
    ImportedDataSource {
        uuid,
        name,
        driver_ref: pick("driver-ref"),
        jdbc_driver: pick("jdbc-driver"),
        jdbc_url: text(shared, "jdbc-url").or_else(|| text(local, "jdbc-url")),
        user: pick("user-name"),
        read_only: flag_of("read-only").unwrap_or(false),
        auto_commit: flag_of("auto-commit"),
        auth_provider: pick("auth-provider"),
        secret_storage: pick("secret-storage"),
        properties: props,
        parsed: None,
        reason: None,
        notes,
    }
}

fn merge(shared: Option<&Element>, local: Option<&Element>) -> Vec<ImportedDataSource> {
    let shared_list = shared
        .map(|r| data_sources(r, "DataSourceManagerImpl"))
        .unwrap_or_default();
    let local_list = local
        .map(|r| data_sources(r, "dataSourceStorageLocal"))
        .unwrap_or_default();
    let key = |e: &Element| -> String {
        e.attr("uuid")
            .or_else(|| e.attr("name"))
            .unwrap_or_default()
            .to_owned()
    };
    let mut used = vec![false; local_list.len()];
    let mut out = Vec::new();
    for s in &shared_list {
        let k = key(s);
        let found = local_list.iter().position(|l| key(l) == k);
        if let Some(i) = found {
            used[i] = true;
        }
        let uuid = s.attr("uuid").unwrap_or_default().to_owned();
        out.push(combine(uuid, Some(s), found.map(|i| local_list[i])));
    }
    for (i, l) in local_list.iter().enumerate() {
        if !used[i] && l.child_text("jdbc-url").is_some() {
            let uuid = l.attr("uuid").unwrap_or_default().to_owned();
            out.push(combine(uuid, None, Some(l)));
        }
    }
    out
}

/// UUIDs name files: accept hex digits and dashes only.
fn safe_uuid(uuid: &str) -> bool {
    !uuid.is_empty() && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn find_data_source<'a>(e: &'a Element, uuid: &str) -> Option<&'a Element> {
    if e.name == "data-source" && e.attr("uuid") == Some(uuid) {
        return Some(e);
    }
    e.children.iter().find_map(|c| find_data_source(c, uuid))
}

/// Fill missing connection settings from `dataSources/<uuid>.xml`, but
/// only when that file is not the schema model (root `dataSource`).
fn merge_model_file(idea: &Path, ds: &mut ImportedDataSource) -> Result<(), ImportError> {
    if !safe_uuid(&ds.uuid) {
        return Ok(());
    }
    let path = idea.join(DATA_SOURCES_DIR).join(format!("{}.xml", ds.uuid));
    let Some(text) = read_optional(&path)? else {
        return Ok(());
    };
    let xml_err = |message| ImportError::Xml {
        path: path.clone(),
        message,
    };
    if xml::root_name(&text).map_err(xml_err)?.as_deref() == Some("dataSource") {
        return Ok(());
    }
    let root = xml::parse(&text).map_err(xml_err)?;
    let Some(e) = find_data_source(&root, &ds.uuid) else {
        return Ok(());
    };
    let mut merged = false;
    for (slot, name) in [
        (&mut ds.jdbc_url, "jdbc-url"),
        (&mut ds.driver_ref, "driver-ref"),
        (&mut ds.jdbc_driver, "jdbc-driver"),
        (&mut ds.user, "user-name"),
    ] {
        if slot.is_none()
            && let Some(v) = e.child_text(name)
        {
            *slot = Some(v.to_owned());
            merged = true;
        }
    }
    if merged {
        ds.notes.push(format!(
            "settings completed from {DATA_SOURCES_DIR}/{}.xml",
            ds.uuid
        ));
    }
    Ok(())
}

fn finish(ds: &mut ImportedDataSource, ctx: &UrlContext<'_>) {
    match ds.jdbc_url.as_deref() {
        None => ds.reason = Some("no JDBC URL".into()),
        Some(url) => match parse_jdbc_url(url, ctx) {
            Ok(parsed) => {
                if ds.user.is_none() {
                    ds.user = parsed.user.clone();
                }
                ds.parsed = Some(parsed);
            }
            Err(e) => ds.reason = Some(e.reason),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="DataSourceManagerImpl" format="xml" multifile-model="true">
    <data-source source="LOCAL" name="reports" read-only="true" uuid="937fe348-db15-4a11-b8ca-eaf20d71ef36">
      <driver-ref>postgresql</driver-ref>
      <synchronize>true</synchronize>
      <jdbc-driver>org.postgresql.Driver</jdbc-driver>
      <jdbc-url>jdbc:postgresql://pg.example.com:5432/reports?sslmode=require</jdbc-url>
      <driver-properties>
        <property name="ApplicationName" value="pycharm-import-test" />
        <property name="password" value="hunter2" />
      </driver-properties>
      <working-dir>$ProjectFileDir$</working-dir>
    </data-source>
    <data-source source="LOCAL" name="cache" uuid="43d34cc4-5b79-413e-9376-347d5bc984c0">
      <driver-ref>sqlite.xerial</driver-ref>
      <jdbc-url>jdbc:sqlite:$PROJECT_DIR$/cache.db</jdbc-url>
    </data-source>
  </component>
</project>"#;

    const LOCAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="dataSourceStorageLocal" created-in="PY-000.0">
    <data-source name="reports" uuid="937fe348-db15-4a11-b8ca-eaf20d71ef36">
      <secret-storage>master_key</secret-storage>
      <user-name>report_reader</user-name>
      <auto-commit>false</auto-commit>
    </data-source>
    <data-source name="cache" uuid="43d34cc4-5b79-413e-9376-347d5bc984c0">
      <auth-provider>no-auth</auth-provider>
    </data-source>
    <data-source name="stale" uuid="681c9f64-85e4-4f9f-b6dd-f8b7afdec4c4">
      <user-name>nobody</user-name>
    </data-source>
  </component>
</project>"#;

    fn ctx() -> UrlContext<'static> {
        UrlContext {
            project_dir: Some(Path::new("/work/demo")),
            user_home: None,
        }
    }

    #[test]
    fn merges_shared_and_local() {
        let all = parse_data_sources(Some(SHARED), Some(LOCAL), &ctx()).unwrap();
        assert_eq!(all.len(), 2, "local-only entries without a URL are skipped");
        let r = &all[0];
        assert_eq!(r.name, "reports");
        assert_eq!(r.uuid, "937fe348-db15-4a11-b8ca-eaf20d71ef36");
        assert_eq!(r.driver_ref.as_deref(), Some("postgresql"));
        assert_eq!(r.user.as_deref(), Some("report_reader"));
        assert!(r.read_only);
        assert_eq!(r.auto_commit, Some(false));
        assert_eq!(r.secret_storage.as_deref(), Some("master_key"));
        assert_eq!(
            r.properties.get("ApplicationName").map(String::as_str),
            Some("pycharm-import-test")
        );
        assert!(!r.properties.contains_key("password"));
        assert!(!format!("{r:?}").contains("hunter2"));
        assert_eq!(r.notes.len(), 1);
        let p = r.parsed.as_ref().unwrap();
        assert_eq!(p.host.as_deref(), Some("pg.example.com"));
        assert_eq!(p.tls_mode.as_deref(), Some("require"));
        assert!(r.needs_password());

        let c = &all[1];
        assert_eq!(c.backend_hint(), Some(BackendHint::Sqlite));
        assert_eq!(
            c.parsed.as_ref().unwrap().path.as_deref(),
            Some("/work/demo/cache.db")
        );
        assert_eq!(c.auth_provider.as_deref(), Some("no-auth"));
        assert!(!c.needs_password());
        assert!(!c.read_only);
        assert_eq!(c.auto_commit, None);
    }

    #[test]
    fn either_file_alone() {
        let shared_only = parse_data_sources(Some(SHARED), None, &ctx()).unwrap();
        assert_eq!(shared_only.len(), 2);
        assert_eq!(shared_only[0].user, None);
        let local_only = parse_data_sources(None, Some(LOCAL), &ctx()).unwrap();
        assert!(local_only.is_empty());
    }

    #[test]
    fn local_only_source_with_url_and_url_user() {
        let local = r#"<project version="4"><component name="dataSourceStorageLocal">
            <data-source name="adhoc" uuid="bb5e2e3a-0119-4ccd-b74b-b03098d88533">
              <jdbc-url>jdbc:oracle:thin:scott@db.example.com:1521:ORCL</jdbc-url>
            </data-source></component></project>"#;
        let all = parse_data_sources(None, Some(local), &ctx()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].user.as_deref(), Some("scott"));
        assert!(all[0].notes[0].contains("only in"));
    }

    #[test]
    fn unsupported_and_missing_urls() {
        let shared = r#"<project version="4"><component name="DataSourceManagerImpl">
            <data-source name="my" uuid="2840d347-dda4-4121-a53c-087ad27281eb">
              <driver-ref>mysql.8</driver-ref>
              <jdbc-url>jdbc:mysql://mysql.example.com:3306/shop</jdbc-url>
            </data-source>
            <data-source name="empty" uuid="97cb3ebe-bb3b-4519-b9f1-5bda800d99f5"/>
            </component></project>"#;
        let all = parse_data_sources(Some(shared), None, &ctx()).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].parsed.is_none());
        assert!(
            all[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("unsupported driver")
        );
        assert_eq!(all[1].reason.as_deref(), Some("no JDBC URL"));
        assert!(all[1].needs_password());
    }

    #[test]
    fn malformed_xml_is_an_error() {
        let e = parse_data_sources(Some("<project><component>"), None, &ctx()).unwrap_err();
        assert!(matches!(e, ImportError::Xml { .. }), "{e}");
    }

    #[test]
    fn project_dir_from_any_path() {
        assert_eq!(project_dir(Path::new("/w/p")), PathBuf::from("/w/p"));
        assert_eq!(project_dir(Path::new("/w/p/.idea")), PathBuf::from("/w/p"));
        assert_eq!(
            project_dir(Path::new("/w/p/.idea/dataSources.xml")),
            PathBuf::from("/w/p")
        );
    }

    #[test]
    fn uuid_must_be_safe_for_file_names() {
        assert!(safe_uuid("937fe348-db15-4a11-b8ca-eaf20d71ef36"));
        assert!(!safe_uuid("../../etc/passwd"));
        assert!(!safe_uuid(""));
    }
}
