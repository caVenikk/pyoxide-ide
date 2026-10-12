//! The reference IDE's own Go to File cases (its `ChooseByNameTest`), on a
//! project folder named `src` as in its test fixture. `goto` is the order
//! the files are found in (what those tests check), `ranked` the order the
//! popup shows (by weight).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use project::{AutoExclusions, Exclusions, Filter};

use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    index: FileIndex,
}

/// Files (and, ending with `/`, folders) under a project folder `src`.
fn fixture(files: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join("src");
    std::fs::create_dir_all(&root).unwrap();
    for file in files {
        let path = root.join(file);
        if file.ends_with('/') {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
    }
    let filter = Filter::new(&root, Exclusions::default(), AutoExclusions::NONE);
    let index = FileIndex::build(&root, &filter, true, &AtomicBool::new(false)).unwrap();
    Fixture {
        _dir: dir,
        root,
        index,
    }
}

impl Fixture {
    fn query<'a>(&self, text: &'a str, non_project: bool, context: Option<&'a std::path::Path>) -> GotoQuery<'a> {
        GotoQuery {
            text,
            non_project,
            open_files: &[],
            context,
        }
    }

    fn names(&self, hits: &[FileHit]) -> Vec<String> {
        hits.iter()
            .map(|h| {
                let rel = self.index.relative(h.id);
                if h.is_dir { format!("{rel}/") } else { rel }
            })
            .collect()
    }

    fn goto(&self, text: &str) -> Vec<String> {
        self.goto_with(text, false, None)
    }

    fn goto_with(&self, text: &str, non_project: bool, context: Option<&str>) -> Vec<String> {
        let context = context.map(|c| self.root.join(c));
        let query = self.query(text, non_project, context.as_deref());
        self.names(&self.index.goto_file(&query, &|| false))
    }

    fn ranked(&self, text: &str) -> Vec<String> {
        let query = self.query(text, false, None);
        let mut hits = self.index.goto_file(&query, &|| false);
        rank(&mut hits);
        self.names(&hits)
    }
}

#[test]
fn order_by_matching_degree() {
    let f = fixture(&["ServiceAccessor.java", "sache.txt"]);
    assert_eq!(f.goto("SA"), ["ServiceAccessor.java", "sache.txt"]);
}

#[test]
fn prefer_exact_extension_and_file_name_matches() {
    let f = fixture(&["relaunch.m", "reference.mod"]);
    assert_eq!(f.goto("re*.m"), ["relaunch.m", "reference.mod"]);
    let f = fixture(&["foo/i.txt", "index.html"]);
    assert_eq!(f.goto("i"), ["foo/i.txt", "index.html"]);
    let f = fixture(&["foo/cp-users.txt", "cp-users-and-smth.html"]);
    assert_eq!(f.goto("cpusers"), ["foo/cp-users.txt", "cp-users-and-smth.html"]);
}

#[test]
fn settings_files_are_not_project_content() {
    let f = fixture(&[".idea/workspace.xml", "workspace.txt"]);
    assert_eq!(f.goto_with("work", false, None), ["workspace.txt"]);
    assert_eq!(
        f.goto_with("work", true, None),
        ["workspace.txt", ".idea/workspace.xml"]
    );
}

#[test]
fn prefer_better_and_closer_path_matches() {
    let f = fixture(&["foo/index.html", "foo/bar/index.html", "bar/foo/index.html"]);
    assert_eq!(
        f.goto("foo/index"),
        ["foo/index.html", "bar/foo/index.html", "foo/bar/index.html"]
    );
    let f = fixture(&[
        "content/objc/features/index.html",
        "content/objc/features/screenshots/i18n.html",
    ]);
    assert_eq!(
        f.goto("objc/features/i"),
        [
            "content/objc/features/index.html",
            "content/objc/features/screenshots/i18n.html"
        ]
    );
}

#[test]
fn same_names_by_path() {
    let files: Vec<String> = (10..=30).rev().map(|i| format!("foo{i}/index.html")).collect();
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let f = fixture(&refs);
    let mut expected = files.clone();
    expected.reverse();
    assert_eq!(f.goto("index"), expected);
}

#[test]
fn middle_matching_for_files_and_folders() {
    let f = fixture(&["foo/index.html", "oo/index.html", "foo/bar/index.html"]);
    assert_eq!(
        f.goto("oo/index"),
        ["oo/index.html", "foo/index.html", "foo/bar/index.html"]
    );
    assert_eq!(
        f.goto("ndex.html"),
        ["foo/index.html", "oo/index.html", "foo/bar/index.html"]
    );
}

#[test]
fn prefer_files_from_the_current_folder() {
    let f = fixture(&[
        "foo/index.html",
        "bar/index.html",
        "foo/context.html",
        "bar/context.html",
    ]);
    assert_eq!(
        f.goto_with("index", false, Some("foo/context.html")),
        ["foo/index.html", "bar/index.html"]
    );
    assert_eq!(
        f.goto_with("index", false, Some("bar/context.html")),
        ["bar/index.html", "foo/index.html"]
    );
}

#[test]
fn paths_starting_with_a_dot_and_with_slashes() {
    let f = fixture(&["foo/index.html"]);
    assert_eq!(f.goto("./foo/in"), ["foo/index.html"]);
    let f = fixture(&["foo/bar/goo/file.txt"]);
    for path in [
        "foo/goo/file.txt",
        "foo/bar/file.txt",
        "bar/goo/file.txt",
        "foo/bar/goo/file.txt",
    ] {
        assert_eq!(f.goto(path), ["foo/bar/goo/file.txt"], "{path}");
        assert_eq!(f.goto(&path.replace('/', "\\")), ["foo/bar/goo/file.txt"], "{path}");
    }
}

#[test]
fn go_to_folders() {
    let f = fixture(&["foo/index.html", "bar.txt/bar.txt"]);
    let goto = |text: &str| f.goto_with(text, false, Some("foo/index.html"));
    for text in ["foo/", "foo\\", "/foo", "\\foo", "foo"] {
        assert_eq!(goto(text), ["foo/"], "{text}");
    }
    assert_eq!(goto("/index.html"), ["foo/index.html"]);
    assert_eq!(goto("\\index.html"), ["foo/index.html"]);
    assert!(goto("index.html/").is_empty());
    assert!(goto("index.html\\").is_empty());
    assert_eq!(goto("bar.txt/"), ["bar.txt/"]);
    assert_eq!(goto("bar.txt\\"), ["bar.txt/"]);
    for text in ["/bar.txt", "\\bar.txt", "bar.txt", "bar"] {
        assert_eq!(goto(text), ["bar.txt/bar.txt", "bar.txt/"], "{text}");
    }
    let f = fixture(&["dir/fooFile.txt", "foo/barFile.txt"]);
    assert_eq!(f.goto("foo"), ["dir/fooFile.txt", "foo/"]);
}

#[test]
fn folder_paths_without_slashes() {
    let f = fixture(&[
        "foo/bar/index_fooBar.html",
        "fb/index_fb.html",
        "fb/some.dir/index_fbSome.html",
        "some/fb/index_someFb.html",
    ]);
    assert_eq!(f.goto("barindex"), ["foo/bar/index_fooBar.html"]);
    assert_eq!(f.goto("fooindex"), ["foo/bar/index_fooBar.html"]);
    let all = [
        "fb/index_fb.html",
        "some/fb/index_someFb.html",
        "fb/some.dir/index_fbSome.html",
        "foo/bar/index_fooBar.html",
    ];
    assert_eq!(f.goto("fbindex"), all);
    assert_eq!(f.goto("fbhtml"), all);
    assert_eq!(f.goto("somefb/index.html"), ["some/fb/index_someFb.html"]);
    assert_eq!(f.goto("somefb\\index.html"), ["some/fb/index_someFb.html"]);
}

#[test]
fn spaces_instead_of_slashes_and_suffixes() {
    let f = fixture(&["config/app.txt", "src/Configuration/ManagesApp.txt"]);
    assert_eq!(f.goto("config app.txt")[0], "config/app.txt");
    let f = fixture(&["sample/EnumControl.java", "sample/ControlSmth.java"]);
    assert_eq!(
        f.goto("samplecontrol"),
        ["sample/EnumControl.java", "sample/ControlSmth.java"]
    );
}

#[test]
fn a_file_in_a_matching_folder() {
    let f = fixture(&["foo/index/index"]);
    assert_eq!(f.goto("in"), ["foo/index/index", "foo/index/"]);
    assert_eq!(f.goto("foin"), ["foo/index/index", "foo/index/"]);
}

#[test]
fn line_suffixes() {
    let f = fixture(&["Bar.java"]);
    for text in [
        "Bar:2",
        "Bar(2)",
        "Bar on line 2",
        "Bar at line 2",
        "Bar 2:39",
        "Bar#L2",
        "Bar?l=2",
    ] {
        assert_eq!(f.goto(text), ["Bar.java"], "{text}");
    }
}

#[test]
fn ranked_by_weight() {
    let f = fixture(&[
        "Folder/Web/SubFolder/Flow.html",
        "Folder/Web/SubFolder/Flow/Helper.html",
    ]);
    assert_eq!(
        f.ranked("Folder/Web/SubFolder/Flow.html"),
        [
            "Folder/Web/SubFolder/Flow.html",
            "Folder/Web/SubFolder/Flow/Helper.html"
        ]
    );
    let f = fixture(&["JBCefBrowser.java", "com/elements/folder/WebBrowser.java"]);
    assert_eq!(
        f.ranked("CefBrowser"),
        ["JBCefBrowser.java", "com/elements/folder/WebBrowser.java"]
    );
}

#[test]
fn exact_case_and_keyboard_layout() {
    let f = fixture(&["foo/SOMECLASS.java", "bar/SomeClass.java"]);
    assert_eq!(f.goto("SomeClass.java"), ["bar/SomeClass.java", "foo/SOMECLASS.java"]);
    let f = fixture(&["String.java"]);
    assert_eq!(f.goto("Ыекштп"), ["String.java"]);
}
