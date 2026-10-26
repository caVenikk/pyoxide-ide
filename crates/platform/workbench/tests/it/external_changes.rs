//! Files changed on disk outside the application (a `git checkout`, a
//! formatter): an open file whose editor is unmodified takes the file's
//! new text when the window gets the focus, after Reload All from Disk,
//! and when the file watcher reports it, with the caret kept on its line;
//! the project tree, the file index and the language services follow the
//! files created and removed. External writes are made by the test; the
//! watcher test waits for the system's events with a generous timeout.

use crate::support;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use code_intel::fake::{Call, FakeCodeIntel};
use code_intel::{CodeIntel, DocumentId, FileChange, FileChangeKind};
use gpui::{App, AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{
    CodeIntelHook, CodeIntelProvider, FileType, Product, SaveSettings, SyncSettings, Workbench,
};

struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }
}

const A: &str = "def f():\n    return 1\n\nx = f()\n";

/// A project with `a.py` and `src/m.py`, opened with `a.py` in a tab and
/// the window active; the fake language services see it all.
fn open(cx: &mut TestAppContext, sync: SyncSettings) -> (H<'_>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a.py"), A).unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/m.py"), "m = 1\n").unwrap();
    let fake = FakeCodeIntel::new();
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        code_intel: vec![CodeIntelHook::new(Provider(fake.clone()))],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
        .update(cx, |w, _, _| w.set_sync_settings(sync))
        .unwrap();
    let mut h = H {
        cx,
        window,
        root,
        fake,
    };
    let root = h.root.display().to_string();
    h.invoke("project.open-folder", json!({ "path": root }));
    h.activate();
    h.invoke("project.open-file", json!({"path": "a.py"}));
    (h, dir)
}

/// As the application runs with `APP_BACKGROUND`: the window never
/// gets the focus, and a file opens by its absolute path (with the folder
/// opened as the project first when `project`).
fn open_in_background(
    cx: &mut TestAppContext,
    sync: SyncSettings,
    project: bool,
) -> (H<'_>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a.py"), "def hello():\n    return 1\n").unwrap();
    let fake = FakeCodeIntel::new();
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        code_intel: vec![CodeIntelHook::new(Provider(fake.clone()))],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
        .update(cx, |w, _, _| w.set_sync_settings(sync))
        .unwrap();
    let mut h = H {
        cx,
        window,
        root,
        fake,
    };
    if project {
        let root = h.root.display().to_string();
        h.invoke("project.open-folder", json!({ "path": root }));
    }
    let a = h.root.join("a.py").display().to_string();
    h.invoke("project.open-file", json!({ "path": a }));
    (h, dir)
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
    fake: Arc<FakeCodeIntel>,
}

impl H<'_> {
    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        out
    }

    fn state(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))
    }

    fn text(&mut self) -> String {
        self.invoke("editor.text", json!(null))
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn caret(&mut self) -> Value {
        self.state()["editor"]["caret"].clone()
    }

    fn rows(&mut self) -> Vec<String> {
        self.state()["tree"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect()
    }

    fn set_caret(&mut self, line: usize, column: usize) {
        let at = json!({"line": line, "column": column});
        self.invoke(
            "editor.set-selection",
            json!({"anchor": at.clone(), "head": at}),
        );
    }

    /// Writes a file of the project as another program would.
    fn write(&self, rel: &str, text: &str) {
        std::fs::write(self.root.join(rel), text).unwrap();
    }

    fn activate(&mut self) {
        self.cx
            .update_window(self.window.into(), |_, window, _| window.activate_window())
            .unwrap();
        self.cx.run_until_parked();
    }

    /// The window loses the focus to another application and gets it back.
    fn switch_away_and_back(&mut self) {
        let vcx = &mut gpui::VisualTestContext::from_window(self.window.into(), self.cx);
        vcx.deactivate_window();
        self.activate();
    }

    fn files_changed(&self) -> Vec<FileChange> {
        self.fake
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::FilesChanged(changes) => Some(changes),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn reloads(&self, path: &Path) -> usize {
        let doc = DocumentId::new(path);
        self.fake
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::DidReload { doc: d, .. } if *d == doc))
            .count()
    }

    /// Lets the executor's clock move on.
    fn advance(&mut self, by: Duration) {
        self.cx.executor().advance_clock(by);
        self.cx.run_until_parked();
    }

    /// Waits for real events (the file watcher) until `f` holds.
    fn wait(&mut self, what: &str, f: impl Fn(&mut Self) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            if f(self) {
                return;
            }
        }
        panic!("timed out waiting for {what}");
    }
}

#[gpui::test]
fn an_unmodified_file_is_reloaded_when_the_window_gets_the_focus(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    let a = h.root.join("a.py");
    h.set_caret(1, 8);
    // A formatter adds an import and changes the caret's line.
    let new = "import os\n\n\ndef f():\n    return 2\n\nx = f()\n";
    h.write("a.py", new);
    assert_eq!(
        h.text(),
        A,
        "nothing happens while the window has the focus"
    );

    h.switch_away_and_back();
    assert_eq!(h.text(), new);
    assert_eq!(h.caret(), json!({"line": 4, "column": 8}));
    assert_eq!(h.state()["tabs"][0]["modified"], json!(false));
    // The language services get the new text and the file's change.
    assert_eq!(h.reloads(&a), 1);
    assert!(
        h.files_changed()
            .contains(&FileChange::new(&a, FileChangeKind::Changed))
    );
    // The reload is one undo step, undone after a question; undone, the
    // editor differs from the file.
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), new, "nothing is undone before the answer");
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Undo"));
    assert_eq!(dialog["message"], json!("Undo Reload from Disk?"));
    assert_eq!(dialog["buttons"], json!(["Cancel", "OK"]));
    h.invoke("dialog.cancel", json!(null));
    assert_eq!(h.text(), new);
    h.invoke("editor.undo", json!(null));
    h.invoke("dialog.confirm", json!(null));
    assert_eq!(h.text(), A);
    assert_eq!(h.state()["tabs"][0]["modified"], json!(true));
    h.invoke("editor.redo", json!(null));
    assert_eq!(
        h.state()["dialog"]["message"],
        json!("Redo Reload from Disk?")
    );
    h.invoke("dialog.confirm", json!(null));
    assert_eq!(h.text(), new);

    // Coming back again without a change reads nothing new.
    h.switch_away_and_back();
    assert_eq!(h.reloads(&a), 1);
    // A change of the line separator only is taken over too.
    h.write("a.py", &new.replace('\n', "\r\n"));
    h.switch_away_and_back();
    assert_eq!(h.text(), new);
    assert_eq!(h.state()["editor"]["line_ending"], json!("CRLF"));
}

#[gpui::test]
fn the_applications_own_saves_are_not_changes(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    let a = h.root.join("a.py");
    h.set_caret(0, 0);
    h.invoke("editor.type", json!({"text": "# top\n"}));
    // Saved when the window loses the focus, then looked at when it comes
    // back: the file is what the editor wrote.
    h.switch_away_and_back();
    let disk = std::fs::read_to_string(&a).unwrap();
    assert_eq!(disk, format!("# top\n{A}"));
    assert_eq!(h.text(), disk);
    assert_eq!(h.reloads(&a), 0);
    // Undo still undoes the typing (no reload step came in between).
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), A);
}

/// Saves only on request, so that the test decides when a save happens.
fn manual_saves(h: &mut H) {
    h.window
        .update(h.cx, |w, _, _| {
            w.set_save_settings(SaveSettings {
                save_on_deactivation: false,
                save_on_idle: false,
                save_on_tab_switch: false,
                ..SaveSettings::default()
            })
        })
        .unwrap();
}

#[gpui::test]
fn a_file_changed_on_disk_and_in_its_editor_asks_which_to_keep(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    manual_saves(&mut h);
    let a = h.root.join("a.py");
    h.invoke("editor.type", json!({"text": "y = 0\n"}));
    let mine = format!("y = 0\n{A}");
    h.write("a.py", "changed = True\n");
    h.switch_away_and_back();
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("File Cache Conflict"));
    assert_eq!(
        dialog["message"],
        json!(format!(
            "Changes have been made to '{}' in memory and on disk.",
            a.display()
        ))
    );
    assert_eq!(
        dialog["buttons"],
        json!(["Keep Memory Changes", "Load File System Changes"])
    );
    assert_eq!(dialog["default"], json!("Load File System Changes"));
    assert_eq!(h.text(), mine, "nothing changes before the answer");

    // While the question is open, no save writes the file.
    h.invoke("dialog.cancel", json!(null));
    // Escape keeps the editor's text, which the next save writes.
    assert_eq!(h.text(), mine);
    assert_eq!(h.state()["tabs"][0]["modified"], json!(true));
    h.invoke("workbench.save-all", json!(null));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), mine);

    // Load File System Changes: the editor takes the file, one undo step.
    h.set_caret(0, 0);
    h.invoke("editor.type", json!({"text": "z = 0\n"}));
    h.write("a.py", "outside = 2\n");
    h.switch_away_and_back();
    h.invoke(
        "dialog.choose",
        json!({"button": "Load File System Changes"}),
    );
    assert_eq!(h.text(), "outside = 2\n");
    assert_eq!(h.state()["tabs"][0]["modified"], json!(false));
    assert!(h.state()["dialog"].is_null());
    h.invoke("editor.undo", json!(null));
    h.invoke("dialog.confirm", json!(null));
    assert!(h.text().starts_with("z = 0\n"), "{}", h.text());
    assert_eq!(h.state()["tabs"][0]["modified"], json!(true));
}

#[gpui::test]
fn no_save_writes_over_a_file_changed_on_disk(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    manual_saves(&mut h);
    let a = h.root.join("a.py");
    // Looked at once, so its stamp is known.
    h.switch_away_and_back();
    h.invoke("editor.type", json!({"text": "mine = 1\n"}));
    // Changed outside; no check has seen it yet.
    h.write("a.py", "theirs = 1\n");
    h.invoke("workbench.save-all", json!(null));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "theirs = 1\n");
    assert_eq!(h.state()["dialog"]["title"], json!("File Cache Conflict"));
    assert_eq!(h.state()["tabs"][0]["modified"], json!(true));
    // Saves wait for the answer, explicit ones too.
    h.invoke("dialog.cancel", json!(null));
    h.invoke("workbench.save-all", json!(null));
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        format!("mine = 1\n{A}")
    );

    // The same without a stamp known yet (no check before the change).
    h.invoke("project.open-file", json!({"path": "src/m.py"}));
    h.invoke("editor.type", json!({"text": "mine = 2\n"}));
    h.write("src/m.py", "theirs = 2\n");
    h.invoke("editor.save", json!(null));
    assert_eq!(
        std::fs::read_to_string(h.root.join("src/m.py")).unwrap(),
        "theirs = 2\n"
    );
    assert_eq!(h.state()["dialog"]["title"], json!("File Cache Conflict"));
}

#[gpui::test]
fn a_deleted_files_tab_closes_unless_its_editor_has_changes(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    h.invoke("project.open-file", json!({"path": "src/m.py"}));
    h.invoke("editor.type", json!({"text": "kept = 1\n"}));
    std::fs::remove_file(h.root.join("a.py")).unwrap();
    std::fs::remove_file(h.root.join("src/m.py")).unwrap();
    // The save when the window loses the focus finds m.py gone and
    // leaves it so.
    h.switch_away_and_back();
    assert!(!h.root.join("src/m.py").exists());
    // Looked at again a moment later (a tool may write it again).
    assert_eq!(h.state()["tabs"].as_array().unwrap().len(), 2);
    h.advance(Duration::from_millis(400));
    let tabs = h.state()["tabs"].clone();
    assert_eq!(
        tabs,
        json!([{"path": "src/m.py", "modified": true, "deleted": true}])
    );
    // Not saved by itself, so the file is not made again.
    h.switch_away_and_back();
    assert!(!h.root.join("src/m.py").exists());

    // Closing it asks; Cancel keeps the tab.
    h.invoke("tabs.close", json!(null));
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Close Document"));
    assert_eq!(
        dialog["message"],
        json!("The document has been modified.\nDo you want to save the changes?")
    );
    assert_eq!(dialog["buttons"], json!(["Cancel", "Discard", "Save"]));
    h.invoke("dialog.cancel", json!(null));
    assert_eq!(h.state()["tabs"].as_array().unwrap().len(), 1);
    // Save writes the file again and closes the tab.
    h.invoke("tabs.close", json!(null));
    h.invoke("dialog.choose", json!({"button": "Save"}));
    assert!(h.state()["tabs"].as_array().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(h.root.join("src/m.py")).unwrap(),
        "kept = 1\nm = 1\n"
    );

    // Discard drops the changes.
    h.invoke("project.open-file", json!({"path": "src/m.py"}));
    h.invoke("editor.type", json!({"text": "dropped = 1\n"}));
    std::fs::remove_file(h.root.join("src/m.py")).unwrap();
    h.switch_away_and_back();
    h.advance(Duration::from_millis(400));
    assert_eq!(h.state()["tabs"][0]["deleted"], json!(true));
    // An explicit save makes the file again.
    h.invoke("workbench.save-all", json!(null));
    assert!(h.root.join("src/m.py").exists());
    assert_eq!(h.state()["tabs"][0]["deleted"], json!(false));
    h.invoke("editor.type", json!({"text": "dropped = 2\n"}));
    std::fs::remove_file(h.root.join("src/m.py")).unwrap();
    h.switch_away_and_back();
    h.advance(Duration::from_millis(400));
    h.invoke("tabs.close", json!(null));
    h.invoke("dialog.choose", json!({"button": "Discard"}));
    assert!(h.state()["tabs"].as_array().unwrap().is_empty());
    assert!(!h.root.join("src/m.py").exists());
}

#[gpui::test]
fn a_file_removed_and_written_again_keeps_its_tab(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    std::fs::remove_file(h.root.join("a.py")).unwrap();
    h.switch_away_and_back();
    // Written again before the second look (as `git checkout` does).
    h.write("a.py", "again = 1\n");
    h.advance(Duration::from_millis(400));
    assert_eq!(h.state()["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(h.text(), "again = 1\n");
}

#[gpui::test]
fn a_rename_outside_is_a_deletion_and_a_creation(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    std::fs::rename(h.root.join("a.py"), h.root.join("renamed.py")).unwrap();
    h.switch_away_and_back();
    h.advance(Duration::from_millis(400));
    assert!(h.state()["tabs"].as_array().unwrap().is_empty());
    let rows = h.rows();
    assert!(rows.contains(&"renamed.py".to_owned()), "{rows:?}");
    assert!(!rows.contains(&"a.py".to_owned()), "{rows:?}");
}

/// The sequence of a release run in the background: a file opened by the
/// command, changed outside, typed into, saved by the idle save, changed
/// outside again, then the application quits. The file written outside is
/// never written over.
fn changes_outside_while_typing(h: &mut H, watched: bool) {
    let a = h.root.join("a.py");
    h.write("a.py", "def hello():\n    return 2  # outside\n");
    if watched {
        h.wait("the change outside to be reloaded", |h| {
            h.text() == "def hello():\n    return 2  # outside\n"
        });
    } else {
        // No watcher, no focus: nothing looks at the file yet.
        h.advance(Duration::from_secs(3));
    }
    h.set_caret(0, 0);
    h.invoke("editor.type", json!({"text": "# mine\n"}));
    h.write("a.py", "def hello():\n    return 3  # outside again\n");
    // The idle save comes 2 s after the typing.
    if watched {
        h.wait("the conflict", |h| !h.state()["dialog"].is_null());
    } else {
        h.advance(Duration::from_secs(3));
    }
    let disk = std::fs::read_to_string(&a).unwrap();
    assert_eq!(disk, "def hello():\n    return 3  # outside again\n");
    assert_eq!(h.state()["dialog"]["title"], json!("File Cache Conflict"));
    assert_eq!(h.state()["tabs"][0]["modified"], json!(true));
    // Quitting does not write over it either.
    h.cx.update(|cx| cx.shutdown());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), disk);
}

#[gpui::test]
fn a_background_window_without_a_project_never_writes_over_a_newer_file(cx: &mut TestAppContext) {
    let (mut h, _dir) = open_in_background(cx, SyncSettings::default(), false);
    changes_outside_while_typing(&mut h, false);
}

fn background_with_the_watcher(cx: &mut TestAppContext, project: bool) {
    support::allow_processes(cx);
    let sync = SyncSettings {
        watch: true,
        ..SyncSettings::default()
    };
    let (mut h, _dir) = open_in_background(cx, sync, project);
    let window = h.window;
    h.wait("the watcher to start", |h| {
        window.read_with(h.cx, |w, _| w.is_watching()).unwrap()
    });
    changes_outside_while_typing(&mut h, true);
}

#[gpui::test]
fn a_background_window_reloads_and_asks_with_the_watcher(cx: &mut TestAppContext) {
    background_with_the_watcher(cx, true);
}

/// A file opened without a project (or outside its folder) is watched by
/// its folder.
#[gpui::test]
fn a_file_without_a_project_is_watched_too(cx: &mut TestAppContext) {
    background_with_the_watcher(cx, false);
}

#[gpui::test]
fn focus_sync_can_be_turned_off(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(
        cx,
        SyncSettings {
            sync_on_activation: false,
            ..SyncSettings::default()
        },
    );
    h.write("a.py", "changed = True\n");
    h.switch_away_and_back();
    assert_eq!(h.text(), A);
    // Reload All from Disk still looks.
    h.invoke("workbench.synchronize", json!(null));
    assert_eq!(h.text(), "changed = True\n");
}

#[gpui::test]
fn reload_all_from_disk_saves_then_looks_at_everything(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    h.invoke("project.tree.reveal", json!({"path": "src/m.py"}));
    let files = h.state()["file_index"].clone();
    // b.py is modified in its editor.
    h.write("b.py", "b = 1\n");
    h.invoke("project.open-file", json!({"path": "b.py"}));
    h.invoke("editor.type", json!({"text": "# b\n"}));
    // Outside: a.py changes, a file is created and one removed.
    h.write("a.py", "a = 2\n");
    h.write("src/new.py", "n = 1\n");
    std::fs::remove_file(h.root.join("src/m.py")).unwrap();

    h.invoke("workbench.synchronize", json!(null));
    assert_eq!(
        std::fs::read_to_string(h.root.join("b.py")).unwrap(),
        "# b\nb = 1\n"
    );
    assert_eq!(h.state()["tabs"][1]["modified"], json!(false));
    h.invoke("tabs.select", json!({"index": 0}));
    assert_eq!(h.text(), "a = 2\n");
    let rows = h.rows();
    assert!(rows.contains(&"src/new.py".to_owned()), "{rows:?}");
    assert!(rows.contains(&"b.py".to_owned()), "{rows:?}");
    assert!(!rows.contains(&"src/m.py".to_owned()), "{rows:?}");
    // a.py, b.py, src, src/new.py (m.py gone): one entry more.
    let now = h.state()["file_index"].clone();
    assert_eq!(now, json!(files.as_u64().unwrap() + 1));
}

#[gpui::test]
fn the_watcher_reports_changes_made_outside(cx: &mut TestAppContext) {
    support::allow_processes(cx);
    let (mut h, _dir) = open(
        cx,
        SyncSettings {
            watch: true,
            ..SyncSettings::default()
        },
    );
    let window = h.window;
    h.wait("the watcher to start", |h| {
        window.read_with(h.cx, |w, _| w.is_watching()).unwrap()
    });
    h.invoke("project.tree.reveal", json!({"path": "src/m.py"}));
    let a = h.root.join("a.py");

    // An open file: reloaded without the window losing the focus.
    h.write("a.py", "a = 3\n");
    h.wait("a.py to be reloaded", |h| h.text() == "a = 3\n");
    assert!(
        h.files_changed()
            .contains(&FileChange::new(&a, FileChangeKind::Changed))
    );

    // A new file and a new folder with a file (moved in at once): the
    // tree, the index and the language services see them.
    h.write("src/new.py", "n = 1\n");
    let staging = tempfile::tempdir_in(&h.root).unwrap();
    std::fs::write(staging.path().join("deep.py"), "d = 1\n").unwrap();
    std::fs::rename(staging.path(), h.root.join("pkg")).unwrap();
    let new = h.root.join("src/new.py");
    h.wait("the new files", |h| {
        let rows = h.rows();
        rows.contains(&"src/new.py".to_owned()) && rows.contains(&"pkg".to_owned())
    });
    let deep = h.root.join("pkg/deep.py");
    h.wait("the new files in the index", |h| {
        h.window
            .read_with(h.cx, |w, cx| {
                let index = w.project_files().read(cx).index();
                index.is_some_and(|i| i.find(&new).is_some() && i.find(&deep).is_some())
            })
            .unwrap()
    });
    let created = |h: &H, p: &Path| {
        h.files_changed()
            .iter()
            .any(|c| c.path == p && c.kind != FileChangeKind::Deleted)
    };
    h.wait("the services to hear about new.py", |h| created(h, &new));

    // Removed: gone from the tree and reported.
    std::fs::remove_file(&new).unwrap();
    h.wait("new.py to go", |h| {
        !h.rows().contains(&"src/new.py".to_owned())
    });
    h.wait("the services to hear about the removal", |h| {
        h.files_changed()
            .contains(&FileChange::new(&new, FileChangeKind::Deleted))
    });

    // The application's own write of a file without an editor (Replace in
    // Files writes this way) reaches the services too, as a change of a
    // file they know; its temporary file does not.
    let m = h.root.join("src/m.py");
    project::write_document(&m, b"m = 2\n").unwrap();
    h.wait("the services to hear about m.py", |h| {
        h.files_changed()
            .contains(&FileChange::new(&m, FileChangeKind::Changed))
    });
    assert!(
        h.files_changed()
            .iter()
            .all(|c| !c.path.to_string_lossy().contains(".app-save-")),
        "{:?}",
        h.files_changed()
    );

    // An open file deleted outside: its tab closes.
    std::fs::remove_file(&a).unwrap();
    h.wait("a.py's tab to close", |h| {
        h.state()["tabs"].as_array().is_some_and(|t| t.is_empty())
    });
}

#[gpui::test]
fn the_open_files_are_saved_when_the_application_quits(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, SyncSettings::default());
    manual_saves(&mut h);
    h.invoke("editor.type", json!({"text": "# last words\n"}));
    let a = h.root.join("a.py");
    h.cx.update(|cx| cx.shutdown());
    assert_eq!(
        std::fs::read_to_string(a).unwrap(),
        format!("# last words\n{A}")
    );
}
