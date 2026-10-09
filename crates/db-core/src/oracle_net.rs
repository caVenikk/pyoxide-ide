//! Oracle Net connect strings: Easy Connect (`host[:port]/service`,
//! `tcps://host:port/service?wallet_location=...`), connect descriptors
//! (`(DESCRIPTION=(ADDRESS=...)(CONNECT_DATA=...))`) and TNS aliases from
//! `tnsnames.ora`. Written from the public Oracle Net Services Reference.
//!
//! Everything here is pure except [`find_tnsnames`] (environment and file
//! existence) and [`resolve`] (reads the file it finds). Drivers and
//! diagnostics use it to learn host, port and service without an Oracle
//! Client.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Default Oracle listener port.
pub const DEFAULT_PORT: u16 = 1521;

/// One listener address with the database to ask for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OracleAddress {
    /// `PROTOCOL=TCPS` / `tcps://`: TLS from the first byte.
    pub protocol_tcps: bool,
    pub host: String,
    pub port: u16,
    pub service: Option<String>,
    pub sid: Option<String>,
    /// Expected server certificate DN (`SSL_SERVER_CERT_DN`,
    /// `ssl_server_cert_dn`).
    pub server_cert_dn: Option<String>,
    /// Wallet directory named by the connect string (`MY_WALLET_DIRECTORY`,
    /// `wallet_location`).
    pub wallet_dir: Option<String>,
}

/// Parse an Easy Connect string or a connect descriptor and return its first
/// address. TNS aliases are not resolved here (see [`resolve`]).
///
/// Addresses after the first one (failover, load balancing, several hosts)
/// are ignored; [`parse_addresses`] returns all of them.
pub fn parse_connect_string(s: &str) -> Result<OracleAddress> {
    parse_addresses(s)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::config("the connect string has no address"))
}

/// All addresses of the first description, in order.
pub fn parse_addresses(s: &str) -> Result<Vec<OracleAddress>> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::config("empty connect string"));
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("ldap:") || lower.starts_with("ldaps:") {
        return Err(Error::config(
            "LDAP directory naming is not supported; use a descriptor, Easy Connect or a tnsnames.ora alias",
        ));
    }
    if s.starts_with('(') {
        parse_descriptor(s)
    } else {
        parse_easy_connect(s)
    }
}

/// Whether `s` looks like a TNS alias rather than Easy Connect or a
/// descriptor (`ORCL`, `prod.example.com`).
pub fn is_alias(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(['/', ':', '(', '@', '[', '?', ' ', '='])
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '$' | '#'))
}

/// Resolve a connect string: a TNS alias is looked up in the `tnsnames.ora`
/// found by [`find_tnsnames`]; anything else is parsed directly.
pub fn resolve(connect: &str) -> Result<OracleAddress> {
    if !is_alias(connect) {
        return parse_connect_string(connect);
    }
    let path = find_tnsnames().ok_or_else(|| {
        Error::config(format!(
            "`{}` looks like a TNS alias, but no tnsnames.ora was found (set TNS_ADMIN or ORACLE_HOME)",
            connect.trim()
        ))
    })?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        Error::config(format!("cannot read {}: {e}", path.display()))
    })?;
    resolve_with(connect, Some(&text))
}

/// [`resolve`] with the `tnsnames.ora` contents given (`None`: no file).
pub fn resolve_with(connect: &str, tnsnames_text: Option<&str>) -> Result<OracleAddress> {
    if !is_alias(connect) {
        return parse_connect_string(connect);
    }
    let alias = connect.trim();
    let text = tnsnames_text.ok_or_else(|| {
        Error::config(format!("`{alias}` looks like a TNS alias, but no tnsnames.ora is available"))
    })?;
    let target = resolve_alias(text, alias)
        .ok_or_else(|| Error::config(format!("TNS alias `{alias}` not found in tnsnames.ora")))?;
    if is_alias(&target) {
        return Err(Error::config(format!(
            "TNS alias `{alias}` points to another name (`{target}`); nested aliases are not supported"
        )));
    }
    parse_connect_string(&target)
}

/// Candidate `tnsnames.ora` locations, in lookup order: `$TNS_ADMIN`, then
/// `$ORACLE_HOME/network/admin`.
pub fn tnsnames_candidates(tns_admin: Option<&str>, oracle_home: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = tns_admin.filter(|d| !d.trim().is_empty()) {
        out.push(Path::new(dir).join("tnsnames.ora"));
    }
    if let Some(home) = oracle_home.filter(|d| !d.trim().is_empty()) {
        out.push(Path::new(home).join("network").join("admin").join("tnsnames.ora"));
    }
    out
}

/// The first existing `tnsnames.ora` from [`tnsnames_candidates`] using the
/// `TNS_ADMIN` and `ORACLE_HOME` environment variables.
pub fn find_tnsnames() -> Option<PathBuf> {
    let tns_admin = std::env::var("TNS_ADMIN").ok();
    let oracle_home = std::env::var("ORACLE_HOME").ok();
    tnsnames_candidates(tns_admin.as_deref(), oracle_home.as_deref())
        .into_iter()
        .find(|p| p.is_file())
}

/// The connect string an alias stands for in a `tnsnames.ora` text.
///
/// Aliases match case-insensitively. When `alias` has no domain and no entry
/// matches exactly, an entry whose first label matches (`ORCL` for
/// `ORCL.EXAMPLE.COM`) is used if it is the only one.
pub fn resolve_alias(tnsnames_text: &str, alias: &str) -> Option<String> {
    let entries = tnsnames_entries(tnsnames_text);
    let wanted = alias.trim();
    if let Some((_, value)) = entries
        .iter()
        .find(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(wanted)))
    {
        return Some(value.clone());
    }
    if wanted.contains('.') {
        return None;
    }
    let mut matches = entries.iter().filter(|(names, _)| {
        names.iter().any(|n| {
            n.split('.')
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case(wanted))
        })
    });
    let first = matches.next()?;
    matches.next().is_none().then(|| first.1.clone())
}

/// `(aliases, value)` pairs of a `tnsnames.ora` text. `IFILE` entries are
/// returned like any other entry and never followed.
fn tnsnames_entries(text: &str) -> Vec<(Vec<String>, String)> {
    let text = strip_comments(text);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // A stray parenthesis outside an entry: skip the balanced group.
        if bytes[i] == b'(' {
            i = skip_group(bytes, i);
            continue;
        }
        let Some(eq) = text[i..].find('=').map(|p| p + i) else {
            break;
        };
        let names: Vec<String> = text[i..eq]
            .split(',')
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect();
        let mut j = eq + 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let (value, next) = if j < bytes.len() && bytes[j] == b'(' {
            let end = skip_group(bytes, j);
            (text[j..end].to_owned(), end)
        } else {
            let end = text[j..].find('\n').map_or(bytes.len(), |p| p + j);
            (text[j..end].trim().to_owned(), end)
        };
        if !names.is_empty() && !value.is_empty() {
            out.push((names, value));
        }
        i = next.max(eq + 1);
    }
    out
}

/// Remove `#` comments (outside double quotes).
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut in_quotes = false;
        let mut cut = line.len();
        for (idx, c) in line.char_indices() {
            match c {
                '"' => in_quotes = !in_quotes,
                '#' if !in_quotes => {
                    cut = idx;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// Index just after the parenthesized group starting at `start`.
fn skip_group(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut in_quotes = false;
    for (k, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'"' => in_quotes = !in_quotes,
            b'(' if !in_quotes => depth += 1,
            b')' if !in_quotes => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

// ---------------------------------------------------------------- Easy Connect

fn parse_easy_connect(s: &str) -> Result<Vec<OracleAddress>> {
    let mut rest = s;
    let mut tcps = false;
    if let Some(pos) = rest.find("://") {
        let proto = rest[..pos].to_ascii_lowercase();
        tcps = match proto.as_str() {
            "tcp" => false,
            "tcps" => true,
            other => {
                return Err(Error::config(format!(
                    "unsupported protocol `{other}` in the connect string (only tcp and tcps)"
                )));
            }
        };
        rest = &rest[pos + 3..];
    } else {
        rest = rest.trim_start_matches('/');
    }

    // Parameters after `?`.
    let (main, query) = match rest.split_once('?') {
        Some((m, q)) => (m, Some(q)),
        None => (rest, None),
    };
    let mut server_cert_dn = None;
    let mut wallet_dir = None;
    if let Some(q) = query {
        for pair in q.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = v.trim().trim_matches('"').to_owned();
            match k.trim().to_ascii_lowercase().as_str() {
                "ssl_server_cert_dn" => server_cert_dn = Some(v),
                "wallet_location" | "my_wallet_directory" => wallet_dir = Some(v),
                "protocol" => tcps = v.eq_ignore_ascii_case("tcps"),
                _ => {}
            }
        }
    }

    // host part: up to the first `/` that is not inside brackets.
    let (hosts_part, path) = split_host_path(main);
    let mut hosts = parse_host_list(hosts_part)?;

    let mut service = None;
    let mut sid = None;
    match path {
        Some(p) => {
            // service_name[:server_type][/instance_name]
            let svc = p.split('/').next().unwrap_or("");
            let svc = svc.split(':').next().unwrap_or("").trim();
            if !svc.is_empty() {
                service = Some(svc.to_owned());
            }
        }
        None => {
            // JDBC-style host:port:SID.
            if let Some(last) = hosts.last_mut()
                && let Some(s) = last.legacy_sid.take()
            {
                sid = Some(s);
            }
        }
    }
    if service.is_none() && sid.is_none() {
        return Err(Error::config(format!(
            "the connect string `{s}` names no service (expected host[:port]/service_name)"
        )));
    }
    Ok(hosts
        .drain(..)
        .map(|h| OracleAddress {
            protocol_tcps: tcps,
            host: h.host,
            port: h.port,
            service: service.clone(),
            sid: sid.clone(),
            server_cert_dn: server_cert_dn.clone(),
            wallet_dir: wallet_dir.clone(),
        })
        .collect())
}

fn split_host_path(s: &str) -> (&str, Option<&str>) {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            '/' if depth == 0 => return (&s[..i], Some(&s[i + 1..])),
            _ => {}
        }
    }
    (s, None)
}

struct HostPort {
    host: String,
    port: u16,
    legacy_sid: Option<String>,
}

/// `host1[,host2][:port]` lists; a port applies to the hosts before it
/// that have none.
fn parse_host_list(s: &str) -> Result<Vec<HostPort>> {
    let mut out: Vec<HostPort> = Vec::new();
    let mut pending = 0usize;
    for item in s.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(Error::config("empty host in the connect string"));
        }
        let (host, after) = if let Some(rest) = item.strip_prefix('[') {
            let end = rest
                .find(']')
                .ok_or_else(|| Error::config("unterminated IPv6 address in the connect string"))?;
            (rest[..end].to_owned(), &rest[end + 1..])
        } else {
            match item.find(':') {
                Some(p) => (item[..p].to_owned(), &item[p..]),
                None => (item.to_owned(), ""),
            }
        };
        if host.is_empty() {
            return Err(Error::config("missing host in the connect string"));
        }
        let mut port = None;
        let mut legacy_sid = None;
        if let Some(after) = after.strip_prefix(':') {
            let mut parts = after.splitn(2, ':');
            let p = parts.next().unwrap_or("");
            port = Some(parse_port(p)?);
            legacy_sid = parts.next().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        } else if !after.is_empty() {
            return Err(Error::config(format!("unexpected `{after}` after host")));
        }
        out.push(HostPort {
            host,
            port: port.unwrap_or(0),
            legacy_sid,
        });
        pending += 1;
        if let Some(p) = port {
            let n = out.len();
            for h in &mut out[n - pending..] {
                if h.port == 0 {
                    h.port = p;
                }
            }
            pending = 0;
        }
    }
    for h in &mut out {
        if h.port == 0 {
            h.port = DEFAULT_PORT;
        }
    }
    Ok(out)
}

fn parse_port(p: &str) -> Result<u16> {
    p.trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| Error::config(format!("invalid port `{p}` in the connect string")))
}

// ----------------------------------------------------------------- descriptors

/// A `(KEY=VALUE)` or `(KEY=(...)(...))` node of a connect descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NvNode {
    key: String,
    value: NvValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NvValue {
    Atom(String),
    List(Vec<NvNode>),
}

impl NvNode {
    fn children(&self) -> &[NvNode] {
        match &self.value {
            NvValue::List(l) => l,
            NvValue::Atom(_) => &[],
        }
    }

    fn atom(&self) -> Option<&str> {
        match &self.value {
            NvValue::Atom(a) => Some(a),
            NvValue::List(_) => None,
        }
    }

    fn child(&self, key: &str) -> Option<&NvNode> {
        self.children()
            .iter()
            .find(|c| c.key.eq_ignore_ascii_case(key))
    }

    fn child_atom(&self, key: &str) -> Option<&str> {
        self.child(key).and_then(NvNode::atom)
    }
}

struct NvParser<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> NvParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            s: text.as_bytes(),
            text,
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        self.skip_ws();
        if self.pos < self.s.len() && self.s[self.pos] == b {
            self.pos += 1;
            Ok(())
        } else {
            Err(Error::config(format!(
                "malformed connect descriptor: expected `{}` at offset {}",
                b as char, self.pos
            )))
        }
    }

    fn node(&mut self) -> Result<NvNode> {
        self.expect(b'(')?;
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len() && !matches!(self.s[self.pos], b'=' | b'(' | b')') {
            self.pos += 1;
        }
        let key = self.text[start..self.pos].trim().to_owned();
        if key.is_empty() {
            return Err(Error::config("malformed connect descriptor: empty keyword"));
        }
        self.expect(b'=')?;
        self.skip_ws();
        let value = if self.pos < self.s.len() && self.s[self.pos] == b'(' {
            let mut list = Vec::new();
            loop {
                self.skip_ws();
                if self.pos < self.s.len() && self.s[self.pos] == b'(' {
                    list.push(self.node()?);
                } else {
                    break;
                }
            }
            NvValue::List(list)
        } else {
            NvValue::Atom(self.atom()?)
        };
        self.expect(b')')?;
        Ok(NvNode { key, value })
    }

    fn atom(&mut self) -> Result<String> {
        self.skip_ws();
        if self.pos < self.s.len() && self.s[self.pos] == b'"' {
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.s.len() && self.s[self.pos] != b'"' {
                self.pos += 1;
            }
            if self.pos >= self.s.len() {
                return Err(Error::config("malformed connect descriptor: unterminated quote"));
            }
            let v = self.text[start..self.pos].to_owned();
            self.pos += 1;
            return Ok(v);
        }
        let start = self.pos;
        while self.pos < self.s.len() && !matches!(self.s[self.pos], b'(' | b')') {
            self.pos += 1;
        }
        Ok(self.text[start..self.pos].trim().to_owned())
    }
}

fn parse_nv(text: &str) -> Result<NvNode> {
    let mut p = NvParser::new(text);
    let node = p.node()?;
    p.skip_ws();
    if p.pos != p.s.len() {
        return Err(Error::config(
            "malformed connect descriptor: unexpected text after the closing parenthesis",
        ));
    }
    Ok(node)
}

fn is_yes(v: Option<&str>) -> bool {
    matches!(
        v.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("yes" | "on" | "true")
    )
}

fn parse_descriptor(text: &str) -> Result<Vec<OracleAddress>> {
    let root = parse_nv(text)?;
    let description = if root.key.eq_ignore_ascii_case("DESCRIPTION_LIST") {
        root.children()
            .iter()
            .find(|c| c.key.eq_ignore_ascii_case("DESCRIPTION"))
            .ok_or_else(|| Error::config("DESCRIPTION_LIST without a DESCRIPTION"))?
    } else if root.key.eq_ignore_ascii_case("DESCRIPTION") {
        &root
    } else if root.key.eq_ignore_ascii_case("ADDRESS") {
        return Err(Error::config(
            "the connect descriptor has no DESCRIPTION/CONNECT_DATA (service name)",
        ));
    } else {
        return Err(Error::config(format!(
            "unsupported connect descriptor keyword `{}`",
            root.key
        )));
    };
    if is_yes(description.child_atom("SOURCE_ROUTE")) {
        return Err(Error::config(
            "SOURCE_ROUTE (Connection Manager chains) is not supported by the thin driver",
        ));
    }

    let mut address_nodes: Vec<&NvNode> = Vec::new();
    for c in description.children() {
        if c.key.eq_ignore_ascii_case("ADDRESS") {
            address_nodes.push(c);
        } else if c.key.eq_ignore_ascii_case("ADDRESS_LIST") {
            if is_yes(c.child_atom("SOURCE_ROUTE")) {
                return Err(Error::config(
                    "SOURCE_ROUTE (Connection Manager chains) is not supported by the thin driver",
                ));
            }
            address_nodes.extend(
                c.children()
                    .iter()
                    .filter(|a| a.key.eq_ignore_ascii_case("ADDRESS")),
            );
        }
    }
    if address_nodes.is_empty() {
        return Err(Error::config("the connect descriptor has no ADDRESS"));
    }

    let connect_data = description
        .child("CONNECT_DATA")
        .ok_or_else(|| Error::config("the connect descriptor has no CONNECT_DATA"))?;
    let service = connect_data
        .child_atom("SERVICE_NAME")
        .map(str::to_owned)
        .filter(|s| !s.is_empty());
    let sid = connect_data
        .child_atom("SID")
        .map(str::to_owned)
        .filter(|s| !s.is_empty());
    if service.is_none() && sid.is_none() {
        return Err(Error::config(
            "CONNECT_DATA names neither SERVICE_NAME nor SID",
        ));
    }
    let security = description.child("SECURITY");
    let server_cert_dn = security
        .and_then(|s| s.child_atom("SSL_SERVER_CERT_DN"))
        .map(str::to_owned);
    let wallet_dir = security
        .and_then(|s| s.child_atom("MY_WALLET_DIRECTORY"))
        .map(str::to_owned);

    let mut out = Vec::new();
    for a in address_nodes {
        let protocol = a.child_atom("PROTOCOL").unwrap_or("TCP").to_ascii_uppercase();
        let tcps = match protocol.as_str() {
            "TCP" => false,
            "TCPS" => true,
            other => {
                return Err(Error::config(format!(
                    "unsupported protocol `{other}` in the connect descriptor (only TCP and TCPS)"
                )));
            }
        };
        if a.child("HTTPS_PROXY").is_some() {
            return Err(Error::config("HTTPS_PROXY addresses are not supported"));
        }
        let host = a
            .child_atom("HOST")
            .map(str::to_owned)
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::config("an ADDRESS in the connect descriptor has no HOST"))?;
        let port = match a.child_atom("PORT") {
            Some(p) => parse_port(p)?,
            None => DEFAULT_PORT,
        };
        out.push(OracleAddress {
            protocol_tcps: tcps,
            host,
            port,
            service: service.clone(),
            sid: sid.clone(),
            server_cert_dn: server_cert_dn.clone(),
            wallet_dir: wallet_dir.clone(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;

    fn addr(s: &str) -> OracleAddress {
        parse_connect_string(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn easy_connect_forms() {
        let a = addr("dbhost:1522/orclpdb");
        assert_eq!((a.host.as_str(), a.port), ("dbhost", 1522));
        assert_eq!(a.service.as_deref(), Some("orclpdb"));
        assert!(!a.protocol_tcps);

        let a = addr("//dbhost/svc.example.com");
        assert_eq!(a.port, DEFAULT_PORT);
        assert_eq!(a.service.as_deref(), Some("svc.example.com"));

        let a = addr("tcps://secure.example.com:2484/prod?wallet_location=/w&ssl_server_cert_dn=\"CN=db\"");
        assert!(a.protocol_tcps);
        assert_eq!(a.port, 2484);
        assert_eq!(a.wallet_dir.as_deref(), Some("/w"));
        assert_eq!(a.server_cert_dn.as_deref(), Some("CN=db"));

        let a = addr("[::1]:1521/FREEPDB1");
        assert_eq!(a.host, "::1");

        let a = addr("host/svc:dedicated/inst1");
        assert_eq!(a.service.as_deref(), Some("svc"));
    }

    #[test]
    fn easy_connect_legacy_sid_and_host_lists() {
        let a = addr("dbhost:1521:ORCL");
        assert_eq!(a.sid.as_deref(), Some("ORCL"));
        assert_eq!(a.service, None);

        let all = parse_addresses("h1,h2:1530,h3/svc").unwrap();
        let ports: Vec<_> = all.iter().map(|a| (a.host.as_str(), a.port)).collect();
        assert_eq!(ports, [("h1", 1530), ("h2", 1530), ("h3", 1521)]);
    }

    #[test]
    fn easy_connect_errors() {
        for bad in ["", "dbhost", "dbhost:1521", "h:0/s", "h:x/s", "ftp://h/s", "ldap://dir/cn=orcl", "[::1/s"] {
            let e = parse_connect_string(bad).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Config, "{bad}");
        }
    }

    #[test]
    fn descriptors() {
        let a = addr(
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=db1)(PORT=1521))(CONNECT_DATA=(SERVICE_NAME=FREEPDB1)))",
        );
        assert_eq!((a.host.as_str(), a.port), ("db1", 1521));
        assert_eq!(a.service.as_deref(), Some("FREEPDB1"));

        let text = "(description = (retry_count=3)
            (address_list = (load_balance=on)
               (address=(protocol=tcps)(host=a.example.com)(port=2484))
               (address=(protocol=tcps)(host=b.example.com)(port=2485)))
            (connect_data=(sid=ORCL)(server=dedicated))
            (security=(ssl_server_cert_dn=\"CN=db,O=Example\")(my_wallet_directory=/opt/w)))";
        let all = parse_addresses(text).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].protocol_tcps);
        assert_eq!(all[0].host, "a.example.com");
        assert_eq!(all[1].port, 2485);
        assert_eq!(all[0].sid.as_deref(), Some("ORCL"));
        assert_eq!(all[0].server_cert_dn.as_deref(), Some("CN=db,O=Example"));
        assert_eq!(all[0].wallet_dir.as_deref(), Some("/opt/w"));

        let list = "(DESCRIPTION_LIST=(DESCRIPTION=(ADDRESS=(HOST=x)(PORT=1600))(CONNECT_DATA=(SERVICE_NAME=s1)))(DESCRIPTION=(ADDRESS=(HOST=y))(CONNECT_DATA=(SERVICE_NAME=s2))))";
        let a = addr(list);
        assert_eq!((a.host.as_str(), a.port, a.service.as_deref()), ("x", 1600, Some("s1")));
    }

    #[test]
    fn descriptor_errors() {
        for bad in [
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=IPC)(KEY=x))(CONNECT_DATA=(SERVICE_NAME=s)))",
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=h))(CONNECT_DATA=(SERVER=dedicated)))",
            "(DESCRIPTION=(CONNECT_DATA=(SERVICE_NAME=s)))",
            "(DESCRIPTION=(SOURCE_ROUTE=yes)(ADDRESS=(HOST=h))(ADDRESS=(HOST=g))(CONNECT_DATA=(SERVICE_NAME=s)))",
            "(DESCRIPTION=(ADDRESS=(HOST=h)(PORT=99999))(CONNECT_DATA=(SERVICE_NAME=s)))",
            "(DESCRIPTION=(ADDRESS=(HOST=h))(CONNECT_DATA=(SERVICE_NAME=s))",
            "(DESCRIPTION=(ADDRESS=(HOST=h))(CONNECT_DATA=(SERVICE_NAME=s))) trailing",
            "(ADDRESS=(HOST=h)(PORT=1521))",
        ] {
            let e = parse_connect_string(bad).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Config, "{bad}");
        }
    }

    const TNSNAMES: &str = r#"
# Comment line
PROD, PROD_ALT =
  (DESCRIPTION =
    (ADDRESS = (PROTOCOL = TCP)(HOST = prod-db)(PORT = 1521)) # trailing comment
    (CONNECT_DATA = (SERVICE_NAME = prod.example.com))
  )

dev.example.com=(DESCRIPTION=(ADDRESS=(PROTOCOL=TCPS)(HOST=dev-db)(PORT=2484))(CONNECT_DATA=(SID=DEV)))
EZ = ez-host:1600/ezsvc
IFILE = /etc/other/tnsnames.ora
"#;

    #[test]
    fn resolves_aliases() {
        let v = resolve_alias(TNSNAMES, "prod").unwrap();
        assert!(v.contains("prod-db"), "{v}");
        assert!(resolve_alias(TNSNAMES, "PROD_ALT").is_some());
        // First label match when the alias has no domain.
        assert!(resolve_alias(TNSNAMES, "DEV").unwrap().contains("dev-db"));
        assert_eq!(resolve_alias(TNSNAMES, "EZ").as_deref(), Some("ez-host:1600/ezsvc"));
        assert_eq!(resolve_alias(TNSNAMES, "missing"), None);
        assert_eq!(resolve_alias(TNSNAMES, "dev.other.com"), None);

        let a = resolve_with("prod", Some(TNSNAMES)).unwrap();
        assert_eq!((a.host.as_str(), a.port), ("prod-db", 1521));
        let a = resolve_with("dev.example.com", Some(TNSNAMES)).unwrap();
        assert!(a.protocol_tcps);
        assert_eq!(a.sid.as_deref(), Some("DEV"));
        let a = resolve_with("EZ", Some(TNSNAMES)).unwrap();
        assert_eq!(a.port, 1600);
        // Not an alias: parsed directly, the file is not needed.
        assert_eq!(resolve_with("h:1/s", None).unwrap().port, 1);
        assert_eq!(resolve_with("prod", None).unwrap_err().kind, ErrorKind::Config);
        assert_eq!(resolve_with("nope", Some(TNSNAMES)).unwrap_err().kind, ErrorKind::Config);
    }

    #[test]
    fn alias_detection() {
        assert!(is_alias("ORCL"));
        assert!(is_alias("prod.example.com"));
        assert!(!is_alias("host:1521/svc"));
        assert!(!is_alias("(DESCRIPTION=)"));
        assert!(!is_alias("host/svc"));
        assert!(!is_alias(""));
    }

    #[test]
    fn tnsnames_lookup_order() {
        let c = tnsnames_candidates(Some("/tns"), Some("/oh"));
        assert_eq!(
            c,
            vec![
                PathBuf::from("/tns").join("tnsnames.ora"),
                PathBuf::from("/oh").join("network").join("admin").join("tnsnames.ora"),
            ]
        );
        assert_eq!(tnsnames_candidates(None, Some("")), Vec::<PathBuf>::new());
    }
}
