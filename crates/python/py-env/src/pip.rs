use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{EnvError, ErrorKind, Result};
use crate::packages::{Outcome, Package, parse_package_list};
use crate::process::{Finished, RunContext, Spec, run};

const QUICK: Duration = Duration::from_secs(60);
const LONG: Duration = Duration::from_secs(30 * 60);

/// `python -m pip` of one interpreter: the fallback when uv is not installed.
#[derive(Debug, Clone)]
pub struct Pip {
    python: PathBuf,
}

impl Pip {
    pub fn new(python: impl Into<PathBuf>) -> Self {
        Self {
            python: python.into(),
        }
    }

    pub fn python(&self) -> &Path {
        &self.python
    }

    fn spec(&self, args: Vec<OsString>) -> Spec {
        let mut a: Vec<OsString> = vec!["-m".into(), "pip".into()];
        a.extend(args);
        let mut spec = Spec::new(&self.python)
            .args(a)
            .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
            .env("PIP_NO_COLOR", "1")
            .env("PIP_NO_INPUT", "1")
            .env("PIP_PROGRESS_BAR", "off")
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8");
        spec.forward_stdout = true;
        spec
    }

    async fn exec(
        &self,
        args: Vec<OsString>,
        ctx: &RunContext,
        timeout: Duration,
        forward_stdout: bool,
    ) -> Result<(Finished, String)> {
        let mut spec = self.spec(args);
        spec.forward_stdout = forward_stdout;
        let command = format!(
            "{} -m pip {}",
            self.python.display(),
            spec.args[2..]
                .iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let done = run("pip", spec, ctx, timeout).await?;
        if done.success() {
            Ok((done, command))
        } else if done.stderr.contains("No module named pip") {
            Err(EnvError::new(
                ErrorKind::ToolMissing,
                format!(
                    "pip is not installed in {}. Install uv, or run `{} -m ensurepip`.",
                    self.python.display(),
                    self.python.display()
                ),
            )
            .with_tool("pip"))
        } else {
            Err(EnvError::from_failure("pip", done.code, &done.stderr))
        }
    }

    /// `python -m pip list --format json`, with `--outdated` when asked.
    pub async fn list(&self, outdated: bool, ctx: &RunContext) -> Result<Vec<Package>> {
        let mut a: Vec<OsString> = vec!["list".into(), "--format".into(), "json".into()];
        if outdated {
            a.push("--outdated".into());
        }
        let (done, _) = self
            .exec(a, ctx, if outdated { LONG } else { QUICK }, false)
            .await?;
        parse_package_list(&done.stdout)
    }

    /// `python -m pip install`
    pub async fn install(
        &self,
        packages: &[String],
        requirements: Option<&Path>,
        upgrade: bool,
        ctx: &RunContext,
    ) -> Result<Outcome> {
        if packages.is_empty() && requirements.is_none() {
            return Err(EnvError::invalid(
                "name a package or a requirements file to install",
            ));
        }
        let started = Instant::now();
        let mut a: Vec<OsString> = vec!["install".into()];
        if upgrade {
            a.push("--upgrade".into());
        }
        if let Some(r) = requirements {
            a.push("--requirement".into());
            a.push(r.into());
        }
        if !packages.is_empty() {
            a.push("--".into());
            a.extend(packages.iter().map(OsString::from));
        }
        let (done, command) = self.exec(a, ctx, LONG, true).await?;
        Ok(Outcome::new(command, &done.stdout, &done.stderr, started))
    }

    /// `python -m pip uninstall --yes`
    pub async fn uninstall(&self, packages: &[String], ctx: &RunContext) -> Result<Outcome> {
        if packages.is_empty() {
            return Err(EnvError::invalid("name at least one package to uninstall"));
        }
        let started = Instant::now();
        let mut a: Vec<OsString> = vec!["uninstall".into(), "--yes".into(), "--".into()];
        a.extend(packages.iter().map(OsString::from));
        let (done, command) = self.exec(a, ctx, LONG, true).await?;
        Ok(Outcome::new(command, &done.stdout, &done.stderr, started))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for an interpreter: a script that answers like `python -m pip`.
    fn fake_python(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("python");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[tokio::test]
    async fn lists_packages() {
        let dir = tempfile::tempdir().unwrap();
        let py = fake_python(
            dir.path(),
            r#"[ "$1 $2 $3 $4 $5" = "-m pip list --format json" ] && echo '[{"name":"pip","version":"24.0"}]' || exit 9"#,
        );
        let list = Pip::new(py)
            .list(false, &RunContext::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "pip");
    }

    #[tokio::test]
    async fn outdated_adds_the_flag() {
        let dir = tempfile::tempdir().unwrap();
        let py = fake_python(
            dir.path(),
            r#"[ "$6" = "--outdated" ] && echo '[{"name":"a","version":"1","latest_version":"2","latest_filetype":"wheel"}]' || exit 9"#,
        );
        let list = Pip::new(py)
            .list(true, &RunContext::default())
            .await
            .unwrap();
        assert_eq!(list[0].latest_version.as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn install_passes_requirements_and_packages() {
        let dir = tempfile::tempdir().unwrap();
        let py = fake_python(dir.path(), r#"echo "args: $*" 1>&2"#);
        let out = Pip::new(py)
            .install(
                &["requests".into()],
                Some(Path::new("req.txt")),
                true,
                &RunContext::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            out.log,
            ["args: -m pip install --upgrade --requirement req.txt -- requests"]
        );
        assert!(
            out.command
                .ends_with("-m pip install --upgrade --requirement req.txt -- requests")
        );
    }

    #[tokio::test]
    async fn uninstall_confirms_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let py = fake_python(dir.path(), r#"echo "args: $*""#);
        let out = Pip::new(py)
            .uninstall(&["a".into(), "b".into()], &RunContext::default())
            .await
            .unwrap();
        assert_eq!(out.log, ["args: -m pip uninstall --yes -- a b"]);
    }

    #[tokio::test]
    async fn missing_pip_is_reported_as_a_missing_tool() {
        let dir = tempfile::tempdir().unwrap();
        let py = fake_python(
            dir.path(),
            "echo '/x/python: No module named pip' 1>&2; exit 1",
        );
        let e = Pip::new(py)
            .list(false, &RunContext::default())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::ToolMissing);
        assert_eq!(e.tool.as_deref(), Some("pip"));
        assert!(e.message.contains("ensurepip"));
    }

    #[tokio::test]
    async fn failures_carry_pips_message() {
        let dir = tempfile::tempdir().unwrap();
        let py = fake_python(
            dir.path(),
            "echo 'ERROR: No matching distribution found for nopkg' 1>&2; exit 1",
        );
        let e = Pip::new(py)
            .install(&["nopkg".into()], None, false, &RunContext::default())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::PackageNotFound);
        assert!(e.message.contains("nopkg"));
    }

    #[tokio::test]
    async fn empty_requests_are_rejected_without_running_anything() {
        let pip = Pip::new("/nonexistent/python");
        assert_eq!(
            pip.install(&[], None, false, &RunContext::default())
                .await
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput
        );
        assert_eq!(
            pip.uninstall(&[], &RunContext::default())
                .await
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput
        );
    }
}
