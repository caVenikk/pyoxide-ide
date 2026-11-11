//! The SQL language services in the editor (ADR 0022): `sql-intel`'s
//! [`SqlCodeIntel`] registered as the code intelligence of documents with
//! the language id `sql`, over the hub's dialects, scopes and models.
//! Completion, quick documentation, parameter info and the diagnostics all
//! come through it; this module only says what a document's names mean.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use code_intel::{CodeIntel, DocumentId};
use gpui::App;
use sql_intel::{Scope, ScopeProvider, SqlCodeIntel, SqlContext};
use sql_syntax::Dialect;
use workbench::CodeIntelProvider;

use super::LANGUAGE_ID;
use super::highlight::SqlLexer;
use super::hub::Hub;

/// The dialect and scope of a document, from the hub: the dialect and data
/// source its mappings (or its console) give, the data source's model, the
/// mapped schema as the search path. Called on the analysis thread before
/// each analysis, so every answer is a lookup in memory.
pub struct HubScope {
    hub: Arc<Hub>,
    /// The project whose window the services belong to: the home of
    /// documents the hub knows no project of (consoles live outside it).
    root: PathBuf,
}

impl HubScope {
    pub fn new(hub: Arc<Hub>, root: &Path) -> Self {
        Self {
            hub,
            root: root.to_owned(),
        }
    }
}

impl ScopeProvider for HubScope {
    fn context(&self, doc: &DocumentId) -> SqlContext {
        let resolved = self.hub.resolve(doc.path());
        let scope = resolved.data_source.as_deref().and_then(|source| {
            let root = self.hub.root_of(doc.path());
            let model = self
                .hub
                .model(root.as_deref().unwrap_or(&self.root), source)?;
            let scope = Scope::new(model);
            Some(match &resolved.schema {
                Some(schema) => scope.with_search_path([schema.as_str()]),
                None => scope,
            })
        });
        SqlContext::new(resolved.dialect, scope)
    }
}

/// The SQL language services of the database module.
pub struct SqlIntelProvider {
    hub: Arc<Hub>,
}

impl SqlIntelProvider {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self { hub }
    }
}

impl CodeIntelProvider for SqlIntelProvider {
    fn languages(&self) -> &[&'static str] {
        &[LANGUAGE_ID]
    }

    fn start(&self, project_root: &Path, _cx: &mut App) -> Arc<dyn CodeIntel> {
        let intel = Arc::new(SqlCodeIntel::new(Arc::new(HubScope::new(
            self.hub.clone(),
            project_root,
        ))));
        // A binding, a mapping, a refreshed model: the documents are
        // analysed again.
        let weak: Weak<SqlCodeIntel> = Arc::downgrade(&intel);
        self.hub.subscribe(move || match weak.upgrade() {
            Some(intel) => {
                intel.context_changed(None);
                true
            }
            None => false,
        });
        intel
    }

    fn lexer(&self, language: &str) -> Option<Arc<dyn syntax::Lexer>> {
        (language == LANGUAGE_ID).then(|| Arc::new(SqlLexer::new(Dialect::Generic)) as _)
    }
}
