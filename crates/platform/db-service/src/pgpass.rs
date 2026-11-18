//! PostgreSQL password-file lookup (libpq documentation, "The Password File").

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use db_core::{Profile, Secret};
use secrets::EnvLookup;

fn path(profile: &Profile, env: &EnvLookup) -> Option<PathBuf> {
    if let Some(path) = profile
        .options
        .get("passfile")
        .filter(|s| !s.is_empty())
        .cloned()
        .or_else(|| env("PGPASSFILE"))
        .filter(|s| !s.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        env("APPDATA")
            .map(PathBuf::from)
            .or_else(dirs::data_dir)
            .map(|p| p.join("postgresql").join("pgpass.conf"))
    }
    #[cfg(not(windows))]
    {
        env("HOME")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .map(|p| p.join(".pgpass"))
    }
}

pub(crate) fn lookup(profile: &Profile, env: &EnvLookup) -> Option<Secret> {
    if profile.backend != db_core::Backend::Postgres {
        return None;
    }
    let path = path(profile, env)?;
    if !std::fs::metadata(&path).ok()?.is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if meta.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    let port = profile.port_or_default().to_string();
    let host = if profile.host.is_empty() {
        "localhost"
    } else {
        profile.host.as_str()
    };
    let fields = [
        host,
        &port,
        profile.database.as_deref().unwrap_or(&profile.user),
        &profile.user,
    ];
    for line in BufReader::new(file).lines() {
        let line = Secret::new(line.ok()?);
        if let Some(value) = matching_password(line.expose(), &fields) {
            return Some(Secret::new(value));
        }
    }
    None
}

fn matching_password(line: &str, fields: &[&str; 4]) -> Option<String> {
    if line.starts_with('#') {
        return None;
    }
    let mut rest = line;
    for wanted in fields {
        let mut chars = rest.char_indices();
        let mut field = String::new();
        let delimiter = loop {
            match chars.next()? {
                (at, ':') => break at,
                (_, '\\') => field.push(chars.next()?.1),
                (_, c) => field.push(c),
            }
        };
        if &rest[..delimiter] != "*" && field != *wanted {
            return None;
        }
        rest = &rest[delimiter + 1..];
    }
    let mut value = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        value.push(if c == '\\' {
            chars.next().unwrap_or(c)
        } else {
            c
        });
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::Arc;

    #[test]
    fn password_file_matches_first_entry_and_checks_open_file_permissions() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "#*:*:*:*:ignored").unwrap();
        writeln!(file, "incomplete:row").unwrap();
        writeln!(file, r"db\:server:5432:app:guest:no").unwrap();
        writeln!(file, r"db\:server:5432:app:dev\\ops:p\:a\\ss").unwrap();
        writeln!(file, "*:*:*:*:later").unwrap();
        let mut profile = Profile::new(db_core::Backend::Postgres);
        profile.host = "db:server".into();
        profile.database = Some("app".into());
        profile.user = r"dev\ops".into();
        let path = file.path().to_str().unwrap().to_owned();
        let env: EnvLookup = Arc::new(move |name| (name == "PGPASSFILE").then(|| path.clone()));
        assert_eq!(lookup(&profile, &env).unwrap().expose(), r"p:a\ss");
        profile.user = "other".into();
        assert_eq!(lookup(&profile, &env).unwrap().expose(), "later");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o640))
                .unwrap();
            assert!(lookup(&profile, &env).is_none());
        }
    }

    #[test]
    fn defaults_and_explicit_override_follow_the_platform() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_str().unwrap().to_owned();
        let env: EnvLookup =
            Arc::new(move |key| matches!(key, "HOME" | "APPDATA").then(|| home.clone()));
        let mut profile = Profile::new(db_core::Backend::Postgres);
        let expected = if cfg!(windows) {
            dir.path().join("postgresql").join("pgpass.conf")
        } else {
            dir.path().join(".pgpass")
        };
        assert_eq!(path(&profile, &env), Some(expected));
        profile
            .options
            .insert("passfile".into(), "explicit.pass".into());
        assert_eq!(path(&profile, &env), Some(PathBuf::from("explicit.pass")));
        assert_eq!(
            matching_password(r"\*:*:*:*:literal", &["host", "1", "d", "u"]),
            None
        );
    }
}
