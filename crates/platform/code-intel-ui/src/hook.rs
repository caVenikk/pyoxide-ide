//! What a product or a module supplies: the language services of a
//! project for some languages, the icons and names of its servers, and what
//! its languages add to completion.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::{CodeIntel, Markup, ServerStatus};
use gpui::{App, Task};
use ui::IconName;

use crate::completion::Parentheses;
use crate::file_doc::FileDoc;

/// The language services of a product or a module.
pub trait CodeIntelProvider: 'static {
    /// The language ids (`"python"`) whose documents these services serve
    /// (ADR 0022); none serves every id. With several providers the
    /// documents of a language go to the providers that serve it, and the
    /// first that names the language (else the first that serves every
    /// id) answers for the UI (`parentheses`, `lexer`, `doc_location`).
    fn languages(&self) -> &[&'static str] {
        &[]
    }

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

    /// The lexer that colours code of a language (`"python"`) in
    /// documentation: signatures and fenced code blocks. `None` leaves the
    /// code in the plain text colour.
    fn lexer(&self, language: &str) -> Option<Arc<dyn syntax::Lexer>> {
        let _ = language;
        None
    }

    /// The header of the documentation of a symbol declared in a file: the
    /// module the file is (`json` for `.../json/__init__.pyi`) and its
    /// icon. By default the file's name without its extension.
    fn doc_location(&self, path: &Path) -> Option<DocLocation> {
        crate::doc::default_location(path)
    }

    /// Whether the documentation a service sent for a symbol (`hover`) is
    /// about a whole file, the one at `declaration` (a Python module):
    /// then the header, the file's type and the link to its external
    /// documentation, when the task ends (it may look things up off the UI
    /// thread); the popup adds the file's facts. `None` keeps the
    /// service's documentation as it is.
    fn file_doc(
        &self,
        hover: &Markup,
        declaration: &Path,
        project_root: Option<&Path>,
        cx: &mut App,
    ) -> Option<Task<Option<FileDoc>>> {
        let _ = (hover, declaration, project_root, cx);
        None
    }
}

/// Where a symbol is declared, as the header of its documentation shows
/// it: an icon and a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocLocation {
    pub name: String,
    pub icon: IconName,
}

/// A [`CodeIntelProvider`] in a `workbench::Product` or `workbench::Module`.
#[derive(Clone)]
pub struct CodeIntelHook(pub Rc<dyn CodeIntelProvider>);

impl CodeIntelHook {
    pub fn new(provider: impl CodeIntelProvider) -> Self {
        Self(Rc::new(provider))
    }

    /// The language ids it serves; none: every id.
    pub fn languages(&self) -> &[&'static str] {
        self.0.languages()
    }

    /// Whether documents of a language id go to it.
    pub fn serves(&self, language: &str) -> bool {
        let languages = self.languages();
        languages.is_empty() || languages.contains(&language)
    }
}

impl std::fmt::Debug for CodeIntelHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeIntelHook")
    }
}
