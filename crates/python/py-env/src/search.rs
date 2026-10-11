use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{EnvError, ErrorKind, Result};
use crate::version::{Version, normalize_name};

/// One release of a project on an index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexVersion {
    pub version: String,
    /// Every file of the release is yanked.
    pub yanked: bool,
    pub prerelease: bool,
    pub requires_python: Option<String>,
    /// ISO 8601 upload time of the first file.
    pub uploaded: Option<String>,
}

/// What an index says about a project: enough for a package search result
/// and a version picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexProject {
    pub name: String,
    pub summary: Option<String>,
    /// The index's latest (non-pre-release) version.
    pub latest: String,
    /// Newest first.
    pub versions: Vec<IndexVersion>,
    pub requires_python: Option<String>,
    pub home_page: Option<String>,
    pub project_urls: BTreeMap<String, String>,
    pub license: Option<String>,
}

/// A source of package metadata. The IDE uses [`PyPi`]; tests use
/// [`StaticIndex`], so no unit test needs the network.
#[async_trait]
pub trait PackageIndex: Send + Sync {
    /// The project called `name` (any spelling PEP 503 considers the same).
    /// A missing project is [`ErrorKind::PackageNotFound`].
    async fn project(&self, name: &str) -> Result<IndexProject>;
}

/// An in-memory index.
#[derive(Debug, Clone, Default)]
pub struct StaticIndex {
    projects: BTreeMap<String, IndexProject>,
}

impl StaticIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, project: IndexProject) -> Self {
        self.projects.insert(normalize_name(&project.name), project);
        self
    }
}

#[async_trait]
impl PackageIndex for StaticIndex {
    async fn project(&self, name: &str) -> Result<IndexProject> {
        check_name(name)?;
        self.projects
            .get(&normalize_name(name))
            .cloned()
            .ok_or_else(|| not_found(name))
    }
}

fn not_found(name: &str) -> EnvError {
    EnvError::new(
        ErrorKind::PackageNotFound,
        format!("`{}` was not found on the index", name.trim()),
    )
}

fn check_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty()
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(EnvError::invalid(format!(
            "`{name}` is not a valid project name (letters, digits, `-`, `_` and `.` only)"
        )));
    }
    Ok(())
}

// ---------- PyPI JSON API ----------

#[derive(Deserialize)]
struct RawProject {
    info: RawInfo,
    #[serde(default)]
    releases: BTreeMap<String, Vec<RawFile>>,
}

#[derive(Deserialize)]
struct RawInfo {
    name: String,
    version: String,
    summary: Option<String>,
    home_page: Option<String>,
    requires_python: Option<String>,
    license: Option<String>,
    license_expression: Option<String>,
    project_urls: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    yanked: bool,
    requires_python: Option<String>,
    upload_time_iso_8601: Option<String>,
}

/// Parse the body of `https://pypi.org/pypi/<name>/json`.
pub fn parse_pypi_json(body: &[u8]) -> Result<IndexProject> {
    let raw: RawProject = serde_json::from_slice(body)
        .map_err(|e| EnvError::parse(format!("unreadable answer from the index: {e}")))?;
    let mut versions: Vec<(Option<Version>, IndexVersion)> = raw
        .releases
        .iter()
        // A release without files was deleted.
        .filter(|(_, files)| !files.is_empty())
        .map(|(v, files)| {
            let parsed = Version::parse(v);
            (
                parsed.clone(),
                IndexVersion {
                    version: v.clone(),
                    yanked: files.iter().all(|f| f.yanked),
                    prerelease: parsed.is_some_and(|p| p.is_prerelease()),
                    requires_python: files
                        .iter()
                        .find_map(|f| f.requires_python.clone())
                        .filter(|s| !s.is_empty()),
                    uploaded: files.iter().find_map(|f| f.upload_time_iso_8601.clone()),
                },
            )
        })
        .collect();
    versions.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let mut versions: Vec<IndexVersion> = versions.into_iter().map(|(_, v)| v).collect();
    if versions.is_empty() {
        // The `releases` map is deprecated and may go away; the latest version is always known.
        versions.push(IndexVersion {
            version: raw.info.version.clone(),
            yanked: false,
            prerelease: Version::parse(&raw.info.version).is_some_and(|v| v.is_prerelease()),
            requires_python: raw.info.requires_python.clone().filter(|s| !s.is_empty()),
            uploaded: None,
        });
    }
    let license = raw
        .info
        .license_expression
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            // The legacy field often holds a whole license text.
            raw.info
                .license
                .as_deref()
                .and_then(|l| l.lines().next())
                .map(str::trim)
                .filter(|l| !l.is_empty() && l.len() <= 100)
                .map(str::to_owned)
        });
    Ok(IndexProject {
        name: raw.info.name,
        summary: raw.info.summary.filter(|s| !s.trim().is_empty()),
        latest: raw.info.version,
        versions,
        requires_python: raw.info.requires_python.filter(|s| !s.is_empty()),
        home_page: raw.info.home_page.filter(|s| !s.trim().is_empty()),
        project_urls: raw.info.project_urls.unwrap_or_default(),
        license,
    })
}

/// The PyPI JSON API (`/pypi/<name>/json`). Blocking HTTP runs on the
/// blocking pool; a request is bounded by a timeout. Proxies are taken from
/// the standard environment variables and certificates from the OS store.
#[derive(Clone)]
pub struct PyPi {
    base_url: String,
    timeout: Duration,
    agent: Arc<OnceLock<ureq::Agent>>,
}

pub const PYPI_URL: &str = "https://pypi.org/pypi";

impl Default for PyPi {
    fn default() -> Self {
        Self::with_base_url(PYPI_URL)
    }
}

impl PyPi {
    pub fn new() -> Self {
        Self::default()
    }

    /// For a mirror that serves the same JSON API, or a local test server.
    pub fn with_base_url(url: impl Into<String>) -> Self {
        Self {
            base_url: url.into().trim_end_matches('/').to_owned(),
            timeout: Duration::from_secs(15),
            agent: Arc::new(OnceLock::new()),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn url(&self, name: &str) -> String {
        format!("{}/{}/json", self.base_url, normalize_name(name))
    }

    fn agent(&self) -> &ureq::Agent {
        self.agent.get_or_init(|| {
            use ureq::tls::{Certificate, RootCerts, TlsConfig};
            let native = rustls_native_certs::load_native_certs();
            let certs: Vec<Certificate<'static>> = native
                .certs
                .iter()
                .map(|c| Certificate::from_der(c.as_ref()).to_owned())
                .collect();
            let roots = if certs.is_empty() {
                RootCerts::WebPki
            } else {
                RootCerts::new_with_certs(&certs)
            };
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(self.timeout))
                .http_status_as_error(false)
                .user_agent(concat!("pyoxide-ide/", env!("CARGO_PKG_VERSION")))
                .tls_config(TlsConfig::builder().root_certs(roots).build())
                .build();
            ureq::Agent::new_with_config(config)
        })
    }

    fn fetch(&self, name: &str) -> Result<IndexProject> {
        let url = self.url(name);
        let mut response = self
            .agent()
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| {
                EnvError::new(
                    ErrorKind::Network,
                    format!("could not reach the package index: {e}"),
                )
            })?;
        let status = response.status().as_u16();
        match status {
            200 => {}
            404 => return Err(not_found(name)),
            other => {
                return Err(EnvError::new(
                    ErrorKind::Network,
                    format!("the package index answered HTTP {other}"),
                ));
            }
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| {
                EnvError::new(
                    ErrorKind::Network,
                    format!("reading the answer of the package index failed: {e}"),
                )
            })?;
        parse_pypi_json(&body)
    }
}

impl std::fmt::Debug for PyPi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyPi")
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[async_trait]
impl PackageIndex for PyPi {
    async fn project(&self, name: &str) -> Result<IndexProject> {
        check_name(name)?;
        let this = self.clone();
        let name = name.trim().to_owned();
        tokio::task::spawn_blocking(move || this.fetch(&name))
            .await
            .map_err(|e| {
                EnvError::new(ErrorKind::Failed, format!("the package lookup failed: {e}"))
            })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const SAMPLE: &str = r#"{
      "info": {
        "name": "Example-Pkg",
        "version": "2.1.0",
        "summary": "An example package",
        "home_page": "",
        "requires_python": ">=3.9",
        "license": "MIT License\n\nCopyright (c) someone, a very long license text follows...",
        "license_expression": null,
        "project_urls": {"Source": "https://example.org/src", "Docs": "https://example.org/docs"}
      },
      "releases": {
        "1.0.0": [{"yanked": false, "requires_python": ">=3.7", "upload_time_iso_8601": "2020-01-01T00:00:00.000000Z"}],
        "2.0.0": [{"yanked": true, "requires_python": ">=3.8", "upload_time_iso_8601": "2023-03-01T00:00:00.000000Z"},
                  {"yanked": true, "requires_python": ">=3.8", "upload_time_iso_8601": "2023-03-01T00:00:01.000000Z"}],
        "2.1.0": [{"yanked": false, "requires_python": ">=3.9", "upload_time_iso_8601": "2024-05-01T00:00:00.000000Z"}],
        "2.2.0rc1": [{"yanked": false, "requires_python": null, "upload_time_iso_8601": "2024-06-01T00:00:00.000000Z"}],
        "0.9": [],
        "1.10": [{"yanked": false}],
        "weird-version": [{"yanked": false}]
      }
    }"#;

    #[test]
    fn parses_a_pypi_answer() {
        let p = parse_pypi_json(SAMPLE.as_bytes()).unwrap();
        assert_eq!(p.name, "Example-Pkg");
        assert_eq!(p.latest, "2.1.0");
        assert_eq!(p.summary.as_deref(), Some("An example package"));
        assert_eq!(p.home_page, None);
        assert_eq!(p.requires_python.as_deref(), Some(">=3.9"));
        assert_eq!(p.license.as_deref(), Some("MIT License"));
        assert_eq!(p.project_urls["Source"], "https://example.org/src");
        let order: Vec<_> = p.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(
            order,
            [
                "2.2.0rc1",
                "2.1.0",
                "2.0.0",
                "1.10",
                "1.0.0",
                "weird-version"
            ]
        );
        let rc = &p.versions[0];
        assert!(rc.prerelease && !rc.yanked);
        assert!(p.versions[2].yanked);
        assert_eq!(p.versions[1].requires_python.as_deref(), Some(">=3.9"));
        assert_eq!(
            p.versions[1].uploaded.as_deref(),
            Some("2024-05-01T00:00:00.000000Z")
        );
    }

    #[test]
    fn works_without_the_releases_map() {
        let body = r#"{"info": {"name": "p", "version": "1.2.3"}}"#;
        let p = parse_pypi_json(body.as_bytes()).unwrap();
        assert_eq!(p.versions.len(), 1);
        assert_eq!(p.versions[0].version, "1.2.3");
        assert!(p.project_urls.is_empty());
        assert_eq!(p.license, None);
    }

    #[test]
    fn prefers_the_license_expression() {
        let body = r#"{"info": {"name": "p", "version": "1", "license": "Apache Software License", "license_expression": "Apache-2.0"}}"#;
        assert_eq!(
            parse_pypi_json(body.as_bytes()).unwrap().license.as_deref(),
            Some("Apache-2.0")
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(
            parse_pypi_json(b"<html>").unwrap_err().kind,
            ErrorKind::Parse
        );
        assert_eq!(parse_pypi_json(b"{}").unwrap_err().kind, ErrorKind::Parse);
    }

    #[test]
    fn urls_use_normalized_names() {
        let pypi = PyPi::with_base_url("https://index.example/pypi/");
        assert_eq!(
            pypi.url("Zope.Interface"),
            "https://index.example/pypi/zope-interface/json"
        );
    }

    #[test]
    fn names_are_validated_before_any_request() {
        for bad in ["", "  ", "../etc", "a/b", "a b", "-x", "a?b=c", "名前"] {
            assert_eq!(
                check_name(bad).unwrap_err().kind,
                ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        for ok in ["requests", "Zope.Interface", "a_b-c", "x1"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
    }

    #[tokio::test]
    async fn static_index_finds_by_normalized_name() {
        let project = parse_pypi_json(SAMPLE.as_bytes()).unwrap();
        let index = StaticIndex::new().with(project);
        assert_eq!(index.project("example_pkg").await.unwrap().latest, "2.1.0");
        assert_eq!(
            index.project("nothing").await.unwrap_err().kind,
            ErrorKind::PackageNotFound
        );
        assert_eq!(
            index.project("a/b").await.unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }

    /// Answer each connection with the next canned `(status line, body)`,
    /// recording the request lines. Local only: no external network.
    fn serve(
        responses: Vec<(&'static str, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/pypi", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                seen.push(request.lines().next().unwrap_or_default().to_owned());
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).unwrap();
            }
            seen
        });
        (base, handle)
    }

    #[tokio::test]
    async fn fetches_over_http_from_a_local_server() {
        let (base, server) = serve(vec![
            ("200 OK", SAMPLE.to_owned()),
            ("404 Not Found", "{\"message\": \"Not Found\"}".to_owned()),
            ("503 Service Unavailable", String::new()),
        ]);
        let pypi = PyPi::with_base_url(base);
        let p = pypi.project("Example_Pkg").await.unwrap();
        assert_eq!(p.latest, "2.1.0");
        let e = pypi.project("missing").await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::PackageNotFound);
        assert_eq!(e.message, "`missing` was not found on the index");
        let e = pypi.project("busy").await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Network);
        assert!(e.message.contains("503"));
        let seen = server.join().unwrap();
        assert_eq!(seen[0], "GET /pypi/example-pkg/json HTTP/1.1");
        assert_eq!(seen[1], "GET /pypi/missing/json HTTP/1.1");
    }

    #[tokio::test]
    async fn unreachable_servers_are_network_errors() {
        // Bind and drop to get a port nothing listens on.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let pypi = PyPi::with_base_url(format!("http://127.0.0.1:{port}/pypi"))
            .with_timeout(Duration::from_secs(3));
        let e = pypi.project("anything").await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Network);
    }
}
