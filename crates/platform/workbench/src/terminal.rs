//! Terminal sessions of a main window (ADR 0014): the product's hook and
//! the plan of a new session from the Terminal settings, made off the UI
//! thread. The Terminal tool window starts its tabs with [`plan`].

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Global, Task};
use terminal::{ShellCustomizer, StartPlan, StartRequest, plan_session};

pub use ::settings::{TerminalFontSettings, TerminalSettings};

/// What a product adds to terminal sessions (the Python product activates
/// the project's virtual environment).
pub trait TerminalProvider {
    /// The customizers of a session about to start in the project at
    /// `project_root`. Called on the UI thread, so that they can take what
    /// the product knows there (an interpreter chosen in this session);
    /// they run off it, in [`terminal::plan_session`].
    fn customizers(&self, project_root: Option<&Path>, cx: &App) -> Vec<Arc<dyn ShellCustomizer>>;

    /// What the product links in the output of a session: the filters for a
    /// shell working in a folder (the Python product links the files of
    /// tracebacks). The terminal itself links URLs and, with Cmd (Ctrl)
    /// held, paths.
    fn link_filters(&self) -> Option<terminal_ui::LinkFilters> {
        None
    }
}

/// A product's [`TerminalProvider`].
#[derive(Clone)]
pub struct TerminalHook(pub Rc<dyn TerminalProvider>);

impl TerminalHook {
    pub fn new(provider: impl TerminalProvider + 'static) -> Self {
        Self(Rc::new(provider))
    }
}

impl std::fmt::Debug for TerminalHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminalHook")
    }
}

/// The product's hook of this application.
struct Hook(Option<TerminalHook>);

impl Global for Hook {}

/// Registers the Terminal settings and keeps the product's hook.
pub(crate) fn init(cx: &mut App, hook: Option<TerminalHook>) {
    settings_ui::store::register::<TerminalSettings>(cx);
    settings_ui::store::register::<TerminalFontSettings>(cx);
    settings_ui::store::register::<::settings::EditorFontSettings>(cx);
    follow_font(cx);
    cx.set_global(Hook(hook));
    route_keys(cx);
}

/// The terminal font of the settings: its own choice, else the editor
/// font's family and size (the reference starts from the console font,
/// which is the editor font until it is changed); the spacings are 1 until
/// chosen.
pub fn font_of(
    terminal: &TerminalFontSettings,
    editor: &::settings::EditorFontSettings,
) -> terminal_ui::TerminalFont {
    terminal_ui::TerminalFont {
        family: terminal.family.clone().or_else(|| editor.family.clone()),
        size: terminal
            .size
            .or(editor.size)
            .map(TerminalFontSettings::clamp_size),
        line_spacing: terminal
            .line_spacing
            .map_or(TerminalFontSettings::DEFAULT_SPACING, |s| {
                TerminalFontSettings::clamp_spacing(s)
            }),
        column_spacing: terminal
            .column_spacing
            .map_or(TerminalFontSettings::DEFAULT_SPACING, |s| {
                TerminalFontSettings::clamp_spacing(s)
            }),
        fallback: terminal.fallback_family().map(str::to_owned),
    }
}

/// Keeps the terminal view's font in step with the settings.
fn follow_font(cx: &mut App) {
    fn apply(cx: &mut App) {
        let font = font_of(
            &settings_ui::store::get::<TerminalFontSettings>(cx),
            &settings_ui::store::get::<::settings::EditorFontSettings>(cx),
        );
        terminal_ui::set_font(cx, font);
    }
    apply(cx);
    settings_ui::store::observe(
        cx,
        <TerminalFontSettings as ::settings::Section>::KEY,
        apply,
    );
    settings_ui::store::observe(
        cx,
        <::settings::EditorFontSettings as ::settings::Section>::KEY,
        apply,
    );
}

/// The product's customizers of a session in the project at
/// `project_root`; none without a product hook.
pub fn customizers(project_root: Option<&Path>, cx: &App) -> Vec<Arc<dyn ShellCustomizer>> {
    match cx.try_global::<Hook>().and_then(|h| h.0.clone()) {
        Some(hook) => hook.0.customizers(project_root, cx),
        None => Vec::new(),
    }
}

/// The product's link filters for the sessions of this application.
pub fn link_filters(cx: &App) -> Option<terminal_ui::LinkFilters> {
    cx.try_global::<Hook>()
        .and_then(|h| h.0.clone())
        .and_then(|hook| hook.0.link_filters())
}

/// A request for a new session in the project at `project_root` with the
/// Terminal settings: the shell path and the shell integration switch.
pub fn request(project_root: Option<PathBuf>, cx: &App) -> StartRequest {
    let settings: TerminalSettings = settings_ui::store::get(cx);
    StartRequest {
        shell_path: settings.shell().map(str::to_owned),
        shell_integration: settings.shell_integration,
        project_root,
        ..StartRequest::default()
    }
}

/// The plan of a session for `request` with the product's customizers,
/// made on the background executor ([`terminal::plan_session`] reads
/// folders, writes the integration scripts and runs the customizers).
pub fn plan(request: StartRequest, cx: &App) -> Task<StartPlan> {
    let customizers = customizers(request.project_root.as_deref(), cx);
    cx.background_spawn(async move {
        let customizers: Vec<&dyn ShellCustomizer> = customizers.iter().map(|c| &**c).collect();
        plan_session(&request, &customizers)
    })
}

/// The reference's actions whose shortcuts a focused terminal leaves to the
/// IDE while "Override IDE shortcuts" is on
/// (`TerminalEventDispatcher.ALLOWED_ACTION_IDS`); the terminal's own
/// commands are allowed as well.
pub const ALLOWED_ACTION_IDS: &[&str] = &[
    "ActivateTerminalToolWindow",
    "ActivateProjectToolWindow",
    "ActivateFavoritesToolWindow",
    "ActivateBookmarksToolWindow",
    "ActivateFindToolWindow",
    "ActivateRunToolWindow",
    "ActivateDebugToolWindow",
    "ActivateProblemsViewToolWindow",
    "ActivateTODOToolWindow",
    "ActivateStructureToolWindow",
    "ActivateHierarchyToolWindow",
    "ActivateServicesToolWindow",
    "ActivateCommitToolWindow",
    "ActivateVersionControlToolWindow",
    "HideActiveWindow",
    "HideAllWindows",
    "NextWindow",
    "PreviousWindow",
    "NextProjectWindow",
    "PreviousProjectWindow",
    "ShowBookmarks",
    "ShowTypeBookmarks",
    "FindInPath",
    "GotoBookmark0",
    "GotoBookmark1",
    "GotoBookmark2",
    "GotoBookmark3",
    "GotoBookmark4",
    "GotoBookmark5",
    "GotoBookmark6",
    "GotoBookmark7",
    "GotoBookmark8",
    "GotoBookmark9",
    "GotoAction",
    "GotoFile",
    "GotoClass",
    "GotoSymbol",
    "Vcs.Push",
    "ShowSettings",
    "RecentFiles",
    "Switcher",
    "ResizeToolWindowLeft",
    "ResizeToolWindowRight",
    "ResizeToolWindowUp",
    "ResizeToolWindowDown",
    "MaximizeToolWindow",
    "NextTab",
    "PreviousTab",
    "ShowContent",
];

/// A command whose keys a focused terminal leaves to the IDE: one of the
/// [`ALLOWED_ACTION_IDS`] (through the action table) or a terminal command.
pub fn is_allowed_command(command: &str) -> bool {
    static ALLOWED: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    command.starts_with("terminal.")
        || ALLOWED
            .get_or_init(|| {
                let table = crate::keymap::actions::ActionTable::builtin();
                ALLOWED_ACTION_IDS
                    .iter()
                    .filter_map(|id| table.get(id))
                    .flat_map(|a| a.targets.iter().map(|t| t.command.clone()))
                    .collect()
            })
            .contains(command)
}

/// The key routing of a focused terminal (`TerminalEventDispatcher`): with
/// "Override IDE shortcuts" on, a key whose binding runs a command that is
/// not allowed goes to the shell instead (Ctrl+R to the shell's history
/// search rather than Run). Keys without a binding reach the session's own
/// key handler anyway.
pub(crate) fn route_keys(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        if !event
            .context_stack
            .iter()
            .any(|c| c.contains(terminal_ui::KEY_CONTEXT))
        {
            return;
        }
        let settings: TerminalSettings = settings_ui::store::get(cx);
        if !settings.override_ide_shortcuts {
            return;
        }
        let (bindings, _) = cx
            .key_bindings()
            .borrow()
            .bindings_for_input(std::slice::from_ref(&event.keystroke), &event.context_stack);
        let Some(binding) = bindings.first() else {
            return;
        };
        if is_allowed_command(binding.action().name()) {
            return;
        }
        let Some(session) = window
            .root::<crate::workbench::Workbench>()
            .flatten()
            .and_then(|w| w.read(cx).terminal_session(cx))
        else {
            return;
        };
        if !gpui::Focusable::focus_handle(session.read(cx), cx).is_focused(window) {
            return;
        }
        if session.update(cx, |s, cx| s.send_keystroke(&event.keystroke, cx)) {
            cx.stop_propagation();
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use ::settings::EditorFontSettings;

    use super::*;

    #[test]
    fn the_terminal_inherits_the_editor_font_until_it_has_its_own() {
        let editor = EditorFontSettings {
            family: Some("Fira Code".into()),
            size: Some(15.0),
            ..EditorFontSettings::default()
        };
        let inherited = font_of(&TerminalFontSettings::default(), &editor);
        assert_eq!(inherited.family.as_deref(), Some("Fira Code"));
        assert_eq!(inherited.size, Some(15.0));
        assert_eq!(inherited.line_spacing, 1.0);
        assert_eq!(inherited.column_spacing, 1.0);
        // No font of the machine is added as a fallback.
        assert_eq!(inherited.fallback, None);

        let own = TerminalFontSettings {
            family: Some("JetBrainsMono Nerd Font".into()),
            size: Some(100.0),
            line_spacing: Some(1.2),
            fallback: Some("Monospaced".into()),
            ..TerminalFontSettings::default()
        };
        let font = font_of(&own, &editor);
        assert_eq!(font.family.as_deref(), Some("JetBrainsMono Nerd Font"));
        assert_eq!(font.size, Some(40.0));
        assert_eq!(font.line_spacing, 1.2);
        assert_eq!(font.fallback, None);
        assert_eq!(
            font_of(
                &TerminalFontSettings::default(),
                &EditorFontSettings::default()
            )
            .family,
            None
        );
    }
}
