//! Passwords of imported data sources, read from the credential store the
//! IDE uses: its KeePass database or the OS credential store.
//!
//! The IDE stores each credential under a service name
//! `IntelliJ Platform <subsystem> — <key>` (open-source naming scheme).
//! The subsystem and key the database plugin uses are not open source:
//! [`CredentialNaming::default`] is a documented guess (`DB`, data source
//! UUID). A dry run lists the IDE entry titles that mention a data source
//! UUID, so the naming can be confirmed without showing any password.
//!
//! Values only travel as [`Secret`]; the dry-run report holds booleans and
//! messages only.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrets::intellij::{self, IdeKeePass, IdeStore, Platform};
use secrets::{KeyringStore, Prompter, Secret, SecretErrorKind, SecretStore};
use serde::Serialize;

use super::datasources::ImportedDataSource;

/// Default subsystem in the service names of data source passwords (a
/// guess, see the module docs).
pub const DEFAULT_SUBSYSTEM: &str = "DB";

/// Which data source field completes the service name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NamingKey {
    #[default]
    Uuid,
    Name,
}

/// How the IDE names the credential of a data source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialNaming {
    pub subsystem: String,
    pub key: NamingKey,
}

impl Default for CredentialNaming {
    fn default() -> Self {
        Self {
            subsystem: DEFAULT_SUBSYSTEM.to_owned(),
            key: NamingKey::Uuid,
        }
    }
}

impl CredentialNaming {
    pub fn service_name(&self, ds: &ImportedDataSource) -> String {
        let key = match self.key {
            NamingKey::Uuid => &ds.uuid,
            NamingKey::Name => &ds.name,
        };
        intellij::generate_service_name(&self.subsystem, key)
    }

    /// Common prefix of the service names, e.g. `IntelliJ Platform DB `.
    pub fn prefix(&self) -> String {
        format!("{} {} ", intellij::SERVICE_NAME_PREFIX, self.subsystem)
    }
}

/// Where the IDE keeps its passwords.
pub enum IdeCredentialSource {
    /// The IDE's KeePass database and its master password.
    KeePass { path: PathBuf, master: Secret },
    /// The OS credential store (pass `Arc::new(KeyringStore)`). Only with
    /// the user's consent; the OS may ask again per entry.
    OsStore(Arc<dyn SecretStore>),
}

impl std::fmt::Debug for IdeCredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeePass { path, master } => f
                .debug_struct("KeePass")
                .field("path", path)
                .field("master", master)
                .finish(),
            Self::OsStore(s) => f.debug_tuple("OsStore").field(&s.name()).finish(),
        }
    }
}

impl IdeCredentialSource {
    /// Human-readable description, e.g. `KeePass database /path/c.kdbx`.
    pub fn describe(&self) -> String {
        match self {
            Self::KeePass { path, .. } => format!("KeePass database {}", path.display()),
            Self::OsStore(s) => s.name().to_owned(),
        }
    }
}

/// Outcome for one data source.
#[derive(Debug)]
pub enum PasswordLookup {
    Found(Secret),
    NotFound,
    /// SQLite or no authentication: nothing to look up.
    NotNeeded,
    /// The message never contains secret values.
    Error(String),
}

#[derive(Debug)]
pub struct PasswordResult {
    pub uuid: String,
    pub name: String,
    pub service_name: String,
    pub lookup: PasswordLookup,
}

enum Opened<'a> {
    KeePass(Result<Box<IdeKeePass>, String>),
    Os(&'a dyn SecretStore),
}

fn open(source: &IdeCredentialSource) -> Opened<'_> {
    match source {
        IdeCredentialSource::KeePass { path, master } => Opened::KeePass(
            IdeKeePass::open(path, master)
                .map(Box::new)
                .map_err(|e| e.to_string()),
        ),
        IdeCredentialSource::OsStore(store) => Opened::Os(store.as_ref()),
    }
}

fn lookup_one(
    opened: &Opened<'_>,
    ds: &ImportedDataSource,
    naming: &CredentialNaming,
) -> PasswordResult {
    let service_name = naming.service_name(ds);
    let user = ds.user.as_deref().filter(|u| !u.is_empty());
    let lookup = if !ds.needs_password() {
        PasswordLookup::NotNeeded
    } else {
        match opened {
            Opened::KeePass(Err(e)) => PasswordLookup::Error(e.clone()),
            Opened::KeePass(Ok(db)) => match db.get(&service_name, user) {
                Some(c) => match c.password {
                    Some(p) if !p.is_empty() => PasswordLookup::Found(p),
                    _ => PasswordLookup::NotFound,
                },
                None => PasswordLookup::NotFound,
            },
            Opened::Os(store) => match user {
                None => PasswordLookup::Error(
                    "no user name: the OS store is searched by service and user".into(),
                ),
                Some(u) => match intellij::read_os_store(*store, &service_name, u) {
                    Ok(Some(p)) if !p.is_empty() => PasswordLookup::Found(p),
                    Ok(_) => PasswordLookup::NotFound,
                    Err(e) => PasswordLookup::Error(e.to_string()),
                },
            },
        }
    };
    PasswordResult {
        uuid: ds.uuid.clone(),
        name: ds.name.clone(),
        service_name,
        lookup,
    }
}

/// Look up the password of every data source.
pub fn lookup_passwords(
    sources: &[ImportedDataSource],
    source: &IdeCredentialSource,
    naming: &CredentialNaming,
) -> Vec<PasswordResult> {
    let opened = open(source);
    sources
        .iter()
        .map(|ds| lookup_one(&opened, ds, naming))
        .collect()
}

/// Dry-run line for one data source. No secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunEntry {
    pub name: String,
    pub uuid: String,
    pub service_name: String,
    pub user_known: bool,
    pub password_needed: bool,
    pub password_found: bool,
    pub message: Option<String>,
}

/// What an import would find. No secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunReport {
    /// Description of the IDE store.
    pub store: String,
    pub entries: Vec<DryRunEntry>,
    /// KeePass only: IDE entry titles (service names) that contain a data
    /// source UUID or start with the naming prefix. Shows the naming the
    /// IDE really uses.
    pub related_titles: Vec<String>,
    pub notes: Vec<String>,
}

impl DryRunReport {
    pub fn found(&self) -> usize {
        self.entries.iter().filter(|e| e.password_found).count()
    }

    pub fn needed(&self) -> usize {
        self.entries.iter().filter(|e| e.password_needed).count()
    }
}

impl std::fmt::Display for DryRunReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "store: {}", self.store)?;
        for e in &self.entries {
            let state = match (e.password_needed, e.password_found) {
                (false, _) => "not needed",
                (true, true) => "found",
                (true, false) => "not found",
            };
            write!(f, "  {} [{}]: password {state}", e.name, e.uuid)?;
            if !e.user_known && e.password_needed {
                write!(f, ", no user name")?;
            }
            if let Some(m) = &e.message {
                write!(f, " ({m})")?;
            }
            writeln!(f)?;
        }
        writeln!(f, "found {} of {} needed", self.found(), self.needed())?;
        if !self.related_titles.is_empty() {
            writeln!(f, "matching IDE entries:")?;
            for t in &self.related_titles {
                writeln!(f, "  {t}")?;
            }
        }
        for n in &self.notes {
            writeln!(f, "note: {n}")?;
        }
        Ok(())
    }
}

/// Report what an import would find. Secrets are dropped right away.
pub fn dry_run(
    sources: &[ImportedDataSource],
    source: &IdeCredentialSource,
    naming: &CredentialNaming,
) -> DryRunReport {
    let opened = open(source);
    let entries = sources
        .iter()
        .map(|ds| {
            let r = lookup_one(&opened, ds, naming);
            let (found, message) = match r.lookup {
                PasswordLookup::Found(_) => (true, None),
                PasswordLookup::NotFound => (false, None),
                PasswordLookup::NotNeeded => (false, None),
                PasswordLookup::Error(m) => (false, Some(m)),
            };
            DryRunEntry {
                name: r.name,
                uuid: r.uuid,
                service_name: r.service_name,
                user_known: ds.user.as_deref().is_some_and(|u| !u.is_empty()),
                password_needed: ds.needs_password(),
                password_found: found,
                message,
            }
        })
        .collect();
    let mut notes = Vec::new();
    let related_titles = match &opened {
        Opened::KeePass(Ok(db)) => {
            let prefix = naming.prefix();
            db.titles_with_prefix("")
                .into_iter()
                .filter(|t| {
                    t.starts_with(&prefix)
                        || sources
                            .iter()
                            .any(|ds| !ds.uuid.is_empty() && t.contains(&ds.uuid))
                })
                .collect()
        }
        Opened::KeePass(Err(_)) => Vec::new(),
        Opened::Os(_) => {
            notes.push(
                "the OS credential store cannot be listed; only the expected service names \
                 were tried"
                    .into(),
            );
            Vec::new()
        }
    };
    DryRunReport {
        store: source.describe(),
        entries,
        related_titles,
        notes,
    }
}

/// Work out where the IDE whose config directory is `config_dir` keeps
/// passwords. For KeePass the master key saved by the IDE is used; when it
/// cannot be read or does not open the database, `prompter` asks for the
/// master password. The OS store is used only with `allow_os_store` (the
/// user's consent).
pub fn credential_source(
    config_dir: &Path,
    prompter: Option<&dyn Prompter>,
    allow_os_store: bool,
) -> Result<IdeCredentialSource, String> {
    credential_source_for(
        config_dir,
        prompter,
        allow_os_store,
        Platform::current(),
        Arc::new(KeyringStore),
    )
}

fn credential_source_for(
    config_dir: &Path,
    prompter: Option<&dyn Prompter>,
    allow_os_store: bool,
    platform: Platform,
    os_store: Arc<dyn SecretStore>,
) -> Result<IdeCredentialSource, String> {
    let settings = intellij::read_password_safe_settings(config_dir).map_err(|e| e.to_string())?;
    match intellij::resolve_store(config_dir, &settings, platform) {
        IdeStore::KeePass { db_file, .. } => {
            let ask = |why: String| -> Result<Secret, String> {
                let p = prompter
                    .ok_or_else(|| format!("{why}; the KeePass master password is needed"))?;
                p.secret(&format!(
                    "PyCharm KeePass master password ({}): ",
                    db_file.display()
                ))
                .ok_or_else(|| "no master password entered".to_owned())
            };
            let master = match intellij::read_main_key(config_dir) {
                Ok(Some(key)) => match IdeKeePass::open(&db_file, &key) {
                    Ok(_) => key,
                    Err(e) if e.kind == SecretErrorKind::Locked => ask(e.to_string())?,
                    Err(e) => return Err(e.to_string()),
                },
                Ok(None) => ask("the IDE saved no master key".into())?,
                Err(e) => ask(e.to_string())?,
            };
            Ok(IdeCredentialSource::KeePass {
                path: db_file,
                master,
            })
        }
        IdeStore::OsStore => {
            intellij::os_store_support(platform)?;
            if !allow_os_store {
                return Err(
                    "the IDE keeps passwords in the OS credential store; reading them \
                            needs explicit consent"
                        .into(),
                );
            }
            Ok(IdeCredentialSource::OsStore(os_store))
        }
        IdeStore::MemoryOnly => Err("the IDE is set to not save passwords".into()),
        IdeStore::Unsupported(what) => Err(format!("unsupported IDE setting: {what}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use secrets::MemoryStore;

    use super::*;
    use crate::pycharm::jdbc::{BackendHint, ParsedUrl, UrlContext, parse_jdbc_url};

    fn ds(uuid: &str, name: &str, user: Option<&str>, url: &str) -> ImportedDataSource {
        let parsed: Option<ParsedUrl> = parse_jdbc_url(url, &UrlContext::default()).ok();
        ImportedDataSource {
            uuid: uuid.into(),
            name: name.into(),
            driver_ref: None,
            jdbc_driver: None,
            jdbc_url: Some(url.into()),
            user: user.map(Into::into),
            read_only: false,
            auto_commit: None,
            auth_provider: None,
            secret_storage: Some("master_key".into()),
            properties: BTreeMap::new(),
            parsed,
            reason: None,
            notes: Vec::new(),
        }
    }

    fn sources() -> Vec<ImportedDataSource> {
        vec![
            ds(
                "937fe348-db15-4a11-b8ca-eaf20d71ef36",
                "reports",
                Some("report_reader"),
                "jdbc:postgresql://pg.example.com/reports",
            ),
            ds(
                "43d34cc4-5b79-413e-9376-347d5bc984c0",
                "cache",
                None,
                "jdbc:sqlite:/tmp/cache.db",
            ),
            ds(
                "681c9f64-85e4-4f9f-b6dd-f8b7afdec4c4",
                "billing",
                None,
                "jdbc:oracle:thin:@//db.example.com:1521/BILL",
            ),
        ]
    }

    #[test]
    fn naming() {
        let s = &sources()[0];
        let n = CredentialNaming::default();
        assert_eq!(
            n.service_name(s),
            "IntelliJ Platform DB \u{2014} 937fe348-db15-4a11-b8ca-eaf20d71ef36"
        );
        assert_eq!(n.prefix(), "IntelliJ Platform DB ");
        let by_name = CredentialNaming {
            subsystem: "Database".into(),
            key: NamingKey::Name,
        };
        assert_eq!(
            by_name.service_name(s),
            "IntelliJ Platform Database \u{2014} reports"
        );
        assert_eq!(s.backend_hint(), Some(BackendHint::Postgres));
    }

    #[test]
    fn os_store_lookup_and_dry_run() {
        let store = Arc::new(MemoryStore::default());
        let all = sources();
        let naming = CredentialNaming::default();
        store
            .set(
                &naming.service_name(&all[0]),
                "report_reader",
                &Secret::new("pg-secret"),
            )
            .unwrap();
        let source = IdeCredentialSource::OsStore(store.clone());
        let results = lookup_passwords(&all, &source, &naming);
        assert!(
            matches!(&results[0].lookup, PasswordLookup::Found(p) if p.expose() == "pg-secret")
        );
        assert!(matches!(results[1].lookup, PasswordLookup::NotNeeded));
        assert!(
            matches!(&results[2].lookup, PasswordLookup::Error(m) if m.contains("no user name"))
        );

        let report = dry_run(&all, &source, &naming);
        assert_eq!(report.found(), 1);
        assert_eq!(report.needed(), 2);
        assert_eq!(report.store, "memory (not saved)");
        assert_eq!(report.notes.len(), 1);
        let text = report.to_string();
        let json = serde_json::to_string(&report).unwrap();
        for out in [
            &text,
            &json,
            &format!("{report:?}"),
            &format!("{results:?}"),
        ] {
            assert!(!out.contains("pg-secret"), "{out}");
        }
        assert!(text.contains("reports [937fe348-db15-4a11-b8ca-eaf20d71ef36]: password found"));
    }

    #[test]
    fn keepass_errors_reach_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let source = IdeCredentialSource::KeePass {
            path: dir.path().join("missing.kdbx"),
            master: Secret::new("m"),
        };
        let results = lookup_passwords(&sources(), &source, &CredentialNaming::default());
        assert!(
            matches!(&results[0].lookup, PasswordLookup::Error(m) if m.contains("missing.kdbx"))
        );
        assert!(matches!(results[1].lookup, PasswordLookup::NotNeeded));
        assert!(!format!("{source:?}").contains("\"m\""));
    }

    struct Typed(&'static str);
    impl Prompter for Typed {
        fn secret(&self, _: &str) -> Option<Secret> {
            Some(Secret::new(self.0))
        }
    }

    #[test]
    fn credential_source_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let os: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());

        // No settings file: OS store on macOS, needs consent.
        let e = credential_source_for(dir.path(), None, false, Platform::MacOs, os.clone())
            .unwrap_err();
        assert!(e.contains("consent"), "{e}");
        let s = credential_source_for(dir.path(), None, true, Platform::MacOs, os.clone()).unwrap();
        assert!(matches!(s, IdeCredentialSource::OsStore(_)));
        let e =
            credential_source_for(dir.path(), None, true, Platform::Linux, os.clone()).unwrap_err();
        assert!(e.contains("Secret Service"), "{e}");

        // Windows default: KeePass; no c.pwd and no prompt.
        let e = credential_source_for(dir.path(), None, false, Platform::Windows, os.clone())
            .unwrap_err();
        assert!(e.contains("master password"), "{e}");
        let s = credential_source_for(
            dir.path(),
            Some(&Typed("typed")),
            false,
            Platform::Windows,
            os.clone(),
        )
        .unwrap();
        match s {
            IdeCredentialSource::KeePass { path, master } => {
                assert_eq!(path, dir.path().join("c.kdbx"));
                assert_eq!(master.expose(), "typed");
            }
            other => panic!("{other:?}"),
        }

        std::fs::create_dir_all(dir.path().join("options")).unwrap();
        std::fs::write(
            dir.path().join("options/security.xml"),
            r#"<application><component name="PasswordSafe"><option name="PROVIDER" value="MEMORY_ONLY"/></component></application>"#,
        )
        .unwrap();
        let e = credential_source_for(dir.path(), None, true, Platform::MacOs, os).unwrap_err();
        assert!(e.contains("not save"), "{e}");
    }
}
