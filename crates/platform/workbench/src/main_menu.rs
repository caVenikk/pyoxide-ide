//! The main menu (the reference's `MainMenu` action group of
//! `PlatformActions.xml`, with what `LangActions.xml`,
//! `ExecutionActions.xml`, `VcsActions.xml`, the Git plugin and PyCharm add
//! to it; IntelliJ Community, Apache-2.0): File, Edit, View, Navigate,
//! Code, Refactor, Run, Git, Window and Help, and on macOS the
//! application menu before them (Settings…, Services, Hide, Quit).
//!
//! The menu is a platform-neutral model ([`Menu`], [`Row`]): every row is
//! a command of the registry with its arguments (ADR 0007), so a row runs
//! exactly what its key binding and the control channel run, shows the
//! shortcut of the keymap in use, and is enabled where its command is
//! available in the active window. [`model`] lays the rows out in the
//! reference's order with its separators and submenus; rows of the
//! reference that have no command yet are left out. On macOS the model
//! becomes the menu bar ([`install`]); the same model is meant for the
//! in-window main menu of Windows and Linux.
//!
//! The menu bar is built again when the keymap changes (its shortcuts),
//! when the recent projects change and when another main window becomes
//! active (Recent Projects leaves out the window's own project, as the
//! reference's `RecentProjectsGroup` does).

use std::path::PathBuf;

use gpui::{Action, App, Global, KeyContext, SharedString, Window, WindowHandle};
use serde_json::Value as Json;

use crate::commands::{Hide, HideOthers, ShowAll};
use crate::recent_projects;
use crate::workbench::Workbench;

/// A menu of the menu bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub title: SharedString,
    pub rows: Vec<Row>,
}

/// A row of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A command of the registry and its arguments (`None`: none).
    Command {
        title: SharedString,
        command: &'static str,
        args: Option<Json>,
    },
    Separator,
    Submenu {
        title: SharedString,
        rows: Vec<Row>,
    },
    /// The system's Services menu (macOS).
    Services,
}

/// What the menu depends on.
#[derive(Debug, Clone, Default)]
pub struct MenuSpec {
    /// The product's name (the application menu, Hide and Quit).
    pub product: String,
    /// The macOS menu: the application menu takes Settings… and Quit, and
    /// Hide, Hide Others and Show All join it; elsewhere Settings… and Exit
    /// are rows of File.
    pub macos: bool,
    /// The rows of Recent Projects: name and folder, most recent first.
    pub recent: Vec<(String, PathBuf)>,
}

fn cmd(title: &str, command: &'static str) -> Row {
    Row::Command {
        title: title.to_owned().into(),
        command,
        args: None,
    }
}

fn cmd_args(title: impl Into<SharedString>, command: &'static str, args: Json) -> Row {
    Row::Command {
        title: title.into(),
        command,
        args: Some(args),
    }
}

fn sub(title: &str, rows: Vec<Row>) -> Row {
    Row::Submenu {
        title: title.to_owned().into(),
        rows,
    }
}

const SEP: Row = Row::Separator;

/// The menus in the reference's order (texts of `ActionsBundle`,
/// `GitBundle` and `VcsBundle`, their main menu variants where an action
/// has one). Rows without a command here are not listed; the comments name
/// the reference's groups.
pub fn model(spec: &MenuSpec) -> Vec<Menu> {
    let product = spec.product.as_str();
    let mut menus = Vec::new();
    if spec.macos {
        // The macOS application menu.
        menus.push(menu(
            product,
            vec![
                // About and Check for Updates… have no command yet.
                SEP,
                cmd("Settings\u{2026}", "settings.open"),
                SEP,
                Row::Services,
                SEP,
                cmd(&format!("Hide {product}"), "app.hide"),
                cmd("Hide Others", "app.hide-others"),
                cmd("Show All", "app.show-all"),
                SEP,
                cmd(&format!("Quit {product}"), "app.quit"),
            ],
        ));
    }
    menus.push(file_menu(spec));
    menus.push(edit_menu());
    menus.push(view_menu());
    menus.push(navigate_menu());
    menus.push(code_menu());
    menus.push(refactor_menu());
    menus.push(run_menu());
    // ToolsMenu: none of its rows has a command yet.
    menus.push(git_menu());
    menus.push(window_menu());
    menus.push(help_menu());
    menus.retain(|m| !m.rows.is_empty());
    menus
}

fn menu(title: &str, rows: Vec<Row>) -> Menu {
    Menu {
        title: title.to_owned().into(),
        rows: tidy(rows),
    }
}

/// Drops empty submenus and the separators that would show at an end or
/// twice in a row (the reference hides them the same way).
fn tidy(rows: Vec<Row>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for row in rows {
        match row {
            Row::Submenu { title, rows } => {
                let rows = tidy(rows);
                if !rows.is_empty() {
                    out.push(Row::Submenu { title, rows });
                }
            }
            Row::Separator => {
                if out.last().is_some_and(|r| *r != Row::Separator) {
                    out.push(Row::Separator);
                }
            }
            row => out.push(row),
        }
    }
    if out.last() == Some(&Row::Separator) {
        out.pop();
    }
    out
}

/// `FileMenu` with PyCharm's `PlatformOpenProjectGroup` first.
fn file_menu(spec: &MenuSpec) -> Menu {
    let recent = spec
        .recent
        .iter()
        .map(|(name, path)| {
            cmd_args(
                name.clone(),
                "project.open-recent",
                serde_json::json!({ "path": path.to_string_lossy() }),
            )
        })
        .collect();
    let mut rows = vec![
        // PlatformOpenProjectGroup: New Project… and Project from Version
        // Control… have no command yet; New… creates in the project tool
        // window's selection.
        cmd("New\u{2026}", "project.tree.new"),
        // FileOpenGroup.
        cmd("Open\u{2026}", "project.open-folder"),
        // `$LRU`: the recent projects (Manage Projects… has no command).
        sub("Recent Projects", recent),
        cmd("Close Project", "project.close"),
        SEP,
    ];
    if !spec.macos {
        // FileMainSettingsGroup (on macOS in the application menu).
        rows.push(cmd("Settings\u{2026}", "settings.open"));
    }
    rows.extend([
        SEP,
        cmd("Save All", "workbench.save-all"),
        cmd("Reload All from Disk", "workbench.synchronize"),
    ]);
    if !spec.macos {
        rows.extend([SEP, cmd("Exit", "app.quit")]);
    }
    menu("File", rows)
}

/// `EditMenu` with `FindUsagesMenuGroup` and `EditSelectWordGroup`.
fn edit_menu() -> Menu {
    menu(
        "Edit",
        vec![
            cmd("Undo", "editor.undo"),
            cmd("Redo", "editor.redo"),
            SEP,
            // CutCopyPasteGroup.
            cmd("Cut", "editor.cut"),
            cmd("Copy", "editor.copy"),
            cmd("Copy Paths", "project.tree.copy-paths"),
            sub("Paste", vec![cmd("Paste", "editor.paste")]),
            // EditCreateDeleteGroup.
            cmd("Delete", "editor.delete"),
            SEP,
            sub(
                "Find",
                vec![
                    cmd("Find\u{2026}", "find.open"),
                    cmd("Replace\u{2026}", "find.replace"),
                    cmd("Find Next Occurrence", "find.next"),
                    cmd("Find Previous Occurrence", "find.previous"),
                    SEP,
                    SEP,
                    cmd("Find in Files\u{2026}", "find-in-files.open"),
                    cmd("Replace in Files\u{2026}", "find-in-files.open-replace"),
                ],
            ),
            sub("Find Usages", vec![cmd("Show Usages", "code.show-usages")]),
            SEP,
            // EditSelectGroup.
            cmd("Select All", "editor.select-all"),
            cmd("Extend Selection", "editor.select-word"),
            cmd("Shrink Selection", "editor.shrink-selection"),
            SEP,
            // EditSmartGroup.
            cmd("Join Lines", "editor.join-lines"),
            cmd("Duplicate Line or Selection", "editor.duplicate"),
            SEP,
            cmd("Unindent Line or Selection", "editor.unindent"),
        ],
    )
}

/// `ViewMenu` with `QuickActions` and `CodeEditorViewGroup`.
fn view_menu() -> Menu {
    menu(
        "View",
        vec![
            // ToolWindowsGroup: ordered by the tool windows' number keys,
            // then by id.
            sub(
                "Tool Windows",
                vec![
                    cmd("Commit", "workbench.toggle-commit"),
                    cmd("Project", "workbench.toggle-project"),
                    cmd("Run", "workbench.toggle-run"),
                    cmd("Debug", "workbench.toggle-debug"),
                    cmd("Terminal", "workbench.toggle-terminal"),
                ],
            ),
            // QuickActions.
            SEP,
            cmd("Quick Documentation", "code.quick-doc"),
            // CodeEditorViewGroup.
            cmd("Parameter Info", "code.parameter-info"),
            cmd("Error Description", "code.error-description"),
            SEP,
            cmd("Jump to Source", "project.tree.open"),
            SEP,
            // ViewRecentActions.
            cmd("Recent Files", "recent-files.open"),
            SEP,
            cmd("Quick Switch Scheme\u{2026}", "settings.quick-switch"),
            sub(
                "Active Editor",
                vec![cmd("Soft-Wrap", "editor.toggle-soft-wraps")],
            ),
        ],
    )
}

/// `GoToMenu` with `GoToCodeGroup` and the change markers.
fn navigate_menu() -> Menu {
    menu(
        "Navigate",
        vec![
            cmd("Back", "workbench.back"),
            cmd("Forward", "workbench.forward"),
            SEP,
            cmd("Search Everywhere", "search-everywhere.open"),
            SEP,
            // GoToTargetEx.
            cmd("File\u{2026}", "search-everywhere.go-to-file"),
            cmd("Text\u{2026}", "search-everywhere.go-to-text"),
            cmd("Line:Column\u{2026}", "workbench.go-to-line"),
            SEP,
            // GoToErrorGroup.
            cmd("Next Highlighted Error", "code.next-problem"),
            cmd("Previous Highlighted Error", "code.previous-problem"),
            SEP,
            // NavigateInFileGroup with GoToChangeMarkerGroup.
            sub(
                "Navigate in File",
                vec![
                    SEP,
                    cmd("Next Change", "vcs.next-change"),
                    cmd("Previous Change", "vcs.previous-change"),
                ],
            ),
            // GoToCodeGroup.
            SEP,
            cmd("Declaration or Usages", "code.go-to-declaration"),
            SEP,
        ],
    )
}

/// `CodeMenu`.
fn code_menu() -> Menu {
    menu(
        "Code",
        vec![
            SEP,
            sub("Code Completion", vec![cmd("Basic", "code.complete")]),
            SEP,
            // CommentGroup.
            cmd("Comment with Line Comment", "editor.toggle-comment"),
            // CodeFormatGroup.
            cmd("Reformat Code", "code.reformat"),
            cmd("Optimize Imports", "code.optimize-imports"),
            SEP,
            cmd("Move Statement Down", "editor.move-statement-down"),
            cmd("Move Statement Up", "editor.move-statement-up"),
            cmd("Move Line Down", "editor.move-line-down"),
            cmd("Move Line Up", "editor.move-line-up"),
        ],
    )
}

/// `RefactoringMenu`: the file operations of the project tool window.
fn refactor_menu() -> Menu {
    menu(
        "Refactor",
        vec![
            cmd("Rename\u{2026}", "project.tree.rename"),
            SEP,
            cmd("Move\u{2026}", "project.tree.move"),
            cmd("Copy\u{2026}", "project.tree.copy-element"),
        ],
    )
}

/// `RunMenu` with `DebugMainMenu`.
fn run_menu() -> Menu {
    menu(
        "Run",
        vec![
            // RunnerActions: the executors of the selected configuration.
            cmd("Run", "run.run"),
            cmd("Debug", "debug.debug"),
            SEP,
            cmd("Run\u{2026}", "run.configurations"),
            cmd("Edit Configurations\u{2026}", "run.configurations.edit"),
            SEP,
            cmd("Stop", "run.stop"),
            // DebugMainMenu.
            SEP,
            sub(
                "Debugging Actions",
                vec![
                    cmd("Step Over", "debug.step-over"),
                    cmd("Step Into", "debug.step-into"),
                    // PyCharm's `StepIntoMyCode`, after Step Into.
                    cmd("Step Into My Code", "debug.step-into-my-code"),
                    cmd("Step Out", "debug.step-out"),
                    cmd("Run to Cursor", "debug.run-to-cursor"),
                    cmd("Force Run to Cursor", "debug.force-run-to-cursor"),
                    cmd("Pause Program", "debug.pause"),
                    cmd("Resume Program", "debug.resume"),
                    SEP,
                    cmd("Evaluate Expression\u{2026}", "debug.evaluate-expression"),
                    cmd("Show Execution Point", "debug.show-execution-point"),
                ],
            ),
            sub(
                "Toggle Breakpoint",
                vec![
                    cmd("Line Breakpoint", "debug.toggle-breakpoint"),
                    cmd(
                        "Temporary Line Breakpoint",
                        "debug.toggle-temporary-breakpoint",
                    ),
                    cmd(
                        "Toggle Breakpoint Enabled",
                        "debug.toggle-breakpoint-enabled",
                    ),
                ],
            ),
            cmd("View Breakpoints\u{2026}", "debug.view-breakpoints"),
            SEP,
        ],
    )
}

/// `VcsGroups` with the Git plugin's `Git.MainMenu`.
fn git_menu() -> Menu {
    menu(
        "Git",
        vec![
            cmd("Commit\u{2026}", "vcs.commit"),
            cmd("Push\u{2026}", "vcs.push"),
            cmd("Update Project\u{2026}", "vcs.update-project"),
            // Pull… opens the Pull dialog, which has no command yet
            // (`git.pull` pulls a given branch).
            cmd("Fetch", "git.fetch"),
            SEP,
            SEP,
            cmd("Branches\u{2026}", "vcs.branches-popup"),
            cmd("New Branch\u{2026}", "git.new-branch"),
            SEP,
            SEP,
            // Git.MainMenu.LocalChanges and Git.MainMenu.FileActions.
            sub(
                "Uncommitted Changes",
                vec![cmd("Rollback\u{2026}", "vcs.rollback")],
            ),
            sub("Current File", vec![cmd("Add", "vcs.add")]),
            SEP,
            SEP,
            cmd("VCS Operations\u{2026}", "vcs.operations-popup"),
            SEP,
        ],
    )
}

/// `WindowMenu`.
fn window_menu() -> Menu {
    menu(
        "Window",
        vec![
            cmd("Minimize", "window.minimize"),
            cmd("Zoom", "window.zoom"),
            SEP,
            sub(
                "Active Tool Window",
                vec![cmd("Hide Active Tool Window", "workbench.hide-tool-window")],
            ),
            sub(
                "Editor Tabs",
                vec![
                    cmd("Select Next Tab", "tabs.next"),
                    cmd("Select Previous Tab", "tabs.previous"),
                    cmd("Pin Active Tab", "tabs.toggle-pin"),
                    SEP,
                    // CloseEditorsGroup.
                    cmd("Close Tab", "tabs.close"),
                    cmd("Close Other Tabs", "tabs.close-others"),
                    cmd("Close All Tabs", "tabs.close-all"),
                    cmd("Close Unmodified Tabs", "tabs.close-unmodified"),
                    cmd("Close All but Pinned", "tabs.close-unpinned"),
                    cmd("Close Tabs to the Left", "tabs.close-left"),
                    cmd("Close Tabs to the Right", "tabs.close-right"),
                ],
            ),
            // BackgroundTasks.
            sub("Processes", vec![cmd("Show", "background-tasks.show")]),
        ],
    )
}

/// `HelpMenu` with the Welcome screen's `OpenWelcome` first.
fn help_menu() -> Menu {
    menu(
        "Help",
        vec![
            cmd("Welcome", "welcome.open"),
            cmd("Find Action\u{2026}", "search-everywhere.go-to-action"),
        ],
    )
}

/// The menus as GPUI's menu bar. A row whose command cannot be built (not
/// registered) is left out.
pub fn to_gpui(menus: &[Menu], cx: &App) -> Vec<gpui::Menu> {
    menus
        .iter()
        .map(|m| gpui::Menu {
            name: m.title.clone(),
            items: rows_to_gpui(&m.rows, cx),
        })
        .collect()
}

fn rows_to_gpui(rows: &[Row], cx: &App) -> Vec<gpui::MenuItem> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Command {
                title,
                command,
                args,
            } => build(command, args, cx).map(|action| gpui::MenuItem::Action {
                name: title.clone(),
                action,
                os_action: None,
            }),
            Row::Separator => Some(gpui::MenuItem::Separator),
            Row::Submenu { title, rows } => Some(gpui::MenuItem::Submenu(gpui::Menu {
                name: title.clone(),
                items: rows_to_gpui(rows, cx),
            })),
            Row::Services => Some(gpui::MenuItem::os_submenu(
                "Services",
                gpui::SystemMenuType::Services,
            )),
        })
        .collect()
}

/// The command's action with its arguments (none: an empty object, as
/// the control channel passes them).
pub fn build(command: &str, args: &Option<Json>, cx: &App) -> Option<Box<dyn Action>> {
    let args = args
        .clone()
        .unwrap_or_else(|| Json::Object(Default::default()));
    cx.build_action(command, Some(args)).ok()
}

/// The keystroke a native menu shows for `action`: GPUI's choice (the
/// first binding without a context or with one that holds in an editor,
/// else the first), and only a single keystroke.
pub fn shortcut(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let contexts = default_contexts();
    let mut bindings = keymap.bindings_for_action(action).peekable();
    let first = bindings.peek().cloned();
    let binding = bindings
        .find(|b| {
            b.predicate()
                .is_none_or(|p| p.eval_inner(&contexts, &contexts))
        })
        .or(first)?;
    let [keystroke] = binding.keystrokes() else {
        return None;
    };
    let keystroke = gpui::Keystroke {
        modifiers: *keystroke.modifiers(),
        key: keystroke.key().to_owned(),
        key_char: None,
    };
    Some(ui::keys::format_keystroke(&keystroke, true))
}

/// The key contexts GPUI's macOS menu bar looks bindings up in.
fn default_contexts() -> Vec<KeyContext> {
    let mut workspace = KeyContext::new_with_defaults();
    workspace.add("Workspace");
    let mut pane = KeyContext::new_with_defaults();
    pane.add("Pane");
    let mut editor = KeyContext::new_with_defaults();
    editor.add("Editor");
    pane.extend(&editor);
    workspace.extend(&pane);
    vec![workspace]
}

/// The menus as text, one row per line, indented by submenu: the title,
/// the shortcut the native menu shows and whether the row is enabled in
/// `window` (as the menu bar asks when it opens). For tests and the
/// control channel.
pub fn describe(menus: &[Menu], window: &mut Window, cx: &mut App) -> String {
    let mut out = String::new();
    for m in menus {
        out.push_str(&m.title);
        out.push('\n');
        describe_rows(&m.rows, 1, &mut out, window, cx);
    }
    out
}

fn describe_rows(rows: &[Row], depth: usize, out: &mut String, window: &mut Window, cx: &mut App) {
    let indent = "  ".repeat(depth);
    for row in rows {
        match row {
            Row::Command {
                title,
                command,
                args,
            } => {
                let Some(action) = build(command, args, cx) else {
                    out.push_str(&format!("{indent}{title}  [missing command {command}]\n"));
                    continue;
                };
                let available = window.is_action_available(action.as_ref(), cx)
                    || cx.is_action_available(action.as_ref());
                let mut line = format!("{indent}{title}");
                if let Some(keys) = shortcut(action.as_ref(), cx) {
                    line.push_str(&format!("  {keys}"));
                }
                if !available {
                    line.push_str("  (disabled)");
                }
                out.push_str(&line);
                out.push('\n');
            }
            Row::Separator => out.push_str(&format!("{indent}---\n")),
            Row::Submenu { title, rows } => {
                out.push_str(&format!("{indent}{title} >\n"));
                describe_rows(rows, depth + 1, out, window, cx);
            }
            Row::Services => out.push_str(&format!("{indent}Services >\n")),
        }
    }
}

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Describe the main menu.
    State => "main-menu.state", "Main Menu State",
        "Returns the main menu as the window shows it while active: `text` has a line per row with its shortcut, `(disabled)` where its command is not available, submenus indented.";
}

/// The product's name for the menus, and whether a rebuild is due.
#[derive(Default)]
struct MainMenuState {
    product: String,
    pending: bool,
}

impl Global for MainMenuState {}

/// Registers the application commands of the menu (Hide, Hide Others,
/// Show All) and its state query, sets the menu bar and builds it again when its contents
/// change.
pub fn init(cx: &mut App, product: &str) {
    cx.set_global(MainMenuState {
        product: product.to_owned(),
        pending: false,
    });
    ui::command::register(cx, COMMANDS);
    ui::command::register_query(
        cx,
        "main-menu.state",
        std::rc::Rc::new(|_, window, cx| {
            // The window is being updated: its root, not its handle.
            let own = window
                .root::<Workbench>()
                .flatten()
                .and_then(|w| w.read(cx).project_root(cx));
            let menus = model(&spec_without(own, cx));
            Ok(serde_json::json!({ "text": describe(&menus, window, cx) }))
        }),
    );
    // The test platform has no application to hide.
    cx.on_action(|_: &Hide, cx| {
        if crate::shot::native_windows() {
            cx.hide()
        }
    });
    cx.on_action(|_: &HideOthers, cx| {
        if crate::shot::native_windows() {
            cx.hide_other_apps()
        }
    });
    cx.on_action(|_: &ShowAll, cx| {
        if crate::shot::native_windows() {
            cx.unhide_other_apps()
        }
    });
    cx.observe_global::<recent_projects::Store>(refresh)
        .detach();
    install(cx);
}

/// Builds the menu bar again soon (once for several changes in a row).
pub fn refresh(cx: &mut App) {
    let Some(state) = cx.try_global::<MainMenuState>() else {
        return;
    };
    if state.pending {
        return;
    }
    cx.global_mut::<MainMenuState>().pending = true;
    cx.defer(|cx| {
        cx.global_mut::<MainMenuState>().pending = false;
        install(cx);
    });
}

/// The menu for the application now: the recent projects without the
/// active main window's project.
pub fn current_spec(cx: &App) -> MenuSpec {
    let active = cx.active_window().and_then(|w| w.downcast::<Workbench>());
    spec_for(active, cx)
}

/// The menu while `window` is the active main window (`None`: none is).
pub fn spec_for(window: Option<WindowHandle<Workbench>>, cx: &App) -> MenuSpec {
    spec_without(window.and_then(|w| w.read(cx).ok()?.project_root(cx)), cx)
}

/// The menu with the recent projects but `own`.
fn spec_without(own: Option<PathBuf>, cx: &App) -> MenuSpec {
    let recent = recent_projects::list(cx)
        .projects()
        .iter()
        .filter(|p| own.as_deref() != Some(p.path.as_path()))
        .map(|p| (p.name.clone(), p.path.clone()))
        .collect();
    MenuSpec {
        product: cx
            .try_global::<MainMenuState>()
            .map(|s| s.product.clone())
            .unwrap_or_default(),
        macos: cfg!(target_os = "macos"),
        recent,
    }
}

/// Sets the menu bar from [`current_spec`].
pub fn install(cx: &mut App) {
    let menus = to_gpui(&model(&current_spec(cx)), cx);
    cx.set_menus(menus);
}

/// The commands of every row of `menus` (for checks that they exist).
pub fn commands(menus: &[Menu]) -> Vec<&'static str> {
    fn walk(rows: &[Row], out: &mut Vec<&'static str>) {
        for row in rows {
            match row {
                Row::Command { command, .. } => out.push(command),
                Row::Submenu { rows, .. } => walk(rows, out),
                Row::Separator | Row::Services => {}
            }
        }
    }
    let mut out = Vec::new();
    for m in menus {
        walk(&m.rows, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Command { title, .. } => title.to_string(),
                Row::Separator => "---".into(),
                Row::Submenu { title, .. } => format!("{title} >"),
                Row::Services => "Services >".into(),
            })
            .collect()
    }

    #[test]
    fn separators_never_double_or_end_a_menu_and_empty_submenus_go() {
        let rows = tidy(vec![
            SEP,
            cmd("A", "a"),
            SEP,
            SEP,
            sub("Empty", vec![SEP]),
            SEP,
            cmd("B", "b"),
            SEP,
        ]);
        assert_eq!(titles(&rows), ["A", "---", "B"]);
    }

    #[test]
    fn macos_puts_settings_and_quit_into_the_application_menu() {
        let spec = MenuSpec {
            product: "Pyoxide".into(),
            macos: true,
            recent: vec![("demo".into(), PathBuf::from("/p/demo"))],
        };
        let menus = model(&spec);
        let names: Vec<_> = menus.iter().map(|m| m.title.to_string()).collect();
        assert_eq!(
            names,
            [
                "Pyoxide", "File", "Edit", "View", "Navigate", "Code", "Refactor", "Run", "Git",
                "Window", "Help"
            ]
        );
        let app = titles(&menus[0].rows);
        assert_eq!(
            app,
            [
                "Settings\u{2026}",
                "---",
                "Services >",
                "---",
                "Hide Pyoxide",
                "Hide Others",
                "Show All",
                "---",
                "Quit Pyoxide"
            ]
        );
        let file = titles(&menus[1].rows);
        assert!(!file.contains(&"Exit".to_owned()));
        assert!(!file.contains(&"Settings\u{2026}".to_owned()));
        let Row::Submenu { rows, .. } = &menus[1].rows[2] else {
            panic!("Recent Projects is the third row of File");
        };
        assert_eq!(
            rows[0],
            cmd_args(
                "demo",
                "project.open-recent",
                serde_json::json!({ "path": "/p/demo" })
            )
        );
    }

    #[test]
    fn elsewhere_file_has_settings_and_exit_and_no_application_menu() {
        let spec = MenuSpec {
            product: "Pyoxide".into(),
            macos: false,
            recent: Vec::new(),
        };
        let menus = model(&spec);
        assert_eq!(menus[0].title, "File");
        let file = titles(&menus[0].rows);
        assert_eq!(
            file,
            [
                "New\u{2026}",
                "Open\u{2026}",
                "Close Project",
                "---",
                "Settings\u{2026}",
                "---",
                "Save All",
                "Reload All from Disk",
                "---",
                "Exit"
            ]
        );
    }
}
