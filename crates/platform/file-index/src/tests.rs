use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use project::{AutoExclusions, Exclusions, Filter};

use super::*;

const FILES: &[&str] = &[
    "app.py",
    "application.py",
    "myapp.py",
    "README.md",
    "app/__init__.py",
    "src/flask/app.py",
    "src/flask/helpers.py",
    "src/flask/json/provider.py",
    "tests/test_app.py",
    "docs/app.rst",
    ".git/config",
    "__pycache__/app.cpython-313.pyc",
    ".venv/lib/site.py",
];

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    filter: Filter,
    index: FileIndex,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap().join("proj");
    for file in FILES {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x\n").unwrap();
    }
    let mut excluded = Exclusions::default();
    excluded.add(".venv");
    let filter = Filter::new(&root, excluded, AutoExclusions::NONE);
    let index = FileIndex::build(&root, &filter, false, &AtomicBool::new(false)).unwrap();
    Project {
        _dir: dir,
        root,
        filter,
        index,
    }
}

/// Relative paths of what `text` finds, ranked as the popup shows them.
fn find(index: &FileIndex, text: &str) -> Vec<String> {
    find_with(
        index,
        &GotoQuery {
            text,
            ..Default::default()
        },
    )
}

fn find_with(index: &FileIndex, query: &GotoQuery) -> Vec<String> {
    let mut hits = index.goto_file(query, &|| false);
    rank(index, query, &mut hits);
    hits.iter()
        .map(|h| {
            let rel = index.relative(h.id);
            if h.is_dir { format!("{rel}/") } else { rel }
        })
        .collect()
}

#[test]
fn the_walk_leaves_out_hidden_names_and_excluded_content() {
    let p = project();
    let i = &p.index;
    assert!(i.find(&p.root.join("src/flask/app.py")).is_some());
    assert!(i.find(&p.root.join(".git")).is_none());
    assert!(i.find(&p.root.join("__pycache__")).is_none());
    // The excluded folder is listed, its content is not yet.
    let venv = i.find(&p.root.join(".venv")).unwrap();
    assert!(i.is_excluded(venv));
    assert!(i.find(&p.root.join(".venv/lib/site.py")).is_none());
    let with = i
        .with_non_project(&p.filter, &AtomicBool::new(false))
        .unwrap();
    let site = with.find(&p.root.join(".venv/lib/site.py")).unwrap();
    assert!(with.is_excluded(site));
    assert_eq!(with.full_name(site), "proj/.venv/lib/site.py");
    assert_eq!(with.qualifier(site), "proj/.venv/lib/");
}

#[test]
fn names_rank_by_where_they_match() {
    let p = project();
    assert_eq!(
        find(&p.index, "app"),
        [
            // Named as typed; one name in the shallower folder first.
            "app.py",
            "docs/app.rst",
            "src/flask/app.py",
            // Start matches, by degree (the folder ends with the match),
            // folders among the files.
            "app/",
            "application.py",
            // A word start.
            "tests/test_app.py",
            // Inside a word.
            "myapp.py",
            // Shorter name matches: "a" as a folder ("flask"), "pp" as the
            // humps of the name; "a" as the folder "app/".
            "src/flask/json/provider.py",
            "app/__init__.py",
        ]
    );
    assert_eq!(find(&p.index, "help"), ["src/flask/helpers.py"]);
    assert_eq!(find(&p.index, "tap"), ["tests/test_app.py"]);
    assert!(find(&p.index, "zzz").is_empty());
}

#[test]
fn folder_parts_and_slashless_folders() {
    let p = project();
    assert_eq!(find(&p.index, "flask/app"), ["src/flask/app.py"]);
    assert_eq!(find(&p.index, "src/f/app.py"), ["src/flask/app.py"]);
    assert_eq!(find(&p.index, "json/prov"), ["src/flask/json/provider.py"]);
    // Without a slash: the first letters may name the folder.
    assert!(find(&p.index, "flaskapp").contains(&"src/flask/app.py".to_owned()));
    // A path typed in full comes first.
    assert_eq!(find(&p.index, "src/flask/app.py")[0], "src/flask/app.py");
    assert_eq!(
        find(&p.index, "proj/src/flask/app.py")[0],
        "src/flask/app.py"
    );
    assert_eq!(find(&p.index, "./app.py")[0], "app.py");
}

#[test]
fn line_and_column_are_not_part_of_the_name() {
    let p = project();
    assert_eq!(find(&p.index, "helpers.py:12:3"), ["src/flask/helpers.py"]);
    assert_eq!(find(&p.index, "helpers(4)"), ["src/flask/helpers.py"]);
}

#[test]
fn open_files_and_the_current_folder_go_first() {
    let p = project();
    let open = [p.root.join("src/flask/app.py")];
    let query = GotoQuery {
        text: "app.py",
        open_files: &open,
        ..Default::default()
    };
    assert_eq!(
        &find_with(&p.index, &query)[..2],
        ["src/flask/app.py", "app.py"]
    );
    let context = p.root.join("src/flask/helpers.py");
    let query = GotoQuery {
        text: "app.py",
        context: Some(&context),
        ..Default::default()
    };
    assert_eq!(
        &find_with(&p.index, &query)[..2],
        ["src/flask/app.py", "app.py"]
    );
}

#[test]
fn non_project_items_on_request() {
    let p = project();
    assert!(find(&p.index, "site").is_empty());
    let with = p
        .index
        .with_non_project(&p.filter, &AtomicBool::new(false))
        .unwrap();
    assert!(find(&with, "site").is_empty(), "only when asked");
    let query = GotoQuery {
        text: "site",
        non_project: true,
        ..Default::default()
    };
    assert_eq!(find_with(&with, &query), [".venv/lib/site.py"]);
}

#[test]
fn a_pattern_typed_in_the_russian_layout() {
    let p = project();
    // "рудз" is "help" on the same keys.
    assert_eq!(find(&p.index, "рудз"), ["src/flask/helpers.py"]);
}

#[test]
fn changes_made_by_the_application() {
    let mut p = project();
    let new = p.root.join("src/flask/views.py");
    p.index.add(&new, false, &p.filter);
    assert_eq!(find(&p.index, "views"), ["src/flask/views.py"]);
    p.index.rename(
        &p.root.join("src/flask"),
        &p.root.join("src/quart"),
        &p.filter,
    );
    assert_eq!(find(&p.index, "views"), ["src/quart/views.py"]);
    assert_eq!(find(&p.index, "quart/app"), ["src/quart/app.py"]);
    p.index.remove(&p.root.join("src/quart"));
    assert!(find(&p.index, "views").is_empty());
    assert!(
        find(&p.index, "app.py")
            .iter()
            .all(|f| !f.starts_with("src/"))
    );
    // A file moved into an excluded folder is no project content.
    p.index.apply(
        &[Change::Renamed {
            from: p.root.join("myapp.py"),
            to: p.root.join(".venv/myapp.py"),
        }],
        &p.filter,
    );
    assert!(find(&p.index, "myapp").is_empty());
    assert!(p.index.find(Path::new("/elsewhere")).is_none());
}

/// Index build and per-key match times on a real project (`replica/perf.md`):
/// `FILE_INDEX_BENCH=<project folder> cargo test -p file-index --release --
/// --ignored --nocapture bench`; `FILE_INDEX_BENCH_TYPE` lists the words
/// typed (comma-separated).
#[test]
#[ignore]
fn bench() {
    let Some(root) = std::env::var_os("FILE_INDEX_BENCH") else {
        return;
    };
    let root = native_path::canonicalize(root).unwrap();
    let filter = Filter::for_root(&root);
    let mut builds = Vec::new();
    let mut index = None;
    for _ in 0..10 {
        let start = std::time::Instant::now();
        index = FileIndex::build(&root, &filter, false, &AtomicBool::new(false));
        builds.push(start.elapsed());
    }
    let index = index.unwrap();
    builds.sort();
    println!(
        "{} entries; build p50 {:?} max {:?}",
        index.len(),
        builds[5],
        builds[9]
    );
    let words = std::env::var("FILE_INDEX_BENCH_TYPE").unwrap_or_else(|_| "appctx.py".into());
    for word in words.split(',') {
        let mut times = Vec::new();
        for end in 1..=word.len() {
            let text = &word[..end];
            let mut runs = Vec::new();
            let mut found = 0;
            for _ in 0..20 {
                let start = std::time::Instant::now();
                let query = GotoQuery {
                    text,
                    ..Default::default()
                };
                let mut hits = index.goto_file(&query, &|| false);
                rank(&index, &query, &mut hits);
                runs.push(start.elapsed());
                found = hits.len();
            }
            runs.sort();
            times.push(runs[10]);
            println!(
                "  {text:>16}: p50 {:?} max {:?} ({found} found)",
                runs[10], runs[19]
            );
        }
        times.sort();
        println!(
            "{word}: per key p50 {:?} max {:?}",
            times[times.len() / 2],
            times[times.len() - 1]
        );
    }
}
