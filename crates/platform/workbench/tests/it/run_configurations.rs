//! Run configurations in the main window (tests/scenarios/run-debug/
//! run-configurations.md): the reference IDE's configurations of a
//! project are imported when it opens, listed in the run widget's
//! dropdown in the reference's order with the reference's selection, and
//! run from the widget; `.idea` stays as it was. The test product's
//! "Script" type runs a configuration's command as a shell script (see
//! tests/support).

use crate::support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::keymap::{self, Keymap};
use workbench::{FileType, Product, Workbench};

use crate::support::{script, tick};

const MAC: bool = cfg!(target_os = "macos");

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
}

impl H<'_> {
    /// Draws every window (the main one and an open dialog).
    fn draw(&mut self) {
        self.cx.run_until_parked();
        for window in self.cx.update(|cx| cx.windows()) {
            let _ = self
                .cx
                .update_window(window, |_, window, cx| window.draw(cx).clear());
        }
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
        out
    }

    fn run(&mut self) -> Value {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke("run.state", json!(null), window, cx)
            })
            .unwrap()
            .unwrap()
    }

    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            tick(self.cx);
            last = self.run();
            if f(&last) {
                self.draw();
                return last;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    fn loaded(&mut self) -> Value {
        self.wait("the run configurations", |s| {
            s["configurations"]["loading"] == json!(false)
        })
    }

    fn widget(&mut self) -> Value {
        self.run()["widget"].clone()
    }
}

fn console_text(state: &Value, tab: usize) -> String {
    state["tabs"][tab]["console"]["lines"]
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .map(|l| l["text"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Every file under a folder with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.clone(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// A project of the reference IDE with per-user and shared
/// configurations of the test type, and one of a type nobody knows.
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let beta = script("echo beta-$MODE", "echo beta-!MODE!");
    write(
        &root,
        ".idea/workspace.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="RunManager" selected="Script.beta">
    <configuration name="beta" type="ScriptType" factoryName="Script" folderName="Group">
      <option name="COMMAND" value="{beta}" />
      <envs>
        <env name="MODE" value="one" />
      </envs>
      <method v="2" />
    </configuration>
    <configuration name="alpha" type="ScriptType" factoryName="Script">
      <option name="COMMAND" value="echo alpha" />
      <method v="2" />
    </configuration>
    <configuration name="later" type="FutureType" factoryName="Future">
      <method v="2" />
    </configuration>
  </component>
</project>
"#
        ),
    );
    write(
        &root,
        ".idea/runConfigurations/shared.xml",
        r#"<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="shared" type="ScriptType" factoryName="Script">
    <option name="COMMAND" value="echo shared" />
    <method v="2" />
  </configuration>
</component>
"#,
    );
    write(&root, "a.py", "a = 1\n");
    (dir, root)
}

fn setup<'a>(cx: &'a mut TestAppContext, root: &Path) -> H<'a> {
    setup_with(cx, root, if MAC { keymap::MACOS } else { keymap::DEFAULT })
}

/// The window with the keymap in `text`.
fn setup_with<'a>(cx: &'a mut TestAppContext, root: &Path, text: &str) -> H<'a> {
    support::allow_processes(cx);
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        run: Some(support::run_hook()),
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &Keymap::parse(text).unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H { cx, window };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    h
}

fn names(state: &Value) -> Vec<String> {
    state["configurations"]["configurations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Opening a project imports the reference's configurations once: they
/// are listed in the reference's order with its selection, a
/// notification says what came in, and `.idea` is left as it was.
#[gpui::test]
fn the_reference_configurations_show_in_the_run_widget(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let idea = snapshot(&root.join(".idea"));
    let mut h = setup(cx, &root);
    let s = h.loaded();
    assert_eq!(names(&s), ["beta", "alpha", "shared", "later"]);
    assert_eq!(
        s["configurations"]["selected"],
        json!({"type": "script", "name": "beta"})
    );
    let later = &s["configurations"]["configurations"][3];
    assert_eq!(later["supported"], json!(false));
    assert_eq!(later["type_name"], json!("FutureType"));
    let shared = &s["configurations"]["configurations"][2];
    assert_eq!(shared["storage"], json!("shared"));
    // The import report is a balloon.
    let state = h.invoke("workbench.state", json!(null));
    let balloon = &state["notifications"]["balloons"][0];
    assert_eq!(
        balloon["content"],
        json!("Imported 4 run configurations (1 kept read-only)")
    );
    assert_eq!(balloon["kind"], json!("information"));
    assert_eq!(balloon["sticky"], json!(false));
    let w = h.widget();
    assert_eq!(w["configuration"], json!("beta"));
    assert_eq!(w["configuration_icon"], json!("run/run"));
    assert_eq!(w["run_tooltip"], json!("Run 'beta'"));
    assert_eq!(w["run_enabled"], json!(true));

    // The dropdown: the type's folder as a submenu, its configurations,
    // the unknown type last with its marker, Current File and Edit
    // Configurations…; the configuration in the folder once more for
    // speed search only.
    h.invoke("run.configurations", json!(null));
    let menu = h.run()["menu"].clone();
    let items = menu["levels"][0]["items"].as_array().unwrap().clone();
    let shown: Vec<Value> = items
        .iter()
        .filter(|i| i.get("hidden").is_none())
        .map(|i| i["label"].clone())
        .collect();
    assert_eq!(
        shown,
        [
            json!("Group"),
            json!("alpha"),
            json!("shared"),
            Value::Null,
            json!("later"),
            Value::Null,
            json!("Current File"),
            Value::Null,
            json!("Edit Configurations\u{2026}"),
        ]
    );
    assert_eq!(items[0]["submenu"], json!(true));
    let alpha = &items[1];
    assert_eq!(alpha["command"], json!("run.configuration.select"));
    let inline: Vec<Value> = alpha["inline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["tooltip"].clone())
        .collect();
    // This product has no debugger: no Debug button
    // (`RunSpecifiedConfigExecutorAction` without a runner is hidden).
    assert_eq!(inline, [json!("Run 'alpha'")]);
    let hidden_beta = items.iter().find(|i| i["label"] == json!("beta")).unwrap();
    assert_eq!(hidden_beta["hidden"], json!(true));
    assert_eq!(hidden_beta["secondary"], json!("Group"));
    let later = items.iter().find(|i| i["label"] == json!("later")).unwrap();
    assert_eq!(later["secondary"], json!("not supported yet"));
    assert!(later.get("inline").is_none());
    // A row's submenu: Run, Debug, "Configuration", Edit Configurations…,
    // Delete, Pin.
    h.invoke("menu.down", json!(null));
    h.invoke("menu.down", json!(null));
    h.invoke("menu.open-submenu", json!(null));
    let menu = h.run()["menu"].clone();
    let sub: Vec<Value> = menu["levels"][1]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.get("header").cloned().unwrap_or(i["label"].clone()))
        .collect();
    assert_eq!(
        sub,
        [
            json!("Run"),
            json!("Debug"),
            json!("Configuration"),
            json!("Edit Configurations\u{2026}"),
            json!("Delete"),
            json!("Pin")
        ]
    );
    h.invoke("menu.cancel", json!(null));
    h.invoke("menu.cancel", json!(null));

    // Selecting by command; Current File; an unsupported one cannot run.
    h.invoke("run.configuration.select", json!({"name": "alpha"}));
    assert_eq!(h.widget()["configuration"], json!("alpha"));
    h.invoke("run.select-current-file", json!(null));
    assert_eq!(h.widget()["configuration"], json!("Current File"));
    h.invoke("run.configuration.select", json!({"name": "later"}));
    let w = h.widget();
    assert_eq!(w["run_enabled"], json!(false));
    assert_eq!(
        w["run_tooltip"],
        json!("'later' cannot run: its type is not supported yet")
    );

    // The selection is kept; the next open imports nothing again.
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    let s = h.loaded();
    assert_eq!(names(&s), ["beta", "alpha", "shared", "later"]);
    assert_eq!(
        s["configurations"]["selected"],
        json!({"type": "idea:FutureType", "name": "later"})
    );
    assert_eq!(snapshot(&root.join(".idea")), idea);
    assert!(root.join(".oxide/run/shared.toml").is_file());
}

/// Run runs the selected configuration in the Run tool window, with its
/// environment; its tab is named after it.
#[gpui::test]
fn run_runs_the_selected_configuration(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    h.invoke("run.run", json!(null));
    let s = h.wait("the configuration's output", |s| {
        s["tabs"][0]["status"] == json!("terminated")
    });
    assert_eq!(s["tabs"][0]["title"], json!("beta"));
    let text = console_text(&s, 0);
    assert!(text.contains("\nbeta-one"), "{text}");
    assert!(text.contains("Process finished with exit code 0"), "{text}");
    // It is the first in the history now.
    assert_eq!(
        s["configurations"]["history"],
        json!([{"type": "script", "name": "beta"}])
    );
    // By name, without selecting it.
    h.invoke("run.configuration.run", json!({"name": "shared"}));
    let s = h.wait("the shared configuration's output", |s| {
        s["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["title"] == json!("shared") && t["status"] == json!("terminated"))
    });
    assert_eq!(s["widget"]["configuration"], json!("beta"));
}

/// A Shell Script configuration (the platform's type) runs in the Run
/// tool window with its variables.
#[gpui::test]
fn a_shell_script_configuration_runs_in_the_run_tool_window(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let text = script("echo shell-$MODE", "echo shell-%MODE%");
    write(
        &root,
        ".idea/workspace.xml",
        &format!(
            r#"<project version="4">
  <component name="RunManager">
    <configuration name="inline" type="ShConfigurationType">
      <option name="SCRIPT_TEXT" value="{text}" />
      <option name="EXECUTE_SCRIPT_FILE" value="false" />
      <option name="EXECUTE_IN_TERMINAL" value="true" />
      <envs>
        <env name="MODE" value="two" />
      </envs>
      <method v="2" />
    </configuration>
  </component>
</project>
"#
        ),
    );
    let mut h = setup(cx, &root);
    let s = h.loaded();
    assert_eq!(names(&s), ["inline"]);
    let w = h.widget();
    assert_eq!(w["configuration"], json!("inline"));
    assert_eq!(w["configuration_icon"], json!("fileTypes/shell"));
    h.invoke("run.run", json!(null));
    let s = h.wait("the script's output", |s| {
        s["tabs"][0]["status"] == json!("terminated")
    });
    assert_eq!(s["tabs"][0]["title"], json!("inline"));
    let text = console_text(&s, 0);
    assert!(text.contains("\nshell-two"), "{text}");
}

fn invoke_and_draw(h: &mut H<'_>, id: &str, args: Value) -> Value {
    h.invoke(id, args)
}

fn dialog(h: &mut H<'_>) -> Value {
    h.run()["edit_dialog"].clone()
}

fn tree(h: &mut H<'_>) -> Vec<String> {
    dialog(h)["tree"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            format!(
                "{}{}",
                "  ".repeat(n["depth"].as_u64().unwrap() as usize),
                n["text"].as_str().unwrap()
            )
        })
        .collect()
}

fn field(h: &mut H<'_>, id: &str) -> Value {
    dialog(h)["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == json!(id))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Run Context Configuration (⌃⇧R / Ctrl+Shift+F10) makes a temporary
/// configuration of the file and selects it; at most five are kept, the
/// oldest goes; Save Configuration keeps one; the same file reuses its
/// configuration.
#[gpui::test]
fn context_runs_make_temporary_configurations(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for i in 0..6 {
        write(&root, &format!("t{i}.py"), "x = 1\n");
    }
    let mut h = setup(cx, &root);
    h.loaded();
    support::set_script(&script("echo ok", "echo ok"));
    let keys = if MAC {
        "ctrl-shift-r"
    } else {
        "ctrl-shift-f10"
    };
    for i in 0..6 {
        h.invoke("project.open-file", json!({"path": format!("t{i}.py")}));
        h.cx.simulate_keystrokes(h.window.into(), keys);
        h.draw();
        h.wait(&format!("the run of t{i}"), |s| {
            s["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["title"] == json!(format!("t{i}")) && t["status"] == json!("terminated"))
        });
    }
    let s = h.run();
    // The sixth removed the oldest.
    assert_eq!(names(&s), ["t1", "t2", "t3", "t4", "t5"]);
    assert!(
        s["configurations"]["configurations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["temporary"] == json!(true))
    );
    let w = &s["widget"];
    assert_eq!(w["configuration"], json!("t5"));
    assert_eq!(w["configuration_temporary"], json!(true));
    // The same file again: its configuration is reused.
    h.cx.simulate_keystrokes(h.window.into(), keys);
    h.draw();
    assert_eq!(names(&h.run()).len(), 5);
    // Save Configuration and Pin from the commands.
    h.invoke("run.configuration.save-temporary", json!({"name": "t1"}));
    h.invoke("run.configuration.pin", json!({"name": "t2"}));
    let s = h.run();
    let t1 = s["configurations"]["configurations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == json!("t1"))
        .unwrap()
        .clone();
    assert_eq!(t1["temporary"], json!(false));
    assert_eq!(
        s["configurations"]["pinned"],
        json!([{"type": "script", "name": "t2"}])
    );
    // The dropdown now has Pinned and Recent sections and the toggle.
    h.invoke("run.configurations", json!(null));
    let items = h.run()["menu"]["levels"][0]["items"].clone();
    assert_eq!(items[0]["header"], json!("Pinned"));
    assert_eq!(items[1]["label"], json!("t2"));
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["header"] == json!("Recent Configurations"))
    );
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["label"] == json!("All Configurations  5"))
    );
    h.invoke("menu.cancel", json!(null));
    h.invoke("run.configurations.toggle-all", json!(null));
    assert_eq!(h.run()["configurations"]["show_all"], json!(true));
    h.invoke("menu.cancel", json!(null));
    // Current File runs the file without a configuration.
    h.invoke("run.file", json!(null));
    assert_eq!(names(&h.run()).len(), 5);
}

/// Delete asks ("Are you sure you want to delete …?"); Yes deletes the
/// configuration and a shared one's file.
#[gpui::test]
fn delete_asks_and_removes_a_shared_file(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    assert!(root.join(".oxide/run/shared.toml").is_file());
    h.invoke("run.configuration.delete", json!({"name": "shared"}));
    let state = h.invoke("workbench.state", json!(null));
    assert_eq!(state["dialog"]["title"], json!("Confirmation"));
    assert_eq!(
        state["dialog"]["message"],
        json!("Are you sure you want to delete shared?")
    );
    h.invoke("dialog.choose", json!({"button": "Yes"}));
    let s = h.wait("the file is gone", |_| {
        !root.join(".oxide/run/shared.toml").exists()
    });
    assert_eq!(names(&s), ["beta", "alpha", "later"]);
    // "No" keeps it.
    h.invoke("run.configuration.delete", json!({"name": "alpha"}));
    h.invoke("dialog.choose", json!({"button": "No"}));
    assert_eq!(names(&h.run()), ["beta", "alpha", "later"]);
}

/// The Run/Debug Configurations dialog: the tree by type and folder, the
/// form of the selected configuration, Apply keeps the changes, Cancel
/// drops them; Add, Copy, Create New Folder, Remove; "Store as project
/// file" asks about personal values.
#[gpui::test]
fn the_edit_dialog_edits_configurations(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    h.invoke("run.configurations.edit", json!({"name": "alpha"}));
    // The dialog is a window of its own, modal to the main one.
    let main: gpui::AnyWindowHandle = h.window.into();
    assert!(
        h.cx.update(|cx| ui::dialog_window::modal_of(main, cx))
            .is_some()
    );
    assert_eq!(h.cx.update(|cx| cx.windows().len()), 2);
    let d = dialog(&mut h);
    assert_eq!(d["title"], json!("Run/Debug Configurations"));
    // Collapsed but for the way to the selected configuration.
    assert_eq!(
        tree(&mut h),
        ["Script", "  Group", "  alpha", "  shared", "FutureType"]
    );
    assert_eq!(
        d["selected"],
        json!({"kind": "configuration", "type": "script", "name": "alpha"})
    );
    assert_eq!(field(&mut h, "name")["value"], json!("alpha"));
    assert_eq!(field(&mut h, "command")["value"], json!("echo alpha"));
    assert_eq!(d["store_as_project_file"], json!(false));
    // Edit the command, Apply.
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "command", "value": "echo changed"}),
    );
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "env", "value": "TOKEN=secret-value;MODE=x"}),
    );
    h.invoke("run.configurations.dialog.apply", json!(null));
    assert_eq!(dialog(&mut h)["modified"], json!(false));
    // Validation: an empty command.
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "command", "value": ""}),
    );
    assert_eq!(dialog(&mut h)["error"], json!("Please specify a command"));
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "command", "value": "echo changed"}),
    );
    // Store as project file with personal values asks first.
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "store-as-project-file", "value": "true"}),
    );
    // The question is asked in the dialog's window.
    assert_eq!(
        dialog(&mut h)["prompt"]["title"],
        json!("Store as Project File")
    );
    assert!(h.invoke("workbench.state", json!(null))["dialog"].is_null());
    h.invoke("dialog.choose", json!({"button": "Keep Values Personal"}));
    assert!(dialog(&mut h)["prompt"].is_null());
    assert_eq!(dialog(&mut h)["store_as_project_file"], json!(true));
    assert_eq!(field(&mut h, "shared-env")["value"], json!(""));
    // A rename, then Add, Copy, Create New Folder, Remove, Sort.
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "name", "value": "renamed"}),
    );
    h.invoke("run.configurations.dialog.add", json!({"type": "script"}));
    assert_eq!(field(&mut h, "name")["value"], json!("Unnamed"));
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "command", "value": "echo new"}),
    );
    h.invoke("run.configurations.dialog.copy", json!(null));
    assert_eq!(field(&mut h, "name")["value"], json!("Unnamed (1)"));
    h.invoke("run.configurations.dialog.new-folder", json!(null));
    assert_eq!(
        dialog(&mut h)["selected"],
        json!({"kind": "folder", "type": "script", "folder": "New Folder"})
    );
    h.invoke(
        "run.configurations.dialog.select",
        json!({"name": "Unnamed (1)"}),
    );
    h.invoke("run.configurations.dialog.remove", json!(null));
    assert!(!tree(&mut h).iter().any(|n| n.trim() == "Unnamed (1)"));
    h.invoke("run.configurations.dialog.sort", json!(null));
    // Path fields show their paths expanded (as the reference loads them);
    // they are stored relative to the project.
    h.invoke(
        "run.configurations.dialog.add",
        json!({"type": "shell-script"}),
    );
    let dir = field(&mut h, "working-directory")["value"].clone();
    assert!(
        Path::new(dir.as_str().unwrap()).is_absolute(),
        "{dir}: the project folder"
    );
    let script = Path::new(dir.as_str().unwrap())
        .join("tools")
        .join("build.sh");
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "script", "value": script.display().to_string()}),
    );
    let shown = field(&mut h, "script")["value"].clone();
    assert_eq!(Path::new(shown.as_str().unwrap()), script.as_path());
    h.invoke("run.configurations.dialog.apply", json!(null));
    h.wait("the stored script path", |_| {
        std::fs::read_to_string(root.join(".oxide/workspace.toml"))
            .is_ok_and(|t| t.contains("script = \"tools/build.sh\""))
    });
    h.invoke("run.configurations.dialog.remove", json!(null));
    // Modify options: a common option.
    h.invoke(
        "run.configurations.dialog.select",
        json!({"name": "Unnamed"}),
    );
    h.invoke("run.configurations.dialog.modify-options", json!(null));
    assert!(!dialog(&mut h)["menu"].is_null());
    h.invoke("menu.cancel", json!(null));
    h.invoke(
        "run.configurations.dialog.toggle-option",
        json!({"option": "allow-parallel"}),
    );
    let options = dialog(&mut h)["options"].clone();
    assert!(
        options
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == json!("allow-parallel") && o["checked"] == json!(true))
    );
    // An unsupported one is read-only.
    h.invoke("run.configurations.dialog.select", json!({"name": "later"}));
    assert_eq!(dialog(&mut h)["read_only"], json!(true));
    h.invoke("run.configurations.dialog.ok", json!(null));
    assert!(h.run()["edit_dialog"].is_null());
    assert!(
        h.cx.update(|cx| ui::dialog_window::modal_of(main, cx))
            .is_none()
    );
    assert_eq!(h.cx.update(|cx| cx.windows().len()), 1);
    let s = h.wait("the shared file", |_| {
        root.join(".oxide/run/renamed.toml").is_file()
    });
    assert_eq!(names(&s), ["beta", "renamed", "shared", "Unnamed", "later"]);
    let shared = std::fs::read_to_string(root.join(".oxide/run/renamed.toml")).unwrap();
    assert!(shared.contains("{ name = \"TOKEN\" }"), "{shared}");
    assert!(!shared.contains("secret-value"), "{shared}");
    let personal = std::fs::read_to_string(root.join(".oxide/workspace.toml")).unwrap();
    assert!(personal.contains("secret-value"), "{personal}");
    // Cancel drops changes.
    h.invoke("run.configurations.edit", json!({"name": "beta"}));
    h.invoke("run.configurations.dialog.remove", json!(null));
    h.invoke("run.configurations.dialog.cancel", json!(null));
    let _ = invoke_and_draw(&mut h, "workbench.state", json!(null));
    assert!(names(&h.run()).contains(&"beta".to_owned()));
    // Run applies, closes the dialog and runs the configuration.
    h.invoke("run.configurations.edit", json!({"name": "beta"}));
    h.invoke("run.configurations.dialog.run", json!(null));
    assert!(h.run()["edit_dialog"].is_null());
    h.wait("the run of beta", |s| {
        s["tabs"]
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t["title"] == json!("beta")))
    });
}

/// A configuration's file open in an editor: the application's writes of
/// it are its own: an unmodified editor shows the new text, an editor
/// with changes keeps them, and no File Cache Conflict comes up.
#[gpui::test]
fn writes_of_an_open_configuration_file_are_the_applications_own(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    let file = root.join(".oxide/run/shared.toml");
    h.invoke(
        "project.open-file",
        json!({"path": file.display().to_string()}),
    );
    let text = |h: &mut H<'_>| {
        h.invoke("editor.text", json!(null))
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(text(&mut h).contains("echo shared"));
    // Unmodified: the editor follows the write.
    h.invoke("run.configurations.edit", json!({"name": "shared"}));
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "command", "value": "echo first"}),
    );
    h.invoke("run.configurations.dialog.ok", json!(null));
    h.wait("the editor follows", |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t.contains("echo first"))
    });
    let state = h.invoke("workbench.state", json!(null));
    assert!(state["dialog"].is_null(), "{:#}", state["dialog"]);
    assert!(text(&mut h).contains("echo first"), "{}", text(&mut h));
    // With changes of the person's: kept, no conflict.
    h.invoke("editor.type", json!({"text": "# mine\n"}));
    h.invoke("run.configurations.edit", json!({"name": "shared"}));
    h.invoke(
        "run.configurations.dialog.set",
        json!({"field": "command", "value": "echo second"}),
    );
    h.invoke("run.configurations.dialog.ok", json!(null));
    h.wait("the second write", |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t.contains("echo second"))
    });
    for _ in 0..5 {
        tick(h.cx);
    }
    let state = h.invoke("workbench.state", json!(null));
    assert!(state["dialog"].is_null(), "{:#}", state["dialog"]);
    assert!(text(&mut h).contains("# mine"));
}

/// The Modify options link underlines its text while the pointer is over
/// it (#136); a hover style cannot do it (GPUI lays text out before it
/// knows what is hovered), the dialog keeps the state.
#[gpui::test]
fn the_modify_options_link_underlines_under_the_pointer(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    h.invoke("run.configurations.edit", json!({"name": "alpha"}));
    let main: gpui::AnyWindowHandle = h.window.into();
    let win =
        h.cx.update(|cx| ui::dialog_window::modal_of(main, cx))
            .expect("the dialog window");
    let link = gpui::VisualTestContext::from_window(win, h.cx)
        .debug_bounds("run-dialog-modify-options")
        .expect("the link");
    assert_eq!(dialog(&mut h)["modify_options_underlined"], json!(false));
    gpui::VisualTestContext::from_window(win, h.cx).simulate_mouse_move(
        link.center(),
        None,
        gpui::Modifiers::none(),
    );
    h.draw();
    assert_eq!(dialog(&mut h)["modify_options_underlined"], json!(true));
    // The link shows the pointing hand (a dialog window applies its own
    // pointer shapes).
    let shape = gpui::VisualTestContext::from_window(win, h.cx)
        .update(|window, _| ui::pointer::resolve(window));
    assert_eq!(shape, gpui::CursorStyle::PointingHand);
    gpui::VisualTestContext::from_window(win, h.cx).simulate_mouse_move(
        gpui::point(gpui::px(5.0), gpui::px(300.0)),
        None,
        gpui::Modifiers::none(),
    );
    h.draw();
    assert_eq!(dialog(&mut h)["modify_options_underlined"], json!(false));
    let shape = gpui::VisualTestContext::from_window(win, h.cx)
        .update(|window, _| ui::pointer::resolve(window));
    assert_eq!(shape, gpui::CursorStyle::Arrow);
}

/// The empty Run tool window's link: its bounds in the main window.
fn empty_link(h: &mut H<'_>) -> gpui::Bounds<gpui::Pixels> {
    let main: gpui::AnyWindowHandle = h.window.into();
    gpui::VisualTestContext::from_window(main, h.cx)
        .debug_bounds("run-empty-link")
        .expect("the link of the empty Run tool window")
}

/// Shows the empty Run tool window (no run yet).
fn show_empty_run_window(h: &mut H<'_>) {
    h.loaded();
    h.invoke("workbench.toggle-run", json!(null));
    assert_eq!(h.run()["tabs"], json!([]));
    assert!(!h.run()["empty"].is_null());
}

/// A click on the "run configuration" link of the empty Run tool window
/// opens the Run/Debug Configurations window (`EditConfigurationsDialog`)
/// with the selected configuration selected, or with nothing selected
/// while Current File is selected (#183).
#[gpui::test]
fn the_empty_run_window_link_opens_the_configurations_dialog(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    show_empty_run_window(&mut h);
    let main: gpui::AnyWindowHandle = h.window.into();

    // The project's selected configuration (`selected="Script.beta"`).
    assert_eq!(h.run()["configurations"]["selected"]["name"], json!("beta"));
    assert!(h.run()["edit_dialog"].is_null());
    let link = empty_link(&mut h);
    gpui::VisualTestContext::from_window(main, h.cx)
        .simulate_click(link.center(), gpui::Modifiers::none());
    h.draw();
    assert_eq!(dialog(&mut h)["selected"]["name"], json!("beta"));
    // `EditConfigurationsDialog(project)` has no Run button.
    assert_eq!(dialog(&mut h)["run_button"], json!(false));
    h.invoke("run.configurations.dialog.cancel", json!(null));
    assert!(h.run()["edit_dialog"].is_null());

    // Current File: the dialog opens with nothing selected and the tree
    // collapsed, as the reference shows it.
    h.invoke("run.select-current-file", json!(null));
    let link = empty_link(&mut h);
    gpui::VisualTestContext::from_window(main, h.cx)
        .simulate_click(link.center(), gpui::Modifiers::none());
    h.draw();
    let d = dialog(&mut h);
    assert!(!d.is_null(), "the dialog opened");
    assert_eq!(d["selected"], json!(null), "{d:#}");
    let rows = tree(&mut h);
    assert!(rows.iter().all(|row| !row.starts_with("  ")), "{rows:?}");
}

/// The link shows the hand and underlines its text while the pointer is
/// over it.
#[gpui::test]
fn the_empty_run_window_link_shows_the_hand_and_underlines(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    show_empty_run_window(&mut h);
    let main: gpui::AnyWindowHandle = h.window.into();
    let link = empty_link(&mut h);
    let shape = |h: &mut H<'_>| {
        gpui::VisualTestContext::from_window(main, h.cx)
            .update(|window, _| ui::pointer::resolve(window))
    };
    assert_eq!(h.run()["empty"]["link_hovered"], json!(false));
    assert_eq!(shape(&mut h), gpui::CursorStyle::Arrow);

    gpui::VisualTestContext::from_window(main, h.cx).simulate_mouse_move(
        link.center(),
        None,
        gpui::Modifiers::none(),
    );
    h.draw();
    assert_eq!(h.run()["empty"]["link_hovered"], json!(true));
    assert_eq!(shape(&mut h), gpui::CursorStyle::PointingHand);

    gpui::VisualTestContext::from_window(main, h.cx).simulate_mouse_move(
        gpui::point(link.origin.x - gpui::px(40.0), link.center().y),
        None,
        gpui::Modifiers::none(),
    );
    h.draw();
    assert_eq!(h.run()["empty"]["link_hovered"], json!(false));
    assert_eq!(shape(&mut h), gpui::CursorStyle::Arrow);
}

/// The shortcut is drawn as soon as the Run tool window opens, without
/// the pointer over the link (#201). Only the frames the window asks for
/// are drawn, as in the application: no draw is forced after the key.
#[gpui::test]
fn the_empty_run_window_shows_the_shortcut_when_it_opens(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    let main: gpui::AnyWindowHandle = h.window.into();
    let mut vcx = gpui::VisualTestContext::from_window(main, h.cx);
    vcx.simulate_keystrokes(if MAC { "cmd-4" } else { "alt-4" });
    assert!(vcx.debug_bounds("run-empty").is_some(), "the empty state");
    assert!(
        vcx.debug_bounds("run-empty-shortcut").is_some(),
        "the shortcut is drawn"
    );
}

/// The last line of the empty state and the shortcut in it.
fn launch_line(h: &mut H<'_>) -> (String, Value) {
    let empty = h.run()["empty"].clone();
    let line = empty["lines"][3].as_str().unwrap().to_owned();
    (line, empty["shortcut"].clone())
}

/// The last line ends with the shortcut of Choose Run Configuration in
/// brackets, from the active keymap: symbols on macOS (⌃⌥R in the macOS
/// keymap), the words elsewhere.
#[gpui::test]
fn the_empty_run_window_names_the_key_of_the_macos_keymap(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup_with(cx, &root, keymap::MACOS);
    show_empty_run_window(&mut h);
    let keys = if MAC {
        "\u{2303}\u{2325}R"
    } else {
        "Ctrl+Alt+R"
    };
    let (line, shortcut) = launch_line(&mut h);
    assert_eq!(
        line,
        format!("\u{2014} Launch a run configuration ({keys})")
    );
    assert_eq!(shortcut, json!(keys));
}

/// The default keymap's key (Alt+Shift+F10), the other lines of the
/// reference, and no brackets for a keymap without a key for the action.
#[gpui::test]
fn the_empty_run_window_names_the_key_of_the_default_keymap(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup_with(cx, &root, keymap::DEFAULT);
    show_empty_run_window(&mut h);
    let keys = if MAC {
        "\u{2325}\u{21E7}F10"
    } else {
        "Alt+Shift+F10"
    };
    let (line, shortcut) = launch_line(&mut h);
    assert_eq!(
        line,
        format!("\u{2014} Launch a run configuration ({keys})")
    );
    assert_eq!(shortcut, json!(keys));
    let lines = h.run()["empty"]["lines"].clone();
    assert_eq!(lines.as_array().unwrap().len(), 4);
    assert_eq!(
        lines[0],
        json!("To run your code, do one of the following:")
    );
    assert_eq!(
        lines[1],
        json!("\u{2014} Click the Run icon in the editor gutter")
    );
    assert_eq!(
        lines[2],
        json!("\u{2014} Select \"Run\u{2026}\" in the editor context menu")
    );

    h.cx.update(|cx| cx.clear_key_bindings());
    h.draw();
    let (line, shortcut) = launch_line(&mut h);
    assert_eq!(line, "\u{2014} Launch a run configuration");
    assert_eq!(shortcut, json!(null));
}

/// With nothing selected the dialog's form shows the reference's tip; its
/// link asks for the type of a new configuration.
#[gpui::test]
fn the_dialog_without_a_selection_shows_the_tip(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = setup(cx, &root);
    h.loaded();
    h.invoke("run.select-current-file", json!(null));
    h.invoke("run.configurations.edit", json!(null));
    assert_eq!(dialog(&mut h)["selected"], json!(null));
    // From the dropdown's Edit Configurations the dialog has Run.
    assert_eq!(dialog(&mut h)["run_button"], json!(true));
    let main: gpui::AnyWindowHandle = h.window.into();
    let win =
        h.cx.update(|cx| ui::dialog_window::modal_of(main, cx))
            .expect("the dialog window");
    let link = gpui::VisualTestContext::from_window(win, h.cx)
        .debug_bounds("run-dialog-tip-add")
        .expect("the tip's link");
    assert!(dialog(&mut h)["menu"].is_null());
    gpui::VisualTestContext::from_window(win, h.cx)
        .simulate_click(link.center(), gpui::Modifiers::none());
    h.draw();
    assert!(!dialog(&mut h)["menu"].is_null(), "the types are offered");
}
