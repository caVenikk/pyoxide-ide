//! The URL field of the Data Sources dialog: the connection as one line
//! that follows the fields, and that sets the fields when it is edited.
//!
//! - PostgreSQL: `postgresql://[user@]host[:port]/database` (the libpq
//!   connection URI; `postgres://` is taken too);
//! - Oracle: `oracle://host[:port]/service` or `oracle://host[:port]?sid=SID`;
//!   any other text is a connect string (a TNS alias or a descriptor);
//! - SQLite: `sqlite:path`.
//!
//! A pasted JDBC URL of the same database (`jdbc:postgresql://…`,
//! `jdbc:oracle:thin:@…`, `jdbc:sqlite:…`) is read too.

use db_core::{Dialect, Profile};

/// The URL of the profile's connection.
pub fn format(profile: &Profile) -> String {
    match profile.dialect() {
        Dialect::Postgres => {
            let user = if profile.user.is_empty() {
                String::new()
            } else {
                format!("{}@", profile.user)
            };
            format!(
                "postgresql://{user}{}{}/{}",
                profile.host,
                port_part(profile),
                profile.database.as_deref().unwrap_or("")
            )
        }
        Dialect::Oracle => {
            if let Some(dsn) = &profile.dsn {
                return dsn.clone();
            }
            match &profile.sid {
                Some(sid) => format!("oracle://{}{}?sid={sid}", profile.host, port_part(profile)),
                None => format!(
                    "oracle://{}{}/{}",
                    profile.host,
                    port_part(profile),
                    profile.service.as_deref().unwrap_or("")
                ),
            }
        }
        Dialect::Sqlite => format!("sqlite:{}", profile.path.as_deref().unwrap_or("")),
    }
}

fn port_part(profile: &Profile) -> String {
    match profile.port {
        Some(port) => format!(":{port}"),
        None => match profile.backend.default_port() {
            Some(port) => format!(":{port}"),
            None => String::new(),
        },
    }
}

/// Sets the profile's connection fields from `url`; an error says what is
/// wrong and changes nothing.
pub fn apply(profile: &mut Profile, url: &str) -> Result<(), String> {
    let url = url.trim();
    let mut next = profile.clone();
    match profile.dialect() {
        Dialect::Postgres => {
            let rest = strip_any(url, &["jdbc:postgresql://", "postgresql://", "postgres://"])
                .ok_or("the URL should start with postgresql://")?;
            let (rest, _query) = rest.split_once('?').unwrap_or((rest, ""));
            let (authority, database) = rest.split_once('/').unwrap_or((rest, ""));
            let (user, host_port) = match authority.rsplit_once('@') {
                Some((user, host)) => (Some(user.split(':').next().unwrap_or("")), host),
                None => (None, authority),
            };
            let (host, port) = host_and_port(host_port)?;
            next.host = host;
            next.port = port;
            next.database = (!database.is_empty()).then(|| database.to_owned());
            if let Some(user) = user {
                next.user = user.to_owned();
            }
        }
        Dialect::Oracle => {
            if let Some(rest) = strip_any(url, &["jdbc:oracle:thin:@//", "oracle://"]) {
                let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
                let (host_port, service) = rest.split_once('/').unwrap_or((rest, ""));
                let (host, port) = host_and_port(host_port)?;
                let sid = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("sid="))
                    .filter(|s| !s.is_empty());
                next.host = host;
                next.port = port;
                next.dsn = None;
                match sid {
                    Some(sid) => {
                        next.sid = Some(sid.to_owned());
                        next.service = None;
                    }
                    None => {
                        next.service = (!service.is_empty()).then(|| service.to_owned());
                        next.sid = None;
                    }
                }
            } else if let Some(rest) = url.strip_prefix("jdbc:oracle:thin:@") {
                // host:port:SID
                let mut parts = rest.split(':');
                let (host, port, sid) = (parts.next(), parts.next(), parts.next());
                let (Some(host), Some(port), Some(sid)) = (host, port, sid) else {
                    return Err("expected jdbc:oracle:thin:@host:port:SID".into());
                };
                next.host = host.to_owned();
                next.port = Some(port.parse().map_err(|_| format!("bad port {port:?}"))?);
                next.sid = Some(sid.to_owned());
                next.service = None;
                next.dsn = None;
            } else if url.is_empty() {
                return Err("the URL is empty".into());
            } else {
                next.dsn = Some(url.to_owned());
                next.service = None;
                next.sid = None;
            }
        }
        Dialect::Sqlite => {
            let path = strip_any(url, &["jdbc:sqlite:", "sqlite:"])
                .ok_or("the URL should start with sqlite:")?;
            next.path = (!path.is_empty()).then(|| path.to_owned());
        }
    }
    *profile = next;
    Ok(())
}

fn strip_any<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| text.strip_prefix(p))
}

fn host_and_port(text: &str) -> Result<(String, Option<u16>), String> {
    match text.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => {
            let port = port.parse().map_err(|_| format!("bad port {port:?}"))?;
            Ok((host.to_owned(), Some(port)))
        }
        Some((host, _)) => Ok((host.to_owned(), None)),
        None => Ok((text.to_owned(), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_core::Backend;

    fn pg() -> Profile {
        let mut p = Profile::new(Backend::Postgres);
        p.host = "localhost".into();
        p.port = Some(55432);
        p.database = Some("pyoxide".into());
        p
    }

    #[test]
    fn postgres_round_trip() {
        let mut p = pg();
        assert_eq!(format(&p), "postgresql://localhost:55432/pyoxide");
        p.port = None;
        assert_eq!(format(&p), "postgresql://localhost:5432/pyoxide");
        apply(
            &mut p,
            "postgresql://app@db.example.com:6543/sales?sslmode=require",
        )
        .unwrap();
        assert_eq!(
            (
                p.host.as_str(),
                p.port,
                p.database.as_deref(),
                p.user.as_str()
            ),
            ("db.example.com", Some(6543), Some("sales"), "app")
        );
        assert_eq!(format(&p), "postgresql://app@db.example.com:6543/sales");
        apply(&mut p, "jdbc:postgresql://localhost/x").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("localhost", None));
        let before = p.clone();
        assert!(apply(&mut p, "mysql://x").is_err());
        assert!(apply(&mut p, "postgresql://h:port/x").is_err());
        assert_eq!(p, before);
    }

    #[test]
    fn oracle_service_sid_and_connect_strings() {
        let mut p = Profile::new(Backend::OracleThin);
        p.host = "db".into();
        p.service = Some("ORCLPDB1".into());
        assert_eq!(format(&p), "oracle://db:1521/ORCLPDB1");
        apply(&mut p, "oracle://db2:1522?sid=ORCL").unwrap();
        assert_eq!(
            (
                p.host.as_str(),
                p.port,
                p.sid.as_deref(),
                p.service.as_deref()
            ),
            ("db2", Some(1522), Some("ORCL"), None)
        );
        assert_eq!(format(&p), "oracle://db2:1522?sid=ORCL");
        apply(&mut p, "jdbc:oracle:thin:@h:1521:XE").unwrap();
        assert_eq!((p.host.as_str(), p.sid.as_deref()), ("h", Some("XE")));
        apply(&mut p, "jdbc:oracle:thin:@//h:1521/FREEPDB1").unwrap();
        assert_eq!(
            (p.service.as_deref(), p.sid.as_deref()),
            (Some("FREEPDB1"), None)
        );
        apply(&mut p, "PRODDB").unwrap();
        assert_eq!(p.dsn.as_deref(), Some("PRODDB"));
        assert_eq!(format(&p), "PRODDB");
    }

    #[test]
    fn sqlite_paths() {
        let mut p = Profile::new(Backend::Sqlite);
        apply(&mut p, "sqlite:/tmp/x.db").unwrap();
        assert_eq!(p.path.as_deref(), Some("/tmp/x.db"));
        assert_eq!(format(&p), "sqlite:/tmp/x.db");
        apply(&mut p, "jdbc:sqlite:C:\\data\\x.db").unwrap();
        assert_eq!(p.path.as_deref(), Some("C:\\data\\x.db"));
    }
}
