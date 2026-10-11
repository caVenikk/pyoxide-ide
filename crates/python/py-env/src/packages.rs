use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{EnvError, Result};

/// Which tool answered or performed a package operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Uv,
    Pip,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Uv => "uv",
            Backend::Pip => "pip",
        }
    }
}

/// A package installed in an environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    /// The source directory when installed in editable mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable_project_location: Option<PathBuf>,
    /// Newest version on the index; only set by outdated listings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// `wheel` or `sdist`; only set by outdated listings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_filetype: Option<String>,
}

/// Parse `uv pip list --format json` / `pip list --format json`, with or
/// without `--outdated`.
pub fn parse_package_list(json: &str) -> Result<Vec<Package>> {
    let json = json.trim();
    if json.is_empty() {
        return Ok(Vec::new());
    }
    // pip may print warnings before the JSON array on stdout; the array is on the last such line.
    let start = json.find('[').unwrap_or(0);
    serde_json::from_str(&json[start..])
        .map_err(|e| EnvError::parse(format!("unreadable package list: {e}")))
}

/// What an operation that changes an environment did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// The command line that ran.
    pub command: String,
    /// The tool's own report (`Installed 3 packages in 12ms`, `+ requests==2.32.5`).
    pub log: Vec<String>,
    pub duration_ms: u64,
}

const LOG_LINES: usize = 80;

impl Outcome {
    pub(crate) fn new(
        command: String,
        stdout: &str,
        stderr: &str,
        started: std::time::Instant,
    ) -> Self {
        let mut log: Vec<String> = stderr
            .lines()
            .chain(stdout.lines())
            .map(|l| l.trim_end().to_owned())
            .filter(|l| !l.is_empty())
            .collect();
        if log.len() > LOG_LINES {
            let cut = log.len() - LOG_LINES;
            log.drain(..cut);
        }
        Outcome {
            command,
            log,
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uv_pip_list() {
        let json = r#"[{"name":"certifi","version":"2024.8.30"},{"name":"mypkg","version":"0.1.0","editable_project_location":"/work/mypkg"}]"#;
        let p = parse_package_list(json).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "certifi");
        assert_eq!(p[0].editable_project_location, None);
        assert_eq!(
            p[1].editable_project_location,
            Some(PathBuf::from("/work/mypkg"))
        );
    }

    #[test]
    fn parses_outdated_listing() {
        let json = r#"[{"name":"requests","version":"2.31.0","latest_version":"2.32.5","latest_filetype":"wheel"}]"#;
        let p = parse_package_list(json).unwrap();
        assert_eq!(p[0].latest_version.as_deref(), Some("2.32.5"));
        assert_eq!(p[0].latest_filetype.as_deref(), Some("wheel"));
    }

    #[test]
    fn tolerates_empty_output_and_leading_warnings() {
        assert!(parse_package_list("").unwrap().is_empty());
        assert!(parse_package_list("[]\n").unwrap().is_empty());
        let noisy = "WARNING: something odd\n[{\"name\":\"a\",\"version\":\"1\"}]";
        assert_eq!(parse_package_list(noisy).unwrap().len(), 1);
        assert!(parse_package_list("not json [").is_err());
    }

    #[test]
    fn outcome_keeps_the_last_lines() {
        let stderr: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let o = Outcome::new("uv sync".into(), "", &stderr, std::time::Instant::now());
        assert_eq!(o.log.len(), LOG_LINES);
        assert_eq!(o.log.last().unwrap(), "line 199");
        assert_eq!(o.command, "uv sync");
    }
}
