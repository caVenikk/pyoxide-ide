//! Go to File's order on the layout of the Flask repository (its file
//! paths, `testdata/flask-files.txt`), in a project folder named `flask`
//! and in one named `lead-flask`.
//!
//! The reference IDE sorts its Files tab with a machine-learning model by
//! default; the first rows it showed for `flask/app` in a project folder
//! named `flask` are pinned here exactly, the rest follow [`rank`].

use std::sync::atomic::AtomicBool;

use project::{AutoExclusions, Exclusions, Filter};

use super::*;

const FILES: &str = include_str!("../testdata/flask-files.txt");

struct Fixture {
    _dir: tempfile::TempDir,
    index: FileIndex,
}

fn flask(folder: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap().join(folder);
    for file in FILES.lines().filter(|l| !l.is_empty()) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    let filter = Filter::new(&root, Exclusions::default(), AutoExclusions::NONE);
    let index = FileIndex::build(&root, &filter, false, &AtomicBool::new(false)).unwrap();
    Fixture { _dir: dir, index }
}

impl Fixture {
    /// The rows the popup shows, folders with a trailing slash.
    fn rows(&self, text: &str) -> Vec<String> {
        let query = GotoQuery {
            text,
            ..Default::default()
        };
        let mut hits = self.index.goto_file(&query, &|| false);
        rank(&self.index, &query, &mut hits);
        hits.iter()
            .map(|h| {
                let rel = self.index.relative(h.id);
                if h.is_dir { format!("{rel}/") } else { rel }
            })
            .collect()
    }
}

/// The first rows the reference showed for `flask/app` (Files tab, project
/// folder `flask`).
const REFERENCE_TOP: [&str; 7] = [
    "src/flask/app.py",
    "src/flask/sansio/app.py",
    "tests/test_apps/cliapp/app.py",
    "docs/appcontext.rst",
    "docs/patterns/appdispatch.rst",
    "docs/patterns/appfactories.rst",
    "tests/test_apps/blueprintapp/apps/",
];

/// The rest of the first page by Pyoxide's rule: word starts, then
/// matches inside a word, each by degree and name.
const FIRST_PAGE_REST: [&str; 8] = [
    "examples/celery/src/task_app/",
    "tests/test_appctx.py",
    "tests/test_apps/",
    "tests/type_check/typing_app_decorators.py",
    "tests/test_apps/blueprintapp/",
    "tests/test_apps/cliapp/",
    "tests/test_apps/cliapp/importerrorapp.py",
    "tests/test_apps/cliapp/multiapp.py",
];

fn first_page() -> Vec<&'static str> {
    REFERENCE_TOP
        .iter()
        .chain(&FIRST_PAGE_REST)
        .copied()
        .collect()
}

#[test]
fn a_folder_part_and_a_name() {
    for folder in ["flask", "lead-flask"] {
        let f = flask(folder);
        let rows = f.rows("flask/app");
        assert_eq!(&rows[..7], REFERENCE_TOP, "{folder}");
        assert_eq!(&rows[..15], first_page(), "{folder}");
        // More on the next page: the other matches inside a word, then
        // the shorter name matches.
        assert_eq!(
            &rows[15..18],
            [
                "docs/patterns/singlepageapplications.rst",
                "src/flask/wrappers.py",
                "src/flask/json/provider.py",
            ],
            "{folder}"
        );
        // The folder part only filters, and the project folder's name
        // matches it: the order is that of the name alone.
        assert_eq!(f.rows("app"), rows, "{folder}");
    }
}

#[test]
fn a_folder_without_a_slash() {
    for folder in ["flask", "lead-flask"] {
        let f = flask(folder);
        let rows = f.rows("flaskapp");
        // "flask" as the folder, "app" at a word start of the name.
        assert_eq!(
            &rows[..11],
            [
                "src/flask/app.py",
                "src/flask/sansio/app.py",
                "tests/test_apps/cliapp/app.py",
                "docs/appcontext.rst",
                "docs/patterns/appdispatch.rst",
                "docs/patterns/appfactories.rst",
                "tests/test_apps/blueprintapp/apps/",
                "examples/celery/src/task_app/",
                "tests/test_appctx.py",
                "tests/test_apps/",
                "tests/type_check/typing_app_decorators.py",
            ],
            "{folder}"
        );
        // Then "flaska" or "flaskap" as the folder.
        assert_eq!(
            rows[11], "tests/test_apps/blueprintapp/__init__.py",
            "{folder}"
        );
    }
}

#[test]
fn a_folder_that_names_one_package() {
    let f = flask("flask");
    assert_eq!(f.rows("sansio/app"), ["src/flask/sansio/app.py"]);
    // A name typed in full first, in the shallower folder first; the
    // folder part matches "src/flask".
    assert_eq!(
        f.rows("src/f/app.py"),
        [
            "src/flask/app.py",
            "src/flask/sansio/app.py",
            "src/flask/wrappers.py",
        ]
    );
}

#[test]
fn a_folder_named_in_full() {
    let f = flask("flask");
    assert_eq!(
        f.rows("cliapp"),
        [
            // The name.
            "tests/test_apps/cliapp/",
            // "cli" as the folder, "app" at a word start.
            "tests/test_apps/cliapp/app.py",
            // "cliap" as the folder, "p" at a word start.
            "tests/test_apps/cliapp/__init__.py",
            "tests/test_apps/cliapp/inner1/__init__.py",
            "tests/test_apps/cliapp/inner1/inner2/__init__.py",
            "tests/test_apps/cliapp/factory.py",
            "tests/test_apps/cliapp/inner1/inner2/flask.py",
            "tests/test_apps/cliapp/importerrorapp.py",
            "tests/test_apps/cliapp/multiapp.py",
        ]
    );
}
