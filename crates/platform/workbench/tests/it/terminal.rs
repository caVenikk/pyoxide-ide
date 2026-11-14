//! Terminal sessions of a main window (ADR 0014): a new session's plan
//! takes the shell path and the shell integration switch from the
//! Terminal settings and the product's customizers, and is made on the
//! background executor.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, TestAppContext};
use serde_json::json;
use terminal::{Customization, ShellContext, ShellCustomizer, StartPlan};
use workbench::{Product, TerminalHook, TerminalProvider};

/// Names the project and what the session was told.
struct Customizer {
    root: Option<PathBuf>,
}

impl ShellCustomizer for Customizer {
    fn customize(&self, context: &ShellContext<'_>) -> Customization {
        let root = self
            .root
            .as_deref()
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_default();
        Customization {
            env: vec![
                ("TEST_PROJECT".into(), root),
                (
                    "TEST_INTEGRATION".into(),
                    context.shell_integration.to_string(),
                ),
            ],
            source: None,
        }
    }
}

struct Provider;

impl TerminalProvider for Provider {
    fn customizers(&self, project_root: Option<&Path>, _: &App) -> Vec<Arc<dyn ShellCustomizer>> {
        vec![Arc::new(Customizer {
            root: project_root.map(Path::to_owned),
        })]
    }
}

fn get<'a>(plan: &'a StartPlan, name: &str) -> Option<&'a str> {
    plan.options
        .env
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

#[gpui::test]
fn sessions_follow_the_settings_and_the_product(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    std::fs::create_dir(&root).unwrap();
    let file = dir.path().join("config").join("settings.toml");
    let product = Product {
        name: "Test IDE",
        terminal: Some(TerminalHook::new(Provider)),
        ..Product::default()
    };
    cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, file.clone());
    });

    let plan_for = |cx: &mut TestAppContext| -> StartPlan {
        let out: Rc<RefCell<Option<StartPlan>>> = Rc::default();
        cx.update(|cx| {
            let mut request = workbench::terminal::request(Some(root.clone()), cx);
            request.integration_dir = Some(dir.path().join("integration"));
            let task = workbench::terminal::plan(request, cx);
            let out = out.clone();
            cx.spawn(async move |_| *out.borrow_mut() = Some(task.await))
                .detach();
        });
        cx.run_until_parked();
        out.take().expect("the plan")
    };

    // The defaults: the default shell with the integration.
    let plan = plan_for(cx);
    assert!(!plan.options.command.program.is_empty());
    assert_eq!(plan.options.working_directory, root);
    let project = root.to_string_lossy().into_owned();
    assert_eq!(get(&plan, "TEST_PROJECT"), Some(project.as_str()));
    assert_eq!(get(&plan, "TEST_INTEGRATION"), Some("true"));

    // The settings: a shell path and the integration off.
    let (shell, program, option) = if cfg!(windows) {
        ("cmd.exe /k", "cmd.exe", "/k")
    } else {
        ("/bin/sh -x", "/bin/sh", "-x")
    };
    cx.update(|cx| {
        settings_ui::store::set_key(cx, "terminal.shell-path", json!(shell)).unwrap();
        settings_ui::store::set_key(cx, "terminal.shell-integration", json!(false)).unwrap();
        settings_ui::store::flush(cx);
    });
    cx.run_until_parked();
    let plan = plan_for(cx);
    assert_eq!(plan.options.command.program, program);
    assert_eq!(plan.options.command.args[0], option);
    assert!(!plan.shell_integration);
    assert_eq!(get(&plan, "TEST_INTEGRATION"), Some("false"));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("[terminal]"), "{text}");
    assert!(text.contains("shell-integration = false"), "{text}");
}

#[gpui::test]
fn a_product_without_a_hook_adds_nothing(cx: &mut TestAppContext) {
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    cx.update(|cx| workbench::init(cx, &product));
    let customizers = cx.update(|cx| workbench::terminal::customizers(Some(Path::new("/p")), cx));
    assert!(customizers.is_empty());
}

#[gpui::test]
fn imported_project_options_reach_a_real_shell(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join(".idea")).unwrap();
    std::fs::create_dir(root.join("child")).unwrap();
    std::fs::write(
        root.join(".idea/workspace.xml"),
        r#"<project>
        <component name="TerminalProjectNonSharedOptionsProvider">
          <option name="startingDirectory" value="$PROJECT_DIR$/child" />
          <envs><env key="TERMINAL_PROJECT_VALUE" value="from-project"/></envs>
        </component>
    </project>"#,
    )
    .unwrap();
    cx.update(|cx| workbench::init(cx, &Product::default()));
    let options = |root: &Path, cx: &mut TestAppContext| {
        let out = Rc::new(RefCell::new(None));
        cx.update(|cx| {
            let mut request = workbench::terminal::request(Some(root.to_owned()), cx);
            request.shell_integration = false;
            request.command = Some(if cfg!(windows) {
                terminal::ShellCommand {
                    program: "cmd.exe".into(),
                    args: vec![
                        "/D".into(),
                        "/C".into(),
                        "echo".into(),
                        "%TERMINAL_PROJECT_VALUE%>probe.txt".into(),
                    ],
                }
            } else {
                terminal::ShellCommand {
                    program: "/bin/sh".into(),
                    args: vec![
                        "-c".into(),
                        r#"printf %s "$TERMINAL_PROJECT_VALUE" > probe.txt"#.into(),
                    ],
                }
            });
            let task = workbench::terminal::plan(request, cx);
            let out = out.clone();
            cx.spawn(async move |_| *out.borrow_mut() = Some(task.await))
                .detach();
        });
        cx.run_until_parked();
        out.take().unwrap().options
    };
    let (session, events) = terminal::Session::spawn(options(root, cx)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let event = events.recv_deadline(deadline).expect("shell exits");
        if matches!(event, terminal::SessionEvent::Exited(_)) {
            break;
        }
    }
    drop(session);
    let probe = std::fs::read_to_string(root.join("child/probe.txt")).unwrap();
    assert_eq!(probe.trim_end(), "from-project");

    // A different project that forbids inheriting the parent environment
    // never starts the command, including when a tab is restored.
    let isolated = tempfile::tempdir().unwrap();
    let root = isolated.path();
    std::fs::create_dir(root.join(".idea")).unwrap();
    std::fs::write(
        root.join(".idea/workspace.xml"),
        r#"<project><component name="TerminalProjectNonSharedOptionsProvider">
        <option name="isPassParentEnvs" value="false" /></component></project>"#,
    )
    .unwrap();
    assert!(matches!(
        terminal::Session::spawn(options(root, cx)),
        Err(terminal::SpawnError::IsolatedEnvironment)
    ));
    assert!(!root.join("probe.txt").exists());
}
