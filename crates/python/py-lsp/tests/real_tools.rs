//! The Python tools of this machine on a sample project: the lookup, the
//! versions and ruff's rule names, then the servers as the product
//! configures them, running in the LSP client. Ignored by default: they
//! need ruff, ty and basedpyright installed (and uv for the environment of
//! the sample). The tools are looked up as the IDE looks them up; the
//! folder `PYOXIDE_PY_TOOLS` names is searched first, for tools installed
//! only for the tests. Run them one at a time: they report the memory of
//! the servers this process started.
//!
//! ```sh
//! export UV_TOOL_DIR=/tmp/py-tools/tools UV_TOOL_BIN_DIR=/tmp/py-tools/bin
//! uv tool install ruff && uv tool install ty && uv tool install basedpyright
//! PYOXIDE_PY_TOOLS=/tmp/py-tools/bin cargo test -p py-lsp --test real_tools \
//!     -- --ignored --test-threads=1 --nocapture
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use code_intel::{
    CodeActionContext, CodeIntel, Diagnostic, DocumentId, IntelEvent, Rope, ServerState,
};
use futures::executor::block_on;
use lsp_client::LspCodeIntel;
use py_lsp::lookup::{Executable, SearchPaths};
use py_lsp::ruff::{
    DISABLE_FOR_FILE, DISABLE_FOR_LINE, DISABLE_FOR_PROJECT, RuffCustomizer, load_rules,
};
use py_lsp::services::{PROBE_TIMEOUT, PrepareOptions, ProjectState, prepare, probe};
use py_lsp::settings::{self, Tool, TypeEngine};

const WAIT: Duration = Duration::from_secs(90);

/// An unused import, a call with a wrong argument, a name defined
/// nowhere, and a module of the sample's environment.
const SAMPLE: &str = "import os\n\nimport samplepkg\n\n\ndef greet(name: str) -> str:\n    return \"hi \" + name + samplepkg.NAME\n\n\ngreet(1)\nprint(undefined_name)\n";

/// Where the tools are searched: `PYOXIDE_PY_TOOLS` first.
fn search() -> SearchPaths {
    let mut search = SearchPaths::from_env();
    if let Some(dir) = std::env::var_os("PYOXIDE_PY_TOOLS") {
        search.path.insert(0, PathBuf::from(dir));
    }
    search
}

/// A small project: `pyproject.toml`, [`SAMPLE`] in `app.py`, a file with
/// a syntax error, and (with uv) a virtual environment outside the
/// project with the package `samplepkg`, so that only the interpreter
/// the servers get resolves it. Returns the project and the
/// environment's interpreter.
fn sample(search: &SearchPaths) -> (tempfile::TempDir, PathBuf, Option<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"sample\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(root.join("app.py"), SAMPLE).unwrap();
    std::fs::write(root.join("broken.py"), "def broken(:\n    pass\n").unwrap();
    let env = root.parent().unwrap().join("env");
    let python = search.find("uv").and_then(|uv| {
        let made = std::process::Command::new(uv)
            .args(["venv", "--quiet"])
            .arg(&env)
            .status()
            .ok()?
            .success();
        let python = py_env::paths::venv_python(&env);
        (made && python.exists()).then_some(python)
    });
    if let Some(python) = &python {
        let site = site_packages(&env).expect("the environment has site-packages");
        std::fs::create_dir_all(site.join("samplepkg")).unwrap();
        std::fs::write(
            site.join("samplepkg").join("__init__.py"),
            "NAME: str = \"sample\"\n",
        )
        .unwrap();
        eprintln!("environment: {}", python.display());
    } else {
        eprintln!("uv did not make an environment: imports stay unresolved");
    }
    (dir, root, python)
}

fn site_packages(env: &Path) -> Option<PathBuf> {
    let windows = env.join("Lib").join("site-packages");
    if windows.is_dir() {
        return Some(windows);
    }
    std::fs::read_dir(env.join("lib"))
        .ok()?
        .flatten()
        .map(|e| e.path().join("site-packages"))
        .find(|p| p.is_dir())
}

fn prepared(root: &Path, python: Option<&Path>, search: &SearchPaths) -> ProjectState {
    let mut state = prepare(
        root,
        python,
        PrepareOptions {
            import: true,
            search: Some(search),
        },
    );
    probe(&mut state, PROBE_TIMEOUT);
    for tool in &state.tools {
        if tool.runs {
            eprintln!(
                "{}: {} {} ({})",
                tool.name,
                tool.command.as_deref().unwrap_or("-"),
                tool.version.as_deref().unwrap_or("?"),
                tool.lookup.as_deref().unwrap_or("-")
            );
        }
    }
    state
}

fn from(item: &Diagnostic, server: &str) -> bool {
    item.server.as_ref().is_some_and(|s| s.as_str() == server)
}

fn has(items: &[Diagnostic], server: &str, code: &str) -> bool {
    items
        .iter()
        .any(|d| from(d, server) && d.code.as_deref() == Some(code))
}

fn any_from(items: &[Diagnostic], server: &str) -> bool {
    items.iter().any(|d| from(d, server))
}

/// When each server was running and first reported problems, from the
/// moment the specs were set.
struct Timing {
    started: Instant,
    running: HashMap<String, Duration>,
    reported: HashMap<String, Duration>,
}

impl Timing {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            running: HashMap::new(),
            reported: HashMap::new(),
        }
    }

    fn print(&self) {
        let mut servers: Vec<_> = self.running.keys().collect();
        servers.sort();
        for server in servers {
            eprintln!(
                "{server}: running after {} ms, first problems after {} ms",
                self.running[server].as_millis(),
                self.reported
                    .get(server)
                    .map_or("-".into(), |d| d.as_millis().to_string())
            );
        }
    }
}

/// The diagnostics of `doc` once `accept` takes them.
fn wait(
    events: &process::flume::Receiver<IntelEvent>,
    doc: &DocumentId,
    timing: &mut Timing,
    accept: impl Fn(&[Diagnostic]) -> bool,
) -> Vec<Diagnostic> {
    let deadline = Instant::now() + WAIT;
    let mut last = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(IntelEvent::Diagnostics { doc: of, items, .. }) if of == *doc => {
                for item in &items {
                    if let Some(server) = &item.server {
                        let elapsed = timing.started.elapsed();
                        timing.reported.entry(server.to_string()).or_insert(elapsed);
                    }
                }
                if accept(&items) {
                    return items;
                }
                last = items;
            }
            Ok(IntelEvent::Status(status)) => match status.state {
                ServerState::Running => {
                    let elapsed = timing.started.elapsed();
                    timing
                        .running
                        .entry(status.id.to_string())
                        .or_insert(elapsed);
                }
                ServerState::Terminated { error_output } => {
                    panic!("{} terminated: {error_output}", status.id)
                }
                _ => {}
            },
            Ok(_) => {}
            Err(_) => panic!("no such diagnostics within {WAIT:?}; last: {last:#?}"),
        }
    }
}

/// The memory of the servers this process started: each child process
/// with its descendants (a launcher and its runtime), in MB.
#[cfg(unix)]
fn server_memory() -> Vec<(String, u64)> {
    fn run(program: &str, args: &[&str]) -> String {
        std::process::Command::new(program)
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }
    fn children(pid: u32) -> Vec<u32> {
        run("pgrep", &["-P", &pid.to_string()])
            .split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect()
    }
    fn tree_kb(pid: u32) -> u64 {
        let own: u64 = run("ps", &["-o", "rss=", "-p", &pid.to_string()])
            .trim()
            .parse()
            .unwrap_or(0);
        own + children(pid).into_iter().map(tree_kb).sum::<u64>()
    }
    children(std::process::id())
        .into_iter()
        .map(|pid| {
            let name = run("ps", &["-o", "comm=", "-p", &pid.to_string()]);
            let name = Path::new(name.trim())
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name, tree_kb(pid) / 1024)
        })
        .collect()
}

#[cfg(not(unix))]
fn server_memory() -> Vec<(String, u64)> {
    Vec::new()
}

fn print_memory() {
    for (name, mb) in server_memory() {
        eprintln!("{name}: {mb} MB");
    }
}

#[test]
#[ignore = "needs ruff and ty installed"]
fn ruff_and_ty_on_a_sample_project() {
    let search = search();
    let (_dir, root, python) = sample(&search);
    let state = prepared(&root, python.as_deref(), &search);
    assert!(state.errors.is_empty(), "{:?}", state.errors);
    for tool in [Tool::Ruff, Tool::Ty] {
        let tool = state.tool(tool);
        assert_eq!(tool.problem, None, "{tool:?}");
        assert!(tool.version.is_some(), "{tool:?}");
        assert!(matches!(tool.executable(), Some(Executable::Path(_))));
    }
    let ruff = Arc::new(RuffCustomizer::new(&root));
    let rules = load_rules(state.tool(Tool::Ruff).executable().unwrap(), PROBE_TIMEOUT);
    assert!(rules.len() > 500, "{} rules", rules.len());
    assert_eq!(rules["F401"], "unused-import");
    ruff.set_rules(rules);
    let specs = state.specs(Some(&ruff));
    let ids: Vec<String> = specs.iter().map(|s| s.id.to_string()).collect();
    assert_eq!(ids, ["ruff", "ty"]);

    // The client starts without servers; the specs come once found.
    let intel = LspCodeIntel::new(Vec::new());
    let events = intel.events();
    let app = DocumentId::new(root.join("app.py"));
    intel.did_open(&app, "python", 1, &Rope::from_str(SAMPLE));
    let mut timing = Timing::new();
    intel.set_specs(specs);
    let items = wait(&events, &app, &mut timing, |items| {
        has(items, "ruff", "unused-import") && has(items, "ty", "invalid-argument-type")
    });
    // Rule names instead of codes.
    let unused = items
        .iter()
        .find(|d| d.code.as_deref() == Some("unused-import"))
        .unwrap();
    assert!(
        unused
            .summary()
            .starts_with("`os` imported but unused  Ruff(unused-import)"),
        "{}",
        unused.summary()
    );
    // The environment's package resolves through the interpreter ty got.
    if python.is_some() {
        assert!(
            !items
                .iter()
                .any(|d| from(d, "ty") && d.message.contains("samplepkg")),
            "{items:#?}"
        );
    }

    // ruff's quick fix as the reference shows it.
    let context = CodeActionContext {
        diagnostics: vec![unused.clone()],
        ..CodeActionContext::default()
    };
    let actions = block_on(intel.code_actions(&app, unused.range.clone(), context)).unwrap();
    let fix = actions
        .iter()
        .find(|a| {
            a.title
                .starts_with("Ruff (unused-import): Remove unused import")
        })
        .unwrap_or_else(|| panic!("{actions:#?}"));
    let subs: Vec<&str> = fix.children.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(
        subs,
        [DISABLE_FOR_FILE, DISABLE_FOR_LINE, DISABLE_FOR_PROJECT]
    );
    assert!(
        !actions
            .iter()
            .any(|a| a.title.contains("Disable for this line")),
        "{actions:#?}"
    );
    let file = fix.children[0].edit.as_ref().unwrap();
    let edits: Vec<_> = file.edits_for(&root.join("app.py")).collect();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].text, "# ruff: noqa: F401\n");
    let project = fix.children[2].edit.as_ref().unwrap();
    assert_eq!(project.files[0].path, root.join("pyproject.toml"));
    assert!(
        project.files[0].edits[0]
            .text
            .contains("ignore = [\"F401\"]")
    );

    // A syntax error: ty reports it, ruff does not while ty runs.
    let broken = DocumentId::new(root.join("broken.py"));
    intel.did_open(
        &broken,
        "python",
        1,
        &Rope::from_str("def broken(:\n    pass\n"),
    );
    wait(&events, &broken, &mut timing, |items| any_from(items, "ty"));
    let quiet = Instant::now() + Duration::from_secs(3);
    while let Ok(event) = events.recv_timeout(quiet.saturating_duration_since(Instant::now())) {
        if let IntelEvent::Diagnostics { doc, items, .. } = event
            && doc == broken
        {
            assert!(!any_from(&items, "ruff"), "{items:#?}");
        }
    }

    timing.print();
    print_memory();
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
#[ignore = "needs ruff and basedpyright installed"]
fn basedpyright_on_a_sample_project() {
    let search = search();
    let (_dir, root, python) = sample(&search);
    settings::select_type_engine(&root, TypeEngine::Basedpyright).unwrap();
    let state = prepared(&root, python.as_deref(), &search);
    let basedpyright = state.tool(Tool::Basedpyright);
    assert_eq!(basedpyright.problem, None, "{basedpyright:?}");
    assert!(basedpyright.version.is_some());
    let specs = state.specs(None);
    let server = specs
        .iter()
        .find(|s| s.id.as_str() == "basedpyright")
        .unwrap();
    assert_eq!(server.command.args, ["--stdio"]);
    assert_eq!(
        server.settings["basedpyright"]["analysis"]["typeCheckingMode"],
        "standard"
    );

    let intel = LspCodeIntel::new(Vec::new());
    let events = intel.events();
    let app = DocumentId::new(root.join("app.py"));
    intel.did_open(&app, "python", 1, &Rope::from_str(SAMPLE));
    let mut timing = Timing::new();
    intel.set_specs(specs);
    let items = wait(&events, &app, &mut timing, |items| {
        has(items, "ruff", "F401") && has(items, "basedpyright", "reportArgumentType")
    });
    // Its unused-import rule is off while ruff runs; the environment's
    // package resolves through `python.pythonPath`.
    assert!(
        !has(&items, "basedpyright", "reportUnusedImport"),
        "{items:#?}"
    );
    if python.is_some() {
        assert!(
            !has(&items, "basedpyright", "reportMissingImports"),
            "{items:#?}"
        );
    }
    timing.print();
    print_memory();
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}
