//! The Python tools of this machine on a sample project. Ignored by
//! default: they need ruff, ty and basedpyright installed. The tools are
//! looked up as the IDE looks them up; the folder `PYOXIDE_PY_TOOLS`
//! names is searched first, for tools installed only for the tests:
//!
//! ```sh
//! export UV_TOOL_DIR=/tmp/py-tools/tools UV_TOOL_BIN_DIR=/tmp/py-tools/bin
//! uv tool install ruff && uv tool install ty && uv tool install basedpyright
//! PYOXIDE_PY_TOOLS=/tmp/py-tools/bin cargo test -p py-lsp --test real_tools -- --ignored
//! ```

use std::path::{Path, PathBuf};

use py_lsp::lookup::{Executable, SearchPaths};
use py_lsp::ruff::load_rules;
use py_lsp::services::{PROBE_TIMEOUT, PrepareOptions, ProjectState, prepare, probe};
use py_lsp::settings::{self, Tool, TypeEngine};

/// Where the tools are searched: `PYOXIDE_PY_TOOLS` first.
fn search() -> SearchPaths {
    let mut search = SearchPaths::from_env();
    if let Some(dir) = std::env::var_os("PYOXIDE_PY_TOOLS") {
        search.path.insert(0, PathBuf::from(dir));
    }
    search
}

/// A small project: a `pyproject.toml` and a file with an unused import.
fn sample() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("pyproject.toml"),
        "[project]\nname = \"sample\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("app.py"),
        "import os\n\n\ndef main() -> int:\n    return 1\n",
    )
    .unwrap();
    dir
}

fn prepared(root: &Path, search: &SearchPaths) -> ProjectState {
    let mut state = prepare(
        root,
        None,
        PrepareOptions {
            import: true,
            search: Some(search),
        },
    );
    probe(&mut state, PROBE_TIMEOUT);
    state
}

#[test]
#[ignore = "needs ruff and ty installed"]
fn ruff_and_ty_on_a_sample_project() {
    let project = sample();
    let search = search();
    let state = prepared(project.path(), &search);
    assert!(state.errors.is_empty(), "{:?}", state.errors);
    for tool in [Tool::Ruff, Tool::Ty] {
        let tool = state.tool(tool);
        assert_eq!(tool.problem, None, "{tool:?}");
        assert!(tool.version.is_some(), "{tool:?}");
        assert!(matches!(tool.executable(), Some(Executable::Path(_))));
    }
    let rules = load_rules(state.tool(Tool::Ruff).executable().unwrap(), PROBE_TIMEOUT);
    assert!(rules.len() > 500, "{} rules", rules.len());
    assert_eq!(rules["F401"], "unused-import");
    let ids: Vec<String> = state.specs(None).iter().map(|s| s.id.to_string()).collect();
    assert_eq!(ids, ["ruff", "ty"]);
}

#[test]
#[ignore = "needs basedpyright installed"]
fn basedpyright_on_a_sample_project() {
    let project = sample();
    settings::select_type_engine(project.path(), TypeEngine::Basedpyright).unwrap();
    let search = search();
    let state = prepared(project.path(), &search);
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
}
