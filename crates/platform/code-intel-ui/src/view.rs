//! The code intelligence of one main window.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use code_intel::{CodeIntel, FileChange};
use editor::Editor;
use gpui::{AnyElement, App, AppContext as _, Context, Entity, InteractiveElement, Window};
use serde_json::{Value as Json, json};

use crate::commands::*;
use crate::hook::CodeIntelHook;

/// The code intelligence of a main window: the project's language
/// services, the documents of the window's editor tabs and the popups over
/// them. The workbench reports editors and saves; the view does the rest.
pub struct CodeIntelView {
    hook: CodeIntelHook,
    /// The services of the open project.
    services: Option<Arc<dyn CodeIntel>>,
    root: Option<PathBuf>,
    _quit: gpui::Subscription,
}

impl CodeIntelView {
    pub fn new(hook: CodeIntelHook, cx: &mut Context<Self>) -> Self {
        // Servers do not outlive the application.
        let quit = cx.on_app_quit(|this: &mut Self, cx| {
            let services = this.services.take();
            cx.background_spawn(async move {
                if let Some(services) = services {
                    let _ = services.shutdown().await;
                }
            })
        });
        Self {
            hook,
            services: None,
            root: None,
            _quit: quit,
        }
    }

    pub fn hook(&self) -> &CodeIntelHook {
        &self.hook
    }

    /// The language services of the open project.
    pub fn services(&self) -> Option<&Arc<dyn CodeIntel>> {
        self.services.as_ref()
    }

    /// The project folder whose services these are.
    pub fn project_root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// A project was opened in the window: its services replace the
    /// previous project's, which shut down. The editors of the previous
    /// project are gone (no `editor_closed` for them).
    pub fn project_opened(&mut self, root: &Path, cx: &mut Context<Self>) {
        if let Some(old) = self.services.take() {
            cx.background_spawn(old.shutdown()).detach();
        }
        self.root = Some(root.to_owned());
        self.services = Some(self.hook.0.start(root, cx));
        cx.notify();
    }

    /// An editor tab was opened for a file of this language (`None` when
    /// its file type has no language id: no services).
    pub fn editor_opened(
        &mut self,
        path: &Path,
        language: Option<&str>,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        let _ = (path, language, editor, cx);
    }

    /// The editor tab of a file was closed.
    pub fn editor_closed(&mut self, path: &Path, cx: &mut Context<Self>) {
        let _ = (path, cx);
    }

    /// The file of an editor tab was written to disk.
    pub fn file_saved(&mut self, path: &Path, cx: &mut Context<Self>) {
        let _ = (path, cx);
    }

    /// The application changed files on disk (outside the editors).
    pub fn files_changed(&mut self, changes: &[FileChange], cx: &mut Context<Self>) {
        let _ = (changes, cx);
    }

    /// The popups over the editor (completion, documentation, parameter
    /// info), drawn by the workbench above its content.
    pub fn render_overlay(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        let _ = (window, cx);
        None
    }

    /// The state reported by `code.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        let _ = cx;
        json!({
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "services": self.services.is_some(),
        })
    }

    /// The servers reported by `code.servers`.
    pub fn servers_json(&self, cx: &App) -> Json {
        let _ = cx;
        let servers = self
            .services
            .as_ref()
            .map(|s| s.status())
            .unwrap_or_default();
        serde_json::to_value(servers).unwrap_or(Json::Null)
    }

    // ---- commands ----------------------------------------------------------

    fn complete(&mut self, _: &Complete, _: &mut Window, _: &mut Context<Self>) {}

    fn quick_doc(&mut self, _: &QuickDoc, _: &mut Window, _: &mut Context<Self>) {}

    fn parameter_info(&mut self, _: &ParameterInfo, _: &mut Window, _: &mut Context<Self>) {}

    fn go_to_declaration(&mut self, _: &GoToDeclaration, _: &mut Window, _: &mut Context<Self>) {}

    fn show_usages(&mut self, _: &ShowUsages, _: &mut Window, _: &mut Context<Self>) {}

    fn show_intentions(&mut self, _: &ShowIntentions, _: &mut Window, _: &mut Context<Self>) {}

    fn reformat(&mut self, _: &Reformat, _: &mut Window, _: &mut Context<Self>) {}

    fn optimize_imports(&mut self, _: &OptimizeImports, _: &mut Window, _: &mut Context<Self>) {}

    fn error_description(&mut self, _: &ErrorDescription, _: &mut Window, _: &mut Context<Self>) {}

    fn restart_servers(&mut self, action: &RestartServers, _: &mut Window, _: &mut Context<Self>) {
        if let Some(services) = &self.services {
            let server = action.server.as_deref().map(code_intel::ServerId::new);
            services.restart(server.as_ref());
        }
    }

    fn stop_server(&mut self, action: &StopServer, _: &mut Window, _: &mut Context<Self>) {
        if let Some(services) = &self.services {
            services.stop(&code_intel::ServerId::new(&action.server));
        }
    }
}

/// Adds handlers to `el` (the workbench root) that send the code
/// intelligence commands to `view`, wherever the focus is in the window.
pub fn forward_actions<E: InteractiveElement>(el: E, view: Entity<CodeIntelView>) -> E {
    macro_rules! forward {
        ($el:expr, $($action:ty => $method:ident),* $(,)?) => {
            $el $(.on_action({
                let view = view.clone();
                move |a: &$action, window, cx| view.update(cx, |v, cx| v.$method(a, window, cx))
            }))*
        };
    }
    forward!(el,
        Complete => complete,
        QuickDoc => quick_doc,
        ParameterInfo => parameter_info,
        GoToDeclaration => go_to_declaration,
        ShowUsages => show_usages,
        ShowIntentions => show_intentions,
        Reformat => reformat,
        OptimizeImports => optimize_imports,
        ErrorDescription => error_description,
        RestartServers => restart_servers,
        StopServer => stop_server,
    )
}

#[cfg(test)]
mod tests {
    use code_intel::fake::{Call, FakeCodeIntel};
    use gpui::TestAppContext;

    use super::*;
    use crate::CodeIntelProvider;

    struct Fake(Arc<FakeCodeIntel>);

    impl CodeIntelProvider for Fake {
        fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
            self.0.clone()
        }
    }

    #[gpui::test]
    fn a_project_gets_its_services(cx: &mut TestAppContext) {
        let fake = FakeCodeIntel::new();
        let hook = CodeIntelHook::new(Fake(fake.clone()));
        let view = cx.new(|cx| CodeIntelView::new(hook, cx));
        view.update(cx, |v, cx| {
            assert!(v.services().is_none());
            v.project_opened(Path::new("/p"), cx);
            assert_eq!(v.project_root(), Some(Path::new("/p")));
            assert!(v.services().is_some());
            assert_eq!(v.state_json(cx)["services"], true);
            assert_eq!(v.servers_json(cx), json!([]));
        });
        // Another project: the previous services shut down.
        view.update(cx, |v, cx| v.project_opened(Path::new("/q"), cx));
        cx.run_until_parked();
        assert_eq!(fake.calls(), [Call::Shutdown]);
    }
}
