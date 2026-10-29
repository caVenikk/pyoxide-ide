//! Completion (ADR 0022): what can stand at the caret, from the parser's
//! expected set, the sources of the statement around it, the metadata
//! model and the objects the script creates. The answer is plain data;
//! `SqlCodeIntel` turns it into `code-intel` items.

use std::collections::HashSet;
use std::ops::Range;

use code_intel::SymbolKind;
use db_model::{Group, Model, NameMatch, NodeId, NodeKind, NodeRef};
use name_match::{Matcher, MatchingMode};
use sql_syntax::{
    Dialect, Expected, HOLE, NameKind, Script, SyntaxKind as K, SyntaxNode, SyntaxToken, TokenKind,
};

use crate::builtins::{self, FnKind};
use crate::context::{KeywordCase, SqlContext};
use crate::cursor::{Caret, QualifierPart};
use crate::locals::{LocalIndex, LocalKind, LocalObject};
use crate::names::{self, Part};
use crate::reference::{Col, Columns};
use crate::resolve::{Env, Frame, Resolver, Snapshot, Source};

/// What an item stands for, to document it when it is selected.
#[derive(Debug, Clone)]
pub(crate) enum Resolve {
    Node(NodeId),
    Function(&'static builtins::Function),
    Type(&'static builtins::DataType),
    Keyword(String),
}

/// One completion item, before it becomes a `code-intel` item.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub label: String,
    pub insert: String,
    pub kind: SymbolKind,
    /// Grey text right after the name.
    pub tail: Option<String>,
    /// Grey text at the right edge.
    pub detail: Option<String>,
    pub filter: Option<String>,
    pub sort: String,
    /// The insert text has a `$0` for the caret.
    pub snippet: bool,
    pub resolve: Option<Resolve>,
    pub deprecated: bool,
}

impl Item {
    fn new(label: impl Into<String>, kind: SymbolKind, tier: char) -> Self {
        let label = label.into();
        Self {
            sort: format!("{tier}{}", label.to_lowercase()),
            insert: label.clone(),
            label,
            kind,
            tail: None,
            detail: None,
            filter: None,
            snippet: false,
            resolve: None,
            deprecated: false,
        }
    }

    fn insert(mut self, text: impl Into<String>) -> Self {
        self.insert = text.into();
        self
    }

    fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }

    fn resolve(mut self, r: Resolve) -> Self {
        self.resolve = Some(r);
        self
    }
}

/// The answer to a completion request.
#[derive(Debug, Default)]
pub(crate) struct Answer {
    /// The text the chosen item replaces: the identifier before the caret.
    pub start: usize,
    /// The end of the identifier the caret is in (Tab replaces to here).
    pub word_end: usize,
    pub items: Vec<Item>,
    /// Typing more asks again (objects of other schemas are left out until
    /// something is typed).
    pub incomplete: bool,
}

/// The objects the statements of a script create, with the start of the
/// statement that creates each, so that a statement sees what the ones
/// before it create.
#[derive(Debug, Default)]
pub(crate) struct ScriptObjects {
    defs: Vec<(usize, LocalObject)>,
}

impl ScriptObjects {
    pub fn build(script: &Script, ctx: &SqlContext) -> Self {
        let env = Env::new(ctx);
        let mut locals = LocalIndex::default();
        let mut defs = Vec::new();
        for root in script.statements() {
            let Some(first) = root.child_nodes().next() else {
                continue;
            };
            if !matches!(
                first.kind(),
                K::CreateTable
                    | K::CreateView
                    | K::CreateMaterializedView
                    | K::CreateFunction
                    | K::CreateProcedure
                    | K::CreateTrigger
                    | K::CreatePackage
                    | K::CreatePackageBody
                    | K::CreateType
                    | K::CreateSequence
                    | K::CreateSchema
            ) {
                continue;
            }
            let out = Resolver::new(&env, &locals).unit(root);
            for d in out.defs {
                locals.push(ctx.dialect, d.clone());
                defs.push((root.range().start, d));
            }
        }
        Self { defs }
    }

    /// The objects created by statements that start before `offset`.
    pub fn before(&self, dialect: Dialect, offset: usize) -> LocalIndex {
        let mut index = LocalIndex::default();
        for (start, d) in &self.defs {
            if *start < offset {
                index.push(dialect, d.clone());
            }
        }
        index
    }
}

/// The items at `offset` of `script`.
pub(crate) fn complete(
    script: &Script,
    ctx: &SqlContext,
    objects: &ScriptObjects,
    offset: usize,
) -> Answer {
    let Some(caret) = Caret::at(script, offset) else {
        return Answer {
            start: offset,
            word_end: offset,
            ..Answer::default()
        };
    };
    let expected = script.expected_at(caret.start);
    let unit = script.root_at(caret.offset);
    let locals = objects.before(ctx.dialect, unit.range().start);
    let mut e = Engine::new(script, ctx, caret, expected, unit, locals);
    e.run();
    e.finish()
}

fn mode_of(dialect: Dialect) -> NameMatch {
    if dialect == Dialect::Sqlite {
        NameMatch::IgnoreCase
    } else {
        NameMatch::Exact
    }
}

/// How many objects of the search path are offered without a prefix; a
/// bigger schema is cut and the answer is incomplete.
const MAX_UNFILTERED: usize = 2000;

/// The kinds a `FROM` clause can name.
const RELATIONS: [NodeKind; 4] = [
    NodeKind::Table,
    NodeKind::View,
    NodeKind::MaterializedView,
    NodeKind::Synonym,
];

const RELATION_GROUPS: [Group; 4] = [
    Group::Tables,
    Group::Views,
    Group::MaterializedViews,
    Group::Synonyms,
];

struct Engine<'a> {
    script: &'a Script,
    ctx: &'a SqlContext,
    dialect: Dialect,
    model: Option<&'a Model>,
    mode: NameMatch,
    caret: Caret<'a>,
    expected: Vec<Expected>,
    unit: SyntaxNode<'a>,
    locals: LocalIndex,
    snapshot: Option<Option<Snapshot>>,
    /// Matches the typed prefix like the popup does, a little wider.
    matcher: Option<Matcher>,
    /// The prefix in lower case when it is letters, digits and `_` only.
    ascii_prefix: Option<Vec<u8>>,
    items: Vec<Item>,
    seen: HashSet<(char, String)>,
    incomplete: bool,
}

impl<'a> Engine<'a> {
    fn new(
        script: &'a Script,
        ctx: &'a SqlContext,
        caret: Caret<'a>,
        expected: Vec<Expected>,
        unit: SyntaxNode<'a>,
        locals: LocalIndex,
    ) -> Self {
        let ascii_prefix = (caret
            .prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then(|| caret.prefix.to_ascii_lowercase().into_bytes());
        let matcher = (!caret.prefix.is_empty())
            .then(|| Matcher::new(&format!("*{}", caret.prefix), MatchingMode::IgnoreCase));
        Engine {
            script,
            ctx,
            dialect: ctx.dialect,
            model: ctx.scope.as_ref().map(|s| &*s.model),
            mode: mode_of(ctx.dialect),
            caret,
            expected,
            unit,
            locals,
            snapshot: None,
            matcher,
            ascii_prefix,
            items: Vec::new(),
            seen: HashSet::new(),
            incomplete: false,
        }
    }

    fn finish(self) -> Answer {
        Answer {
            start: self.caret.start,
            word_end: self.caret.word_end,
            items: self.items,
            incomplete: self.incomplete,
        }
    }

    fn has(&self, e: Expected) -> bool {
        self.expected.contains(&e)
    }

    fn text(&self) -> &'a str {
        self.script.text()
    }

    // ---- the places ----------------------------------------------------------

    fn run(&mut self) {
        // Next to a placeholder of the host language the word is not
        // known: only keywords.
        let text = self.text();
        let before = text
            .get(..self.caret.start)
            .and_then(|t| t.chars().next_back());
        let after = text
            .get(self.caret.word_end..)
            .and_then(|t| t.chars().next());
        if before == Some(HOLE) || after == Some(HOLE) {
            self.keywords();
            return;
        }
        let qualifier = self.caret.qualifier(self.dialect);
        if !qualifier.is_empty() {
            self.qualified(&qualifier);
            return;
        }
        let place = self.column_place();
        if self.has(Expected::Expression) {
            self.expression();
        } else if self.has(Expected::Name(NameKind::Column)) {
            self.column_name(place);
        }
        if self.has(Expected::Name(NameKind::Table)) {
            self.relations();
        }
        if self.has(Expected::Name(NameKind::Schema)) {
            self.schemas();
        }
        if self.has(Expected::Name(NameKind::Function)) {
            self.functions();
        }
        let ddl = self.ddl_place();
        if self.has(Expected::Name(NameKind::Type)) || ddl == Ddl::Type {
            self.types(None);
        }
        if ddl == Ddl::Constraint {
            for k in builtins::COLUMN_CONSTRAINTS {
                self.keyword_item(k);
            }
        }
        self.keywords();
    }

    /// The ancestors of the token before the identifier, nearest first.
    fn context_chain(&self) -> Vec<SyntaxNode<'a>> {
        let Some(prev) = self.caret.previous() else {
            return Vec::new();
        };
        let Some(p) = prev.parent() else {
            return Vec::new();
        };
        std::iter::once(p).chain(p.ancestors()).collect()
    }

    // ---- sources of the statement --------------------------------------------

    fn snapshot(&mut self) -> Option<&Snapshot> {
        if self.snapshot.is_none() {
            let env = Env::new(self.ctx);
            let probe = self.caret.anchor();
            let found = Resolver::probing(&env, &self.locals, probe).visible_at_probe(self.unit);
            self.snapshot = Some(found);
        }
        self.snapshot.as_ref().and_then(Option::as_ref)
    }

    fn source_columns(&self, source: &Source) -> Vec<Col> {
        match &source.columns {
            Columns::Known(c) => c.clone(),
            Columns::Unknown => Vec::new(),
        }
    }

    fn column_type(&self, col: &Col) -> Option<String> {
        let model = self.model?;
        let node = model.node(col.id?)?;
        node.column().map(|c| c.data_type.clone())
    }

    fn column_item(&self, col: &Col, qualifier: Option<&str>, tier: char) -> Item {
        let q = |n: &str| self.dialect.quote(n);
        let mut item = Item::new(col.name.clone(), SymbolKind::Field, tier);
        item.insert = match qualifier {
            Some(qual) => format!("{qual}.{}", q(&col.name)),
            None => q(&col.name),
        };
        item.filter = Some(col.name.clone());
        if let Some(t) = self.column_type(col) {
            item.detail = Some(t);
        }
        if let Some(id) = col.id {
            item.resolve = Some(Resolve::Node(id));
        }
        item
    }

    /// Columns of the sources in scope, nearest statement first.
    fn expression(&mut self) {
        let (frames, ctes_unused) = match self.snapshot() {
            Some(s) => (s.frames.clone(), s.ctes.len()),
            None => (Vec::new(), 0),
        };
        let _ = ctes_unused;
        let dialect = self.dialect;
        let in_aliases = self.select_aliases_visible();
        let mut taken: HashSet<String> = HashSet::new();
        for (depth, frame) in frames.iter().rev().enumerate() {
            let tier = if depth == 0 { '1' } else { '2' };
            let mut counts: std::collections::HashMap<String, usize> = Default::default();
            for s in &frame.sources {
                for c in self.source_columns(s) {
                    *counts.entry(dialect.fold(&c.name)).or_default() += 1;
                }
            }
            let many = frame.sources.len() > 1;
            for s in &frame.sources {
                for c in self.source_columns(s) {
                    if c.name.is_empty() {
                        continue;
                    }
                    let key = dialect.fold(&c.name);
                    let ambiguous = counts.get(&key).copied().unwrap_or(0) > 1;
                    if !ambiguous && depth > 0 && taken.contains(&key) {
                        continue;
                    }
                    let qualifier =
                        (ambiguous && !s.qualifier.is_empty()).then(|| s.qualifier.clone());
                    let mut item = self.column_item(&c, qualifier.as_deref(), tier);
                    if many && !s.qualifier.is_empty() {
                        item.tail = Some(format!(" ({})", s.qualifier));
                    }
                    self.push(item);
                }
            }
            for s in &frame.sources {
                for c in self.source_columns(s) {
                    taken.insert(dialect.fold(&c.name));
                }
            }
            if depth == 0 && in_aliases {
                for (_, range) in &frame.aliases {
                    let written = self.text().get(range.clone()).unwrap_or("").to_string();
                    if written.is_empty() {
                        continue;
                    }
                    let mut item = Item::new(written.clone(), SymbolKind::Variable, '1');
                    item.detail = Some("alias".into());
                    self.push(item);
                }
            }
            for s in &frame.sources {
                if s.qualifier.is_empty() {
                    continue;
                }
                let mut item = Item::new(s.qualifier.clone(), SymbolKind::Class, '4');
                if s.alias.is_some()
                    && let Some((_, table)) = &s.table
                {
                    item.detail = Some(table.clone());
                }
                self.push(item);
            }
        }
        self.ddl_columns();
        self.join_conditions();
        self.named_arguments();
        self.functions();
    }

    /// Select-list aliases are visible in `ORDER BY`, `GROUP BY`, `HAVING`
    /// (everywhere in SQLite).
    fn select_aliases_visible(&self) -> bool {
        if self.dialect == Dialect::Sqlite {
            return true;
        }
        self.context_chain().iter().any(|n| {
            matches!(
                n.kind(),
                K::OrderByClause | K::GroupByClause | K::HavingClause
            )
        })
    }

    /// Columns in DDL that has no `FROM`: the columns of the table an
    /// index is created on, the columns a `CREATE TABLE` defines.
    fn ddl_columns(&mut self) {
        let chain = self.context_chain();
        let has_snapshot = self.snapshot().is_some_and(|s| !s.frames.is_empty());
        if has_snapshot {
            return;
        }
        for n in &chain {
            match n.kind() {
                K::CreateIndex => {
                    if let Some(t) = n.child(K::TableRef) {
                        self.table_columns_of(t, '1');
                    }
                    return;
                }
                K::CreateTable => {
                    self.defined_columns(*n, '1');
                    return;
                }
                _ => {}
            }
        }
    }

    fn defined_columns(&mut self, create: SyntaxNode<'a>, tier: char) {
        let mut out = Vec::new();
        for def in create.child_nodes().filter(|n| n.kind() == K::ColumnDef) {
            if let Some(name) = def.child(K::DefName)
                && let Some(p) = names::parts(name, self.dialect).pop()
                && !p.broken
                && !(p.range.start >= self.caret.start && p.range.start <= self.caret.word_end)
            {
                out.push(Item::new(p.written.clone(), SymbolKind::Field, tier));
            }
        }
        for item in out {
            self.push(item);
        }
    }

    // ---- columns of one table --------------------------------------------------

    /// The node of the table a name node (`TableRef`) refers to.
    fn table_node(&self, name: SyntaxNode<'_>) -> Option<NodeRef<'a>> {
        let ps = names::parts(name, self.dialect);
        let q: Vec<(String, String)> = ps
            .iter()
            .map(|p| (p.written.clone(), p.name.clone()))
            .collect();
        self.find_table(&q)
    }

    /// The table, view or synonym with a (folded) qualified name.
    fn find_table(&self, parts: &[(String, String)]) -> Option<NodeRef<'a>> {
        let model = self.model?;
        let (_, name) = parts.last()?;
        if parts.len() >= 2 {
            let schema = &parts[parts.len() - 2].1;
            return model.find(schema, name, self.mode, &RELATIONS);
        }
        for s in self.path_schemas() {
            if let Some(n) = s.find(name, self.mode, &RELATIONS) {
                return Some(n);
            }
        }
        None
    }

    fn table_columns_of(&mut self, name: SyntaxNode<'_>, tier: char) {
        let Some(node) = self.table_node(name) else {
            return;
        };
        self.columns_of_node(node, tier, &HashSet::new());
    }

    fn columns_of_node(&mut self, node: NodeRef<'_>, tier: char, skip: &HashSet<String>) {
        let mut items = Vec::new();
        for c in node.group(Group::Columns).nodes() {
            if skip.contains(&self.dialect.fold(c.name())) {
                continue;
            }
            let mut item = Item::new(c.name(), SymbolKind::Field, tier)
                .insert(self.dialect.quote(c.name()))
                .resolve(Resolve::Node(c.id()));
            item.filter = Some(c.name().to_string());
            if let Some(info) = c.column() {
                item.detail = Some(info.data_type.clone());
            }
            items.push(item);
        }
        for i in items {
            self.push(i);
        }
    }

    // ---- qualified names -------------------------------------------------------

    fn qualified(&mut self, q: &[QualifierPart]) {
        let col = self.has(Expected::Name(NameKind::Column)) || self.has(Expected::Expression);
        let table = self.has(Expected::Name(NameKind::Table));
        let ty = self.has(Expected::Name(NameKind::Type));
        let parts: Vec<Part> = q
            .iter()
            .map(|p| Part {
                written: p.written.clone(),
                name: p.name.clone(),
                range: p.range.clone(),
                kind: K::TableRef,
                quoted: p.written.starts_with('"'),
                broken: false,
            })
            .collect();
        if col {
            let dialect = self.dialect;
            let found = self.snapshot().and_then(|snap| {
                snap.frames
                    .iter()
                    .rev()
                    .flat_map(|f| f.sources.iter())
                    .find(|s| s.named_by(dialect, &parts))
                    .cloned()
            });
            if let Some(source) = found {
                for c in self.source_columns(&source) {
                    if !c.name.is_empty() {
                        let item = self.column_item(&c, None, '1');
                        self.push(item);
                    }
                }
                self.star();
                return;
            }
            let names: Vec<(String, String)> = q
                .iter()
                .map(|p| (p.written.clone(), p.name.clone()))
                .collect();
            if let Some(node) = self.find_table(&names) {
                self.columns_of_node(node, '1', &HashSet::new());
                self.star();
                return;
            }
            if q.len() == 1 {
                self.schema_objects(&q[0].name, true);
            }
            return;
        }
        if table {
            if let Some(last) = q.last() {
                self.schema_objects(&last.name, false);
            }
            return;
        }
        if ty && let Some(last) = q.last() {
            self.types(Some(&last.name));
            return;
        }
        if self.has(Expected::Name(NameKind::Function))
            && let Some(last) = q.last()
        {
            self.schema_routines(&last.name);
        }
    }

    fn star(&mut self) {
        let mut item = Item::new("*", SymbolKind::Operator, '0');
        item.sort = "0*".into();
        self.push(item);
    }

    /// Tables, views and routines of a schema named `schema` (folded).
    fn schema_objects(&mut self, schema: &str, with_routines: bool) {
        let Some(model) = self.model else {
            return;
        };
        let Some(s) = model.schema(schema, self.mode) else {
            return;
        };
        let mut items = Vec::new();
        for g in RELATION_GROUPS {
            for n in s.group(g).nodes() {
                items.push(self.relation_item(n, false, '1'));
            }
        }
        for item in items {
            self.push(item);
        }
        if with_routines {
            self.schema_routines(schema);
        }
    }

    fn schema_routines(&mut self, schema: &str) {
        let Some(model) = self.model else {
            return;
        };
        let Some(s) = model.schema(schema, self.mode) else {
            return;
        };
        let mut names: HashSet<String> = HashSet::new();
        let mut items = Vec::new();
        for n in s.group(Group::Functions).nodes() {
            if names.insert(n.name().to_string()) {
                items.push(self.routine_item(n, '1'));
            }
        }
        for item in items {
            self.push(item);
        }
    }

    // ---- relations --------------------------------------------------------------

    fn path_schemas(&self) -> Vec<&'a db_model::SchemaModel> {
        let (Some(model), Some(scope)) = (self.model, self.ctx.scope.as_ref()) else {
            return Vec::new();
        };
        scope
            .search_path(self.dialect)
            .iter()
            .filter_map(|name| model.schema(name, self.mode))
            .collect()
    }

    fn kind_word(kind: NodeKind) -> &'static str {
        match kind {
            NodeKind::Table => "table",
            NodeKind::View => "view",
            NodeKind::MaterializedView => "materialized view",
            NodeKind::Synonym => "synonym",
            NodeKind::Function => "function",
            NodeKind::Procedure => "procedure",
            NodeKind::Sequence => "sequence",
            NodeKind::Type => "type",
            NodeKind::Package => "package",
            _ => "",
        }
    }

    fn relation_item(&self, n: NodeRef<'_>, qualify: bool, tier: char) -> Item {
        let kind = match n.kind() {
            NodeKind::Table => SymbolKind::Class,
            NodeKind::Synonym => SymbolKind::Reference,
            _ => SymbolKind::Interface,
        };
        let q = |s: &str| self.dialect.quote(s);
        let mut item = Item::new(n.name(), kind, tier).resolve(Resolve::Node(n.id()));
        item.filter = Some(n.name().to_string());
        item.insert = if qualify {
            format!("{}.{}", q(n.schema().name()), q(n.name()))
        } else {
            q(n.name())
        };
        item.detail = Some(Self::kind_word(n.kind()).to_string());
        if qualify {
            item.tail = Some(format!(" ({})", n.schema().name()));
        }
        item
    }

    fn routine_item(&self, n: NodeRef<'_>, tier: char) -> Item {
        let mut item = Item::new(n.name(), SymbolKind::Function, tier)
            .insert(self.dialect.quote(n.name()))
            .resolve(Resolve::Node(n.id()));
        let args: Vec<String> = n
            .group(Group::Arguments)
            .nodes()
            .filter(|a| {
                a.argument()
                    .is_some_and(|i| i.mode != db_model_mode_table())
            })
            .map(|a| a.name().to_string())
            .collect();
        item.tail = Some(format!("({})", args.join(", ")));
        if let Some(d) = n.object().and_then(|o| match &o.detail {
            Some(db_core::schema::ObjectDetail::Routine(r)) => r.return_type.clone(),
            _ => None,
        }) {
            item.detail = Some(d);
        }
        self.callable(&mut item, n.name());
        item
    }

    /// Makes the item write a call: `name(` with the caret inside, unless
    /// a `(` follows already.
    fn callable(&self, item: &mut Item, name: &str) {
        let rest = self.text().get(self.caret.word_end..).unwrap_or("");
        if rest.trim_start_matches([' ', '\t']).starts_with('(') {
            return;
        }
        item.insert = format!("{}($0)", self.dialect.quote(name));
        item.snippet = true;
    }

    fn relations(&mut self) {
        let in_from = self
            .context_chain()
            .iter()
            .any(|n| matches!(n.kind(), K::FromClause | K::Join | K::UsingSource));
        let ctes: Vec<(String, Range<usize>)> = if in_from {
            self.snapshot()
                .map(|s| {
                    s.ctes
                        .iter()
                        .map(|c| (c.name.clone(), c.def.clone()))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for (_, def) in ctes {
            let written = self.text().get(def).unwrap_or("").to_string();
            if written.is_empty() {
                continue;
            }
            let item = Item::new(written, SymbolKind::Class, '0').detail("CTE");
            self.push(item);
        }
        let used_aliases = if in_from && self.ctx.completion.insert_alias {
            self.used_names()
        } else {
            HashSet::new()
        };
        // Objects of the search path, unqualified. With a prefix typed only
        // the matching ones are built; without one a huge schema is cut
        // and the popup asks again with the first letters.
        let path = self.path_schemas();
        let mut counts: std::collections::HashMap<String, usize> = Default::default();
        if path.len() > 1 {
            for s in &path {
                for g in RELATION_GROUPS {
                    for n in s.group(g).nodes() {
                        *counts.entry(n.name().to_string()).or_default() += 1;
                    }
                }
            }
        }
        // Names that start with the prefix come before the others when the
        // list is cut.
        let typed = self.caret.prefix.to_lowercase();
        let mut first: Vec<(usize, NodeRef<'a>)> = Vec::new();
        let mut rest: Vec<(usize, NodeRef<'a>)> = Vec::new();
        let mut chars: Vec<char> = Vec::new();
        'path: for (i, s) in path.iter().enumerate() {
            for g in RELATION_GROUPS {
                for n in s.group(g).nodes() {
                    if first.len() >= MAX_UNFILTERED {
                        self.incomplete = true;
                        break 'path;
                    }
                    if starts_with_ignore_case(n.name(), &typed) {
                        first.push((i, n));
                    } else if self.name_matches(n.name(), &mut chars) {
                        rest.push((i, n));
                    }
                }
            }
        }
        if first.len() + rest.len() > MAX_UNFILTERED {
            self.incomplete = true;
            rest.truncate(MAX_UNFILTERED.saturating_sub(first.len()));
            first.truncate(MAX_UNFILTERED);
        }
        let mut items = Vec::new();
        for (i, n) in first.into_iter().chain(rest) {
            let dup = counts.get(n.name()).copied().unwrap_or(0) > 1;
            let mut item = self.relation_item(n, false, '1');
            if dup {
                item.tail = Some(format!(" ({})", path[i].name()));
            }
            items.push(item);
        }
        // Objects the script creates.
        let created: Vec<LocalObject> = self
            .locals
            .iter()
            .filter(|o| o.kind == LocalKind::Relation)
            .cloned()
            .collect();
        for o in created {
            let item = Item::new(o.name.clone(), SymbolKind::Class, '1')
                .insert(self.dialect.quote(&o.name))
                .detail("created in script");
            items.push(item);
        }
        if in_from && self.ctx.completion.insert_alias {
            let rest = self.text().get(self.caret.word_end..).unwrap_or("");
            let next_is_word = rest
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '"');
            if !next_is_word {
                let mut used = used_aliases;
                for item in &mut items {
                    let alias = self.alias_for(&item.label, &used);
                    item.insert = format!("{} {alias}", item.insert);
                }
                used.clear();
            }
        }
        for item in items {
            self.push(item);
        }
        // Objects of other schemas, qualified; only when something is typed,
        // else the list would be the whole database.
        if let Some(model) = self.model {
            let others = model
                .schemas()
                .any(|s| !path.iter().any(|p| std::ptr::eq(*p, s)) && !s.info().is_system);
            if self.matcher.is_none() {
                self.incomplete |= others;
            } else {
                self.incomplete = true;
                let mut found = Vec::new();
                'schemas: for s in model.schemas() {
                    if path.iter().any(|p| std::ptr::eq(*p, s)) {
                        continue;
                    }
                    for g in RELATION_GROUPS {
                        for n in s.group(g).nodes() {
                            if found.len() >= MAX_UNFILTERED {
                                break 'schemas;
                            }
                            if self.name_matches(n.name(), &mut chars) {
                                found.push(self.relation_item(n, true, '5'));
                            }
                        }
                    }
                }
                for item in found {
                    self.push(item);
                }
            }
        }
        self.schemas();
    }

    /// The name matches the typed prefix. A name and a prefix of plain ASCII
    /// are checked for the letters in order first (every match has them),
    /// which spares the matcher most of the names.
    fn name_matches(&self, name: &str, chars: &mut Vec<char>) -> bool {
        let Some(m) = &self.matcher else {
            return true;
        };
        if let Some(wanted) = &self.ascii_prefix
            && name.is_ascii()
        {
            let mut wanted = wanted.iter();
            let mut next = wanted.next();
            for b in name.bytes() {
                if next == Some(&b.to_ascii_lowercase()) {
                    next = wanted.next();
                }
            }
            if next.is_some() {
                return false;
            }
        }
        chars.clear();
        chars.extend(name.chars());
        m.matches_chars(chars)
    }

    /// Names of the sources already in the statement (aliases, tables).
    fn used_names(&mut self) -> HashSet<String> {
        let mut out = HashSet::new();
        if let Some(s) = self.snapshot() {
            for f in &s.frames {
                for src in &f.sources {
                    out.insert(src.qualifier.to_lowercase());
                }
            }
        }
        out
    }

    /// The initials of a table name, made unique.
    fn alias_for(&self, name: &str, used: &HashSet<String>) -> String {
        let mut initials = String::new();
        let mut word_start = true;
        let chars: Vec<char> = name.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if !c.is_alphanumeric() {
                word_start = true;
                continue;
            }
            let hump = c.is_uppercase()
                && i > 0
                && chars[i - 1].is_lowercase()
                && !name.chars().all(|c| !c.is_lowercase());
            if word_start || hump {
                initials.extend(c.to_lowercase());
            }
            word_start = false;
        }
        if initials.is_empty() {
            initials.push('t');
        }
        let mut alias = initials.clone();
        let mut n = 1;
        while used.contains(&alias) || self.dialect.keyword(&alias).is_some() {
            n += 1;
            alias = format!("{initials}{n}");
        }
        alias
    }

    fn schemas(&mut self) {
        let Some(model) = self.model else {
            return;
        };
        let mut items = Vec::new();
        for s in model.schemas() {
            if s.info().is_system && self.matcher.is_none() {
                continue;
            }
            let item = Item::new(s.name(), SymbolKind::Module, '6')
                .insert(self.dialect.quote(s.name()))
                .detail("schema")
                .resolve(Resolve::Node(s.id()));
            items.push(item);
        }
        for item in items {
            self.push(item);
        }
    }

    // ---- functions ----------------------------------------------------------------

    fn functions(&mut self) {
        let mut names: HashSet<String> = HashSet::new();
        let mut items = Vec::new();
        for s in self.path_schemas() {
            for n in s.group(Group::Functions).nodes() {
                if names.insert(self.dialect.fold(n.name())) {
                    items.push(self.routine_item(n, '3'));
                }
            }
        }
        for f in builtins::functions(self.dialect) {
            if !names.insert(f.name.to_lowercase()) && f.kind != FnKind::Special {
                continue;
            }
            let name = self.function_name(f.name);
            let mut item = Item::new(name.clone(), SymbolKind::Function, '3')
                .detail(f.returns.to_string())
                .resolve(Resolve::Function(f));
            item.filter = Some(f.name.to_string());
            if f.kind == FnKind::Special {
                item.kind = SymbolKind::Constant;
            } else {
                item.tail = Some(format!("({})", f.params));
                self.callable(&mut item, &name);
            }
            items.push(item);
        }
        for item in items {
            self.push(item);
        }
    }

    /// A built-in function's name as the dialect writes it: lower case,
    /// upper case in Oracle.
    fn function_name(&self, name: &str) -> String {
        match self.dialect {
            Dialect::Oracle => name.to_uppercase(),
            _ => name.to_string(),
        }
    }

    // ---- types ---------------------------------------------------------------------

    fn types(&mut self, schema: Option<&str>) {
        let mut items = Vec::new();
        if schema.is_none() {
            for t in builtins::types(self.dialect) {
                let mut item =
                    Item::new(t.name, SymbolKind::TypeParameter, '1').resolve(Resolve::Type(t));
                if !t.args.is_empty() {
                    item.tail = Some(t.args.to_string());
                }
                items.push(item);
            }
        }
        let schemas: Vec<&db_model::SchemaModel> = match (schema, self.model) {
            (Some(s), Some(m)) => m.schema(s, self.mode).into_iter().collect(),
            _ => self.path_schemas(),
        };
        for s in schemas {
            for n in s.group(Group::Types).nodes() {
                items.push(
                    Item::new(n.name(), SymbolKind::TypeParameter, '1')
                        .insert(self.dialect.quote(n.name()))
                        .detail("type")
                        .resolve(Resolve::Node(n.id())),
                );
            }
        }
        for item in items {
            self.push(item);
        }
    }

    /// Where in a column definition the caret is.
    fn ddl_place(&self) -> Ddl {
        let Some(prev) = self.caret.previous() else {
            return Ddl::None;
        };
        let Some(parent) = prev.parent() else {
            return Ddl::None;
        };
        let start = self.caret.start;
        // `name |`: the type.
        if parent.kind() == K::DefName
            && let Some(def) = parent.parent().filter(|d| d.kind() == K::ColumnDef)
            && !def
                .child_nodes()
                .any(|n| n.kind() == K::TypeRef && n.range().end <= start)
            && def.range().end <= self.caret.word_end.max(start)
            && def
                .child(K::DefName)
                .is_some_and(|n| n.range().end == prev.range().end)
        {
            return Ddl::Type;
        }
        // `name type |`: constraints.
        for n in std::iter::once(parent).chain(parent.ancestors()) {
            match n.kind() {
                K::ColumnConstraint | K::ReferencesClause | K::TableConstraint => {
                    return Ddl::None;
                }
                K::ColumnDef => {
                    let typed = n
                        .child_nodes()
                        .any(|c| c.kind() == K::TypeRef && c.range().end <= start);
                    return if typed { Ddl::Constraint } else { Ddl::None };
                }
                K::Query | K::Select | K::Root => return Ddl::None,
                _ => {}
            }
        }
        Ddl::None
    }

    // ---- columns where a name is written, not an expression --------------------------

    fn column_place(&self) -> Place<'a> {
        let chain = self.context_chain();
        for n in &chain {
            match n.kind() {
                K::ColumnList => {
                    return match n.parent().map(|p| p.kind()) {
                        Some(K::Insert | K::InsertInto | K::MergeInsert) => Place::InsertList(*n),
                        Some(K::TableConstraint) => Place::Defined,
                        Some(K::ReferencesClause) => Place::Referenced(n.parent().unwrap()),
                        _ => Place::Other,
                    };
                }
                K::SetClause => return Place::Set(*n),
                K::ConflictTarget => return Place::Conflict,
                K::AlterAction | K::AlterTable => return Place::Alter,
                K::Select | K::Query | K::Root => return Place::Other,
                _ => {}
            }
        }
        Place::Other
    }

    fn column_name(&mut self, place: Place) {
        let dialect = self.dialect;
        match place {
            Place::InsertList(list) | Place::Set(list) => {
                let target = self.snapshot().and_then(|s| {
                    s.target
                        .clone()
                        .or_else(|| s.frames.last().and_then(|f| f.sources.first().cloned()))
                });
                let listed = self.listed_columns(list);
                if let Some(t) = target {
                    for c in self.source_columns(&t) {
                        if c.name.is_empty() || listed.contains(&dialect.fold(&c.name)) {
                            continue;
                        }
                        let item = self.column_item(&c, None, '1');
                        self.push(item);
                    }
                }
            }
            Place::Conflict => {
                let target = self.snapshot().and_then(|s| s.target.clone());
                if let Some(t) = target {
                    for c in self.source_columns(&t) {
                        if !c.name.is_empty() {
                            let item = self.column_item(&c, None, '1');
                            self.push(item);
                        }
                    }
                }
            }
            Place::Defined => {
                if let Some(create) = self
                    .context_chain()
                    .into_iter()
                    .find(|n| n.kind() == K::CreateTable)
                {
                    self.defined_columns(create, '1');
                }
            }
            Place::Referenced(refs) => {
                if let Some(t) = refs.child(K::TableRef) {
                    self.table_columns_of(t, '1');
                }
            }
            Place::Alter => {
                let chain = self.context_chain();
                // `add |` and `add column |` name a new column.
                let new_column = self.caret.previous_is("add")
                    || (self.caret.previous_is("column")
                        && self
                            .caret
                            .before
                            .iter()
                            .rev()
                            .nth(1)
                            .is_some_and(|t| t.text().eq_ignore_ascii_case("add")));
                if !new_column
                    && let Some(alter) = chain.into_iter().find(|n| n.kind() == K::AlterTable)
                    && let Some(t) = alter.child(K::TableRef)
                {
                    self.table_columns_of(t, '1');
                }
            }
            Place::Other => {
                // A name in an expression-like list (`USING (..)`): the
                // columns of the sources.
                let frames = self
                    .snapshot()
                    .map(|s| s.frames.clone())
                    .unwrap_or_default();
                for f in frames.iter().rev().take(1) {
                    for s in &f.sources {
                        for c in self.source_columns(s) {
                            if !c.name.is_empty() {
                                let item = self.column_item(&c, None, '1');
                                self.push(item);
                            }
                        }
                    }
                }
            }
        }
    }

    /// The columns already written in a column list or `SET` clause (not the
    /// one being typed), folded.
    fn listed_columns(&self, list: SyntaxNode<'_>) -> HashSet<String> {
        let mut out = HashSet::new();
        let (start, end) = (self.caret.start, self.caret.word_end);
        let mut stack: Vec<SyntaxNode<'_>> = list.child_nodes().collect();
        while let Some(n) = stack.pop() {
            match n.kind() {
                K::ColumnRef => {
                    let r = n.range();
                    if (r.end <= start || r.start >= end)
                        && let Some(p) = names::parts(n, self.dialect).pop()
                        && !p.broken
                    {
                        out.insert(p.name);
                    }
                }
                K::Assignment => {
                    if let Some(lhs) = n.child_nodes().next() {
                        stack.push(lhs);
                    }
                }
                K::TupleExpr | K::ParenExpr => stack.extend(n.child_nodes()),
                _ => {}
            }
        }
        out
    }

    // ---- join conditions ---------------------------------------------------------

    /// In the `ON` of a join: the conditions the foreign keys between the
    /// joined table and the ones before it give.
    fn join_conditions(&mut self) {
        let chain = self.context_chain();
        let Some(on) = chain.iter().find(|n| n.kind() == K::OnClause).copied() else {
            return;
        };
        let Some(join) = on.parent().filter(|j| j.kind() == K::Join) else {
            return;
        };
        let right = join
            .child_nodes()
            .take_while(|n| n.range().start < on.range().start)
            .filter(|n| !matches!(n.kind(), K::Missing | K::Error))
            .last();
        let Some(right) = right else {
            return;
        };
        let Some(model) = self.model else {
            return;
        };
        let frame: Frame = match self.snapshot().and_then(|s| s.frames.last().cloned()) {
            Some(f) => f,
            None => return,
        };
        let right_range = right.range();
        let Some(new) = frame
            .sources
            .iter()
            .find(|s| right_range.start <= s.def.start && s.def.end <= right_range.end)
            .cloned()
        else {
            return;
        };
        let earlier: Vec<Source> = frame
            .sources
            .iter()
            .filter(|s| s.def.end <= right_range.start)
            .cloned()
            .collect();
        let Some(new_node) = self.source_node(model, &new) else {
            return;
        };
        let mut conditions: Vec<String> = Vec::new();
        for other in &earlier {
            let Some(other_node) = self.source_node(model, other) else {
                continue;
            };
            // The new table references the earlier one, or the other way.
            for (owner, os, target, ts) in [
                (new_node, &new, other_node, other),
                (other_node, other, new_node, &new),
            ] {
                for fk in owner.group(Group::ForeignKeys).nodes() {
                    let Some(info) = fk.constraint() else {
                        continue;
                    };
                    if !self.references(info, target)
                        || info.columns.len() != info.ref_columns.len()
                    {
                        continue;
                    }
                    let q = |s: &Source, col: &str| {
                        let name = self.dialect.quote(col);
                        if s.qualifier.is_empty() {
                            name
                        } else {
                            format!("{}.{name}", s.qualifier)
                        }
                    };
                    let text = info
                        .columns
                        .iter()
                        .zip(&info.ref_columns)
                        .map(|(a, b)| format!("{} = {}", q(os, a), q(ts, b)))
                        .collect::<Vec<_>>()
                        .join(" and ");
                    if !conditions.contains(&text) {
                        conditions.push(text);
                    }
                }
            }
        }
        for text in conditions {
            let mut item = Item::new(text.clone(), SymbolKind::Snippet, '0');
            item.sort = format!("0{text}");
            item.detail = Some("foreign key".into());
            self.push(item);
        }
    }

    fn references(&self, info: &db_core::schema::ConstraintInfo, target: NodeRef<'_>) -> bool {
        let Some(table) = &info.ref_table else {
            return false;
        };
        if !self.mode.matches(target.name(), table) {
            return false;
        }
        match &info.ref_schema {
            Some(s) => self.mode.matches(target.schema().name(), s),
            None => true,
        }
    }

    /// The table node behind a source of `FROM`.
    fn source_node(&self, model: &'a Model, s: &Source) -> Option<NodeRef<'a>> {
        if let Columns::Known(cols) = &s.columns
            && let Some(id) = cols.iter().find_map(|c| c.id)
            && let Some(col) = model.node(id)
            && let Some(parent) = col.parent()
        {
            return Some(parent);
        }
        let (schema, name) = s.table.as_ref()?;
        let schema = s.object_schema.clone().or_else(|| schema.clone());
        match schema {
            Some(sc) => model.find(&sc, name, self.mode, &RELATIONS),
            None => self
                .path_schemas()
                .into_iter()
                .find_map(|sc| sc.find(name, self.mode, &RELATIONS)),
        }
    }

    // ---- named arguments -----------------------------------------------------------

    /// In the arguments of a call of a routine of the model: the names of
    /// its parameters (`name =>`).
    fn named_arguments(&mut self) {
        if !matches!(self.dialect, Dialect::Postgres | Dialect::Oracle) {
            return;
        }
        let Some(prev) = self.caret.previous() else {
            return;
        };
        if !matches!(prev.kind(), TokenKind::LParen | TokenKind::Comma) {
            return;
        }
        let Some(parent) = prev.parent().filter(|p| p.kind() == K::ArgList) else {
            return;
        };
        let Some(call) = parent.parent().filter(|c| c.kind() == K::CallExpr) else {
            return;
        };
        let Some(fname) = call.child(K::FunctionRef) else {
            return;
        };
        let ps = names::parts(fname, self.dialect);
        let Some(last) = ps.last() else {
            return;
        };
        let Some(model) = self.model else {
            return;
        };
        let schemas: Vec<&db_model::SchemaModel> = if ps.len() >= 2 {
            model
                .schema(&ps[ps.len() - 2].name, self.mode)
                .into_iter()
                .collect()
        } else {
            self.path_schemas()
        };
        let mut given: HashSet<String> = HashSet::new();
        for a in call.descendants() {
            if a.kind() == K::NamedArg
                && let Some(t) = a.child_tokens().next()
            {
                given.insert(self.dialect.fold(t.text()));
            }
        }
        let mut offered: HashSet<String> = HashSet::new();
        let mut items = Vec::new();
        for s in schemas {
            for r in s.lookup(
                &last.name,
                self.mode,
                &[NodeKind::Function, NodeKind::Procedure],
            ) {
                for a in r.group(Group::Arguments).nodes() {
                    let Some(info) = a.argument() else {
                        continue;
                    };
                    let Some(name) = &info.name else {
                        continue;
                    };
                    if matches!(info.mode, db_core::schema::ArgumentMode::Table)
                        || given.contains(name)
                        || !offered.insert(name.clone())
                    {
                        continue;
                    }
                    let q = self.dialect.quote(name);
                    let mut item = Item::new(format!("{name} =>"), SymbolKind::Variable, '0');
                    item.insert = format!("{q} => ");
                    item.filter = Some(name.clone());
                    item.detail = Some(info.data_type.clone());
                    items.push(item);
                }
            }
        }
        for item in items {
            self.push(item);
        }
    }

    // ---- keywords --------------------------------------------------------------------

    fn keywords(&mut self) {
        let expression = self.has(Expected::Expression);
        let typed = !self.caret.prefix.is_empty();
        let mut words: Vec<&'static str> = Vec::new();
        for e in &self.expected {
            if let Expected::Keyword(k) = e {
                words.push(k);
            }
        }
        if expression {
            for k in builtins::EXPRESSION_KEYWORDS {
                if !words.contains(k) {
                    words.push(k);
                }
            }
            if !typed {
                // Between `select` and the first name: only what starts an
                // expression, not the clauses that could follow.
                words.retain(|k| builtins::EXPRESSION_KEYWORDS.contains(k) || *k == "DEFAULT");
            }
        }
        for k in words {
            self.keyword_item(k);
        }
    }

    fn keyword_item(&mut self, k: &str) {
        let phrase = match k {
            "GROUP" | "ORDER" if true => format!("{k} BY"),
            "PRIMARY" => "PRIMARY KEY".to_string(),
            "FOREIGN" => "FOREIGN KEY".to_string(),
            other => other.to_string(),
        };
        let text = self.keyword_case(&phrase);
        let mut item = Item::new(text, SymbolKind::Keyword, '8');
        item.sort = format!("8{}", phrase.to_lowercase());
        item.resolve = Some(Resolve::Keyword(
            phrase.split(' ').next().unwrap_or(&phrase).to_string(),
        ));
        self.push(item);
    }

    fn keyword_case(&self, word: &str) -> String {
        let lower = match self.ctx.completion.keyword_case {
            KeywordCase::Upper => false,
            KeywordCase::Lower => true,
            KeywordCase::Keep => self.written_lower(),
        };
        if lower {
            word.to_lowercase()
        } else {
            word.to_uppercase()
        }
    }

    /// Whether the keywords around the caret are written in lower case.
    fn written_lower(&self) -> bool {
        let typed = &self.caret.prefix;
        if typed.chars().any(char::is_alphabetic) {
            return !typed.chars().any(char::is_uppercase);
        }
        for t in self.caret.before.iter().rev() {
            if t.kind() == TokenKind::Word
                && self.dialect.keyword(t.text()).is_some()
                && t.text().chars().any(char::is_alphabetic)
            {
                return !t.text().chars().any(char::is_uppercase);
            }
        }
        false
    }

    // ---- items ----------------------------------------------------------------------

    fn push(&mut self, item: Item) {
        if !self
            .seen
            .insert((item.sort.chars().next().unwrap_or(' '), item.insert.clone()))
        {
            return;
        }
        self.items.push(item);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ddl {
    None,
    Type,
    Constraint,
}

enum Place<'a> {
    InsertList(SyntaxNode<'a>),
    Set(SyntaxNode<'a>),
    Conflict,
    Defined,
    Referenced(SyntaxNode<'a>),
    Alter,
    Other,
}

/// `name` starts with `typed` (lower case), whatever the case.
fn starts_with_ignore_case(name: &str, typed: &str) -> bool {
    let mut lower = name.chars().flat_map(char::to_lowercase);
    typed.chars().all(|c| lower.next() == Some(c))
}

fn db_model_mode_table() -> db_core::schema::ArgumentMode {
    db_core::schema::ArgumentMode::Table
}

/// The significant tokens before an offset, for the callers that need a
/// token scan without a caret.
#[allow(dead_code)]
pub(crate) fn token_before<'a>(root: SyntaxNode<'a>, offset: usize) -> Option<SyntaxToken<'a>> {
    root.tokens().take_while(|t| t.range().end <= offset).last()
}
