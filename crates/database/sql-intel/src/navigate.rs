//! Navigation inside a document and to the database (ADR 0022): what the
//! name under the caret stands for, where it is declared, where it is used,
//! and the edits that rename it. The symbols are the ones a statement or a
//! script defines (aliases, CTEs, select-list aliases, columns of those,
//! objects the script creates, PL/SQL and PL/pgSQL variables and
//! parameters) and the objects of the model, which have usages but no
//! declaration in the file.

use std::ops::Range;

use db_model::{Model, NodeId, NodeKind};
use sql_syntax::{Dialect, Script, SyntaxKind as K, SyntaxNode, SyntaxToken, TokenKind};

use crate::analysis::DocumentAnalysis;
use crate::context::SqlContext;
use crate::hover::word_at;
use crate::names::{self, parts};
use crate::reference::{Reference, Resolution, Target};

/// The scheme of the object targets this crate gives out.
pub const DB_SCHEME: &str = "db";

/// The id of an object target: the node and the data source it is in.
pub fn object_id(node: NodeId, data_source: &str) -> String {
    format!("{node}:{data_source}")
}

/// The node and the data source of an object target's id.
pub fn parse_object_id(id: &str) -> Option<(NodeId, String)> {
    let (node, data_source) = id.split_once(':')?;
    Some((node.parse().ok()?, data_source.to_string()))
}

/// What a name stands for, in a way that two names of one thing compare
/// equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Symbol {
    /// An alias, a CTE, a select-list alias, or the name of a table in
    /// `FROM` that has no alias: the range of the name that defines it.
    Local { def: Range<usize> },
    /// A schema, table, view, routine or column of the model.
    Object(NodeId),
    /// An object a `CREATE` statement of the script defines.
    ScriptObject {
        schema: Option<String>,
        name: String,
    },
    /// A column of a table the script creates.
    ScriptColumn {
        schema: Option<String>,
        table: String,
        name: String,
    },
    /// A column of a CTE or a subquery: its source's defining name.
    DerivedColumn { source: Range<usize>, name: String },
    /// A variable, parameter, cursor or loop variable: its declared name.
    Variable { decl: Range<usize>, name: String },
}

/// Where a name is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    /// A name in the document.
    File(Range<usize>),
    /// An object of the database: the node, the data source, how lists
    /// show it.
    Object {
        id: NodeId,
        data_source: String,
        title: String,
    },
}

/// A place the symbol occurs at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Occurrence {
    pub range: Range<usize>,
    /// The place that declares it.
    pub declaration: bool,
}

/// An object a statement of the script creates.
struct ScriptDef<'a> {
    name: Range<usize>,
    schema: Option<String>,
    object: String,
    /// The `CREATE` statement.
    node: SyntaxNode<'a>,
}

const CREATES: [K; 11] = [
    K::CreateTable,
    K::CreateView,
    K::CreateMaterializedView,
    K::CreateFunction,
    K::CreateProcedure,
    K::CreateTrigger,
    K::CreatePackage,
    K::CreatePackageBody,
    K::CreateType,
    K::CreateSequence,
    K::CreateSchema,
];

const NAMES: [K; 5] = [
    K::TableRef,
    K::FunctionRef,
    K::ObjectRef,
    K::SchemaRef,
    K::DefName,
];

pub(crate) struct Nav<'a> {
    pub script: &'a Script,
    pub ctx: &'a SqlContext,
    pub analysis: &'a DocumentAnalysis,
}

impl<'a> Nav<'a> {
    fn dialect(&self) -> Dialect {
        self.ctx.dialect
    }

    fn model(&self) -> Option<&'a Model> {
        self.ctx.scope.as_ref().map(|s| &*s.model)
    }

    fn same(&self, a: &str, b: &str) -> bool {
        names::same(self.dialect(), a, b)
    }

    /// The token that contains `offset`, also inside the program embedded
    /// in a string (the script's own lookup stops at the string).
    fn token_at(&self, offset: usize) -> Option<SyntaxToken<'a>> {
        let root = self.script.root_at(offset);
        let root = root.fragment_at(offset).map_or(root, |f| f.root);
        root.tokens().find(|t| t.range().contains(&offset))
    }

    // ---- the symbol under the caret -------------------------------------------

    /// The symbols of the name at `offset` (more than one for an ambiguous
    /// column) and the range of the name.
    pub fn symbols_at(&self, offset: usize) -> Option<(Vec<Symbol>, Range<usize>)> {
        let token = word_at(self.script, offset)?;
        let range = token.range();
        let reference = self
            .analysis
            .reference_at(range.start)
            .filter(|r| r.range == range);
        if let Some(r) = &reference {
            let targets: Vec<&Target> = match &r.resolution {
                Resolution::Resolved(t) => vec![t],
                Resolution::Ambiguous(ts) => ts.iter().collect(),
                _ => Vec::new(),
            };
            let symbols: Vec<Symbol> = targets
                .into_iter()
                .filter_map(|t| self.symbol_of(t, r))
                .collect();
            if !symbols.is_empty() {
                return Some((symbols, range));
            }
        }
        let resolved = reference
            .as_ref()
            .is_some_and(|r| r.resolution.is_resolved());
        let symbol = self
            .declared_symbol(&token)
            .or_else(|| (!resolved).then(|| self.variable_used(&token)).flatten())?;
        Some((vec![symbol], range))
    }

    fn symbol_of(&self, target: &Target, r: &Reference) -> Option<Symbol> {
        match target {
            Target::Local(def) => Some(Symbol::Local { def: def.clone() }),
            Target::Object(id) => Some(Symbol::Object(*id)),
            Target::ScriptObject { schema, name } => Some(Symbol::ScriptObject {
                schema: schema.clone(),
                name: name.clone(),
            }),
            Target::SourceColumn { source, column, .. } => match column {
                Some(id) => Some(Symbol::Object(*id)),
                None => Some(self.derived_column(source, &self.dialect().fold(&r.written))),
            },
            Target::Builtin => None,
        }
    }

    /// A column of the source defined at `source`, traced to where it is
    /// declared: a column of a table the script creates, of a CTE, or of a
    /// subquery.
    fn derived_column(&self, source: &Range<usize>, name: &str) -> Symbol {
        let mut source = source.clone();
        // An alias of a CTE or a script table is the column of that.
        for _ in 0..8 {
            let Some(table) = self.source_table(&source) else {
                break;
            };
            match table {
                Target::ScriptObject { schema, name: t } => {
                    return Symbol::ScriptColumn {
                        schema,
                        table: t,
                        name: name.to_string(),
                    };
                }
                Target::Local(def) if def != source => source = def,
                _ => break,
            }
        }
        Symbol::DerivedColumn {
            source,
            name: name.to_string(),
        }
    }

    /// What the table of a source in `FROM` resolves to: `source` is the
    /// alias that defines the source, or the table name when it has none.
    fn source_table(&self, source: &Range<usize>) -> Option<Target> {
        let token = self.token_at(source.start)?;
        let parent = token.parent()?;
        let table_name = match parent.kind() {
            K::AliasDef => parent.parent()?.child(K::TableRef)?,
            K::TableRef => parent,
            _ => return None,
        };
        let last = parts(table_name, self.dialect()).pop()?;
        match self.analysis.reference_at(last.range.start)?.resolution {
            Resolution::Resolved(t @ (Target::ScriptObject { .. } | Target::Local(_))) => Some(t),
            _ => None,
        }
    }

    /// The symbol a name declares (an alias, a CTE, a variable, an object
    /// the script creates).
    fn declared_symbol(&self, token: &SyntaxToken<'a>) -> Option<Symbol> {
        let range = token.range();
        let parent = token.parent()?;
        let dialect = self.dialect();
        let last_part = |node: SyntaxNode<'_>| parts(node, dialect).pop().filter(|p| !p.broken);
        match parent.kind() {
            K::AliasDef | K::CteDef => {
                let p = last_part(parent)?;
                (p.range == range).then_some(Symbol::Local { def: range })
            }
            K::DefName => {
                let p = last_part(parent)?;
                if p.range != range {
                    return None;
                }
                let owner = parent.parent()?;
                match owner.kind() {
                    K::VarDecl
                    | K::CursorDecl
                    | K::ExceptionDecl
                    | K::RoutineParam
                    | K::ForStmt
                    | K::ForallStmt => Some(Symbol::Variable {
                        decl: range,
                        name: p.name,
                    }),
                    K::ColumnDef => {
                        let table = owner.parent().filter(|t| t.kind() == K::CreateTable)?;
                        let def = self.script_def_of(table)?;
                        Some(Symbol::ScriptColumn {
                            schema: def.schema,
                            table: def.object,
                            name: p.name,
                        })
                    }
                    k if CREATES.contains(&k) => self.created_symbol(owner, parent),
                    _ => None,
                }
            }
            k if NAMES.contains(&k) => {
                let owner = parent.parent()?;
                if !CREATES.contains(&owner.kind()) {
                    return None;
                }
                let p = last_part(parent)?;
                if p.range != range {
                    return None;
                }
                self.created_symbol(owner, parent)
            }
            _ => None,
        }
    }

    /// The object a `CREATE` statement defines, when `name` is its name.
    fn created_symbol(&self, create: SyntaxNode<'a>, name: SyntaxNode<'a>) -> Option<Symbol> {
        let def = self.script_def_of(create)?;
        let first = parts(name, self.dialect()).pop()?;
        (first.range == def.name).then_some(Symbol::ScriptObject {
            schema: def.schema,
            name: def.object,
        })
    }

    /// A name used as a variable: the declaration the enclosing scopes give
    /// it.
    fn variable_used(&self, token: &SyntaxToken<'a>) -> Option<Symbol> {
        let node = token.parent()?;
        if node.kind() != K::ColumnRef || parts(node, self.dialect()).len() != 1 {
            return None;
        }
        let name = self.dialect().fold(token.text());
        let decl = self.declaration_for(node, &name)?;
        Some(Symbol::Variable { decl, name })
    }

    // ---- scopes of variables --------------------------------------------------

    /// The enclosing nodes of `node`, nearest first; out of the program in
    /// a string into the statement that holds the string.
    fn scopes(&self, node: SyntaxNode<'a>) -> Vec<SyntaxNode<'a>> {
        let mut out = Vec::new();
        let mut cur = node;
        loop {
            out.extend(cur.ancestors());
            let root = out.last().copied().unwrap_or(cur);
            if root.kind() != K::Root {
                break;
            }
            let start = root.range().start;
            let unit = self.script.root_at(start);
            let host = unit
                .descendants()
                .find(|h| h.fragment().is_some_and(|f| f.root == root));
            match host {
                Some(h) => {
                    out.push(h);
                    cur = h;
                }
                None => break,
            }
        }
        out
    }

    /// The name `scope` declares for its body: `name`'s range.
    fn declared_in(&self, scope: SyntaxNode<'a>, name: &str) -> Option<Range<usize>> {
        // The section lists the declarations; their scope is its block.
        if scope.kind() == K::DeclareSection {
            return None;
        }
        let dialect = self.dialect();
        let mut found = None;
        let mut check = |def: SyntaxNode<'a>| {
            if found.is_none()
                && let Some(p) = parts(def, dialect).pop().filter(|p| !p.broken)
                && names::same(dialect, &p.name, name)
            {
                found = Some(p.range);
            }
        };
        let decl = |item: SyntaxNode<'a>, check: &mut dyn FnMut(SyntaxNode<'a>)| {
            if matches!(item.kind(), K::VarDecl | K::CursorDecl | K::ExceptionDecl)
                && let Some(d) = item.child(K::DefName)
            {
                check(d);
            }
        };
        if matches!(scope.kind(), K::ForStmt | K::ForallStmt) {
            for c in scope.child_nodes().filter(|c| c.kind() == K::DefName) {
                check(c);
            }
        }
        for c in scope.child_nodes() {
            match c.kind() {
                K::DeclareSection => {
                    for item in c.child_nodes() {
                        decl(item, &mut check);
                    }
                }
                K::ParamList => {
                    for p in c.child_nodes().filter(|p| p.kind() == K::RoutineParam) {
                        if let Some(d) = p.child(K::DefName) {
                            check(d);
                        }
                    }
                }
                _ => decl(c, &mut check),
            }
        }
        found
    }

    /// The declaration the nearest scope around `node` gives `name`.
    fn declaration_for(&self, node: SyntaxNode<'a>, name: &str) -> Option<Range<usize>> {
        self.scopes(node)
            .into_iter()
            .find_map(|s| self.declared_in(s, name))
    }

    // ---- what the script creates ----------------------------------------------

    fn script_def_of(&self, create: SyntaxNode<'a>) -> Option<ScriptDef<'a>> {
        let dialect = self.dialect();
        let name = create.child_nodes().find(|n| NAMES.contains(&n.kind()))?;
        let ps = parts(name, dialect);
        if ps.is_empty() || ps.iter().any(|p| p.broken) {
            return None;
        }
        let last = &ps[ps.len() - 1];
        Some(ScriptDef {
            name: last.range.clone(),
            schema: (ps.len() >= 2).then(|| ps[ps.len() - 2].name.clone()),
            object: last.name.clone(),
            node: create,
        })
    }

    fn script_defs(&self) -> Vec<ScriptDef<'a>> {
        self.script
            .statements()
            .filter_map(|root| root.child_nodes().next())
            .filter(|n| CREATES.contains(&n.kind()))
            .filter_map(|n| self.script_def_of(n))
            .collect()
    }

    fn defs_named(&self, schema: Option<&str>, name: &str) -> Vec<ScriptDef<'a>> {
        self.script_defs()
            .into_iter()
            .filter(|d| {
                self.same(&d.object, name)
                    && match (schema, &d.schema) {
                        (Some(a), Some(b)) => self.same(a, b),
                        _ => true,
                    }
            })
            .collect()
    }

    // ---- declarations ---------------------------------------------------------

    /// Where the symbol is declared.
    pub fn declarations(&self, symbol: &Symbol, from: usize) -> Vec<Destination> {
        match symbol {
            Symbol::Local { def } => vec![Destination::File(def.clone())],
            Symbol::Variable { decl, .. } => vec![Destination::File(decl.clone())],
            Symbol::Object(id) => self.object_destination(*id).into_iter().collect(),
            Symbol::ScriptObject { schema, name } => {
                let defs = self.defs_named(schema.as_deref(), name);
                // The statement a use sees: the last one before it.
                let before = defs.iter().rev().find(|d| d.name.start <= from);
                before
                    .or(defs.first())
                    .map(|d| Destination::File(d.name.clone()))
                    .into_iter()
                    .collect()
            }
            Symbol::ScriptColumn {
                schema,
                table,
                name,
            } => self
                .script_column_decl(schema.as_deref(), table, name, from)
                .map(Destination::File)
                .into_iter()
                .collect(),
            Symbol::DerivedColumn { source, name } => {
                let range = self.derived_column_decl(source, name);
                vec![Destination::File(range.unwrap_or_else(|| source.clone()))]
            }
        }
    }

    fn object_destination(&self, id: NodeId) -> Option<Destination> {
        let model = self.model()?;
        let node = model.node(id)?;
        let dialect = self.dialect();
        let title = match node.kind() {
            NodeKind::Schema => dialect.quote(node.name()),
            k if k == NodeKind::Column || k == NodeKind::Argument => {
                let owner = node.parent()?;
                format!(
                    "{}.{}",
                    dialect.quote(owner.name()),
                    dialect.quote(node.name())
                )
            }
            _ => format!(
                "{}.{}",
                dialect.quote(node.schema().name()),
                dialect.quote(node.name())
            ),
        };
        Some(Destination::Object {
            id,
            data_source: model.data_source().as_str().to_string(),
            title,
        })
    }

    fn script_column_decl(
        &self,
        schema: Option<&str>,
        table: &str,
        name: &str,
        from: usize,
    ) -> Option<Range<usize>> {
        let defs: Vec<ScriptDef<'a>> = self
            .defs_named(schema, table)
            .into_iter()
            .filter(|d| d.node.kind() == K::CreateTable)
            .collect();
        let def = defs
            .iter()
            .rev()
            .find(|d| d.name.start <= from)
            .or(defs.first())?;
        self.column_def_name(def.node, name)
    }

    fn column_def_name(&self, create: SyntaxNode<'a>, name: &str) -> Option<Range<usize>> {
        create
            .child_nodes()
            .filter(|c| c.kind() == K::ColumnDef)
            .filter_map(|c| c.child(K::DefName))
            .filter_map(|d| parts(d, self.dialect()).pop())
            .find(|p| !p.broken && self.same(&p.name, name))
            .map(|p| p.range)
    }

    /// The name that declares a column of a CTE or a subquery: its alias,
    /// the column list, or the column the select item names.
    fn derived_column_decl(&self, source: &Range<usize>, name: &str) -> Option<Range<usize>> {
        let token = self.token_at(source.start)?;
        let def = token.parent()?;
        let owner = def.parent()?;
        let (list_owner, query) = match def.kind() {
            K::CteDef => (owner, owner.child_nodes().find(|n| n.kind() == K::Query)),
            K::AliasDef => (
                owner,
                owner
                    .child_nodes()
                    .find(|n| matches!(n.kind(), K::Query | K::ParenQuery)),
            ),
            _ => return None,
        };
        if let Some(list) = list_owner.child(K::ColumnAliasList) {
            return list
                .tokens()
                .filter(|t| matches!(t.kind(), TokenKind::Word | TokenKind::QuotedIdent))
                .find(|t| self.same(&self.dialect().fold(t.text()), name))
                .map(|t| t.range());
        }
        let select = query?.descendants().find(|n| n.kind() == K::Select)?;
        let list = select.child(K::SelectList)?;
        list.child_nodes()
            .filter(|n| n.kind() == K::SelectItem)
            .find_map(|item| {
                let named = |node: SyntaxNode<'_>| parts(node, self.dialect()).pop();
                let p = match item.child(K::AliasDef) {
                    Some(a) => named(a),
                    None => item.child(K::ColumnRef).and_then(named),
                }?;
                (!p.broken && self.same(&p.name, name)).then_some(p.range)
            })
    }

    // ---- occurrences ----------------------------------------------------------

    /// Every place the symbol occurs at in the document, in order.
    pub fn occurrences(&self, symbol: &Symbol, from: usize) -> Vec<Occurrence> {
        let mut out: Vec<Occurrence> = Vec::new();
        let mut add = |range: Range<usize>, declaration: bool| match out
            .iter_mut()
            .find(|o| o.range == range)
        {
            Some(o) => o.declaration |= declaration,
            None => out.push(Occurrence { range, declaration }),
        };
        match symbol {
            Symbol::Local { def } => {
                if self.token_at(def.start).is_some_and(|t| t.range() == *def) {
                    add(def.clone(), self.declares_local(def));
                }
                for r in self.analysis.unit_references(def.start) {
                    if self.targets(&r, |t| matches!(t, Target::Local(d) if d == def)) {
                        add(r.range, false);
                    }
                }
            }
            Symbol::Object(id) => {
                for r in self.analysis.references() {
                    let hit = self.targets(&r, |t| match t {
                        Target::Object(o) => o == id,
                        Target::SourceColumn { column, .. } => *column == Some(*id),
                        _ => false,
                    });
                    if hit {
                        add(r.range, false);
                    }
                }
            }
            Symbol::ScriptObject { schema, name } => {
                for d in self.defs_named(schema.as_deref(), name) {
                    add(d.name, true);
                }
                for r in self.analysis.references() {
                    let hit = self.targets(&r, |t| match t {
                        Target::ScriptObject { schema: s, name: n } => {
                            self.same(n, name)
                                && match (s, schema) {
                                    (Some(a), Some(b)) => self.same(a, b),
                                    _ => true,
                                }
                        }
                        _ => false,
                    });
                    if hit {
                        add(r.range, false);
                    }
                }
            }
            Symbol::ScriptColumn {
                schema,
                table,
                name,
            } => {
                if let Some(d) = self.script_column_decl(schema.as_deref(), table, name, from) {
                    add(d, true);
                }
                for r in self.analysis.references() {
                    if self.column_ref_symbol(&r).as_ref() == Some(symbol) {
                        add(r.range, false);
                    }
                }
            }
            Symbol::DerivedColumn { source, name } => {
                if let Some(d) = self.derived_column_decl(source, name) {
                    add(d, true);
                }
                for r in self.analysis.unit_references(source.start) {
                    if self.column_ref_symbol(&r).as_ref() == Some(symbol) {
                        add(r.range, false);
                    }
                }
            }
            Symbol::Variable { decl, name } => {
                add(decl.clone(), true);
                for range in self.variable_uses(decl, name) {
                    add(range, false);
                }
            }
        }
        out.sort_by_key(|o| o.range.start);
        out
    }

    /// The reference is a column of a source that is not a table of the
    /// model: the symbol of that column.
    fn column_ref_symbol(&self, r: &Reference) -> Option<Symbol> {
        let Resolution::Resolved(Target::SourceColumn {
            source,
            column: None,
            ..
        }) = &r.resolution
        else {
            return None;
        };
        Some(self.derived_column(source, &self.dialect().fold(&r.written)))
    }

    fn targets(&self, r: &Reference, hit: impl Fn(&Target) -> bool) -> bool {
        match &r.resolution {
            Resolution::Resolved(t) => hit(t),
            Resolution::Ambiguous(ts) => ts.iter().any(hit),
            _ => false,
        }
    }

    /// The name at `def` is written where an alias or a CTE is defined.
    fn declares_local(&self, def: &Range<usize>) -> bool {
        self.token_at(def.start)
            .and_then(|t| t.parent())
            .is_some_and(|p| matches!(p.kind(), K::AliasDef | K::CteDef))
    }

    /// The names inside the scope of the declaration that mean it.
    fn variable_uses(&self, decl: &Range<usize>, name: &str) -> Vec<Range<usize>> {
        let Some(node) = self.token_at(decl.start).and_then(|t| t.parent()) else {
            return Vec::new();
        };
        let Some(scope) = self
            .scopes(node)
            .into_iter()
            .find(|s| self.declared_in(*s, name).as_ref() == Some(decl))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.walk(scope, &mut |n| {
            if n.kind() != K::ColumnRef {
                return;
            }
            let ps = parts(n, self.dialect());
            let [p] = ps.as_slice() else {
                return;
            };
            if p.broken || !self.same(&p.name, name) {
                return;
            }
            // A column of a table the statement reads wins.
            if self
                .analysis
                .reference_at(p.range.start)
                .is_some_and(|r| r.range == p.range && r.resolution.is_resolved())
            {
                return;
            }
            if self.declaration_for(n, name).as_ref() == Some(decl) {
                out.push(p.range.clone());
            }
        });
        out
    }

    /// Visits the nodes under `node`, also the programs inside strings.
    fn walk(&self, node: SyntaxNode<'a>, f: &mut dyn FnMut(SyntaxNode<'a>)) {
        for n in node.descendants() {
            f(n);
            if let Some(fragment) = n.fragment() {
                self.walk(fragment.root, f);
            }
        }
    }

    // ---- rename ---------------------------------------------------------------

    /// The text that replaces every occurrence when the symbol is renamed
    /// to `new_name`, or why it cannot be.
    pub fn rename_text(
        &self,
        symbol: &Symbol,
        new_name: &str,
        from: usize,
    ) -> Result<String, String> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err("The name is empty".into());
        }
        let decl = self
            .occurrences(symbol, from)
            .into_iter()
            .find(|o| o.declaration);
        let renamable = match symbol {
            Symbol::Object(_) => {
                return Err(
                    "A database object is renamed in the database, not in the script".into(),
                );
            }
            Symbol::Local { def } => self.declares_local(def),
            Symbol::DerivedColumn { .. } => {
                decl.as_ref().is_some_and(|d| self.is_alias_name(&d.range))
            }
            _ => decl.is_some(),
        };
        if !renamable {
            return Err("There is no declaration to rename in this file".into());
        }
        let quoted = decl.as_ref().is_some_and(|d| {
            self.token_at(d.range.start)
                .is_some_and(|t| t.kind() == TokenKind::QuotedIdent)
        });
        let dialect = self.dialect();
        let plain = !quoted && is_identifier(new_name) && !dialect.is_reserved(new_name);
        Ok(if plain {
            new_name.to_string()
        } else {
            let inner = dialect
                .unquote(new_name)
                .unwrap_or_else(|| new_name.to_string());
            dialect.quote_always(&inner)
        })
    }

    /// The name is written in an alias or in the column list of a CTE.
    fn is_alias_name(&self, range: &Range<usize>) -> bool {
        self.token_at(range.start)
            .and_then(|t| t.parent())
            .is_some_and(|p| matches!(p.kind(), K::AliasDef | K::ColumnAliasList | K::CteDef))
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '#'))
}
