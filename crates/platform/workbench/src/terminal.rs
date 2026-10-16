//! Terminal sessions of a main window (ADR 0014): the product's hook and
//! the plan of a new session from the Terminal settings, made off the UI
//! thread. The Terminal tool window starts its tabs with [`plan`].

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Global, Task};
use terminal::{ShellCustomizer, StartPlan, StartRequest, plan_session};

pub use ::settings::TerminalSettings;

/// What a product adds to terminal sessions (the Python product activates
/// the project's virtual environment).
pub trait TerminalProvider {
    /// The customizers of a session about to start in the project at
    /// `project_root`. Called on the UI thread, so that they can take what
    /// the product knows there (an interpreter chosen in this session);
    /// they run off it, in [`terminal::plan_session`].
    fn customizers(&self, project_root: Option<&Path>, cx: &App) -> Vec<Arc<dyn ShellCustomizer>>;
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
    cx.set_global(Hook(hook));
}

/// The product's customizers of a session in the project at
/// `project_root`; none without a product hook.
pub fn customizers(project_root: Option<&Path>, cx: &App) -> Vec<Arc<dyn ShellCustomizer>> {
    match cx.try_global::<Hook>().and_then(|h| h.0.clone()) {
        Some(hook) => hook.0.customizers(project_root, cx),
        None => Vec::new(),
    }
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
