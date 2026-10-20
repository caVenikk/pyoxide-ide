use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
pub use tokio_util::sync::CancellationToken;

use crate::error::{EnvError, ErrorKind, Result, redact_credentials};

/// Which output stream a progress line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One line of output of a running tool (uv prints its progress on stderr).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub stream: Stream,
    pub line: String,
}

/// Receives output lines while a tool runs. Called from the task that reads
/// the pipe: keep it quick and non-blocking (send on a channel, push to a
/// buffer).
pub type ProgressSink = Arc<dyn Fn(Progress) + Send + Sync>;

/// Per-call knobs shared by every operation: progress lines, cooperative
/// cancellation and a timeout that overrides the operation's default.
///
/// Dropping the future of an operation also stops the tool (child processes
/// are killed on drop), which is what the command registry relies on.
#[derive(Clone, Default)]
pub struct RunContext {
    pub progress: Option<ProgressSink>,
    pub cancel: Option<CancellationToken>,
    pub timeout: Option<Duration>,
}

impl RunContext {
    pub fn with_progress(mut self, sink: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(sink));
        self
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl std::fmt::Debug for RunContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunContext")
            .field("progress", &self.progress.is_some())
            .field("cancel", &self.cancel.is_some())
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Whether an environment variable steers which Python a tool picks or where
/// it puts environments and packages: `UV_*`, `PIP_*`, `VIRTUAL_ENV`,
/// `CONDA_PREFIX`, `CONDA_DEFAULT_ENV`, `PYTHONHOME` and `PYTHONPATH`.
pub fn is_steering_var(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    name.starts_with("UV_")
        || name.starts_with("PIP_")
        || matches!(
            name.as_str(),
            "VIRTUAL_ENV" | "CONDA_PREFIX" | "CONDA_DEFAULT_ENV" | "PYTHONHOME" | "PYTHONPATH"
        )
}

/// The steering variables ([`is_steering_var`]) set in this process. A test
/// that spawns a tool removes them from the child, so that the developer's
/// shell does not decide its outcome; the product leaves them alone.
pub fn steering_env() -> Vec<OsString> {
    std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| is_steering_var(name))
        .collect()
}

/// What to run.
#[derive(Debug, Clone, Default)]
pub(crate) struct Spec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
    /// Variables of this process's environment the child must not inherit.
    /// Applied before `env`, so a variable in both is set.
    pub env_remove: Vec<OsString>,
    /// Forward stdout lines to the progress sink too. Off for tools whose
    /// stdout is data (JSON), on for tools that narrate on stdout.
    pub forward_stdout: bool,
}

impl Spec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Keep the child from inheriting the variables that steer Python tooling
    /// ([`steering_env`]).
    pub fn without_steering_env(mut self) -> Self {
        self.env_remove.extend(steering_env());
        self
    }

    /// The command line for messages and logs. Never contains environment values.
    pub fn display(&self) -> String {
        let mut s = self.program.to_string_lossy().into_owned();
        for a in &self.args {
            s.push(' ');
            s.push_str(&a.to_string_lossy());
        }
        redact_credentials(&s)
    }
}

/// A finished process.
#[derive(Debug, Clone)]
pub(crate) struct Finished {
    pub code: Option<i32>,
    pub stdout: String,
    /// Head and tail of the error output, enough to explain a failure.
    pub stderr: String,
}

impl Finished {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

const STDOUT_LIMIT: usize = 32 * 1024 * 1024;
const STDERR_HEAD_LINES: usize = 100;
const STDERR_TAIL_LINES: usize = 200;

/// Run a program to completion with a timeout and cooperative cancellation.
/// `tool` names the program in error messages.
pub(crate) async fn run(
    tool: &str,
    spec: Spec,
    ctx: &RunContext,
    default_timeout: Duration,
) -> Result<Finished> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    for k in &spec.env_remove {
        cmd.env_remove(k);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        // No console window when the IDE (a GUI process) starts a tool.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = spawn(&mut cmd)
        .await
        .map_err(|e| spawn_error(tool, &spec, &e))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let sink = ctx.progress.clone();
    let timeout = ctx.timeout.unwrap_or(default_timeout);
    let cancelled = async {
        match &ctx.cancel {
            Some(t) => t.cancelled().await,
            None => std::future::pending().await,
        }
    };

    let out = pump(
        stdout,
        Stream::Stdout,
        spec.forward_stdout.then(|| sink.clone()).flatten(),
        Keep::All(STDOUT_LIMIT),
    );
    let err = pump(
        stderr,
        Stream::Stderr,
        sink,
        Keep::HeadTail(STDERR_HEAD_LINES, STDERR_TAIL_LINES),
    );
    let work = async {
        let (out, err, status) = tokio::join!(out, err, child.wait());
        (out, err, status)
    };

    tokio::select! {
        (out, err, status) = work => {
            let status = status.map_err(|e| EnvError::io(format!("waiting for {tool}"), &e).with_tool(tool))?;
            Ok(Finished { code: status.code(), stdout: out, stderr: err })
        }
        () = cancelled => {
            let _ = child.kill().await;
            Err(EnvError::new(ErrorKind::Cancelled, format!("{tool} was cancelled")).with_tool(tool))
        }
        () = tokio::time::sleep(timeout) => {
            let _ = child.kill().await;
            Err(EnvError::new(
                ErrorKind::Timeout,
                format!("{tool} did not finish within {}", human_duration(timeout)),
            )
            .with_tool(tool))
        }
    }
}

/// Like [`run`], but a non-zero exit becomes a typed error.
pub(crate) async fn run_ok(
    tool: &str,
    spec: Spec,
    ctx: &RunContext,
    default_timeout: Duration,
) -> Result<Finished> {
    let done = run(tool, spec, ctx, default_timeout).await?;
    if done.success() {
        Ok(done)
    } else {
        Err(EnvError::from_failure(tool, done.code, &done.stderr))
    }
}

/// Linux refuses to execute a file that another process still holds open for
/// writing (ETXTBSY). That happens when an executable was written a moment
/// ago and a concurrent fork inherited the descriptor until its exec; it
/// clears within milliseconds, so retry a few times.
async fn spawn(cmd: &mut Command) -> std::io::Result<tokio::process::Child> {
    const ETXTBSY: i32 = 26;
    let mut attempt = 0u64;
    loop {
        match cmd.spawn() {
            Err(e) if cfg!(unix) && e.raw_os_error() == Some(ETXTBSY) && attempt < 5 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(10 * attempt)).await;
            }
            other => return other,
        }
    }
}

fn spawn_error(tool: &str, spec: &Spec, e: &std::io::Error) -> EnvError {
    if e.kind() == std::io::ErrorKind::NotFound {
        let mut err = EnvError::new(
            ErrorKind::ToolMissing,
            format!("{tool} was not found ({})", spec.program.to_string_lossy()),
        );
        err.tool = Some(tool.to_owned());
        err
    } else {
        EnvError::io(format!("starting {tool}"), e).with_tool(tool)
    }
}

pub(crate) fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 120 && s.is_multiple_of(60) {
        format!("{} minutes", s / 60)
    } else if s >= 1 {
        format!("{s} s")
    } else {
        format!("{} ms", d.as_millis())
    }
}

enum Keep {
    /// Everything, up to a byte limit (for data on stdout).
    All(usize),
    /// The first and the last lines (for error output).
    HeadTail(usize, usize),
}

async fn pump<R: AsyncRead + Unpin>(
    reader: R,
    stream: Stream,
    sink: Option<ProgressSink>,
    keep: Keep,
) -> String {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    let mut all = String::new();
    let mut head: Vec<String> = Vec::new();
    let mut tail: VecDeque<String> = VecDeque::new();
    let mut dropped = 0usize;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let mut line = String::from_utf8_lossy(&buf).into_owned();
        while line.ends_with(['\n', '\r']) {
            line.pop();
        }
        if let Some(sink) = &sink {
            sink(Progress {
                stream,
                line: redact_credentials(&line),
            });
        }
        match keep {
            Keep::All(limit) => {
                if all.len() + line.len() < limit {
                    all.push_str(&line);
                    all.push('\n');
                }
            }
            Keep::HeadTail(h, t) => {
                if head.len() < h {
                    head.push(line);
                } else {
                    tail.push_back(line);
                    if tail.len() > t {
                        tail.pop_front();
                        dropped += 1;
                    }
                }
            }
        }
    }
    match keep {
        Keep::All(_) => all,
        Keep::HeadTail(..) => {
            let mut out = head.join("\n");
            if dropped > 0 {
                out.push_str(&format!("\n... ({dropped} lines omitted)"));
            }
            for l in tail {
                out.push('\n');
                out.push_str(&l);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::sync::Mutex;
    #[cfg(unix)]
    use std::time::Instant;

    #[cfg(unix)]
    fn sh(script: &str) -> Spec {
        Spec::new("sh").args(["-c", script])
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn captures_output_and_exit_code() {
        let done = run(
            "sh",
            sh("echo out; echo err 1>&2; exit 3"),
            &RunContext::default(),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert_eq!(done.code, Some(3));
        assert_eq!(done.stdout, "out\n");
        assert_eq!(done.stderr, "err");
        assert!(!done.success());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removed_variables_are_not_inherited_unless_set_again() {
        let probe = |spec: Spec| async {
            run("sh", spec, &RunContext::default(), Duration::from_secs(10))
                .await
                .unwrap()
                .stdout
        };
        let script = r#"echo "${HOME-unset}""#;
        let mut spec = sh(script);
        spec.env_remove.push("HOME".into());
        assert_eq!(probe(spec).await, "unset\n");
        let mut spec = sh(script).env("HOME", "set-again");
        spec.env_remove.push("HOME".into());
        assert_eq!(probe(spec).await, "set-again\n");
    }

    #[test]
    fn steering_variables_are_recognised() {
        for name in [
            "UV_PYTHON",
            "UV_INDEX_URL",
            "uv_cache_dir",
            "PIP_INDEX_URL",
            "VIRTUAL_ENV",
            "CONDA_PREFIX",
            "PYTHONHOME",
        ] {
            assert!(is_steering_var(OsStr::new(name)), "{name}");
        }
        for name in ["PATH", "HOME", "PYOXIDE_UV", "NO_COLOR", "UVICORN_PORT"] {
            assert!(!is_steering_var(OsStr::new(name)), "{name}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_ok_maps_failure_to_error() {
        let e = run_ok(
            "sh",
            sh("echo 'error: it broke' 1>&2; exit 2"),
            &RunContext::default(),
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Failed);
        assert_eq!(e.exit_code, Some(2));
        assert_eq!(e.message, "it broke");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forwards_progress_lines_in_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let ctx = RunContext::default().with_progress(move |p| s.lock().unwrap().push(p));
        let mut spec = sh("echo one 1>&2; echo two 1>&2; echo data");
        spec.forward_stdout = false;
        run("sh", spec, &ctx, Duration::from_secs(10))
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        let lines: Vec<_> = seen.iter().map(|p| p.line.as_str()).collect();
        assert_eq!(lines, ["one", "two"]);
        assert!(seen.iter().all(|p| p.stream == Stream::Stderr));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn redacts_credentials_in_progress_lines() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let ctx = RunContext::default().with_progress(move |p| s.lock().unwrap().push(p.line));
        run(
            "sh",
            sh("echo 'GET https://me:pw@example.org/simple' 1>&2"),
            &ctx,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            "GET https://***@example.org/simple"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_process() {
        let started = Instant::now();
        let e = run(
            "sh",
            sh("sleep 30"),
            &RunContext::default(),
            Duration::from_millis(150),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_is_prompt() {
        let token = CancellationToken::new();
        let ctx = RunContext::default().with_cancel(token.clone());
        let t = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            t.cancel();
        });
        let started = Instant::now();
        let e = run("sh", sh("sleep 30"), &ctx, Duration::from_secs(60))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn missing_program_is_tool_missing() {
        let e = run(
            "nonexistent-tool",
            Spec::new("/definitely/not/here/nonexistent-tool"),
            &RunContext::default(),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::ToolMissing);
        assert_eq!(e.tool.as_deref(), Some("nonexistent-tool"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn long_error_output_keeps_head_and_tail() {
        let done = run(
            "sh",
            sh("i=0; while [ $i -lt 1000 ]; do echo line$i 1>&2; i=$((i+1)); done"),
            &RunContext::default(),
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        assert!(done.stderr.starts_with("line0\n"));
        assert!(done.stderr.contains("lines omitted"));
        assert!(done.stderr.ends_with("line999"));
    }

    #[test]
    fn display_hides_credentials() {
        let s = Spec::new("uv").args(["pip", "install", "https://u:p@host/pkg.whl"]);
        assert_eq!(s.display(), "uv pip install https://***@host/pkg.whl");
    }

    #[test]
    fn durations_read_well() {
        assert_eq!(human_duration(Duration::from_secs(30)), "30 s");
        assert_eq!(human_duration(Duration::from_secs(1800)), "30 minutes");
        assert_eq!(human_duration(Duration::from_millis(250)), "250 ms");
    }
}
