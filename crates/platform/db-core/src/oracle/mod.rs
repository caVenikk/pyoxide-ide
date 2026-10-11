//! Oracle helpers shared by the thin and the Oracle Client drivers: error
//! code extraction and classification (codes from the public Oracle Database
//! Error Messages reference), and statement text helpers in [`sql`].

pub mod sql;

use crate::{Error, ErrorKind};

/// First Oracle-style error code in a message: `ORA-01017`, `TNS-12514`,
/// `DPI-1047`. The code that appears first wins (`DPI-1067: ... ORA-03136`
/// is a DPI-1067). ORA- and TNS- codes are normalized to five digits.
pub fn error_code(message: &str) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for prefix in ["ORA-", "TNS-", "DPI-"] {
        let mut from = 0;
        while let Some(rel) = message[from..].find(prefix) {
            let pos = from + rel;
            let after = &message[pos + prefix.len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() {
                let code = if prefix == "DPI-" {
                    format!("DPI-{digits}")
                } else {
                    match digits.parse::<u32>() {
                        Ok(n) => format!("ORA-{n:05}"),
                        Err(_) => break,
                    }
                };
                if best.as_ref().is_none_or(|(p, _)| pos < *p) {
                    best = Some((pos, code));
                }
                break;
            }
            from = pos + prefix.len();
        }
    }
    best.map(|(_, code)| code)
}

/// Error kind for an Oracle error code (`ORA-12514`, `DPI-1047`).
/// Unknown `ORA-` codes are statement errors.
pub fn classify(code: &str) -> ErrorKind {
    match code {
        // Credentials and accounts.
        "ORA-01017" | "ORA-28000" | "ORA-28001" | "ORA-01045" | "ORA-01005" | "ORA-01004"
        | "ORA-28009" => ErrorKind::Auth,
        // Name resolution of the connect identifier.
        "ORA-12154" | "ORA-12162" | "ORA-12262" | "ORA-12263" => ErrorKind::Config,
        // Listener reachable but the request was refused.
        "ORA-12514" | "ORA-12505" | "ORA-12516" | "ORA-12519" | "ORA-12520" | "ORA-12521"
        | "ORA-12526" | "ORA-12527" | "ORA-12528" | "ORA-28040" | "ORA-12660" | "ORA-12650"
        | "ORA-12599" | "ORA-12638" | "ORA-01034" | "ORA-01033" | "ORA-01089" => ErrorKind::Connect,
        // Network path.
        "ORA-12541" | "ORA-12543" | "ORA-12545" | "ORA-12170" | "ORA-12535" | "ORA-12537"
        | "ORA-12547" | "ORA-12560" | "ORA-12552" | "ORA-03136" => ErrorKind::Network,
        // TLS and wallets.
        "ORA-28759" | "ORA-29024" | "ORA-28860" | "ORA-28862" | "ORA-28865" | "ORA-29003"
        | "ORA-29106" | "ORA-28864" => ErrorKind::Tls,
        // Session gone.
        "ORA-03113" | "ORA-03114" | "ORA-03135" | "ORA-02396" | "ORA-00028" | "ORA-01012"
        | "DPI-1080" | "DPI-1010" => ErrorKind::ConnectionLost,
        "ORA-01013" => ErrorKind::Cancelled,
        "DPI-1067" => ErrorKind::Timeout,
        // Oracle Client libraries missing or unusable.
        "DPI-1047" | "DPI-1072" | "DPI-1050" => ErrorKind::Unavailable,
        _ if code.starts_with("DPI-") => ErrorKind::Internal,
        _ => ErrorKind::Query,
    }
}

/// Build an [`Error`] from an Oracle message, using `fallback` when the
/// message has no recognizable code (e.g. a socket error).
pub fn error_from_message(message: impl Into<String>, fallback: ErrorKind) -> Error {
    let message = message.into();
    match error_code(&message) {
        Some(code) => Error::new(classify(&code), message).with_code(code),
        None => Error::new(fallback, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_codes() {
        assert_eq!(
            error_code("ORA-01017: invalid username/password; logon denied").as_deref(),
            Some("ORA-01017")
        );
        assert_eq!(
            error_code("TNS-12514: TNS:listener does not currently know of service").as_deref(),
            Some("ORA-12514")
        );
        assert_eq!(
            error_code("DPI-1047: Cannot locate a 64-bit Oracle Client library").as_deref(),
            Some("DPI-1047")
        );
        assert_eq!(
            error_code("ORA-3113 end of file").as_deref(),
            Some("ORA-03113")
        );
        assert_eq!(error_code("connection refused"), None);
        assert_eq!(
            error_code("DPI-1067: call timeout of 1000 ms exceeded with ORA-03136").as_deref(),
            Some("DPI-1067")
        );
        assert_eq!(error_code("ORA-: nothing"), None);
    }

    #[test]
    fn classifies() {
        assert_eq!(classify("ORA-01017"), ErrorKind::Auth);
        assert_eq!(classify("ORA-12514"), ErrorKind::Connect);
        assert_eq!(classify("ORA-12541"), ErrorKind::Network);
        assert_eq!(classify("ORA-01013"), ErrorKind::Cancelled);
        assert_eq!(classify("ORA-03113"), ErrorKind::ConnectionLost);
        assert_eq!(classify("DPI-1047"), ErrorKind::Unavailable);
        assert_eq!(classify("ORA-00942"), ErrorKind::Query);
    }

    #[test]
    fn builds_errors() {
        let e = error_from_message("ORA-28000: the account is locked", ErrorKind::Connect);
        assert_eq!(e.kind, ErrorKind::Auth);
        assert_eq!(e.code.as_deref(), Some("ORA-28000"));
        let e = error_from_message("Connection refused (os error 61)", ErrorKind::Network);
        assert_eq!(e.kind, ErrorKind::Network);
        assert_eq!(e.code, None);
    }
}
