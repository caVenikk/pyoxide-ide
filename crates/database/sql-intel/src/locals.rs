//! Objects a script creates: a script is its own scope (ADR 0022), so a
//! table created by an earlier statement resolves without a model.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use sql_syntax::Dialect;

use crate::names;
use crate::reference::Columns;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LocalKind {
    /// A table or a view: something `FROM` can name.
    Relation,
    Routine,
    Schema,
    /// An index, sequence, type, trigger, package...
    Other,
}

/// An object created by a statement: catalog names, no ranges (the
/// statements that use it are cached apart from the one that creates it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalObject {
    pub kind: LocalKind,
    pub schema: Option<String>,
    pub name: String,
    pub columns: Columns,
}

impl LocalObject {
    pub fn hash_into<H: Hasher>(&self, h: &mut H) {
        self.kind.hash(h);
        self.schema.hash(h);
        self.name.hash(h);
        self.columns.hash_into(h);
    }
}

/// The objects created by the statements before the one being analysed.
#[derive(Debug, Default)]
pub(crate) struct LocalIndex {
    objects: Vec<LocalObject>,
    by_name: HashMap<String, Vec<usize>>,
}

impl LocalIndex {
    pub fn push(&mut self, dialect: Dialect, object: LocalObject) {
        let key = names::key(dialect, &object.name);
        self.by_name
            .entry(key)
            .or_default()
            .push(self.objects.len());
        self.objects.push(object);
    }

    /// The latest object named `name` of `kind`; with `schema`, only one
    /// created in that schema or without a schema.
    pub fn find(
        &self,
        dialect: Dialect,
        schema: Option<&str>,
        name: &str,
        kind: LocalKind,
    ) -> Option<&LocalObject> {
        let found = self.by_name.get(&names::key(dialect, name))?;
        found.iter().rev().map(|&i| &self.objects[i]).find(|o| {
            o.kind == kind
                && names::same(dialect, &o.name, name)
                && match (schema, &o.schema) {
                    (Some(s), Some(os)) => names::same(dialect, s, os),
                    _ => true,
                }
        })
    }

    /// A schema created by the script.
    pub fn has_schema(&self, dialect: Dialect, name: &str) -> bool {
        self.find(dialect, None, name, LocalKind::Schema).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LocalObject> {
        self.objects.iter()
    }
}
