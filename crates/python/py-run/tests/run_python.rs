//! A real Python run through `process` and `console`. Needs `python3` (or
//! `python` on Windows) on `PATH`, so it is ignored by default:
//!
//! ```text
//! cargo test -p py-run --test run_python -- --ignored
//! ```

use std::time::{Duration, Instant};

use console::{Console, ContentKind, LinkTarget};
use process::{ProcessEvent, spawn};
use py_run::PythonRun;

#[test]
#[ignore = "needs a Python interpreter on PATH"]
fn a_failing_script_prints_a_linked_traceback() {
    let python = if cfg!(windows) { "python" } else { "python3" };
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("main.py");
    std::fs::write(
        &script,
        "import os, sys\n\
         print(os.environ['PYTHONUNBUFFERED'], os.environ['PYCHARM_HOSTED'], sys.stdout.encoding)\n\
         print('\\x1b[31mred\\x1b[0m')\n\
         def f():\n    return 1 / 0\n\
         f()\n",
    )
    .unwrap();
    let mut run = PythonRun::current_file(python, &script);
    run.content_roots = vec![dir.path().to_path_buf()];

    let mut console = Console::new();
    console.set_link_filters(run.link_filters());
    console.push_str(ContentKind::System, &format!("{}\n", run.display()));
    let (_, events) = spawn(&run.command_line()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let exit = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left).unwrap() {
            ProcessEvent::Stdout(b) => console.push(ContentKind::Normal, &b),
            ProcessEvent::Stderr(b) => console.push(ContentKind::Error, &b),
            ProcessEvent::Exited(status) => break status,
        }
    };
    console.finish();
    console.push_str(
        ContentKind::System,
        &format!("\n{}\n", exit.finished_message()),
    );

    let text = console.text();
    assert!(text.starts_with(&format!("{python} {} \n", script.display())));
    assert!(text.contains("1 1 utf-8\nred\n"), "{text}");
    assert!(
        text.contains("ZeroDivisionError: division by zero"),
        "{text}"
    );
    assert!(
        text.ends_with("\n\nProcess finished with exit code 1\n"),
        "{text}"
    );
    let linked: Vec<_> = console
        .lines()
        .flat_map(|l| l.links().iter().map(|k| k.target.clone()))
        .collect();
    let lines: Vec<Option<u32>> = linked
        .iter()
        .filter_map(|t| match t {
            LinkTarget::File { path, line, .. } if path.ends_with("main.py") => Some(*line),
            _ => None,
        })
        .collect();
    assert_eq!(lines, [Some(6), Some(5)], "{text}");
}
