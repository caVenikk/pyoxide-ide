//! What a product supplies: the language services of a project and the
//! icons of its servers.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use code_intel::{CodeIntel, ServerStatus};
use gpui::App;
use ui::IconName;

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
