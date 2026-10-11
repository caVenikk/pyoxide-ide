//! Plain-text rendering of the results, for people. `--json` prints the
//! result types as they are.

use std::fmt::Write as _;

use commands::CommandMeta;
use py_env::discovery::{DiscoveryReport, ToolStatus};
use py_env::ops::{Changed, PackageList, VenvCreated};
use py_env::project::{ProjectInfo, PyProject};
use py_env::{IndexProject, Interpreter};

/// Left-aligned columns separated by two spaces.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let mut s = String::new();
        for (i, c) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                s.push_str(c);
            } else {
                let _ = write!(s, "{c:<w$}  ", w = widths[i]);
            }
        }
        s.trim_end().to_owned()
    };
    let mut out = line(headers.to_vec());
    for r in rows {
        out.push('\n');
        out.push_str(&line(r.iter().map(String::as_str).collect()));
    }
    out
}

pub fn commands(metas: &[&CommandMeta]) -> String {
    let rows: Vec<Vec<String>> = metas
        .iter()
        .map(|m| vec![m.id.to_owned(), m.summary.to_owned(), m.args_example.to_owned()])
        .collect();
    table(&["COMMAND", "SUMMARY", "EXAMPLE ARGS"], &rows)
}

fn interpreter_row(i: &Interpreter) -> Vec<String> {
    let mut kind = i.kind.as_str().to_owned();
    if i.free_threaded {
        kind.push_str(" (free-threaded)");
    }
    vec![
        i.version.clone(),
        kind,
        i.implementation.clone(),
        i.source.clone(),
        i.path.display().to_string(),
    ]
}

pub fn interpreters(report: &DiscoveryReport) -> String {
    let mut out = String::new();
    if report.interpreters.is_empty() {
        out.push_str("No Python interpreters found.\n");
    } else {
        let rows: Vec<_> = report.interpreters.iter().map(interpreter_row).collect();
        out.push_str(&table(&["VERSION", "KIND", "IMPLEMENTATION", "SOURCE", "PATH"], &rows));
        out.push('\n');
    }
    out.push_str("\nTools:\n");
    for t in &report.tools {
        out.push_str(&tool_line(t));
        out.push('\n');
    }
    if !report.skipped.is_empty() {
        out.push_str("\nSkipped:\n");
        for s in &report.skipped {
            let _ = writeln!(out, "  {}: {}", s.path.display(), first_line(&s.reason));
        }
    }
    out.trim_end().to_owned()
}

fn tool_line(t: &ToolStatus) -> String {
    match (&t.path, t.installed) {
        (Some(p), true) => format!(
            "  {:<7} {}{}",
            t.name,
            p.display(),
            t.version.as_ref().map(|v| format!(" ({v})")).unwrap_or_default()
        ),
        _ => format!(
            "  {:<7} not installed{}",
            t.name,
            t.note.as_ref().map(|n| format!(". {n}")).unwrap_or_default()
        ),
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn pyproject_summary(p: &PyProject, out: &mut String) {
    let mut head = p.name.clone().unwrap_or_else(|| "(unnamed)".into());
    if let Some(v) = &p.version {
        let _ = write!(head, " {v}");
    }
    if let Some(r) = &p.requires_python {
        let _ = write!(head, ", requires-python {r}");
    }
    let _ = writeln!(out, "  {head}");
    if !p.dependencies.is_empty() {
        let _ = writeln!(out, "  dependencies ({}):", p.dependencies.len());
        for d in &p.dependencies {
            let _ = writeln!(out, "    {d}");
        }
    }
    for (name, deps) in &p.optional_dependencies {
        let _ = writeln!(out, "  optional-dependencies.{name} ({}): {}", deps.len(), deps.join(", "));
    }
    for (name, g) in &p.dependency_groups {
        let mut parts = g.requirements.clone();
        parts.extend(g.includes.iter().map(|i| format!("(include {i})")));
        let _ = writeln!(out, "  dependency-groups.{name} ({}): {}", parts.len(), parts.join(", "));
    }
    if let Some(uv) = &p.uv {
        if !uv.dev_dependencies.is_empty() {
            let _ = writeln!(out, "  tool.uv.dev-dependencies: {}", uv.dev_dependencies.join(", "));
        }
        if !uv.workspace_members.is_empty() {
            let _ = writeln!(out, "  tool.uv.workspace members: {}", uv.workspace_members.join(", "));
        }
        if !uv.sources.is_empty() {
            let _ = writeln!(out, "  tool.uv.sources: {}", uv.sources.join(", "));
        }
    }
    for w in &p.warnings {
        let _ = writeln!(out, "  warning: {w}");
    }
}

pub fn project(info: &ProjectInfo) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "project root: {}", info.root.display());
    match (&info.pyproject_path, &info.pyproject) {
        (Some(p), Some(pp)) => {
            let _ = writeln!(out, "{}:", p.display());
            pyproject_summary(pp, &mut out);
        }
        (Some(p), None) => {
            let _ = writeln!(out, "{}: unreadable", p.display());
        }
        _ => out.push_str("no pyproject.toml\n"),
    }
    if let Some(lock) = &info.uv_lock {
        let _ = writeln!(out, "uv.lock: {} packages", lock.packages.len());
    }
    for l in &info.other_locks {
        let _ = writeln!(out, "lock file: {}", l.display());
    }
    for r in &info.requirements_files {
        let _ = writeln!(out, "{}: {} requirements", r.path.display(), r.requirements.len());
    }
    if let Some(v) = &info.python_version_file {
        let _ = writeln!(out, ".python-version: {v}");
    }
    match &info.venv {
        Some(v) => {
            let _ = writeln!(out, "environment: {}", v.display());
        }
        None => out.push_str("environment: none (create one with `pyoxide env venv` or `pyoxide env sync`)\n"),
    }
    if !info.managers.is_empty() {
        let names: Vec<String> = info
            .managers
            .iter()
            .map(|m| format!("{m:?}").to_lowercase())
            .collect();
        let _ = writeln!(out, "managed with: {}", names.join(", "));
    }
    if let Some(s) = &info.script {
        let _ = writeln!(out, "script metadata:");
        if let Some(r) = &s.requires_python {
            let _ = writeln!(out, "  requires-python {r}");
        }
        for d in &s.dependencies {
            let _ = writeln!(out, "  {d}");
        }
    }
    for p in &info.problems {
        let _ = writeln!(out, "problem: {p}");
    }
    out.trim_end().to_owned()
}

pub fn packages(list: &PackageList) -> String {
    let outdated = list.packages.iter().any(|p| p.latest_version.is_some());
    let mut out = format!("{} packages in {} (via {})\n", list.packages.len(), list.python.display(), list.backend.as_str());
    if list.packages.is_empty() {
        return out.trim_end().to_owned();
    }
    let rows: Vec<Vec<String>> = list
        .packages
        .iter()
        .map(|p| {
            let mut row = vec![p.name.clone(), p.version.clone()];
            if outdated {
                row.push(p.latest_version.clone().unwrap_or_default());
            }
            if let Some(loc) = &p.editable_project_location {
                row.push(format!("editable: {}", loc.display()));
            }
            row
        })
        .collect();
    let width = rows.iter().map(Vec::len).max().unwrap_or(2);
    let rows: Vec<Vec<String>> = rows
        .into_iter()
        .map(|mut r| {
            r.resize(width, String::new());
            r
        })
        .collect();
    let headers: Vec<&str> = match (outdated, width) {
        (true, 4) => vec!["NAME", "VERSION", "LATEST", ""],
        (true, _) => vec!["NAME", "VERSION", "LATEST"],
        (false, 3) => vec!["NAME", "VERSION", ""],
        _ => vec!["NAME", "VERSION"],
    };
    out.push_str(&table(&headers, &rows));
    out
}

/// A short form of the command line: the program's file name, no directory.
fn short_command(command: &str) -> String {
    let mut parts = command.splitn(2, ' ');
    let program = parts.next().unwrap_or_default();
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    match parts.next() {
        Some(rest) => format!("{name} {rest}"),
        None => name.to_owned(),
    }
}

pub fn changed(c: &Changed) -> String {
    format!(
        "ok: {} ({:.1} s, via {})",
        short_command(&c.outcome.command),
        c.outcome.duration_ms as f64 / 1000.0,
        c.backend.as_str()
    )
}

pub fn venv(v: &VenvCreated) -> String {
    let what = match &v.interpreter {
        Some(i) => format!("{} {}", i.implementation, i.version),
        None => "Python".to_owned(),
    };
    format!("created {} with {what}\ninterpreter: {}", v.path.display(), v.python.display())
}

pub fn search(p: &IndexProject, versions: usize) -> String {
    let mut out = format!("{} {}\n", p.name, p.latest);
    if let Some(s) = &p.summary {
        let _ = writeln!(out, "{s}");
    }
    if let Some(r) = &p.requires_python {
        let _ = writeln!(out, "requires-python {r}");
    }
    if let Some(l) = &p.license {
        let _ = writeln!(out, "license {l}");
    }
    if let Some(h) = &p.home_page {
        let _ = writeln!(out, "home page {h}");
    }
    for (k, v) in &p.project_urls {
        let _ = writeln!(out, "{k}: {v}");
    }
    let shown = p.versions.iter().take(versions);
    let rows: Vec<Vec<String>> = shown
        .map(|v| {
            let mut flags = Vec::new();
            if v.yanked {
                flags.push("yanked");
            }
            if v.prerelease {
                flags.push("pre-release");
            }
            vec![
                v.version.clone(),
                v.requires_python.clone().unwrap_or_default(),
                v.uploaded.as_deref().map(|u| u.get(..10).unwrap_or(u).to_owned()).unwrap_or_default(),
                flags.join(", "),
            ]
        })
        .collect();
    if !rows.is_empty() {
        let _ = writeln!(out, "\nversions ({} of {}):", rows.len(), p.versions.len());
        out.push_str(&table(&["VERSION", "REQUIRES-PYTHON", "UPLOADED", ""], &rows));
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use py_env::packages::{Backend, Outcome};
    use py_env::{InterpreterKind, Package};
    use std::path::PathBuf;

    #[test]
    fn tables_align_columns() {
        let t = table(
            &["A", "LONGER"],
            &[vec!["x".into(), "1".into()], vec!["yyyy".into(), "22".into()]],
        );
        assert_eq!(t, "A     LONGER\nx     1\nyyyy  22");
    }

    #[test]
    fn short_commands_drop_the_directory() {
        assert_eq!(short_command("/home/u/.local/bin/uv add -- requests"), "uv add -- requests");
        assert_eq!(short_command("C:\\bin\\uv.exe sync"), "uv.exe sync");
        assert_eq!(short_command("uv"), "uv");
    }

    #[test]
    fn package_tables_show_latest_only_for_outdated_listings() {
        let pkg = |latest: Option<&str>| Package {
            name: "requests".into(),
            version: "2.31.0".into(),
            editable_project_location: None,
            latest_version: latest.map(str::to_owned),
            latest_filetype: None,
        };
        let plain = PackageList {
            backend: Backend::Uv,
            python: PathBuf::from("/v/bin/python"),
            packages: vec![pkg(None)],
        };
        let text = packages(&plain);
        assert!(text.contains("1 packages in /v/bin/python (via uv)"));
        assert!(text.contains("NAME      VERSION"));
        assert!(!text.contains("LATEST"));
        let outdated = PackageList {
            packages: vec![pkg(Some("2.32.5"))],
            ..plain
        };
        let text = packages(&outdated);
        assert!(text.contains("LATEST"));
        assert!(text.contains("2.32.5"));
    }

    #[test]
    fn interpreter_listing_and_tools() {
        let report = DiscoveryReport {
            interpreters: vec![Interpreter {
                path: PathBuf::from("/p/.venv/bin/python"),
                version: "3.12.4".into(),
                major: 3,
                minor: 12,
                micro: 4,
                implementation: "cpython".into(),
                kind: InterpreterKind::Venv,
                is_venv: true,
                prefix: PathBuf::from("/p/.venv"),
                base_prefix: PathBuf::from("/usr"),
                venv_root: Some(PathBuf::from("/p/.venv")),
                arch: None,
                free_threaded: false,
                source: ".venv".into(),
            }],
            tools: vec![
                ToolStatus {
                    name: "uv".into(),
                    installed: true,
                    path: Some(PathBuf::from("/bin/uv")),
                    version: Some("0.11.25".into()),
                    note: None,
                },
                ToolStatus {
                    name: "poetry".into(),
                    installed: false,
                    path: None,
                    version: None,
                    note: Some("Poetry environments are not listed.".into()),
                },
            ],
            skipped: vec![],
        };
        let text = interpreters(&report);
        assert!(text.contains("3.12.4  venv"), "{text}");
        assert!(text.contains("/p/.venv/bin/python"));
        assert!(text.contains("uv      /bin/uv (0.11.25)"));
        assert!(text.contains("poetry  not installed. Poetry environments are not listed."));
    }

    #[test]
    fn change_summary_is_one_line() {
        let c = Changed {
            backend: Backend::Uv,
            outcome: Outcome {
                command: "/usr/bin/uv sync".into(),
                log: vec![],
                duration_ms: 1234,
            },
        };
        assert_eq!(changed(&c), "ok: uv sync (1.2 s, via uv)");
    }
}
