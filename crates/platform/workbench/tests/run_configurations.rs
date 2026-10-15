//! Run configurations in the main window (tests/scenarios/run-debug/
//! run-configurations.md): the reference IDE's configurations of a
//! project are imported when it opens, listed in the run widget's
//! dropdown in the reference's order with the reference's selection, and
//! run from the widget; `.idea` stays as it was. The test product's
//! "Script" type runs a configuration's command as a shell script (see
//! tests/support).

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::keymap::{self, Keymap};
use workbench::{FileType, Product, Workbench};

use support::{script, tick};

const MAC: bool = cfg!(target_os = "macos");

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
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
    let text = if MAC { keymap::MACOS } else { keymap::DEFAULT };
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
    let state = h.invoke("workbench.state", json!(null));
    assert_eq!(
        state["notice"],
        json!("Imported 4 run configurations (1 kept read-only)")
    );
    let w = h.widget();
    assert_eq!(w["configuration"], json!("beta"));
    assert_eq!(w["configuration_icon"], json!("run/run"));
    assert_eq!(w["run_tooltip"], json!("Run 'beta'"));
    assert_eq!(w["run_enabled"], json!(true));

    // The dropdown: the type's folder as a submenu, its configurations,
    // the unknown type last, Current File and Edit Configurations….
    h.invoke("run.configurations", json!(null));
    let menu = h.run()["menu"].clone();
    let labels: Vec<Value> = menu["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].clone())
        .collect();
    assert_eq!(
        labels,
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
    assert_eq!(menu["levels"][0]["items"][0]["submenu"], json!(true));
    assert_eq!(
        menu["levels"][0]["items"][4]["description"],
        json!("FutureType, not supported yet")
    );
    // The selected configuration is in the folder: nothing is preselected
    // at the top level but the first row.
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
