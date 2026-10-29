//! Reference resolution over one statement's tree and the model (ADR 0022):
//! query scopes with their sources (tables, CTEs, subqueries, functions),
//! aliases, `*` expansion for the columns of subqueries and CTEs, the search
//! path, objects created earlier in the script, and the names a dialect
//! knows by itself.

use std::ops::Range;

use db_model::{Model, NameMatch, NodeKind, NodeRef};
use name_match::{Matcher, MatchingMode};
use sql_syntax::{Dialect, SyntaxKind as K, SyntaxNode};

use crate::context::{Scope, SqlContext};
use crate::locals::{LocalIndex, LocalKind, LocalObject};
use crate::names::{self, Part, parts};
use crate::reference::{Col, Columns, RefKind, Reference, Resolution, Suggestion, Target};

/// Relations `FROM` can name.
const FROM_KINDS: [NodeKind; 4] = [
    NodeKind::Table,
    NodeKind::View,
    NodeKind::MaterializedView,
    NodeKind::Synonym,
];

/// How many similar names an unresolved reference offers.
const MAX_SUGGESTIONS: usize = 3;

/// What resolution needs for a whole document: the dialect, the scope with
/// its search path worked out once, and how names are compared.
pub(crate) struct Env<'c> {
    pub dialect: Dialect,
    pub scope: Option<&'c Scope>,
    /// The search path: catalog names and whether the model has each.
    path: Vec<(String, bool)>,
    mode: NameMatch,
    resolves: bool,
}

impl<'c> Env<'c> {
    pub fn new(ctx: &'c SqlContext) -> Self {
        let dialect = ctx.dialect;
        let mode = if dialect == Dialect::Sqlite {
            NameMatch::IgnoreCase
        } else {
            NameMatch::Exact
        };
        let path = ctx.scope.as_ref().map_or_else(Vec::new, |s| {
            s.search_path(dialect)
                .into_iter()
                .map(|name| {
                    let present = s.model.schema(&name, mode).is_some();
                    (name, present)
                })
                .collect()
        });
        Env {
            dialect,
            scope: ctx.scope.as_ref(),
            path,
            mode,
            resolves: ctx.resolves(),
        }
    }

    fn model(&self) -> Option<&'c Model> {
        self.scope.map(|s| &*s.model)
    }
}

/// What the analysis of one statement found besides its references.
#[derive(Debug, Default)]
pub(crate) struct StatementOut {
    pub refs: Vec<Reference>,
    /// The starts of the names of the CTEs that are referenced.
    pub used_ctes: Vec<usize>,
    /// The objects the statement creates.
    pub defs: Vec<LocalObject>,
    /// `INSERT` rows whose number of values differs from the number of
    /// target columns: the range of the row, columns, values.
    pub arity: Vec<(Range<usize>, usize, usize)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Source {
    /// The alias, folded.
    pub alias: Option<String>,
    /// The table or CTE name as written in `FROM`, folded: schema and name.
    pub table: Option<(Option<String>, String)>,
    /// The schema of the model object, exactly.
    pub object_schema: Option<String>,
    /// How the source is qualified in a fix: the alias, else the table name,
    /// as written.
    pub qualifier: String,
    /// The range of the name that defines it (alias, else table name).
    pub def: Range<usize>,
    pub columns: Columns,
    /// A table of the database: it has system columns.
    pub is_table: bool,
}

impl Source {
    /// The source a qualifier (`t`, `s.t`, `a`) names.
    pub fn named_by(&self, dialect: Dialect, q: &[Part]) -> bool {
        let Some(last) = q.last() else {
            return false;
        };
        if let Some(alias) = &self.alias {
            return q.len() == 1 && names::same(dialect, alias, &last.name);
        }
        let Some((schema, name)) = &self.table else {
            return false;
        };
        if !names::same(dialect, name, &last.name) {
            return false;
        }
        match q.len() {
            1 => true,
            2 => {
                let qs = &q[0].name;
                schema.as_ref().is_some_and(|s| names::same(dialect, s, qs))
                    || self
                        .object_schema
                        .as_ref()
                        .is_some_and(|s| names::same(dialect, s, qs))
            }
            _ => false,
        }
    }

    fn column(&self, dialect: Dialect, name: &str) -> Option<&Col> {
        match &self.columns {
            Columns::Known(cols) => cols.iter().find(|c| names::same(dialect, &c.name, name)),
            Columns::Unknown => None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Frame {
    pub sources: Vec<Source>,
    /// Select-list aliases: folded name and range.
    pub aliases: Vec<(String, Range<usize>)>,
    /// Columns merged by `USING` or a natural join.
    pub using: Vec<String>,
    pub natural: bool,
    /// Something in `FROM` hides names (a hole, a syntax error, a pivot).
    pub uncertain: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CteBinding {
    pub name: String,
    pub def: Range<usize>,
    pub columns: Columns,
}

/// What a name at a position could refer to: the sources of the statement
/// around the position and of the statements around that, the CTEs in
/// scope, and the target of a data modification.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    /// The part of the statement the snapshot is valid for.
    pub range: Range<usize>,
    /// Outermost first.
    pub frames: Vec<Frame>,
    pub ctes: Vec<CteBinding>,
    /// The table an `INSERT`, `UPDATE`, `DELETE` or `MERGE` changes.
    pub target: Option<Source>,
}

/// Where select-list aliases are visible to an unqualified name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aliases {
    No,
    /// Before the columns of the sources (`ORDER BY`).
    First,
    /// After them (`GROUP BY`, `HAVING`; SQLite everywhere).
    Last,
}

/// Resolves the references of one statement.
pub(crate) struct Resolver<'c, 'l> {
    env: &'c Env<'c>,
    locals: &'l LocalIndex,
    frames: Vec<Frame>,
    ctes: Vec<CteBinding>,
    aliases: Aliases,
    /// Inside PL/SQL, PL/pgSQL, a trigger or a routine body: a name that is
    /// no column may be a variable or a parameter.
    procedural: u32,
    /// A position completion asks about: the sources visible there are
    /// recorded in `snapshots`.
    probe: Option<usize>,
    pub snapshots: Vec<Snapshot>,
    pub out: StatementOut,
}

impl<'c, 'l> Resolver<'c, 'l> {
    pub fn new(env: &'c Env<'c>, locals: &'l LocalIndex) -> Self {
        Resolver {
            env,
            locals,
            frames: Vec::new(),
            ctes: Vec::new(),
            aliases: Aliases::No,
            procedural: 0,
            probe: None,
            snapshots: Vec::new(),
            out: StatementOut::default(),
        }
    }

    /// A resolver that records what is visible at `offset`.
    pub fn probing(env: &'c Env<'c>, locals: &'l LocalIndex, offset: usize) -> Self {
        let mut r = Self::new(env, locals);
        r.probe = Some(offset);
        r
    }

    /// Records the frames and CTEs in scope when the probe is in `range`.
    fn snapshot(&mut self, range: Range<usize>, target: Option<&Source>) {
        let Some(p) = self.probe else {
            return;
        };
        if range.start <= p && p <= range.end {
            self.snapshots.push(Snapshot {
                range,
                frames: self.frames.clone(),
                ctes: self.ctes.clone(),
                target: target.cloned(),
            });
        }
    }

    /// The smallest snapshot of the statement that is valid for the probe
    /// (the innermost one when equal).
    pub fn best_snapshot(&mut self) -> Option<Snapshot> {
        let mut best: Option<usize> = None;
        for (i, s) in self.snapshots.iter().enumerate() {
            let better = match best {
                None => true,
                Some(b) => s.range.len() <= self.snapshots[b].range.len(),
            };
            if better {
                best = Some(i);
            }
        }
        best.map(|i| self.snapshots.swap_remove(i))
    }

    fn dialect(&self) -> Dialect {
        self.env.dialect
    }

    /// Walks the statement of a parse unit and returns what is visible at
    /// the probe (see [`Resolver::probing`]).
    pub fn visible_at_probe(mut self, root: SyntaxNode<'_>) -> Option<Snapshot> {
        for node in root.child_nodes() {
            self.statement(node);
        }
        self.best_snapshot()
    }

    /// Resolves the statement of a parse unit (its root).
    pub fn unit(mut self, root: SyntaxNode<'_>) -> StatementOut {
        for node in root.child_nodes() {
            self.statement(node);
        }
        if !self.env.resolves {
            // Without a scope or a dialect nothing is reported (ADR 0022).
            for r in &mut self.out.refs {
                if matches!(
                    r.resolution,
                    Resolution::Unresolved | Resolution::Ambiguous(_)
                ) {
                    r.resolution = Resolution::Unknown;
                    r.suggestions.clear();
                }
            }
        }
        self.out
    }

    fn push_ref(
        &mut self,
        part: &Part,
        kind: RefKind,
        resolution: Resolution,
        suggestions: Vec<Suggestion>,
    ) {
        self.out.refs.push(Reference {
            range: part.range.clone(),
            kind,
            written: part.written.clone(),
            resolution,
            suggestions,
        });
    }

    // ---- statements ----------------------------------------------------------

    fn statement(&mut self, node: SyntaxNode<'_>) {
        match node.kind() {
            K::Query | K::Select | K::SetOperation | K::ParenQuery | K::Values | K::TableQuery => {
                self.query(node);
            }
            K::Insert => self.insert(node),
            K::Update => self.update(node),
            K::Delete => self.delete(node),
            K::Merge => self.merge(node),
            K::CreateTable => self.create_table(node),
            K::CreateView | K::CreateMaterializedView => self.create_view(node),
            K::CreateIndex => self.create_index(node),
            K::AlterTable => self.alter_table(node),
            K::DropStatement => self.drop(node),
            K::Truncate => {
                for t in node.child_nodes().filter(|n| n.kind() == K::TableRef) {
                    self.table_ref(t);
                }
            }
            K::CreateFunction | K::CreateProcedure => {
                self.define_first(node, LocalKind::Routine, Columns::Unknown);
                self.procedural(node, true);
            }
            K::CreateTrigger => {
                self.define_first(node, LocalKind::Other, Columns::Unknown);
                for t in node.child_nodes().filter(|n| n.kind() == K::TableRef) {
                    self.table_ref(t);
                }
                self.procedural(node, true);
            }
            K::CreatePackage | K::CreatePackageBody | K::CreateType | K::CreateSequence => {
                self.define_first(node, LocalKind::Other, Columns::Unknown);
                self.procedural(node, true);
            }
            K::CreateSchema => self.define_first(node, LocalKind::Schema, Columns::Unknown),
            K::Block | K::DoStatement => self.procedural(node, false),
            K::Error
            | K::Opaque
            | K::Missing
            | K::Hole
            | K::CommentStatement
            | K::Pragma
            | K::AlterObject => {}
            _ => self.expr(node),
        }
    }

    /// Walks procedural code; `skip_def` leaves the name a `CREATE` defines.
    fn procedural(&mut self, node: SyntaxNode<'_>, skip_def: bool) {
        self.procedural += 1;
        let frames = std::mem::take(&mut self.frames);
        let mut skipped = !skip_def;
        for c in node.child_nodes() {
            if !skipped && matches!(c.kind(), K::TableRef | K::FunctionRef | K::ObjectRef) {
                skipped = true;
                continue;
            }
            if c.kind() == K::TableRef {
                // `ON table` of a trigger: resolved by the caller.
                continue;
            }
            self.expr(c);
        }
        self.frames = frames;
        self.procedural -= 1;
    }

    /// Records the object a `CREATE` statement defines with its first name.
    fn define_first(&mut self, node: SyntaxNode<'_>, kind: LocalKind, columns: Columns) {
        let Some(name) = node.child_nodes().find(|n| {
            matches!(
                n.kind(),
                K::TableRef | K::FunctionRef | K::ObjectRef | K::SchemaRef | K::DefName
            )
        }) else {
            return;
        };
        let ps = parts(name, self.dialect());
        if ps.iter().any(|p| p.broken) || ps.is_empty() {
            return;
        }
        let last = ps[ps.len() - 1].name.clone();
        let schema = (ps.len() >= 2).then(|| ps[ps.len() - 2].name.clone());
        self.out.defs.push(LocalObject {
            kind,
            schema,
            name: last,
            columns,
        });
    }

    fn create_table(&mut self, node: SyntaxNode<'_>) {
        let mut columns: Vec<Col> = Vec::new();
        let mut any_def = false;
        let mut query_cols = None;
        for c in node.child_nodes() {
            match c.kind() {
                K::ColumnDef => {
                    any_def = true;
                    if let Some(d) = c.child(K::DefName) {
                        let ps = parts(d, self.dialect());
                        if let Some(p) = ps.last().filter(|p| !p.broken) {
                            columns.push(Col {
                                name: p.name.clone(),
                                id: None,
                            });
                        }
                    }
                }
                K::Query => query_cols = Some(self.query(c)),
                _ => {}
            }
        }
        for r in node
            .descendants()
            .filter(|n| n.kind() == K::ReferencesClause)
        {
            self.references_clause(r);
        }
        let cols = match query_cols {
            Some(q) => q,
            None if any_def => Columns::Known(columns),
            None => Columns::Unknown,
        };
        self.define_first(node, LocalKind::Relation, cols);
    }

    fn references_clause(&mut self, node: SyntaxNode<'_>) {
        let Some(t) = node.child(K::TableRef) else {
            return;
        };
        let source = self.table_ref(t);
        if let Some(list) = node.child(K::ColumnList) {
            for c in list.child_nodes().filter(|n| n.kind() == K::ColumnRef) {
                self.target_column(c, source.as_ref());
            }
        }
    }

    fn create_view(&mut self, node: SyntaxNode<'_>) {
        let mut cols = Columns::Unknown;
        if let Some(q) = node.child(K::Query) {
            cols = self.query(q);
        }
        if let Some(list) = node.child(K::ColumnList) {
            // The view's own column names: definitions, not references.
            let names: Vec<Col> = list
                .child_nodes()
                .filter(|n| n.kind() == K::ColumnRef)
                .filter_map(|n| parts(n, self.dialect()).pop())
                .map(|p| Col {
                    name: p.name,
                    id: None,
                })
                .collect();
            cols = Columns::Known(names);
        }
        self.define_first(node, LocalKind::Relation, cols);
    }

    fn create_index(&mut self, node: SyntaxNode<'_>) {
        self.define_first(node, LocalKind::Other, Columns::Unknown);
        let Some(t) = node.child(K::TableRef) else {
            return;
        };
        let source = self.table_ref(t);
        let mut frame = Frame::default();
        match source {
            Some(s) => frame.sources.push(s),
            None => frame.uncertain = true,
        }
        self.frames.push(frame);
        for c in node.child_nodes() {
            if matches!(c.kind(), K::IndexElement | K::WhereClause) {
                self.expr(c);
            }
        }
        self.frames.pop();
    }

    fn alter_table(&mut self, node: SyntaxNode<'_>) {
        if let Some(t) = node.child(K::TableRef) {
            self.table_ref(t);
        }
        for r in node
            .descendants()
            .filter(|n| n.kind() == K::ReferencesClause)
        {
            self.references_clause(r);
        }
    }

    fn drop(&mut self, node: SyntaxNode<'_>) {
        let if_exists = node
            .child_tokens()
            .any(|t| t.text().eq_ignore_ascii_case("exists"));
        if if_exists {
            return;
        }
        for t in node.child_nodes().filter(|n| n.kind() == K::TableRef) {
            self.table_ref(t);
        }
    }

    // ---- queries -------------------------------------------------------------

    /// A query or query body; returns its output columns.
    fn query(&mut self, node: SyntaxNode<'_>) -> Columns {
        match node.kind() {
            K::Query => {
                let mark = self.ctes.len();
                let mut out = Columns::Unknown;
                let mut body_frame: Option<Frame> = None;
                for c in node.child_nodes() {
                    match c.kind() {
                        K::WithClause => self.with_clause(c),
                        K::Select
                        | K::SetOperation
                        | K::ParenQuery
                        | K::Values
                        | K::TableQuery
                        | K::Query => {
                            let (cols, frame) = self.body(c);
                            out = cols;
                            body_frame = frame;
                        }
                        K::OrderByClause => {
                            let mut frame = body_frame.clone().unwrap_or_default();
                            if body_frame.is_none() {
                                // After a set operation: the output columns.
                                frame.aliases = output_aliases(&out);
                                frame.uncertain = matches!(out, Columns::Unknown);
                            }
                            self.frames.push(frame);
                            self.snapshot(c.range(), None);
                            let saved = std::mem::replace(&mut self.aliases, Aliases::First);
                            self.expr(c);
                            self.aliases = saved;
                            self.frames.pop();
                        }
                        _ => self.expr(c),
                    }
                }
                self.ctes.truncate(mark);
                out
            }
            _ => self.body(node).0,
        }
    }

    /// A query body: its output columns and, for a `SELECT`, its frame (for
    /// the `ORDER BY` of the query around it).
    fn body(&mut self, node: SyntaxNode<'_>) -> (Columns, Option<Frame>) {
        match node.kind() {
            K::Select => {
                let (cols, frame) = self.select(node);
                (cols, Some(frame))
            }
            K::SetOperation => {
                let mut first = None;
                for c in node.child_nodes() {
                    let cols = self.query(c);
                    if first.is_none() {
                        first = Some(cols);
                    }
                }
                (first.unwrap_or(Columns::Unknown), None)
            }
            K::ParenQuery => {
                let mut out = Columns::Unknown;
                for c in node.child_nodes() {
                    out = self.query(c);
                }
                (out, None)
            }
            K::TableQuery => {
                let cols = node
                    .child(K::TableRef)
                    .and_then(|t| self.table_ref(t))
                    .map_or(Columns::Unknown, |s| s.columns);
                (cols, None)
            }
            K::Query => (self.query(node), None),
            _ => {
                // VALUES: positional columns.
                self.expr(node);
                (Columns::Unknown, None)
            }
        }
    }

    fn with_clause(&mut self, node: SyntaxNode<'_>) {
        for cte in node.child_nodes().filter(|n| n.kind() == K::Cte) {
            let Some(def) = cte.child(K::CteDef) else {
                continue;
            };
            let ps = parts(def, self.dialect());
            let Some(name) = ps.last().filter(|p| !p.broken).cloned() else {
                continue;
            };
            // Visible in its own query (a recursive CTE), columns unknown there.
            let index = self.ctes.len();
            self.ctes.push(CteBinding {
                name: name.name.clone(),
                def: name.range.clone(),
                columns: Columns::Unknown,
            });
            let mut cols = Columns::Unknown;
            for c in cte.child_nodes() {
                if c.kind() == K::Query {
                    cols = self.query(c);
                }
            }
            if let Some(list) = cte.child(K::ColumnAliasList) {
                cols = Columns::Known(alias_list(list, self.dialect()));
            }
            self.ctes[index].columns = cols;
        }
    }

    fn select(&mut self, node: SyntaxNode<'_>) -> (Columns, Frame) {
        let mut frame = Frame {
            // A hole in clause position may be a join or a whole FROM.
            uncertain: node.child(K::Hole).is_some(),
            ..Frame::default()
        };
        let from = node.child(K::FromClause);
        if let Some(from) = from {
            if names::has_error(from) || names::has_hole(from) {
                frame.uncertain = true;
            }
            for item in from.child_nodes() {
                self.add_source(item, &mut frame);
            }
        }
        if let Some(list) = node.child(K::SelectList) {
            for item in list.child_nodes().filter(|n| n.kind() == K::SelectItem) {
                if let Some(a) = item.child(K::AliasDef)
                    && let Some(p) = parts(a, self.dialect()).pop()
                    && !p.broken
                {
                    frame.aliases.push((p.name, p.range));
                }
            }
        }
        self.frames.push(frame);
        self.snapshot(node.range(), None);
        if let Some(from) = from {
            self.join_conditions(from);
        }
        let mut output = Columns::Known(Vec::new());
        for c in node.child_nodes() {
            match c.kind() {
                K::FromClause => {}
                K::SelectList => output = self.select_list(c),
                K::IntoClause => {}
                K::GroupByClause | K::HavingClause => {
                    let saved = std::mem::replace(&mut self.aliases, Aliases::Last);
                    self.expr(c);
                    self.aliases = saved;
                }
                _ => {
                    let mode = if self.dialect() == Dialect::Sqlite {
                        Aliases::Last
                    } else {
                        Aliases::No
                    };
                    let saved = std::mem::replace(&mut self.aliases, mode);
                    self.expr(c);
                    self.aliases = saved;
                }
            }
        }
        let frame = self.frames.pop().unwrap_or_default();
        (output, frame)
    }

    /// Resolves the select list and returns its columns, `*` expanded.
    fn select_list(&mut self, list: SyntaxNode<'_>) -> Columns {
        let dialect = self.dialect();
        let mut cols: Vec<Col> = Vec::new();
        let mut known = true;
        for item in list.child_nodes() {
            if item.kind() != K::SelectItem {
                if item.kind() == K::Error || item.kind() == K::Missing {
                    known = false;
                }
                continue;
            }
            let alias = item
                .child(K::AliasDef)
                .and_then(|a| parts(a, dialect).pop());
            let mut expr_name: Option<String> = None;
            for c in item.child_nodes() {
                match c.kind() {
                    K::AliasDef => {}
                    K::Star => match self.star(c) {
                        Some(expanded) => cols.extend(expanded),
                        None => known = false,
                    },
                    K::ColumnRef => {
                        self.column_ref(c);
                        expr_name = parts(c, dialect)
                            .pop()
                            .filter(|p| !p.broken)
                            .map(|p| p.name);
                    }
                    K::CallExpr => {
                        self.expr(c);
                        if dialect == Dialect::Postgres {
                            expr_name = c
                                .child(K::FunctionRef)
                                .and_then(|f| parts(f, dialect).pop())
                                .map(|p| p.name);
                        }
                    }
                    K::Hole => known = false,
                    _ => self.expr(c),
                }
            }
            if item.child(K::Star).is_some() {
                continue;
            }
            let name = match alias {
                Some(a) if !a.broken => a.name,
                Some(_) => {
                    known = false;
                    String::new()
                }
                None => expr_name.unwrap_or_default(),
            };
            cols.push(Col { name, id: None });
        }
        if known {
            Columns::Known(cols)
        } else {
            Columns::Unknown
        }
    }

    /// `*` or `q.*`: resolves the qualifier and expands to the columns, or
    /// `None` when they are not known.
    fn star(&mut self, star: SyntaxNode<'_>) -> Option<Vec<Col>> {
        let frame = self.frames.last()?.clone();
        if let Some(q) = star.child(K::TableRef) {
            let qparts = parts(q, self.dialect());
            if qparts.iter().any(|p| p.broken) {
                return None;
            }
            let found = self
                .frames
                .iter()
                .rev()
                .flat_map(|f| f.sources.iter())
                .find(|s| s.named_by(self.dialect(), &qparts))
                .cloned();
            return match found {
                Some(s) => {
                    let last = qparts.last()?.clone();
                    self.push_ref(
                        &last,
                        RefKind::Table,
                        Resolution::Resolved(Target::Local(s.def.clone())),
                        Vec::new(),
                    );
                    match s.columns {
                        Columns::Known(c) => Some(c),
                        Columns::Unknown => None,
                    }
                }
                None => {
                    self.unresolved_qualifier(&qparts);
                    None
                }
            };
        }
        if frame.uncertain {
            return None;
        }
        let mut out = Vec::new();
        for s in &frame.sources {
            match &s.columns {
                Columns::Known(c) => out.extend(c.iter().cloned()),
                Columns::Unknown => return None,
            }
        }
        Some(out)
    }

    /// Adds a `FROM` item to `frame`.
    fn add_source(&mut self, item: SyntaxNode<'_>, frame: &mut Frame) {
        let dialect = self.dialect();
        match item.kind() {
            K::TableSource => {
                let Some(t) = item.child(K::TableRef) else {
                    frame.uncertain = true;
                    return;
                };
                let alias = item
                    .child(K::AliasDef)
                    .and_then(|a| parts(a, dialect).pop())
                    .filter(|p| !p.broken);
                let mut source = match self.table_ref(t) {
                    Some(s) => s,
                    None => {
                        if alias.is_none() {
                            frame.uncertain = true;
                        }
                        Source {
                            alias: None,
                            table: None,
                            object_schema: None,
                            qualifier: String::new(),
                            def: t.range(),
                            columns: Columns::Unknown,
                            is_table: false,
                        }
                    }
                };
                if let Some(a) = alias {
                    source.alias = Some(a.name.clone());
                    source.qualifier = a.written.clone();
                    source.def = a.range.clone();
                }
                if let Some(list) = item.child(K::ColumnAliasList) {
                    source.columns = rename(&source.columns, alias_list(list, dialect));
                }
                if item
                    .child_nodes()
                    .any(|n| matches!(n.kind(), K::PivotClause | K::UnpivotClause))
                {
                    source.columns = Columns::Unknown;
                    frame.uncertain = true;
                }
                frame.sources.push(source);
            }
            K::SubquerySource => {
                let lateral = item
                    .child_tokens()
                    .any(|t| t.text().eq_ignore_ascii_case("lateral"));
                if lateral {
                    self.frames.push(frame.clone());
                }
                let mut cols = Columns::Unknown;
                for c in item.child_nodes() {
                    if matches!(c.kind(), K::Query | K::ParenQuery) {
                        cols = self.query(c);
                    }
                }
                if lateral {
                    self.frames.pop();
                }
                let alias = item
                    .child(K::AliasDef)
                    .and_then(|a| parts(a, dialect).pop())
                    .filter(|p| !p.broken);
                if let Some(list) = item.child(K::ColumnAliasList) {
                    cols = rename(&cols, alias_list(list, dialect));
                }
                frame.sources.push(Source {
                    alias: alias.as_ref().map(|a| a.name.clone()),
                    table: None,
                    object_schema: None,
                    qualifier: alias
                        .as_ref()
                        .map(|a| a.written.clone())
                        .unwrap_or_default(),
                    def: alias.as_ref().map_or(item.range(), |a| a.range.clone()),
                    columns: cols,
                    is_table: false,
                });
            }
            K::FunctionSource => {
                self.frames.push(frame.clone());
                let mut fname = None;
                for c in item.child_nodes() {
                    if c.kind() == K::CallExpr {
                        fname = c
                            .child(K::FunctionRef)
                            .and_then(|f| parts(f, dialect).pop())
                            .filter(|p| !p.broken);
                        self.expr(c);
                    }
                }
                self.frames.pop();
                let alias = item
                    .child(K::AliasDef)
                    .and_then(|a| parts(a, dialect).pop())
                    .filter(|p| !p.broken);
                let cols = match item.child(K::ColumnAliasList) {
                    Some(list) => Columns::Known(alias_list(list, dialect)),
                    None => Columns::Unknown,
                };
                let named = alias.as_ref().or(fname.as_ref());
                frame.sources.push(Source {
                    alias: named.map(|a| a.name.clone()),
                    table: None,
                    object_schema: None,
                    qualifier: named.map(|a| a.written.clone()).unwrap_or_default(),
                    def: named.map_or(item.range(), |a| a.range.clone()),
                    columns: cols,
                    is_table: false,
                });
            }
            K::Join => {
                if item
                    .child_tokens()
                    .any(|t| t.text().eq_ignore_ascii_case("natural"))
                {
                    frame.natural = true;
                }
                for c in item.child_nodes() {
                    match c.kind() {
                        K::OnClause => {}
                        K::UsingClause => {
                            for col in c.child_nodes().filter(|n| n.kind() == K::ColumnRef) {
                                if let Some(p) = parts(col, dialect).pop() {
                                    frame.using.push(p.name);
                                }
                            }
                        }
                        K::Missing => {}
                        _ => self.add_source(c, frame),
                    }
                }
            }
            K::ParenSource => {
                for c in item.child_nodes() {
                    if c.kind() != K::AliasDef {
                        self.add_source(c, frame);
                    }
                }
            }
            K::Missing => {}
            _ => frame.uncertain = true,
        }
    }

    /// The `ON` conditions of the joins of a `FROM` clause, once all its
    /// sources are known.
    fn join_conditions(&mut self, from: SyntaxNode<'_>) {
        let mut stack: Vec<SyntaxNode<'_>> = from.child_nodes().collect();
        while let Some(n) = stack.pop() {
            match n.kind() {
                K::Join | K::ParenSource => stack.extend(n.child_nodes()),
                K::OnClause => self.expr(n),
                _ => {}
            }
        }
    }

    // ---- data modification ---------------------------------------------------

    /// The target of an `INSERT`, `UPDATE`, `DELETE` or `MERGE` as a frame.
    fn target(&mut self, node: SyntaxNode<'_>, frame: &mut Frame) -> Option<Source> {
        let t = node
            .child_nodes()
            .find(|n| matches!(n.kind(), K::TableSource | K::SubquerySource))?;
        let before = frame.sources.len();
        self.add_source(t, frame);
        frame.sources.get(before).cloned()
    }

    fn with_first(&mut self, node: SyntaxNode<'_>) -> usize {
        let mark = self.ctes.len();
        if let Some(w) = node.child(K::WithClause) {
            self.with_clause(w);
        }
        mark
    }

    fn insert(&mut self, node: SyntaxNode<'_>) {
        let mark = self.with_first(node);
        if node.child(K::InsertInto).is_some() || node.child(K::WhenClause).is_some() {
            self.multi_insert(node);
            self.ctes.truncate(mark);
            return;
        }
        let mut frame = Frame::default();
        let target = self.target(node, &mut frame);
        self.frames.push(frame.clone());
        self.snapshot(node.range(), target.as_ref());
        self.frames.pop();
        let mut listed: Option<usize> = None;
        let mut values_or_query: Option<SyntaxNode<'_>> = None;
        for c in node.child_nodes() {
            match c.kind() {
                K::ColumnList => {
                    let cols: Vec<_> = c
                        .child_nodes()
                        .filter(|n| n.kind() == K::ColumnRef)
                        .collect();
                    listed = (!names::has_error(c)).then_some(cols.len());
                    for col in cols {
                        self.target_column(col, target.as_ref());
                    }
                }
                K::Values => {
                    self.expr(c);
                    values_or_query = Some(c);
                }
                K::Query => {
                    self.query(c);
                    values_or_query = Some(c);
                }
                K::OnConflictClause => {
                    let mut f = frame.clone();
                    if let Some(t) = &target
                        && matches!(self.dialect(), Dialect::Postgres | Dialect::Sqlite)
                    {
                        f.sources.push(Source {
                            alias: Some("excluded".into()),
                            table: None,
                            object_schema: None,
                            qualifier: "excluded".into(),
                            def: t.def.clone(),
                            columns: t.columns.clone(),
                            is_table: false,
                        });
                    }
                    self.frames.push(f);
                    self.conflict(c, target.as_ref());
                    self.frames.pop();
                }
                K::ReturningClause => {
                    self.frames.push(frame.clone());
                    self.expr(c);
                    self.frames.pop();
                }
                _ => {}
            }
        }
        let expected = listed.or_else(|| target.as_ref().and_then(|t| t.columns.len()));
        if let (Some(expected), Some(v)) = (expected, values_or_query) {
            self.check_arity(v, expected);
        }
        self.ctes.truncate(mark);
    }

    /// Rows with a different number of values than `expected` columns.
    fn check_arity(&mut self, source: SyntaxNode<'_>, expected: usize) {
        if names::has_hole(source) {
            return;
        }
        match source.kind() {
            K::Values => {
                for row in source.child_nodes() {
                    if row.kind() != K::TupleExpr || names::has_error(row) {
                        continue;
                    }
                    let got = row.child_nodes().count();
                    if got != expected {
                        self.out.arity.push((row.range(), expected, got));
                    }
                }
            }
            K::Query => {
                let mut body = source.child_nodes().find(|n| {
                    matches!(
                        n.kind(),
                        K::Select | K::SetOperation | K::Values | K::ParenQuery
                    )
                });
                while let Some(b) = body.filter(|b| b.kind() == K::SetOperation) {
                    body = b.child_nodes().next();
                }
                match body {
                    Some(b) if b.kind() == K::Values => self.check_arity(b, expected),
                    Some(b) if b.kind() == K::Select => {
                        let Some(list) = b.child(K::SelectList) else {
                            return;
                        };
                        if names::has_error(list) {
                            return;
                        }
                        let items: Vec<_> = list
                            .child_nodes()
                            .filter(|n| n.kind() == K::SelectItem)
                            .collect();
                        if items.iter().any(|i| i.child(K::Star).is_some()) {
                            return;
                        }
                        if items.len() != expected {
                            self.out.arity.push((list.range(), expected, items.len()));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn conflict(&mut self, node: SyntaxNode<'_>, target: Option<&Source>) {
        for c in node.child_nodes() {
            match c.kind() {
                K::ConflictTarget => {
                    for e in c.child_nodes() {
                        match e.kind() {
                            K::IndexElement => {
                                for col in e.child_nodes() {
                                    if col.kind() == K::ColumnRef {
                                        self.target_column(col, target);
                                    } else {
                                        self.expr(col);
                                    }
                                }
                            }
                            _ => self.expr(e),
                        }
                    }
                }
                K::SetClause => self.set_clause(c, target),
                _ => self.expr(c),
            }
        }
    }

    fn set_clause(&mut self, node: SyntaxNode<'_>, target: Option<&Source>) {
        for a in node.child_nodes() {
            if a.kind() != K::Assignment {
                self.expr(a);
                continue;
            }
            let mut lhs_done = false;
            for c in a.child_nodes() {
                if !lhs_done {
                    lhs_done = true;
                    match c.kind() {
                        K::ColumnRef => self.target_column(c, target),
                        K::TupleExpr | K::ParenExpr => {
                            for col in c.child_nodes() {
                                if col.kind() == K::ColumnRef {
                                    self.target_column(col, target);
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                self.expr(c);
            }
        }
    }

    /// A column of the target of a statement (`INSERT` column list, `SET`):
    /// only the target's columns count.
    fn target_column(&mut self, node: SyntaxNode<'_>, target: Option<&Source>) {
        let ps = parts(node, self.dialect());
        let Some(last) = ps.last() else {
            return;
        };
        if last.broken {
            return;
        }
        let resolution = match target {
            None => Resolution::Unknown,
            Some(t) => match t.column(self.dialect(), &last.name) {
                Some(col) => Resolution::Resolved(Target::SourceColumn {
                    qualifier: t.qualifier.clone(),
                    source: t.def.clone(),
                    column: col.id,
                }),
                None if matches!(t.columns, Columns::Unknown) => Resolution::Unknown,
                None if self.procedural > 0 && ps.len() > 1 => Resolution::Unknown,
                None => Resolution::Unresolved,
            },
        };
        let suggestions = if resolution == Resolution::Unresolved {
            let cands: Vec<&str> = match target.map(|t| &t.columns) {
                Some(Columns::Known(c)) => c.iter().map(|c| c.name.as_str()).collect(),
                _ => Vec::new(),
            };
            similar(self.dialect(), last, cands)
        } else {
            Vec::new()
        };
        let last = last.clone();
        self.push_ref(&last, RefKind::Column, resolution, suggestions);
    }

    fn update(&mut self, node: SyntaxNode<'_>) {
        let mark = self.with_first(node);
        let mut frame = Frame {
            uncertain: node.child(K::Hole).is_some(),
            ..Frame::default()
        };
        let target = self.target(node, &mut frame);
        if let Some(from) = node.child(K::FromClause) {
            if names::has_error(from) || names::has_hole(from) {
                frame.uncertain = true;
            }
            for item in from.child_nodes() {
                self.add_source(item, &mut frame);
            }
        }
        self.frames.push(frame);
        self.snapshot(node.range(), target.as_ref());
        if let Some(from) = node.child(K::FromClause) {
            self.join_conditions(from);
        }
        for c in node.child_nodes() {
            match c.kind() {
                K::SetClause => self.set_clause(c, target.as_ref()),
                K::WhereClause | K::ReturningClause => self.expr(c),
                _ => {}
            }
        }
        self.frames.pop();
        self.ctes.truncate(mark);
    }

    fn delete(&mut self, node: SyntaxNode<'_>) {
        let mark = self.with_first(node);
        let mut frame = Frame {
            uncertain: node.child(K::Hole).is_some(),
            ..Frame::default()
        };
        let target = self.target(node, &mut frame);
        if let Some(using) = node.child(K::UsingSource) {
            if names::has_error(using) || names::has_hole(using) {
                frame.uncertain = true;
            }
            for item in using.child_nodes() {
                self.add_source(item, &mut frame);
            }
        }
        self.frames.push(frame);
        self.snapshot(node.range(), target.as_ref());
        for c in node.child_nodes() {
            match c.kind() {
                K::UsingSource => self.join_conditions(c),
                K::WhereClause | K::ReturningClause => self.expr(c),
                _ => {}
            }
        }
        self.frames.pop();
        self.ctes.truncate(mark);
    }

    fn merge(&mut self, node: SyntaxNode<'_>) {
        let mark = self.with_first(node);
        let mut frame = Frame {
            uncertain: node.child(K::Hole).is_some(),
            ..Frame::default()
        };
        let target = self.target(node, &mut frame);
        if let Some(using) = node.child(K::UsingSource) {
            if names::has_error(using) || names::has_hole(using) {
                frame.uncertain = true;
            }
            for item in using.child_nodes() {
                self.add_source(item, &mut frame);
            }
        }
        self.frames.push(frame);
        self.snapshot(node.range(), target.as_ref());
        for c in node.child_nodes() {
            match c.kind() {
                K::OnClause => self.expr(c),
                K::MergeClause => self.merge_clause(c, target.as_ref()),
                _ => {}
            }
        }
        self.frames.pop();
        self.ctes.truncate(mark);
    }

    fn merge_clause(&mut self, node: SyntaxNode<'_>, target: Option<&Source>) {
        for c in node.child_nodes() {
            match c.kind() {
                K::MergeUpdate => {
                    for u in c.child_nodes() {
                        match u.kind() {
                            K::SetClause => self.set_clause(u, target),
                            _ => self.expr(u),
                        }
                    }
                }
                K::MergeInsert => {
                    for i in c.child_nodes() {
                        match i.kind() {
                            K::ColumnList => {
                                for col in i.child_nodes().filter(|n| n.kind() == K::ColumnRef) {
                                    self.target_column(col, target);
                                }
                            }
                            _ => self.expr(i),
                        }
                    }
                }
                _ => self.expr(c),
            }
        }
    }

    /// Oracle's `INSERT ALL | FIRST`: the query's columns feed the rows.
    fn multi_insert(&mut self, node: SyntaxNode<'_>) {
        let cols = node
            .child_nodes()
            .filter(|n| n.kind() == K::Query)
            .last()
            .map_or(Columns::Unknown, |q| self.query(q));
        let mut frame = Frame::default();
        frame.sources.push(Source {
            alias: None,
            table: None,
            object_schema: None,
            qualifier: String::new(),
            def: node.range(),
            columns: cols,
            is_table: false,
        });
        self.frames.push(frame);
        let mut stack: Vec<SyntaxNode<'_>> = node.child_nodes().collect();
        stack.reverse();
        while let Some(c) = stack.pop() {
            match c.kind() {
                K::InsertInto => {
                    let mut f = Frame::default();
                    let target = self.target(c, &mut f);
                    for i in c.child_nodes() {
                        match i.kind() {
                            K::ColumnList => {
                                for col in i.child_nodes().filter(|n| n.kind() == K::ColumnRef) {
                                    self.target_column(col, target.as_ref());
                                }
                            }
                            K::Values => self.expr(i),
                            _ => {}
                        }
                    }
                }
                K::WhenClause | K::ElseClause => {
                    let mut inner: Vec<SyntaxNode<'_>> = c.child_nodes().collect();
                    inner.reverse();
                    stack.extend(inner);
                }
                K::Query | K::WithClause => {}
                _ => self.expr(c),
            }
        }
        self.frames.pop();
    }

    // ---- expressions ---------------------------------------------------------

    fn expr(&mut self, node: SyntaxNode<'_>) {
        match node.kind() {
            K::ColumnRef => self.column_ref(node),
            K::FunctionRef => self.function_ref(node),
            K::Query | K::Select | K::SetOperation | K::ParenQuery | K::TableQuery => {
                self.query(node);
            }
            K::Insert
            | K::Update
            | K::Delete
            | K::Merge
            | K::CreateTable
            | K::CreateView
            | K::CreateIndex
            | K::Truncate
            | K::DropStatement
            | K::AlterTable => {
                let frames = std::mem::take(&mut self.frames);
                self.statement(node);
                self.frames = frames;
            }
            K::EmbeddedBody => {
                if let Some(f) = node.fragment() {
                    self.procedural += 1;
                    let frames = std::mem::take(&mut self.frames);
                    let ctes = std::mem::take(&mut self.ctes);
                    for s in f.root.child_nodes() {
                        self.statement(s);
                    }
                    self.ctes = ctes;
                    self.frames = frames;
                    self.procedural -= 1;
                }
            }
            K::Block => {
                self.procedural += 1;
                for c in node.child_nodes() {
                    self.expr(c);
                }
                self.procedural -= 1;
            }
            K::Error
            | K::Opaque
            | K::Missing
            | K::Hole
            | K::AliasDef
            | K::DefName
            | K::TypeRef
            | K::CteDef
            | K::WindowDef
            | K::WindowRef
            | K::ObjectRef
            | K::SchemaRef
            | K::CatalogRef
            | K::TableRef
            | K::Literal
            | K::Param
            | K::IntoClause
            | K::Label => {}
            _ => {
                for c in node.child_nodes() {
                    self.expr(c);
                }
            }
        }
    }

    fn column_ref(&mut self, node: SyntaxNode<'_>) {
        let dialect = self.dialect();
        let ps = parts(node, dialect);
        let Some(col) = ps.last().cloned() else {
            return;
        };
        if ps.iter().any(|p| p.broken) {
            if !col.broken {
                self.push_ref(&col, RefKind::Column, Resolution::Unknown, Vec::new());
            }
            return;
        }
        let qualifier = &ps[..ps.len() - 1];
        if !qualifier.is_empty() {
            self.qualified_column(qualifier, &col);
            return;
        }
        let (resolution, suggestions) = self.unqualified_column(&col, node);
        self.push_ref(&col, RefKind::Column, resolution, suggestions);
    }

    fn qualified_column(&mut self, q: &[Part], col: &Part) {
        let dialect = self.dialect();
        let found = self
            .frames
            .iter()
            .rev()
            .flat_map(|f| f.sources.iter())
            .find(|s| s.named_by(dialect, q))
            .cloned();
        let Some(source) = found else {
            let pseudo = dialect == Dialect::Oracle
                && matches!(
                    col.name.to_ascii_lowercase().as_str(),
                    "nextval" | "currval"
                );
            if pseudo {
                self.push_ref(
                    col,
                    RefKind::Column,
                    Resolution::Resolved(Target::Builtin),
                    Vec::new(),
                );
                return;
            }
            if q.len() == 1 {
                self.unresolved_qualifier(q);
            }
            self.push_ref(col, RefKind::Column, Resolution::Unknown, Vec::new());
            return;
        };
        let last = q[q.len() - 1].clone();
        self.push_ref(
            &last,
            RefKind::Table,
            Resolution::Resolved(Target::Local(source.def.clone())),
            Vec::new(),
        );
        let (resolution, suggestions) = match source.column(dialect, &col.name) {
            Some(c) => (
                Resolution::Resolved(Target::SourceColumn {
                    qualifier: source.qualifier.clone(),
                    source: source.def.clone(),
                    column: c.id,
                }),
                Vec::new(),
            ),
            None if source.is_table && names::is_system_column(dialect, &col.name) => {
                (Resolution::Resolved(Target::Builtin), Vec::new())
            }
            None => match &source.columns {
                Columns::Unknown => (Resolution::Unknown, Vec::new()),
                Columns::Known(cols) => {
                    let cands = cols.iter().map(|c| c.name.as_str()).collect();
                    (Resolution::Unresolved, similar(dialect, col, cands))
                }
            },
        };
        self.push_ref(col, RefKind::Column, resolution, suggestions);
    }

    /// A one-part qualifier that names no source of `FROM`.
    fn unresolved_qualifier(&mut self, q: &[Part]) {
        let Some(qual) = q.last().cloned() else {
            return;
        };
        if self.procedural > 0 || self.frames.iter().any(|f| f.uncertain) {
            self.push_ref(&qual, RefKind::Table, Resolution::Unknown, Vec::new());
            return;
        }
        let dialect = self.dialect();
        let mut suggestions = Vec::new();
        // A table that is in FROM under an alias.
        for s in self.frames.iter().rev().flat_map(|f| f.sources.iter()) {
            if s.alias.is_some()
                && let Some((_, t)) = &s.table
                && names::same(dialect, t, &qual.name)
            {
                suggestions.push(Suggestion::UseAlias(s.qualifier.clone()));
            }
        }
        if suggestions.is_empty() {
            let cands: Vec<String> = self
                .frames
                .iter()
                .rev()
                .flat_map(|f| f.sources.iter())
                .filter_map(|s| {
                    s.alias
                        .clone()
                        .or_else(|| s.table.as_ref().map(|t| t.1.clone()))
                })
                .collect();
            suggestions = similar(dialect, &qual, cands.iter().map(String::as_str).collect());
        }
        self.push_ref(&qual, RefKind::Table, Resolution::Unresolved, suggestions);
    }

    fn unqualified_column(
        &self,
        col: &Part,
        node: SyntaxNode<'_>,
    ) -> (Resolution, Vec<Suggestion>) {
        let dialect = self.dialect();
        let name = &col.name;
        let alias_hit = |frame: &Frame| {
            frame
                .aliases
                .iter()
                .find(|(a, _)| names::same(dialect, a, name))
                .map(|(_, r)| Resolution::Resolved(Target::Local(r.clone())))
        };
        for (depth, frame) in self.frames.iter().rev().enumerate() {
            let innermost = depth == 0;
            if innermost
                && self.aliases == Aliases::First
                && let Some(r) = alias_hit(frame)
            {
                return (r, Vec::new());
            }
            let mut matches: Vec<Target> = Vec::new();
            let mut unknown = frame.uncertain;
            for s in &frame.sources {
                match s.column(dialect, name) {
                    Some(c) => matches.push(Target::SourceColumn {
                        qualifier: s.qualifier.clone(),
                        source: s.def.clone(),
                        column: c.id,
                    }),
                    None => {
                        if matches!(s.columns, Columns::Unknown) {
                            unknown = true;
                        }
                    }
                }
            }
            match matches.len() {
                0 => {}
                1 => return (Resolution::Resolved(matches.remove(0)), Vec::new()),
                _ => {
                    let merged =
                        frame.natural || frame.using.iter().any(|u| names::same(dialect, u, name));
                    if merged || unknown {
                        return (Resolution::Resolved(matches.remove(0)), Vec::new());
                    }
                    return (Resolution::Ambiguous(matches), Vec::new());
                }
            }
            // A whole row by its source's name (PostgreSQL).
            if let Some(s) = frame.sources.iter().find(|s| {
                s.alias
                    .as_ref()
                    .or(s.table.as_ref().map(|t| &t.1))
                    .is_some_and(|n| names::same(dialect, n, name))
            }) {
                return (
                    Resolution::Resolved(Target::Local(s.def.clone())),
                    Vec::new(),
                );
            }
            if innermost
                && self.aliases == Aliases::Last
                && let Some(r) = alias_hit(frame)
            {
                return (r, Vec::new());
            }
            if innermost
                && names::is_system_column(dialect, name)
                && frame.sources.iter().any(|s| s.is_table)
            {
                return (Resolution::Resolved(Target::Builtin), Vec::new());
            }
            if unknown {
                return (Resolution::Unknown, Vec::new());
            }
        }
        if names::is_builtin_value(dialect, &col.written) {
            return (Resolution::Resolved(Target::Builtin), Vec::new());
        }
        if self.procedural > 0 || in_trigger_or_routine(node) {
            return (Resolution::Unknown, Vec::new());
        }
        // SQLite reads a double-quoted name that is no column as a string.
        if dialect == Dialect::Sqlite && col.quoted {
            return (Resolution::Unknown, Vec::new());
        }
        let mut cands: Vec<&str> = Vec::new();
        for frame in self.frames.iter().rev() {
            for s in &frame.sources {
                if let Columns::Known(cols) = &s.columns {
                    cands.extend(cols.iter().map(|c| c.name.as_str()));
                }
            }
            cands.extend(frame.aliases.iter().map(|(a, _)| a.as_str()));
        }
        (Resolution::Unresolved, similar(dialect, col, cands))
    }

    fn function_ref(&mut self, node: SyntaxNode<'_>) {
        let dialect = self.dialect();
        let ps = parts(node, dialect);
        let Some(name) = ps.last().cloned() else {
            return;
        };
        if ps.iter().any(|p| p.broken) {
            return;
        }
        if ps.len() == 1 {
            let resolution = if self
                .locals
                .find(dialect, None, &name.name, LocalKind::Routine)
                .is_some()
            {
                Resolution::Resolved(Target::ScriptObject {
                    schema: None,
                    name: name.name.clone(),
                })
            } else {
                match self.lookup_path(&name.name, &NodeKind::ROUTINES) {
                    Some(n) => Resolution::Resolved(Target::Object(n.id())),
                    // Built-in functions are not in the model.
                    None => Resolution::Unknown,
                }
            };
            self.push_ref(&name, RefKind::Routine, resolution, Vec::new());
            return;
        }
        if ps.len() > 2 {
            self.push_ref(&name, RefKind::Routine, Resolution::Unknown, Vec::new());
            return;
        }
        let schema = &ps[0];
        let Some(model) = self.env.model().filter(|_| self.env.resolves) else {
            self.push_ref(&name, RefKind::Routine, Resolution::Unknown, Vec::new());
            return;
        };
        let resolution = match model.schema(&schema.name, self.env.mode) {
            Some(s) => {
                self.push_ref(
                    schema,
                    RefKind::Schema,
                    Resolution::Resolved(Target::Object(s.id())),
                    Vec::new(),
                );
                let kinds = [NodeKind::Function, NodeKind::Procedure, NodeKind::Package];
                match s.find(&name.name, self.env.mode, &kinds) {
                    Some(n) => Resolution::Resolved(Target::Object(n.id())),
                    None if dialect == Dialect::Oracle => Resolution::Unknown,
                    None => Resolution::Unresolved,
                }
            }
            // Oracle: `package.member`; elsewhere a schema outside the model.
            None => Resolution::Unknown,
        };
        let suggestions = if resolution == Resolution::Unresolved && self.probe.is_none() {
            let s = model.schema(&schema.name, self.env.mode);
            let cands: Vec<&str> = s
                .map(|s| {
                    s.nodes()
                        .filter(|n| n.kind().is_routine())
                        .map(|n| n.name())
                        .collect()
                })
                .unwrap_or_default();
            similar(dialect, &name, cands)
        } else {
            Vec::new()
        };
        self.push_ref(&name, RefKind::Routine, resolution, suggestions);
    }

    // ---- tables --------------------------------------------------------------

    /// The first object named `name` of `kinds` in the search path.
    fn lookup_path(&self, name: &str, kinds: &[NodeKind]) -> Option<NodeRef<'c>> {
        let model = self.env.model()?;
        self.env
            .path
            .iter()
            .filter(|(_, present)| *present)
            .find_map(|(schema, _)| model.find(schema, name, self.env.mode, kinds))
    }

    /// Resolves a table name and returns it as a source of `FROM` (without
    /// an alias), or `None` when the name is broken.
    fn table_ref(&mut self, node: SyntaxNode<'_>) -> Option<Source> {
        let dialect = self.dialect();
        let ps = parts(node, dialect);
        let name = ps.last()?.clone();
        if ps.iter().any(|p| p.broken) {
            return None;
        }
        let written_schema = (ps.len() >= 2).then(|| ps[ps.len() - 2].clone());
        let mut source = Source {
            alias: None,
            table: Some((
                written_schema.as_ref().map(|s| s.name.clone()),
                name.name.clone(),
            )),
            object_schema: None,
            qualifier: name.written.clone(),
            def: name.range.clone(),
            columns: Columns::Unknown,
            is_table: false,
        };
        let (resolution, suggestions) = match &written_schema {
            None => self.unqualified_table(&name, &mut source),
            Some(schema) => self.qualified_table(schema, &name, &mut source),
        };
        if let Resolution::Resolved(Target::Local(def)) = &resolution {
            self.out.used_ctes.push(def.start);
        }
        self.push_ref(&name, RefKind::Table, resolution, suggestions);
        Some(source)
    }

    fn unqualified_table(
        &mut self,
        name: &Part,
        source: &mut Source,
    ) -> (Resolution, Vec<Suggestion>) {
        let dialect = self.dialect();
        if let Some(cte) = self
            .ctes
            .iter()
            .rev()
            .find(|c| names::same(dialect, &c.name, &name.name))
        {
            source.columns = cte.columns.clone();
            return (
                Resolution::Resolved(Target::Local(cte.def.clone())),
                Vec::new(),
            );
        }
        if let Some(local) = self
            .locals
            .find(dialect, None, &name.name, LocalKind::Relation)
        {
            source.columns = local.columns.clone();
            source.is_table = true;
            return (
                Resolution::Resolved(Target::ScriptObject {
                    schema: local.schema.clone(),
                    name: local.name.clone(),
                }),
                Vec::new(),
            );
        }
        if !self.env.resolves {
            return (Resolution::Unknown, Vec::new());
        }
        if let Some(n) = self.lookup_path(&name.name, &FROM_KINDS) {
            self.fill(source, n);
            return (Resolution::Resolved(Target::Object(n.id())), Vec::new());
        }
        if names::is_builtin_relation(dialect, &name.name) {
            if dialect == Dialect::Oracle {
                source.columns = Columns::names(["DUMMY"]);
            }
            return (Resolution::Resolved(Target::Builtin), Vec::new());
        }
        let model = self.env.model().expect("resolves implies a scope");
        if dialect == Dialect::Oracle
            && let Some(n) = model.find("PUBLIC", &name.name, self.env.mode, &[NodeKind::Synonym])
        {
            return (Resolution::Resolved(Target::Object(n.id())), Vec::new());
        }
        if names::is_catalog_relation(dialect, &name.name) {
            return (Resolution::Unknown, Vec::new());
        }
        if self.env.path.is_empty() || self.env.path.iter().any(|(_, present)| !present) {
            return (Resolution::Unknown, Vec::new());
        }
        (Resolution::Unresolved, self.table_suggestions(name, None))
    }

    fn qualified_table(
        &mut self,
        schema: &Part,
        name: &Part,
        source: &mut Source,
    ) -> (Resolution, Vec<Suggestion>) {
        let dialect = self.dialect();
        if let Some(local) =
            self.locals
                .find(dialect, Some(&schema.name), &name.name, LocalKind::Relation)
            && local.schema.is_some()
        {
            source.columns = local.columns.clone();
            source.is_table = true;
            return (
                Resolution::Resolved(Target::ScriptObject {
                    schema: local.schema.clone(),
                    name: local.name.clone(),
                }),
                Vec::new(),
            );
        }
        if !self.env.resolves {
            return (Resolution::Unknown, Vec::new());
        }
        let model = self.env.model().expect("resolves implies a scope");
        let Some(s) = model.schema(&schema.name, self.env.mode) else {
            let resolution = if self.env.scope.is_some_and(|s| s.complete)
                && !names::is_catalog_schema(dialect, &schema.name)
                && !self.locals.has_schema(dialect, &schema.name)
            {
                Resolution::Unresolved
            } else {
                Resolution::Unknown
            };
            let suggestions = if resolution == Resolution::Unresolved {
                let cands = model.schemas().map(|s| s.name()).collect();
                similar(dialect, schema, cands)
            } else {
                Vec::new()
            };
            self.push_ref(schema, RefKind::Schema, resolution, suggestions);
            return (Resolution::Unknown, Vec::new());
        };
        self.push_ref(
            schema,
            RefKind::Schema,
            Resolution::Resolved(Target::Object(s.id())),
            Vec::new(),
        );
        match s.find(&name.name, self.env.mode, &FROM_KINDS) {
            Some(n) => {
                self.fill(source, n);
                (Resolution::Resolved(Target::Object(n.id())), Vec::new())
            }
            None if names::is_catalog_schema(dialect, &schema.name) && s.is_empty() => {
                (Resolution::Unknown, Vec::new())
            }
            None => (
                Resolution::Unresolved,
                self.table_suggestions(name, Some(s.name())),
            ),
        }
    }

    /// The source's columns and schema from the model object.
    fn fill(&self, source: &mut Source, node: NodeRef<'_>) {
        source.object_schema = Some(node.schema().name().to_string());
        source.is_table = matches!(node.kind(), NodeKind::Table | NodeKind::MaterializedView);
        source.columns = if node.kind() == NodeKind::Synonym || !node.structure_loaded() {
            Columns::Unknown
        } else {
            Columns::Known(
                node.children()
                    .filter(|c| c.kind() == NodeKind::Column)
                    .map(|c| Col {
                        name: c.name().to_string(),
                        id: Some(c.id()),
                    })
                    .collect(),
            )
        };
    }

    /// Similar relation names for an unresolved table: in `schema`, or in the
    /// search path with the CTEs and the script's objects; and the schemas
    /// outside the path that have the name.
    fn table_suggestions(&self, name: &Part, schema: Option<&str>) -> Vec<Suggestion> {
        if self.probe.is_some() {
            // Completion reads the scope, not the problems.
            return Vec::new();
        }
        let dialect = self.dialect();
        let Some(model) = self.env.model() else {
            return Vec::new();
        };
        let mut cands: Vec<&str> = Vec::new();
        let schemas: Vec<&str> = match schema {
            Some(s) => vec![s],
            None => self.env.path.iter().map(|(s, _)| s.as_str()).collect(),
        };
        for s in &schemas {
            if let Some(sm) = model.schema(s, NameMatch::Exact) {
                cands.extend(
                    sm.nodes()
                        .filter(|n| FROM_KINDS.contains(&n.kind()) && !n.is_partition())
                        .map(|n| n.name()),
                );
            }
        }
        if schema.is_none() {
            cands.extend(self.ctes.iter().map(|c| c.name.as_str()));
            cands.extend(
                self.locals
                    .iter()
                    .filter(|o| o.kind == LocalKind::Relation)
                    .map(|o| o.name.as_str()),
            );
        }
        let mut out = similar(dialect, name, cands);
        if schema.is_none() {
            let mut seen: Vec<&str> = Vec::new();
            for n in model.lookup_everywhere(&name.name, self.env.mode, &FROM_KINDS) {
                let s = n.schema().name();
                if !schemas.contains(&s) && !seen.contains(&s) {
                    seen.push(s);
                    out.push(Suggestion::Qualify(dialect.quote(s)));
                }
            }
        }
        out
    }
}

/// The columns named by a `ColumnAliasList` (`AliasDef` children).
fn alias_list(list: SyntaxNode<'_>, dialect: Dialect) -> Vec<Col> {
    list.child_nodes()
        .filter(|n| matches!(n.kind(), K::AliasDef | K::ColumnRef | K::DefName))
        .filter_map(|n| parts(n, dialect).pop())
        .map(|p| Col {
            name: p.name,
            id: None,
        })
        .collect()
}

/// The columns renamed positionally by an alias list.
fn rename(cols: &Columns, names: Vec<Col>) -> Columns {
    match cols {
        Columns::Known(old) if old.len() > names.len() => {
            let mut out = names;
            out.extend(old[out.len()..].iter().cloned());
            Columns::Known(out)
        }
        _ => Columns::Known(names),
    }
}

/// The output columns of a set operation as aliases (for its `ORDER BY`).
fn output_aliases(cols: &Columns) -> Vec<(String, Range<usize>)> {
    match cols {
        Columns::Known(c) => c
            .iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| (c.name.clone(), 0..0))
            .collect(),
        Columns::Unknown => Vec::new(),
    }
}

/// Inside the body of a trigger or routine, where `NEW`, `OLD` and the
/// parameters are names without a table.
fn in_trigger_or_routine(node: SyntaxNode<'_>) -> bool {
    node.ancestors().any(|a| {
        matches!(
            a.kind(),
            K::CreateTrigger
                | K::CreateFunction
                | K::CreateProcedure
                | K::Block
                | K::CreatePackageBody
        )
    })
}

/// Names close to `name` among `cands`, written for the dialect in the
/// style of `name` (lower case stays lower case where that folds back).
fn similar(dialect: Dialect, name: &Part, cands: Vec<&str>) -> Vec<Suggestion> {
    let written = name.written.as_str();
    let len = written.chars().count();
    let max = (len / 3).clamp(1, 3);
    let matcher = Matcher::new(&name.name, MatchingMode::IgnoreCase);
    let mut scored: Vec<(usize, i32, &str)> = Vec::new();
    for c in cands {
        if c.is_empty() || names::same(dialect, c, &name.name) {
            continue;
        }
        if scored.iter().any(|(_, _, s)| *s == c) {
            continue;
        }
        let d = names::distance(&name.name, c, max);
        let degree = matcher.degree(c);
        let prefix = degree > 0 && len >= 2 && c.chars().count() > len;
        match d {
            Some(d) => scored.push((d, degree, c)),
            None if prefix => scored.push((max + 1, degree, c)),
            None => {}
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, c)| Suggestion::Rename(write_like(dialect, c, name)))
        .collect()
}

/// A catalog name as it is to be written where `like` was written: bare in
/// the case of `like` when that folds back to the name, else quoted.
pub(crate) fn write_like(dialect: Dialect, name: &str, like: &Part) -> String {
    if !like.quoted {
        let lower = name.to_lowercase();
        let upper = name.to_uppercase();
        let typed_lower = !like.written.chars().any(|c| c.is_uppercase());
        let preferred = if typed_lower { &lower } else { &upper };
        for candidate in [preferred, &lower, &upper] {
            let plain = !dialect.is_reserved(candidate)
                && candidate
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                && candidate
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            if plain && dialect.names_equal(&dialect.fold(candidate), name) {
                return candidate.clone();
            }
        }
    }
    dialect.quote(name)
}
