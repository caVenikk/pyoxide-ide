//! Import of PyCharm data sources as connection profiles, with their
//! passwords moved from the IDE's credential store into the configured
//! secret store (ADR 0004). A dry run reports what would happen and never
//! keeps a secret value.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use db_core::{Backend, Error, Result, TlsMode};
use serde::{Deserialize, Serialize};
use settings_import::pycharm::{
    self, BackendHint, CredentialNaming, ImportedDataSource, NamingKey, PasswordLookup,
};

use crate::profiles::{Profile, TlsProfile};
use crate::secrets::secret_service;
use crate::service::DbService;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PycharmImportArgs {
    /// Project directory (or its `.idea`).
    pub project: PathBuf,
    /// IDE config directory; default: the newest PyCharm one found.
    pub ide_config: Option<PathBuf>,
    /// Allow reading the OS credential store the IDE uses (the OS may ask).
    pub allow_os_store: bool,
    /// Naming of the IDE's credential entries (default `DB` + uuid).
    pub naming_subsystem: Option<String>,
    pub naming_by_name: bool,
    /// Report only; write nothing.
    pub dry_run: bool,
    /// Replace profiles that already exist.
    pub overwrite: bool,
    /// Skip passwords entirely.
    pub no_passwords: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    Created,
    Replaced,
    WouldCreate,
    WouldReplace,
    Exists,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportEntry {
    pub data_source: String,
    pub profile: String,
    pub backend: Option<Backend>,
    pub target: String,
    pub status: EntryStatus,
    /// `found`, `stored`, `not found`, `not needed`, `skipped` or an error.
    pub password: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub dry_run: bool,
    pub profiles_file: String,
    pub secret_store: String,
    pub ide_credentials: Option<String>,
    pub entries: Vec<ImportEntry>,
    /// IDE credential titles related to the data sources (dry run, KeePass).
    pub related_titles: Vec<String>,
    pub notes: Vec<String>,
}

/// A profile name from a data source name: letters, digits, `-`, `_`, `.`.
pub fn profile_name(name: &str, taken: &BTreeSet<String>) -> String {
    let mut base: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let mut base = base.trim_matches('-').to_owned();
    base.truncate(56);
    if base.is_empty() {
        base = "imported".into();
    }
    let mut candidate = base.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

/// Map an imported data source to a profile.
pub fn to_profile(ds: &ImportedDataSource) -> std::result::Result<Profile, String> {
    let parsed = ds.parsed.as_ref().ok_or_else(|| {
        ds.reason
            .clone()
            .unwrap_or_else(|| "unsupported data source".into())
    })?;
    let backend = match parsed.backend_hint {
        BackendHint::Oracle => Backend::OracleThin,
        BackendHint::Postgres => Backend::Postgres,
        BackendHint::Sqlite => Backend::Sqlite,
    };
    let mut p = Profile::new(backend);
    p.host = parsed.host.clone().unwrap_or_default();
    p.port = parsed.port;
    p.service = parsed.service.clone();
    p.sid = parsed.sid.clone();
    p.dsn = parsed.dsn.clone();
    p.database = parsed.database.clone();
    p.path = parsed.path.clone();
    p.user = ds
        .user
        .clone()
        .or_else(|| parsed.user.clone())
        .unwrap_or_default();
    p.read_only = ds.read_only;
    if let Some(mode) = &parsed.tls_mode {
        let mode: TlsMode = serde_json::from_value(serde_json::Value::String(mode.clone()))
            .map_err(|_| format!("unknown TLS mode {mode:?}"))?;
        if mode != TlsMode::Disable {
            p.tls = Some(TlsProfile {
                mode,
                ..TlsProfile::default()
            });
        }
    }
    p.validate().map_err(|e| e.message)?;
    Ok(p)
}

fn naming(args: &PycharmImportArgs) -> CredentialNaming {
    let mut n = CredentialNaming::default();
    if let Some(s) = &args.naming_subsystem {
        n.subsystem = s.clone();
    }
    if args.naming_by_name {
        n.key = NamingKey::Name;
    }
    n
}

pub fn import_pycharm(svc: &DbService, args: &PycharmImportArgs) -> Result<ImportReport> {
    let sources = pycharm::load_project(&args.project)
        .map_err(|e| Error::config(format!("reading the project: {e}")))?;
    let mut file = svc.profiles()?;
    let mut taken: BTreeSet<String> = file.profiles.keys().cloned().collect();
    let mut notes = Vec::new();

    // Credentials of the IDE.
    let credential = if args.no_passwords {
        None
    } else {
        let dir = match &args.ide_config {
            Some(d) => Some(d.clone()),
            None => secrets::intellij::pycharm_config_dirs()
                .into_iter()
                .next()
                .map(|d| d.path),
        };
        match dir {
            None => {
                notes.push(
                    "no PyCharm configuration directory found; passwords not imported".into(),
                );
                None
            }
            Some(dir) => {
                match pycharm::credential_source(&dir, svc.prompter.as_deref(), args.allow_os_store)
                {
                    Ok(c) => Some(c),
                    Err(e) => {
                        notes.push(format!("IDE credentials not available: {e}"));
                        None
                    }
                }
            }
        }
    };
    let naming = naming(args);
    let mut related_titles = Vec::new();
    let mut found_by_uuid: HashMap<String, String> = HashMap::new();
    let mut secrets_by_uuid = HashMap::new();
    if let Some(c) = &credential {
        if args.dry_run {
            let report = pycharm::dry_run(&sources, c, &naming);
            related_titles = report.related_titles.clone();
            notes.extend(report.notes.clone());
            for e in report.entries {
                let state = if !e.password_needed {
                    "not needed".to_owned()
                } else if e.password_found {
                    "found".to_owned()
                } else {
                    e.message.unwrap_or_else(|| "not found".into())
                };
                found_by_uuid.insert(e.uuid, state);
            }
        } else {
            for r in pycharm::lookup_passwords(&sources, c, &naming) {
                let state = match &r.lookup {
                    PasswordLookup::Found(_) => "found".to_owned(),
                    PasswordLookup::NotFound => "not found".to_owned(),
                    PasswordLookup::NotNeeded => "not needed".to_owned(),
                    PasswordLookup::Error(m) => m.clone(),
                };
                found_by_uuid.insert(r.uuid.clone(), state);
                if let PasswordLookup::Found(secret) = r.lookup {
                    secrets_by_uuid.insert(r.uuid, secret);
                }
            }
        }
    }

    let mut entries = Vec::new();
    let mut changed = false;
    for ds in &sources {
        let mut entry_notes = ds.notes.clone();
        if let Some(p) = &ds.parsed {
            entry_notes.extend(p.notes.clone());
        }
        let profile = match to_profile(ds) {
            Ok(p) => p,
            Err(reason) => {
                entries.push(ImportEntry {
                    data_source: ds.name.clone(),
                    profile: String::new(),
                    backend: None,
                    target: ds.jdbc_url.clone().unwrap_or_default(),
                    status: EntryStatus::Skipped,
                    password: "skipped".into(),
                    notes: {
                        entry_notes.insert(0, reason);
                        entry_notes
                    },
                });
                continue;
            }
        };
        // An existing profile of the same name is kept unless overwriting.
        let name = profile_name(&ds.name, &BTreeSet::new());
        let exists = file.profiles.contains_key(&name);
        if exists && !args.overwrite {
            entries.push(ImportEntry {
                data_source: ds.name.clone(),
                profile: name,
                backend: Some(profile.backend),
                target: profile.target(),
                status: EntryStatus::Exists,
                password: "skipped".into(),
                notes: entry_notes,
            });
            continue;
        }
        let name = if exists {
            name
        } else {
            profile_name(&ds.name, &taken)
        };
        let status = match (args.dry_run, exists) {
            (true, true) => EntryStatus::WouldReplace,
            (true, false) => EntryStatus::WouldCreate,
            (false, true) => EntryStatus::Replaced,
            (false, false) => EntryStatus::Created,
        };
        let mut password = found_by_uuid.get(&ds.uuid).cloned().unwrap_or_else(|| {
            if profile.needs_password() {
                "not imported"
            } else {
                "not needed"
            }
            .into()
        });
        if !args.dry_run {
            if let Some(secret) = secrets_by_uuid.get(&ds.uuid) {
                match svc.store.set(&secret_service(), &name, secret) {
                    Ok(()) => password = format!("stored in {}", svc.store.name()),
                    Err(e) => password = format!("not stored: {e}"),
                }
            }
            file.profiles.insert(name.clone(), profile.clone());
            changed = true;
        }
        taken.insert(name.clone());
        entries.push(ImportEntry {
            data_source: ds.name.clone(),
            profile: name,
            backend: Some(profile.backend),
            target: profile.target(),
            status,
            password,
            notes: entry_notes,
        });
    }
    if changed {
        file.save(&svc.profiles_path)?;
    }
    if sources.is_empty() {
        notes.push("no data sources found in the project".into());
    }
    Ok(ImportReport {
        dry_run: args.dry_run,
        profiles_file: svc.profiles_path.display().to_string(),
        secret_store: svc.store.name().to_owned(),
        ide_credentials: credential.map(|c| c.describe()),
        entries,
        related_titles,
        notes,
    })
}

/// Plain-text rendering for the CLI.
pub fn render(r: &ImportReport) -> String {
    let mut out = String::new();
    out.push_str(if r.dry_run {
        "Dry run: nothing written.\n"
    } else {
        ""
    });
    out.push_str(&format!("profiles: {}\n", r.profiles_file));
    out.push_str(&format!("passwords go to: {}\n", r.secret_store));
    out.push_str(&format!(
        "IDE credentials: {}\n",
        r.ide_credentials.as_deref().unwrap_or("not used")
    ));
    for e in &r.entries {
        let status = serde_json::to_value(&e.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let backend = e
            .backend
            .map(|b| b.to_string())
            .unwrap_or_else(|| "-".into());
        out.push_str(&format!(
            "- {} -> {} [{status}] {backend} {} (password: {})\n",
            e.data_source,
            if e.profile.is_empty() {
                "-"
            } else {
                &e.profile
            },
            e.target,
            e.password
        ));
        for n in &e.notes {
            out.push_str(&format!("    note: {n}\n"));
        }
    }
    if !r.related_titles.is_empty() {
        out.push_str("IDE credential entries that look related (titles only):\n");
        for t in &r.related_titles {
            out.push_str(&format!("  {t}\n"));
        }
    }
    for n in &r.notes {
        out.push_str(&format!("note: {n}\n"));
    }
    let found = r
        .entries
        .iter()
        .filter(|e| e.password == "found" || e.password.starts_with("stored"))
        .count();
    let needed = r
        .entries
        .iter()
        .filter(|e| e.password != "not needed" && e.password != "skipped")
        .count();
    out.push_str(&format!(
        "{} data source(s), passwords found {found} of {needed}",
        r.entries.len()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_safe_and_unique() {
        let mut taken = BTreeSet::new();
        assert_eq!(profile_name("Prod DB (main)", &taken), "Prod-DB-main");
        taken.insert("Prod-DB-main".to_owned());
        assert_eq!(profile_name("Prod DB (main)", &taken), "Prod-DB-main-2");
        assert_eq!(profile_name("@@@", &taken), "imported");
        assert!(crate::profiles::validate_name(&profile_name(&"x".repeat(100), &taken)).is_ok());
    }

    #[test]
    fn imports_a_project_without_passwords() {
        let dir = tempfile::tempdir().unwrap();
        let idea = dir.path().join("proj").join(".idea");
        std::fs::create_dir_all(&idea).unwrap();
        std::fs::write(
            idea.join("dataSources.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="DataSourceManagerImpl" format="xml" multifile-model="true">
    <data-source source="LOCAL" name="Local PG" uuid="11111111-2222-3333-4444-555555555555">
      <driver-ref>postgresql</driver-ref>
      <jdbc-driver>org.postgresql.Driver</jdbc-driver>
      <jdbc-url>jdbc:postgresql://db.example.com:5433/app?sslmode=require</jdbc-url>
    </data-source>
    <data-source source="LOCAL" name="Scratch" uuid="66666666-7777-8888-9999-000000000000">
      <driver-ref>sqlite.xerial</driver-ref>
      <jdbc-driver>org.sqlite.JDBC</jdbc-driver>
      <jdbc-url>jdbc:sqlite:$PROJECT_DIR$/data/app.db</jdbc-url>
    </data-source>
  </component>
</project>"#,
        )
        .unwrap();
        std::fs::write(
            idea.join("dataSources.local.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="dataSourceStorageLocal">
    <data-source name="Local PG" uuid="11111111-2222-3333-4444-555555555555">
      <secret-storage>master_key</secret-storage>
      <user-name>app_user</user-name>
    </data-source>
  </component>
</project>"#,
        )
        .unwrap();
        let svc = DbService::new(
            dir.path().join("p.toml"),
            std::sync::Arc::new(secrets::MemoryStore::default()),
            None,
            std::sync::Arc::new(|_| None),
        );
        let args = PycharmImportArgs {
            project: dir.path().join("proj"),
            no_passwords: true,
            ..Default::default()
        };
        let dry = import_pycharm(
            &svc,
            &PycharmImportArgs {
                dry_run: true,
                ..args.clone()
            },
        )
        .unwrap();
        assert_eq!(dry.entries.len(), 2, "{}", render(&dry));
        assert!(
            svc.profiles().unwrap().profiles.is_empty(),
            "dry run wrote profiles"
        );

        let report = import_pycharm(&svc, &args).unwrap();
        let file = svc.profiles().unwrap();
        let pg = file.get("Local-PG").unwrap();
        assert_eq!(pg.backend, Backend::Postgres);
        assert_eq!(pg.port, Some(5433));
        assert_eq!(pg.user, "app_user");
        assert_eq!(pg.tls_mode(), TlsMode::Require);
        let lite = file.get("Scratch").unwrap();
        assert!(lite.path.as_deref().unwrap().ends_with("data/app.db"));
        assert!(render(&report).contains("2 data source(s)"));

        // A second import keeps existing profiles unless asked to replace.
        let again = import_pycharm(&svc, &args).unwrap();
        assert!(matches!(again.entries[0].status, EntryStatus::Exists));
        let replaced = import_pycharm(
            &svc,
            &PycharmImportArgs {
                overwrite: true,
                ..args
            },
        )
        .unwrap();
        assert!(matches!(replaced.entries[0].status, EntryStatus::Replaced));
        assert_eq!(svc.profiles().unwrap().profiles.len(), 2);
    }
}
