//! What names in a document mean: its dialect and its resolution scope
//! (ADR 0022), supplied by whoever knows the bindings of files and consoles
//! to data sources (`db-ui`) through [`ScopeProvider`].

use std::sync::Arc;

use code_intel::DocumentId;
use db_model::{Model, NameMatch};
use sql_syntax::Dialect;

/// A resolution scope: a data source's metadata model and the schemas
/// unqualified names are looked up in.
#[derive(Debug, Clone)]
pub struct Scope {
    /// The model of the data source, from the disk cache or the last
    /// refresh. Resolution reads it and never touches the network.
    pub model: Arc<Model>,
    /// The schemas unqualified names resolve through, in order, as catalog
    /// names (PostgreSQL's search path, Oracle's current schema). Empty:
    /// the dialect's default ([`Scope::search_path`]).
    pub search_path: Vec<String>,
    /// The model lists every schema of the data source, so a schema it does
    /// not have does not exist. When false (the usual case: the
    /// introspection scope names some schemas), a name qualified with a
    /// schema the model lacks is unknown, never unresolved.
    pub complete: bool,
}

impl Scope {
    /// A scope over `model` with the dialect's default search path.
    pub fn new(model: Arc<Model>) -> Self {
        Self {
            model,
            search_path: Vec::new(),
            complete: false,
        }
    }

    pub fn with_search_path<S: Into<String>>(mut self, path: impl IntoIterator<Item = S>) -> Self {
        self.search_path = path.into_iter().map(Into::into).collect();
        self
    }

    pub fn complete(mut self, complete: bool) -> Self {
        self.complete = complete;
        self
    }

    /// The schemas unqualified names resolve through: the explicit search
    /// path, else the schemas the introspection marked current, else the
    /// dialect's default (PostgreSQL `public`; SQLite `main`, `temp` and the
    /// attached databases; nothing for Oracle, whose current schema the
    /// introspection marks).
    pub fn search_path(&self, dialect: Dialect) -> Vec<String> {
        if !self.search_path.is_empty() {
            return self.search_path.clone();
        }
        if dialect == Dialect::Sqlite {
            let mut path: Vec<String> = Vec::new();
            for first in ["main", "temp"] {
                if let Some(s) = self.model.schema(first, NameMatch::IgnoreCase) {
                    path.push(s.name().to_string());
                }
            }
            for s in self.model.schemas() {
                if !path.iter().any(|p| p == s.name()) {
                    path.push(s.name().to_string());
                }
            }
            return path;
        }
        let current: Vec<String> = self
            .model
            .schemas()
            .filter(|s| s.info().is_current)
            .map(|s| s.name().to_string())
            .collect();
        if !current.is_empty() {
            return current;
        }
        match dialect {
            Dialect::Postgres => vec!["public".to_string()],
            _ => Vec::new(),
        }
    }
}

/// The case completion writes keywords in (the code style's keyword case).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeywordCase {
    /// As the keywords around are written: lower case after a lower case
    /// word, else upper case.
    #[default]
    Keep,
    Upper,
    Lower,
}

/// What completion does besides offering names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompletionOptions {
    pub keyword_case: KeywordCase,
    /// A table inserted into `FROM` gets an alias made from the initials of
    /// its name.
    pub insert_alias: bool,
}

/// The dialect and scope of one document.
#[derive(Debug, Clone)]
pub struct SqlContext {
    pub dialect: Dialect,
    /// `None`: the document is bound to no data source. It gets syntax
    /// errors and the inspections that need no metadata, never an
    /// unresolved reference.
    pub scope: Option<Scope>,
    pub completion: CompletionOptions,
}

impl SqlContext {
    pub fn new(dialect: Dialect, scope: Option<Scope>) -> Self {
        Self {
            dialect,
            scope,
            completion: CompletionOptions::default(),
        }
    }

    pub fn with_completion(mut self, completion: CompletionOptions) -> Self {
        self.completion = completion;
        self
    }

    /// A document without a scope.
    pub fn unscoped(dialect: Dialect) -> Self {
        Self::new(dialect, None)
    }

    /// Names can be resolved: there is a scope and the dialect is known.
    pub fn resolves(&self) -> bool {
        self.scope.is_some() && self.dialect != Dialect::Generic
    }
}

/// Gives the dialect and scope of a document (ADR 0022: a console's data
/// source and schema, a file's binding, a folder mapping, the project
/// default). Called off the UI thread before each analysis, so answers
/// must be cheap; when they change, tell the services with
/// [`SqlCodeIntel::context_changed`](crate::SqlCodeIntel::context_changed).
pub trait ScopeProvider: Send + Sync + 'static {
    fn context(&self, doc: &DocumentId) -> SqlContext;
}

/// Every document gets the same context (tests, a single bound console).
#[derive(Debug, Clone)]
pub struct FixedScope(pub SqlContext);

impl ScopeProvider for FixedScope {
    fn context(&self, _doc: &DocumentId) -> SqlContext {
        self.0.clone()
    }
}

impl<F> ScopeProvider for F
where
    F: Fn(&DocumentId) -> SqlContext + Send + Sync + 'static,
{
    fn context(&self, doc: &DocumentId) -> SqlContext {
        self(doc)
    }
}
