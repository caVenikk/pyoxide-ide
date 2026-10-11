//! Plain-language causes and next steps for connection errors. Codes come
//! from the public Oracle Database Error Messages reference, the PostgreSQL
//! error codes appendix and the SQLite result codes page.

use db_core::{Dialect, Error, ErrorKind};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hint {
    pub cause: String,
    pub next: String,
}

fn hint(cause: &str, next: &str) -> Option<Hint> {
    Some(Hint {
        cause: cause.to_owned(),
        next: next.to_owned(),
    })
}

/// Message fragments that mean "the thin Oracle protocol cannot do what this
/// server requires" rather than a wrong setting.
pub fn thin_limitation(err: &Error) -> bool {
    if matches!(
        err.code.as_deref(),
        Some("ORA-12660" | "ORA-12650" | "ORA-12599" | "ORA-28040" | "ORA-12638")
    ) {
        return true;
    }
    let m = err.message.to_ascii_lowercase();
    [
        "verifier",
        "encryption",
        "checksum",
        "unsupported authentication",
    ]
    .iter()
    .any(|k| m.contains(k))
}

const ODPI_NEXT: &str = "use the Oracle Client backend (`backend = \"oracle-odpi\"`, needs Oracle Instant Client); `diagnose --try-all` tries it automatically";

pub fn for_error(dialect: Dialect, err: &Error) -> Option<Hint> {
    let by_code = match (dialect, err.code.as_deref()) {
        (Dialect::Oracle, Some(code)) => oracle(code),
        (Dialect::Postgres, Some(code)) => postgres(code),
        (Dialect::Sqlite, Some(code)) => sqlite(code),
        _ => None,
    };
    if by_code.is_some() {
        return by_code;
    }
    if dialect == Dialect::Oracle && thin_limitation(err) {
        return hint(
            "the server needs a protocol feature the thin driver lacks (native network encryption, an old password verifier or another authentication method)",
            ODPI_NEXT,
        );
    }
    by_kind(err)
}

fn oracle(code: &str) -> Option<Hint> {
    match code {
        "ORA-01017" => hint(
            "the server rejected the user name or password",
            "check both (passwords are case-sensitive); if the same login works in another tool, the account may only have an old password verifier: try the Oracle Client backend",
        ),
        "ORA-28000" => hint("the account is locked", "ask the DBA to unlock it"),
        "ORA-28001" => hint(
            "the password has expired",
            "change it with another tool (for example SQL*Plus `password`), then update the stored secret",
        ),
        "ORA-01045" => hint(
            "the user lacks the CREATE SESSION privilege",
            "ask the DBA to grant CREATE SESSION",
        ),
        "ORA-12514" => hint(
            "the listener is reachable but does not know this service name",
            "check the service name (often like ORCLPDB1 or name.domain); if you have a SID instead, set `sid` rather than `service`; the DBA can list services with `lsnrctl services`",
        ),
        "ORA-12505" => hint(
            "the listener does not know this SID",
            "check the SID, or use the service name (`service`) instead",
        ),
        "ORA-12541" => hint(
            "nothing accepts Oracle connections on this host and port",
            "check the port (1521 by default, often 2484 for TCPS) and that the listener runs",
        ),
        "ORA-12545" | "ORA-12543" => hint(
            "the host cannot be found or reached",
            "check the host name, DNS and VPN",
        ),
        "ORA-12170" | "ORA-12535" => hint(
            "the connection attempt timed out",
            "a firewall or VPN probably blocks the port; check with `tcp` step above and with the network team",
        ),
        "ORA-12154" => hint(
            "the connect identifier (TNS alias) could not be resolved",
            "check that tnsnames.ora contains the alias and that TNS_ADMIN points to its directory",
        ),
        "ORA-12537" | "ORA-12547" => hint(
            "the server closed the connection during the handshake",
            "this often means native network encryption is required or the client host is not allowed (valid node checking); try the Oracle Client backend and ask the DBA",
        ),
        "ORA-28040" => hint(
            "client and server have no authentication protocol in common",
            "the server's SQLNET.ALLOWED_LOGON_VERSION_SERVER or the account's verifiers do not match this client; try the Oracle Client backend or ask the DBA",
        ),
        "ORA-12660" | "ORA-12650" | "ORA-12599" => hint(
            "the server requires native network encryption or checksumming",
            ODPI_NEXT,
        ),
        "ORA-12516" | "ORA-12519" | "ORA-12520" => hint(
            "the listener has no free connection handler",
            "the database is at its process or session limit; retry later or ask the DBA",
        ),
        "ORA-01033" | "ORA-01034" | "ORA-01089" => hint(
            "the database is not open (starting up, shutting down or down)",
            "retry later or ask the DBA",
        ),
        "ORA-28759" | "ORA-28860" | "ORA-29024" | "ORA-28862" | "ORA-28865" => hint(
            "the TLS handshake or the wallet failed",
            "check the wallet directory and password, the CA certificates and the TCPS port; see the `tls` step",
        ),
        "ORA-00942" | "ORA-01031" => hint(
            "the account cannot read some data dictionary views",
            "metadata uses ALL_* views only; objects you have no access to are not listed",
        ),
        "DPI-1047" => hint(
            "the Oracle Client libraries were not found",
            "install Oracle Instant Client (Basic or Basic Light) and add its directory to PATH (Windows), DYLD_LIBRARY_PATH (macOS) or LD_LIBRARY_PATH (Linux)",
        ),
        "DPI-1072" => hint(
            "the installed Oracle Client is too old",
            "install a current Oracle Instant Client",
        ),
        _ => None,
    }
}

fn postgres(code: &str) -> Option<Hint> {
    match code {
        "28P01" => hint(
            "the server rejected the password",
            "check the password and the user name",
        ),
        "28000" => hint(
            "the server's pg_hba.conf does not allow this host, user or database, or it requires TLS",
            "try TLS mode `require`, or ask the administrator to allow this client",
        ),
        "3D000" => hint(
            "the database does not exist",
            "check the database name (it is case-sensitive)",
        ),
        "53300" => hint(
            "the server has no free connection slots",
            "close other sessions or ask the administrator",
        ),
        "57P03" => hint("the server is starting up or shutting down", "retry later"),
        _ => None,
    }
}

fn sqlite(code: &str) -> Option<Hint> {
    match code {
        "SQLITE_CANTOPEN" => hint(
            "the database file cannot be opened",
            "check the path and permissions; set `create = true` to create a new file",
        ),
        "SQLITE_NOTADB" => hint(
            "the file is not a SQLite database (or is encrypted)",
            "check the path",
        ),
        "SQLITE_BUSY" | "SQLITE_LOCKED" => hint(
            "another process holds a lock on the database",
            "close the other writer or retry",
        ),
        _ => None,
    }
}

fn by_kind(err: &Error) -> Option<Hint> {
    match err.kind {
        ErrorKind::Network => hint(
            "the server cannot be reached",
            "check host, port, VPN and firewall; see the `dns` and `tcp` steps",
        ),
        ErrorKind::Tls => hint(
            "the TLS handshake failed",
            "check the TLS mode, CA file or wallet and the server certificate; see the `tls` step",
        ),
        ErrorKind::Auth => hint(
            "the server rejected the credentials",
            "check the user name and the password source shown in the `secret` step",
        ),
        ErrorKind::Timeout => hint(
            "the operation timed out",
            "raise connect_timeout_secs, or check the network path",
        ),
        ErrorKind::Unavailable => hint(
            "this backend cannot be used on this machine",
            "oracle-odpi needs Oracle Instant Client; the other backends are built in",
        ),
        ErrorKind::Config => hint(
            "the connection settings are incomplete or invalid",
            "fix the profile",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ora(code: &str) -> Error {
        db_core::oracle::error_from_message(format!("{code}: something"), ErrorKind::Connect)
    }

    #[test]
    fn oracle_codes_have_hints() {
        for code in [
            "ORA-01017",
            "ORA-12514",
            "ORA-12541",
            "ORA-12660",
            "DPI-1047",
        ] {
            assert!(for_error(Dialect::Oracle, &ora(code)).is_some(), "{code}");
        }
    }

    #[test]
    fn detects_thin_limitations() {
        assert!(thin_limitation(&ora("ORA-12660")));
        let e = Error::new(ErrorKind::Auth, "unsupported verifier type 0x939");
        assert!(thin_limitation(&e));
        assert!(
            for_error(Dialect::Oracle, &e)
                .unwrap()
                .next
                .contains("oracle-odpi")
        );
        assert!(!thin_limitation(&ora("ORA-01017")));
    }

    #[test]
    fn falls_back_to_kind() {
        let e = Error::new(ErrorKind::Network, "connection refused");
        assert!(
            for_error(Dialect::Postgres, &e)
                .unwrap()
                .cause
                .contains("reached")
        );
        let e = Error::query("syntax error");
        assert!(for_error(Dialect::Sqlite, &e).is_none());
    }
}
