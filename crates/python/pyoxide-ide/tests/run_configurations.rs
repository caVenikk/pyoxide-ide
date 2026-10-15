//! Python run configurations in the Pyoxide window: a project of the
//! reference IDE with a Python configuration is opened, the configuration
//! is imported and selected, and Run starts it with the project's
//! interpreter (a stand-in shell script, so no Python is needed).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::Workbench;

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    cx.run_until_parked();
    out
}

fn wait(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    what: &str,
    f: impl Fn(&Value) -> bool,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..2000 {
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        last = invoke(cx, window, "run.state", json!(null));
        if f(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}: {last:#}");
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

const WORKSPACE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="RunManager" selected="Python.server">
    <configuration name="server" type="PythonConfigurationType" factoryName="Python">
      <module name="demo" />
      <option name="ENV_FILES" value="$PROJECT_DIR$/.env" />
      <option name="INTERPRETER_OPTIONS" value="" />
      <option name="PARENT_ENVS" value="true" />
      <envs>
        <env name="PYTHONUNBUFFERED" value="1" />
        <env name="EXPLICIT" value="yes" />
      </envs>
      <option name="SDK_HOME" value="" />
      <option name="WORKING_DIRECTORY" value="$PROJECT_DIR$/app" />
      <option name="IS_MODULE_SDK" value="true" />
      <option name="ADD_CONTENT_ROOTS" value="true" />
      <option name="ADD_SOURCE_ROOTS" value="true" />
      <option name="RUN_TOOL" value="" />
      <option name="SCRIPT_NAME" value="$PROJECT_DIR$/app/main.py" />
      <option name="PARAMETERS" value="--port 8000" />
      <option name="SHOW_COMMAND_LINE" value="false" />
      <option name="EMULATE_TERMINAL" value="false" />
      <option name="MODULE_MODE" value="false" />
      <option name="REDIRECT_INPUT" value="false" />
      <option name="INPUT_FILE" value="" />
      <method v="2" />
    </configuration>
  </component>
</project>
"#;

/// Prints its arguments, folder and two variables.
const PYTHON: &str = "#!/bin/sh\necho \"ARGS:$*\"\necho \"CWD:$(pwd)\"\necho \"FROM_FILE:${FROM_FILE-unset}\"\necho \"EXPLICIT:${EXPLICIT-unset}\"\n";

#[gpui::test]
fn an_imported_python_configuration_runs_with_the_project_interpreter(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    write(&root, ".idea/workspace.xml", WORKSPACE);
    write(&root, ".env", "FROM_FILE=file\n");
    write(&root, "app/main.py", "print('hi')\n");
    write(
        &root,
        ".venv/pyvenv.cfg",
        "home = /usr/bin\nversion = 3.12.1\n",
    );
    let python = root.join(".venv/bin/python");
    write(&root, ".venv/bin/python", PYTHON);
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    let idea = std::fs::read(root.join(".idea/workspace.xml")).unwrap();

    let product = Rc::new(pyoxide_ide::product_with_tools(Default::default()));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    cx.run_until_parked();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    let s = wait(cx, window, "the configurations", |s| {
        s["configurations"]["loading"] == json!(false)
    });
    let config = &s["configurations"]["configurations"][0];
    assert_eq!(config["name"], json!("server"));
    assert_eq!(config["type"], json!("python"));
    assert_eq!(config["type_name"], json!("Python"));
    let widget = &s["widget"];
    assert_eq!(widget["configuration"], json!("server"));
    assert_eq!(widget["configuration_icon"], json!("python/file"));
    assert_eq!(widget["run_tooltip"], json!("Run 'server'"));

    invoke(cx, window, "run.run", json!(null));
    let s = wait(cx, window, "the run", |s| {
        s["tabs"][0]["status"] == json!("terminated")
    });
    assert_eq!(s["tabs"][0]["title"], json!("server"));
    let text: Vec<String> = s["tabs"][0]["console"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["text"].as_str().unwrap().to_owned())
        .collect();
    let main = root.join("app/main.py");
    assert_eq!(
        text[0],
        format!("{} {} --port 8000 ", python.display(), main.display())
    );
    assert!(
        text.contains(&format!("ARGS:{} --port 8000", main.display())),
        "{text:?}"
    );
    assert!(
        text.contains(&format!("CWD:{}", root.join("app").display())),
        "{text:?}"
    );
    assert!(text.contains(&"FROM_FILE:file".to_owned()), "{text:?}");
    assert!(text.contains(&"EXPLICIT:yes".to_owned()), "{text:?}");
    assert_eq!(
        std::fs::read(root.join(".idea/workspace.xml")).unwrap(),
        idea
    );
}
