//! What a product supplies: the language services of a project, the icons
//! and names of its servers, and what its languages add to completion.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::{CodeIntel, ServerStatus};
use gpui::App;
use ui::IconName;

use crate::completion::Parentheses;

/// The language services of a product.
pub trait CodeIntelProvider: 'static {
    /// The language services of a project, when it opens. Cheap: servers
    /// start when the first document that needs them opens.
    fn start(&self, project_root: &Path, cx: &mut App) -> Arc<dyn CodeIntel>;

    /// The product's icon of a server (the Language Services widget and
    /// popup), from the icon key of its status; `None` shows the generic
    /// one.
    fn server_icon(&self, server: &ServerStatus) -> Option<IconName> {
        let _ = server;
        None
    }

    /// The server's icon in the status bar, drawn in the status bar's
    /// colours (the reference colourizes it); its popup icon by default.
    fn server_status_bar_icon(&self, server: &ServerStatus) -> Option<IconName> {
        self.server_icon(server)
    }

    /// The name of the tool a server belongs to, as the widget's tooltip
    /// and the notices name it (`Ruff`, while the server calls itself
    /// `ruff`); the server's name by default.
    fn server_title(&self, server: &ServerStatus) -> String {
        server.name.clone()
    }

    /// The file that holds the settings of the project's language services
    /// (a click on a server in the Language Services popup opens it, until
    /// there is a settings window).
    fn settings_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".oxide").join("project.toml"))
    }

    /// Whether completing a function or method of the language adds
    /// parentheses, and how (Python: `self` and `cls` are not passed);
    /// `None` adds none.
    fn parentheses(&self, language: &str) -> Option<Parentheses> {
        let _ = language;
        None
    }
}

/// The product's [`CodeIntelProvider`] in a `workbench::Product`.
#[derive(Clone)]
pub struct CodeIntelHook(pub Rc<dyn CodeIntelProvider>);

impl CodeIntelHook {
    pub fn new(provider: impl CodeIntelProvider) -> Self {
        Self(Rc::new(provider))
    }
}

impl std::fmt::Debug for CodeIntelHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeIntelHook")
    }
}
