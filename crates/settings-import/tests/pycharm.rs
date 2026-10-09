//! PyCharm import against synthetic fixtures. Nothing here reads the
//! user's real IDE settings, keychain or KeePass files.

use std::path::{Path, PathBuf};

use settings_import::ImportError;
use settings_import::pycharm::{BackendHint, load_project};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pycharm")
        .join(name)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A project directory with the fixture's `idea/` copied to `.idea/`
/// (`.idea` itself is git-ignored in this repository).
fn project(name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(name);
    copy_dir(&fixture(name).join("idea"), &dir.join(".idea"));
    (tmp, dir)
}

#[test]
fn oracle_and_postgres_project() {
    let (_tmp, dir) = project("oracle-pg");
    let all = load_project(&dir).unwrap();
    let names: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "billing",
            "legacy-sid",
            "reports-tns",
            "warehouse",
            "app-pg",
            "pg-ha",
            "shop-mysql"
        ]
    );
    let get = |n: &str| all.iter().find(|d| d.name == n).unwrap();

    let billing = get("billing");
    assert_eq!(billing.user.as_deref(), Some("app_user"));
    assert_eq!(billing.secret_storage.as_deref(), Some("master_key"));
    let p = billing.parsed.as_ref().unwrap();
    assert_eq!(p.backend_hint, BackendHint::Oracle);
    assert_eq!(
        (p.host.as_deref(), p.port, p.service.as_deref()),
        (Some("db.example.com"), Some(1521), Some("BILLPDB"))
    );
    assert!(billing.notes.is_empty(), "model file is not merged");

    let sid = get("legacy-sid").parsed.clone().unwrap();
    assert_eq!(sid.sid.as_deref(), Some("LEGACY"));

    let tns = get("reports-tns").parsed.clone().unwrap();
    assert_eq!(tns.dsn.as_deref(), Some("REPORTS_HIGH"));
    assert_eq!(
        tns.params.get("TNS_ADMIN").map(String::as_str),
        Some("/opt/oracle/wallets/reports")
    );

    let dwh = get("warehouse").parsed.clone().unwrap();
    assert!(dwh.dsn.as_deref().unwrap().starts_with("(DESCRIPTION="));
    assert_eq!(dwh.host.as_deref(), Some("dwh.example.com"));
    assert_eq!(dwh.port, Some(2484));
    assert_eq!(dwh.service.as_deref(), Some("DWH.example.com"));
    assert_eq!(dwh.tls_mode.as_deref(), Some("verify-ca"));

    let pg = get("app-pg");
    assert!(pg.read_only);
    assert_eq!(pg.auto_commit, Some(false));
    assert_eq!(pg.user.as_deref(), Some("app_user"));
    assert_eq!(
        pg.properties.get("ApplicationName").map(String::as_str),
        Some("synthetic-fixture")
    );
    assert!(!pg.properties.contains_key("sslpassword"));
    assert!(!pg.properties.keys().any(|k| k.contains("kubernetes")));
    let pgp = pg.parsed.as_ref().unwrap();
    assert_eq!(pgp.database.as_deref(), Some("app_db"));
    assert_eq!(pgp.tls_mode.as_deref(), Some("verify-full"));

    let ha = get("pg-ha");
    assert_eq!(ha.user, None);
    let hap = ha.parsed.as_ref().unwrap();
    assert_eq!(hap.host.as_deref(), Some("pg-a.example.com"));
    assert!(hap.notes[0].contains("failover"));

    let my = get("shop-mysql");
    assert!(my.parsed.is_none());
    assert!(my.reason.as_deref().unwrap().contains("mysql"));

    // JSON output for the CLI never contains secrets from the files.
    let json = serde_json::to_string(&all).unwrap();
    assert!(!json.contains("not-a-real-secret"));
}

#[test]
fn sqlite_project_expands_project_dir() {
    let (_tmp, dir) = project("sqlite");
    // The `.idea` path works as well.
    let all = load_project(&dir.join(".idea")).unwrap();
    assert_eq!(all.len(), 3);
    let app = &all[0];
    assert_eq!(app.backend_hint(), Some(BackendHint::Sqlite));
    let expected = format!(
        "{}/data/app.sqlite3",
        dir.to_string_lossy().replace('\\', "/")
    );
    assert_eq!(
        app.parsed.as_ref().unwrap().path.as_deref(),
        Some(expected.as_str())
    );
    assert!(!app.needs_password());
    assert_eq!(
        all[1].parsed.as_ref().unwrap().path.as_deref(),
        Some("D:\\data\\inventory.db")
    );
    assert_eq!(
        all[2].parsed.as_ref().unwrap().path.as_deref(),
        Some(":memory:")
    );
}

#[test]
fn missing_project_files() {
    let tmp = tempfile::tempdir().unwrap();
    let e = load_project(tmp.path()).unwrap_err();
    assert!(matches!(e, ImportError::NotFound(_)), "{e}");
}

#[test]
fn model_file_with_connection_settings_is_merged() {
    let tmp = tempfile::tempdir().unwrap();
    let idea = tmp.path().join(".idea");
    std::fs::create_dir_all(idea.join("dataSources")).unwrap();
    let uuid = "3c9b7e5a-1d2f-4e6a-8b0c-9d8e7f6a5b4c";
    std::fs::write(
        idea.join("dataSources.xml"),
        format!(
            r#"<project version="4"><component name="DataSourceManagerImpl">
               <data-source name="split" uuid="{uuid}"><driver-ref>postgresql</driver-ref></data-source>
               </component></project>"#
        ),
    )
    .unwrap();
    std::fs::write(
        idea.join("dataSources").join(format!("{uuid}.xml")),
        format!(
            r#"<project version="4"><component name="DataSourceManagerImpl">
               <data-source name="split" uuid="{uuid}">
                 <jdbc-url>jdbc:postgresql://pg.example.com/split_db</jdbc-url>
               </data-source></component></project>"#
        ),
    )
    .unwrap();
    let all = load_project(tmp.path()).unwrap();
    assert_eq!(
        all[0].parsed.as_ref().unwrap().database.as_deref(),
        Some("split_db")
    );
    assert!(all[0].notes[0].contains("completed from"));
}
