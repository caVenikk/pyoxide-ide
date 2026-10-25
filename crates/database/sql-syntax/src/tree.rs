use std::fmt;
use std::ops::Range;

use crate::lexer::{Token, TokenKind};

/// The kind of a node of the syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SyntaxKind {
    /// A parse unit: one statement with its terminator and the trivia around
    /// it. The units of a script follow each other and cover its text.
    Root,
    /// A statement or clause the parser does not know: its tokens, never an
    /// error inside.
    Opaque,
    /// Unexpected tokens, with the set of what would have been valid
    /// ([`SyntaxNode::expected`]).
    Error,
    /// A token or construct that should be here and is not: no text, the
    /// range is empty at the end of the previous token.
    Missing,
    /// Text outside SQL ([`crate::HOLE`], an injected host expression): where
    /// a name, an expression, a list item or a clause may stand.
    Hole,

    // Queries.
    /// `[WITH ..] body [ORDER BY ..] [LIMIT ..] [OFFSET ..] [FETCH ..] [FOR UPDATE ..]`.
    Query,
    /// `SELECT .. [FROM ..] [WHERE ..] [GROUP BY ..] [HAVING ..] [WINDOW ..]`.
    Select,
    /// `a UNION [ALL] b`, `INTERSECT`, `EXCEPT`, `MINUS`.
    SetOperation,
    /// `( query )` as a query body.
    ParenQuery,
    /// `VALUES (..), (..)`.
    Values,
    /// `TABLE name`.
    TableQuery,
    /// `WITH [RECURSIVE] cte, ..`.
    WithClause,
    /// `name [(columns)] AS [[NOT] MATERIALIZED] ( query )`.
    Cte,
    /// `(a, b)` after a table alias or a CTE name.
    ColumnAliasList,
    /// `DISTINCT [ON (..)]`.
    DistinctClause,
    SelectList,
    /// An expression with an optional alias, or a `*`.
    SelectItem,
    /// `*` or `qualifier.*`.
    Star,
    /// `INTO target, ..` of a select.
    IntoClause,
    FromClause,
    /// A table, view or CTE in `FROM` with its alias.
    TableSource,
    /// `[LATERAL] ( query ) [alias]`.
    SubquerySource,
    /// `[LATERAL] function(..) [WITH ORDINALITY] [alias]`.
    FunctionSource,
    /// `( from item ) [alias]`.
    ParenSource,
    /// `left [kind] JOIN right [ON ..| USING (..)]`.
    Join,
    /// `ON condition` of a join.
    OnClause,
    /// `USING (columns)` of a join.
    UsingClause,
    PivotClause,
    UnpivotClause,
    WhereClause,
    /// `START WITH .. CONNECT BY [NOCYCLE] ..`.
    HierarchicalClause,
    GroupByClause,
    /// `GROUPING SETS (..)`, `ROLLUP (..)`, `CUBE (..)`.
    GroupingSets,
    HavingClause,
    WindowClause,
    /// `name AS (spec)` in a `WINDOW` clause.
    WindowDecl,
    OrderByClause,
    /// `expr [ASC|DESC] [NULLS FIRST|LAST]`.
    OrderItem,
    LimitClause,
    OffsetClause,
    FetchClause,
    /// `FOR UPDATE ..`, `FOR SHARE ..`.
    LockingClause,

    // Names, typed by position. A qualifier is the node before the dot:
    // `ColumnRef(TableRef(SchemaRef(s) . t) . c)`.
    /// A table, view, CTE or alias: in `FROM`, and as the qualifier of a column.
    TableRef,
    ColumnRef,
    SchemaRef,
    /// The qualifier of a schema (a database or catalog).
    CatalogRef,
    FunctionRef,
    TypeRef,
    /// The definition of an alias (`AS c`, `customers c`).
    AliasDef,
    /// The name a CTE defines.
    CteDef,
    /// The name a `WINDOW` clause defines.
    WindowDef,
    /// A reference to a named window.
    WindowRef,

    // Expressions.
    /// A number, a string, `NULL`, `TRUE`, `FALSE`, `DATE '..'`, `INTERVAL '..' DAY`.
    Literal,
    /// A bind parameter (`?`, `$1`, `:name`).
    Param,
    BinaryExpr,
    /// `NOT x`, `-x`, `PRIOR x`.
    UnaryExpr,
    ParenExpr,
    /// `(a, b)`, `ROW(a, b)`, `()`.
    TupleExpr,
    /// `( query )` as an expression.
    SubqueryExpr,
    ExistsExpr,
    InExpr,
    BetweenExpr,
    /// `LIKE`, `ILIKE`, `SIMILAR TO`, `GLOB`, `REGEXP`, `MATCH`, with `ESCAPE`.
    LikeExpr,
    /// `IS [NOT] NULL|TRUE|FALSE|UNKNOWN|DISTINCT FROM ..`, `ISNULL`, `NOTNULL`.
    IsExpr,
    CaseExpr,
    WhenClause,
    ElseClause,
    /// `CAST(x AS type)`, `x::type`.
    CastExpr,
    CallExpr,
    ArgList,
    /// `name => value`.
    NamedArg,
    FilterClause,
    WithinGroup,
    OverClause,
    WindowSpec,
    PartitionBy,
    /// `ROWS|RANGE|GROUPS ..` of a window.
    FrameClause,
    ArrayExpr,
    SubscriptExpr,
    /// `(expr).field`.
    FieldExpr,
    /// Oracle's `(+)`.
    OuterJoinMark,
    CollateExpr,
    AtTimeZone,
    /// `cursor%FOUND`, `sql%ROWCOUNT` (PL/SQL).
    AttributeExpr,

    // Data modification. The target of an `INSERT`, `UPDATE`, `DELETE` and
    // `MERGE` is a `TableSource` (or a `SubquerySource`), the same node as in
    // `FROM`. A leading `WITH` is the first child.
    /// `INSERT INTO table [AS alias] [(columns)] source [ON CONFLICT ..] [RETURNING ..]`;
    /// the source is `VALUES`, a query or `DEFAULT VALUES`. Oracle's
    /// `INSERT ALL|FIRST` has `InsertInto` children (also inside `WhenClause`
    /// and `ElseClause`) and the query last.
    Insert,
    /// `INTO table [(columns)] [VALUES (..)]` of a multi-table insert.
    InsertInto,
    /// `(a, b)`: column names of an insert, a conflict target, an index.
    ColumnList,
    /// `OR REPLACE|IGNORE|ABORT|FAIL|ROLLBACK` of SQLite.
    ConflictResolution,
    /// `ON CONFLICT [target] DO NOTHING | DO UPDATE SET ..`.
    OnConflictClause,
    /// `(columns) [WHERE ..]` or `ON CONSTRAINT name` of `ON CONFLICT`.
    ConflictTarget,
    /// `SET a = 1, (b, c) = (..)`.
    SetClause,
    /// `target = value` in a `SET` clause.
    Assignment,
    /// `RETURNING items [INTO targets]`.
    ReturningClause,
    /// `UPDATE target SET .. [FROM ..] [WHERE ..] [RETURNING ..]`.
    Update,
    /// `DELETE FROM target [USING ..] [WHERE ..] [RETURNING ..]`.
    Delete,
    /// The sources after `USING` of a `DELETE` or a `MERGE`.
    UsingSource,
    /// `MERGE INTO target USING source ON condition WHEN ..`.
    Merge,
    /// `WHEN [NOT] MATCHED [BY ..] [AND condition] THEN action`.
    MergeClause,
    MergeUpdate,
    MergeDelete,
    MergeInsert,
    /// `WHERE CURRENT OF cursor`.
    CurrentOf,

    // Definitions. Names are references typed by position; a name that a
    // statement defines is a `DefName` (a column, a constraint, a variable, a
    // parameter, a label), or the first `TableRef` / `ObjectRef` / `FunctionRef`
    // of a `CREATE` statement.
    /// A name defined here; the parent says what it names.
    DefName,
    /// A named object that is not a table or a function: an index, a sequence,
    /// a trigger, a package, a constraint, an exception. Qualified by `SchemaRef`.
    ObjectRef,
    CreateTable,
    /// `name type [constraints]` (also an attribute of a type).
    ColumnDef,
    ColumnConstraint,
    TableConstraint,
    /// `REFERENCES table [(columns)] [ON DELETE ..]`.
    ReferencesClause,
    CreateView,
    CreateMaterializedView,
    CreateIndex,
    /// An indexed expression or column with its order.
    IndexElement,
    CreateSequence,
    CreateSchema,
    CreateType,
    CreateTypeBody,
    CreateTrigger,
    /// `INSERT`, `UPDATE [OF columns]`, `DELETE`, `TRUNCATE` of a trigger.
    TriggerEvent,
    /// `WHEN (condition)` of a trigger.
    TriggerCondition,
    CreateFunction,
    CreateProcedure,
    CreatePackage,
    CreatePackageBody,
    AlterTable,
    /// One action of `ALTER TABLE`.
    AlterAction,
    /// `ALTER` of anything else: the object and the rest of the statement.
    AlterObject,
    DropStatement,
    Truncate,
    /// `COMMENT ON object IS 'text'`.
    CommentStatement,
    /// SQLite's `PRAGMA [schema.]name [= value | (value)]`.
    Pragma,
    /// PostgreSQL's `DO [LANGUAGE x] 'body'`.
    DoStatement,
    /// `(param, ..)` of a routine.
    ParamList,
    /// `[mode] name type [DEFAULT value]`.
    RoutineParam,
    /// `RETURNS type | TABLE (..)` (PostgreSQL), `RETURN type` (PL/SQL).
    ReturnsClause,
    /// `LANGUAGE x`, `IMMUTABLE`, `SECURITY DEFINER`, `DETERMINISTIC`, ...
    RoutineOption,
    /// The body of a routine given as a string (`$$ .. $$`): the string token
    /// is a leaf and [`SyntaxNode::fragment`] is the parsed program inside it.
    EmbeddedBody,

    // Procedural code: PL/SQL, PL/pgSQL and trigger bodies.
    /// `[DECLARE decls] BEGIN statements [EXCEPTION handlers] END [name]`; the
    /// body of a routine starts at its declarations.
    Block,
    DeclareSection,
    StatementList,
    /// `name [CONSTANT] type [NOT NULL] [:= value]`.
    VarDecl,
    /// `CURSOR name [(params)] [RETURN type] IS|FOR query`.
    CursorDecl,
    /// `TYPE name IS ..`, `SUBTYPE name IS ..`.
    TypeDecl,
    /// `name EXCEPTION`.
    ExceptionDecl,
    /// A procedure or function declared or defined inside a block or package.
    SubprogramDecl,
    /// `<<name>>`.
    Label,
    ExceptionSection,
    /// `WHEN name [OR name] THEN statements`.
    ExceptionHandler,
    IfStmt,
    /// `ELSIF condition THEN statements`.
    ElsifClause,
    CaseStmt,
    LoopStmt,
    WhileStmt,
    /// `FOR i IN a..b | query | cursor LOOP ..`.
    ForStmt,
    ForallStmt,
    ExitStmt,
    ReturnStmt,
    AssignStmt,
    /// A call of a procedure as a statement.
    CallStmt,
    NullStmt,
    RaiseStmt,
    /// `EXECUTE [IMMEDIATE] string [INTO ..] [USING ..]`.
    ExecuteStmt,
    OpenStmt,
    FetchStmt,
    CloseStmt,
}

impl SyntaxKind {
    /// A name node typed by its position.
    pub fn is_name(self) -> bool {
        matches!(
            self,
            SyntaxKind::TableRef
                | SyntaxKind::ColumnRef
                | SyntaxKind::SchemaRef
                | SyntaxKind::CatalogRef
                | SyntaxKind::FunctionRef
                | SyntaxKind::TypeRef
                | SyntaxKind::AliasDef
                | SyntaxKind::CteDef
                | SyntaxKind::WindowDef
                | SyntaxKind::WindowRef
                | SyntaxKind::DefName
                | SyntaxKind::ObjectRef
        )
    }
}

/// What kind of name is expected at a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameKind {
    Table,
    Column,
    Alias,
    Schema,
    Function,
    Type,
    Cte,
    Window,
    /// An index, sequence, trigger, package or other named object.
    Object,
}

/// One entry of an expected set: what would be valid at a position. Sets are
/// sorted in the order of the variants, so tokens and names come before
/// keywords in a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Expected {
    /// A punctuation or operator token (`(`, `)`, `,`, `*`, `;`).
    Punct(&'static str),
    Name(NameKind),
    /// The start of an expression (names, literals, functions, `CASE`, ...).
    Expression,
    /// A keyword, upper case.
    Keyword(&'static str),
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expected::Keyword(k) => f.write_str(k),
            Expected::Punct(p) => write!(f, "'{p}'"),
            Expected::Name(n) => f.write_str(match n {
                NameKind::Table => "table name",
                NameKind::Column => "column name",
                NameKind::Alias => "alias",
                NameKind::Schema => "schema name",
                NameKind::Function => "function name",
                NameKind::Type => "type name",
                NameKind::Cte => "CTE name",
                NameKind::Window => "window name",
                NameKind::Object => "object name",
            }),
            Expected::Expression => f.write_str("expression"),
        }
    }
}

const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Elem {
    Node(u32),
    Token(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeData {
    kind: SyntaxKind,
    parent: u32,
    /// Range into `StatementTree::elems`.
    children: (u32, u32),
    /// Byte range relative to the unit, without leading and trailing trivia.
    range: (u32, u32),
    /// Index into `StatementTree::expected`, or `NONE`.
    expected: u32,
}

/// The syntax tree of one parse unit (a statement and its terminator). Every
/// token of the unit, trivia included, is a leaf, so printing the tree gives
/// the text back. Offsets inside are relative to the unit, which lets a
/// script shift a unit without touching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementTree {
    len: usize,
    tokens: Vec<Token>,
    tok_parent: Vec<u32>,
    nodes: Vec<NodeData>,
    elems: Vec<Elem>,
    expected: Vec<Box<[Expected]>>,
    root: u32,
    fragments: Vec<Fragment>,
}

/// What an embedded program is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FragmentLanguage {
    /// PL/pgSQL: a `DO` block or a `LANGUAGE plpgsql` body.
    PlPgSql,
    /// A `LANGUAGE sql` body: statements.
    Sql,
}

/// A program inside a string token (`$$ .. $$`), parsed on its own. Its
/// offsets are relative to `offset`, the start of the text inside the quotes;
/// quotes that need no unescaping make that map a plain shift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fragment {
    /// Index of the string token in the unit.
    token: u32,
    /// Offset of the body in the unit.
    offset: u32,
    language: FragmentLanguage,
    tree: StatementTree,
}

impl Fragment {
    pub(crate) fn new(
        token: usize,
        offset: usize,
        language: FragmentLanguage,
        tree: StatementTree,
    ) -> Self {
        Fragment {
            token: token as u32,
            offset: offset as u32,
            language,
            tree,
        }
    }
}

impl StatementTree {
    /// The length of the unit's text in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// All tokens of the unit, trivia included, relative to its start.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub(crate) fn has_slash_line(&self) -> bool {
        self.tokens.iter().any(|t| t.kind == TokenKind::SlashLine)
    }

    /// An estimate of the heap memory of the tree in bytes.
    pub fn heap_size(&self) -> usize {
        self.tokens.capacity() * size_of::<Token>()
            + self.tok_parent.capacity() * 4
            + self.nodes.capacity() * size_of::<NodeData>()
            + self.elems.capacity() * size_of::<Elem>()
            + self
                .expected
                .iter()
                .map(|e| e.len() * size_of::<Expected>() + 16)
                .sum::<usize>()
            + self
                .fragments
                .iter()
                .map(|f| size_of::<Fragment>() + f.tree.heap_size())
                .sum::<usize>()
    }

    pub(crate) fn root<'a>(&'a self, base: usize, src: &'a str) -> SyntaxNode<'a> {
        SyntaxNode {
            tree: self,
            base,
            src,
            id: self.root,
        }
    }
}

/// Builds a [`StatementTree`] from the parser's events. Nodes are opened and
/// finished like a stack; a checkpoint lets a node wrap children that were
/// already built (`a + b` wraps `a` after the operator is seen). Trivia is
/// emitted lazily, before the next significant token or node, so it stays
/// outside of nodes that start after it.
pub(crate) struct Builder<'t> {
    toks: &'t [Token],
    emitted: usize,
    stack: Vec<Elem>,
    open: Vec<(SyntaxKind, usize)>,
    nodes: Vec<NodeData>,
    elems: Vec<Elem>,
    expected: Vec<Box<[Expected]>>,
    tok_parent: Vec<u32>,
    last_sig_end: u32,
    fragments: Vec<Fragment>,
}

impl<'t> Builder<'t> {
    pub(crate) fn new(toks: &'t [Token]) -> Self {
        Builder {
            toks,
            emitted: 0,
            stack: Vec::with_capacity(64),
            open: vec![(SyntaxKind::Root, 0)],
            nodes: Vec::with_capacity(toks.len() / 2 + 1),
            elems: Vec::with_capacity(toks.len() + toks.len() / 2 + 1),
            expected: Vec::new(),
            tok_parent: vec![NONE; toks.len()],
            last_sig_end: 0,
            fragments: Vec::new(),
        }
    }

    /// Attaches a parsed program to a string token of this unit.
    pub(crate) fn fragment(&mut self, fragment: Fragment) {
        self.fragments.push(fragment);
    }

    fn flush_to(&mut self, i: usize) {
        let i = i.min(self.toks.len());
        while self.emitted < i {
            self.stack.push(Elem::Token(self.emitted as u32));
            self.emitted += 1;
        }
    }

    /// Emits significant token `i` (an index into the unit's tokens) and the
    /// trivia before it.
    pub(crate) fn token(&mut self, i: usize) {
        if i < self.emitted || i >= self.toks.len() {
            return;
        }
        self.flush_to(i);
        self.stack.push(Elem::Token(i as u32));
        self.emitted = i + 1;
        self.last_sig_end = self.toks[i].end as u32;
    }

    /// Opens a node; `next` is the index of the next significant token, the
    /// trivia before it goes to the enclosing node.
    pub(crate) fn start(&mut self, kind: SyntaxKind, next: usize) {
        self.flush_to(next);
        self.open.push((kind, self.stack.len()));
    }

    pub(crate) fn checkpoint(&mut self, next: usize) -> usize {
        self.flush_to(next);
        self.stack.len()
    }

    /// Opens a node that takes everything built since `cp`.
    pub(crate) fn start_at(&mut self, cp: usize, kind: SyntaxKind) {
        let floor = self.open.last().map_or(0, |o| o.1);
        debug_assert!(cp >= floor && cp <= self.stack.len());
        self.open
            .push((kind, cp.clamp(floor, self.stack.len().max(floor))));
    }

    pub(crate) fn finish(&mut self, expected: Option<Box<[Expected]>>) {
        if self.open.len() <= 1 {
            debug_assert!(false, "finish without an open node");
            return;
        }
        self.finish_top(expected);
    }

    fn finish_top(&mut self, expected: Option<Box<[Expected]>>) -> u32 {
        let (kind, start) = self.open.pop().unwrap_or((SyntaxKind::Root, 0));
        let start = start.min(self.stack.len());
        let id = self.nodes.len() as u32;
        let first = self.elems.len() as u32;
        let (mut lo, mut hi, mut any) = (u32::MAX, 0u32, false);
        for e in self.stack.drain(start..) {
            match e {
                Elem::Token(t) => {
                    let tok = &self.toks[t as usize];
                    self.tok_parent[t as usize] = id;
                    if !tok.is_trivia() {
                        lo = lo.min(tok.start as u32);
                        hi = hi.max(tok.end as u32);
                        any = true;
                    }
                }
                Elem::Node(n) => {
                    let node = &mut self.nodes[n as usize];
                    node.parent = id;
                    lo = lo.min(node.range.0);
                    hi = hi.max(node.range.1);
                    any = true;
                }
            }
            self.elems.push(e);
        }
        if !any {
            lo = self.last_sig_end;
            hi = self.last_sig_end;
        }
        let expected = match expected {
            Some(e) => {
                self.expected.push(e);
                (self.expected.len() - 1) as u32
            }
            None => NONE,
        };
        self.nodes.push(NodeData {
            kind,
            parent: NONE,
            children: (first, self.elems.len() as u32),
            range: (lo, hi),
            expected,
        });
        self.stack.push(Elem::Node(id));
        id
    }

    /// Emits the remaining trivia, closes every open node and returns the
    /// tree's parts; [`TreeParts::into_tree`] takes the tokens.
    pub(crate) fn finish_root(mut self, len: usize) -> TreeParts {
        self.flush_to(self.toks.len());
        while self.open.len() > 1 {
            self.finish_top(None);
        }
        let root = self.finish_top(None);
        self.nodes[root as usize].range = (0, len as u32);
        TreeParts {
            len,
            tok_parent: self.tok_parent,
            nodes: self.nodes,
            elems: self.elems,
            expected: self.expected,
            root,
            fragments: self.fragments,
        }
    }
}

/// A built tree without its tokens.
pub(crate) struct TreeParts {
    len: usize,
    tok_parent: Vec<u32>,
    nodes: Vec<NodeData>,
    elems: Vec<Elem>,
    expected: Vec<Box<[Expected]>>,
    root: u32,
    fragments: Vec<Fragment>,
}

impl TreeParts {
    pub(crate) fn into_tree(self, tokens: Vec<Token>) -> StatementTree {
        StatementTree {
            len: self.len,
            tokens,
            tok_parent: self.tok_parent,
            nodes: self.nodes,
            elems: self.elems,
            expected: self.expected,
            root: self.root,
            fragments: self.fragments,
        }
    }
}

/// A node of a script's syntax tree. Ranges are byte offsets in the script.
#[derive(Clone, Copy)]
pub struct SyntaxNode<'a> {
    tree: &'a StatementTree,
    base: usize,
    src: &'a str,
    id: u32,
}

impl PartialEq for SyntaxNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.tree, other.tree) && self.id == other.id && self.base == other.base
    }
}

impl Eq for SyntaxNode<'_> {}

impl fmt::Debug for SyntaxNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.range();
        write!(f, "{:?}@{}..{}", self.kind(), r.start, r.end)
    }
}

/// A program embedded in a string token, see [`SyntaxNode::fragment`].
#[derive(Debug, Clone)]
pub struct EmbeddedFragment<'a> {
    pub root: SyntaxNode<'a>,
    pub language: FragmentLanguage,
    /// The text inside the quotes, in the script.
    pub body: Range<usize>,
}

/// A token of a script's syntax tree.
#[derive(Clone, Copy)]
pub struct SyntaxToken<'a> {
    tree: &'a StatementTree,
    base: usize,
    src: &'a str,
    idx: u32,
}

impl PartialEq for SyntaxToken<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.tree, other.tree) && self.idx == other.idx && self.base == other.base
    }
}

impl Eq for SyntaxToken<'_> {}

impl fmt::Debug for SyntaxToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.range();
        write!(
            f,
            "{:?}@{}..{} {:?}",
            self.kind(),
            r.start,
            r.end,
            self.text()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxElement<'a> {
    Node(SyntaxNode<'a>),
    Token(SyntaxToken<'a>),
}

impl<'a> SyntaxElement<'a> {
    pub fn range(&self) -> Range<usize> {
        match self {
            SyntaxElement::Node(n) => n.range(),
            SyntaxElement::Token(t) => t.range(),
        }
    }

    pub fn as_node(&self) -> Option<SyntaxNode<'a>> {
        match self {
            SyntaxElement::Node(n) => Some(*n),
            SyntaxElement::Token(_) => None,
        }
    }

    pub fn as_token(&self) -> Option<SyntaxToken<'a>> {
        match self {
            SyntaxElement::Token(t) => Some(*t),
            SyntaxElement::Node(_) => None,
        }
    }
}

impl<'a> SyntaxNode<'a> {
    fn data(&self) -> &'a NodeData {
        &self.tree.nodes[self.id as usize]
    }

    fn node(&self, id: u32) -> SyntaxNode<'a> {
        SyntaxNode { id, ..*self }
    }

    fn token(&self, idx: u32) -> SyntaxToken<'a> {
        SyntaxToken {
            tree: self.tree,
            base: self.base,
            src: self.src,
            idx,
        }
    }

    fn element(&self, e: Elem) -> SyntaxElement<'a> {
        match e {
            Elem::Node(n) => SyntaxElement::Node(self.node(n)),
            Elem::Token(t) => SyntaxElement::Token(self.token(t)),
        }
    }

    pub fn kind(&self) -> SyntaxKind {
        self.data().kind
    }

    /// The byte range in the script, without leading and trailing trivia. A
    /// [`SyntaxKind::Missing`] node, and a node with nothing but missing
    /// parts, is empty at the end of the token before it. The root covers its
    /// whole unit.
    pub fn range(&self) -> Range<usize> {
        let (s, e) = self.data().range;
        self.base + s as usize..self.base + e as usize
    }

    pub fn text(&self) -> &'a str {
        self.src.get(self.range()).unwrap_or("")
    }

    pub fn parent(&self) -> Option<SyntaxNode<'a>> {
        let p = self.data().parent;
        (p != NONE).then(|| self.node(p))
    }

    /// The parents up to the root, nearest first.
    pub fn ancestors(&self) -> impl Iterator<Item = SyntaxNode<'a>> + 'a {
        std::iter::successors(self.parent(), |n| n.parent())
    }

    /// This node, or the nearest ancestor, of `kind`.
    pub fn enclosing(&self, kind: SyntaxKind) -> Option<SyntaxNode<'a>> {
        std::iter::once(*self)
            .chain(self.ancestors())
            .find(|n| n.kind() == kind)
    }

    /// The direct children in order, trivia tokens included.
    pub fn children(&self) -> impl Iterator<Item = SyntaxElement<'a>> + 'a {
        let (s, e) = self.data().children;
        let this = *self;
        self.tree.elems[s as usize..e as usize]
            .iter()
            .map(move |&el| this.element(el))
    }

    pub fn child_nodes(&self) -> impl Iterator<Item = SyntaxNode<'a>> + 'a {
        self.children().filter_map(|e| e.as_node())
    }

    /// The first child node of `kind`.
    pub fn child(&self, kind: SyntaxKind) -> Option<SyntaxNode<'a>> {
        self.child_nodes().find(|n| n.kind() == kind)
    }

    /// The direct child tokens that are not trivia.
    pub fn child_tokens(&self) -> impl Iterator<Item = SyntaxToken<'a>> + 'a {
        self.children()
            .filter_map(|e| e.as_token())
            .filter(|t| !t.is_trivia())
    }

    /// Every element of the subtree in document order, this node first.
    pub fn preorder(&self) -> impl Iterator<Item = SyntaxElement<'a>> + 'a {
        let this = *self;
        let mut stack = vec![Elem::Node(self.id)];
        std::iter::from_fn(move || {
            let e = stack.pop()?;
            if let Elem::Node(n) = e {
                let (s, end) = this.tree.nodes[n as usize].children;
                stack.extend(this.tree.elems[s as usize..end as usize].iter().rev());
            }
            Some(this.element(e))
        })
    }

    /// The nodes of the subtree in document order, this node first.
    pub fn descendants(&self) -> impl Iterator<Item = SyntaxNode<'a>> + 'a {
        self.preorder().filter_map(|e| e.as_node())
    }

    /// The significant tokens of the subtree in order.
    pub fn tokens(&self) -> impl Iterator<Item = SyntaxToken<'a>> + 'a {
        self.preorder()
            .filter_map(|e| e.as_token())
            .filter(|t| !t.is_trivia())
    }

    /// For [`SyntaxKind::Error`] and [`SyntaxKind::Missing`]: what would have
    /// been valid there, sorted. Empty for other nodes.
    pub fn expected(&self) -> &'a [Expected] {
        let e = self.data().expected;
        if e == NONE {
            &[]
        } else {
            &self.tree.expected[e as usize]
        }
    }

    /// The program inside the string token of this node (the body of a
    /// routine in `$$ .. $$`), parsed on its own: the root of its tree, whose
    /// ranges are offsets in the script like all others. The fragment is not
    /// part of [`SyntaxNode::children`], [`SyntaxNode::descendants`] or
    /// [`SyntaxNode::print`]; the string token is.
    pub fn fragment(&self) -> Option<EmbeddedFragment<'a>> {
        let tree = self.tree;
        if tree.fragments.is_empty() {
            return None;
        }
        let (s, e) = self.data().children;
        tree.elems[s as usize..e as usize]
            .iter()
            .find_map(|el| match el {
                Elem::Token(t) => tree.fragments.iter().find(|f| f.token == *t),
                Elem::Node(_) => None,
            })
            .map(|f| EmbeddedFragment {
                root: f.tree.root(self.base + f.offset as usize, self.src),
                language: f.language,
                body: self.base + f.offset as usize..self.base + f.offset as usize + f.tree.len(),
            })
    }

    /// The program embedded in a string of this subtree that contains `offset`.
    pub fn fragment_at(&self, offset: usize) -> Option<EmbeddedFragment<'a>> {
        if self.tree.fragments.is_empty() {
            return None;
        }
        self.descendants().find_map(|n| {
            n.fragment()
                .filter(|f| f.body.start <= offset && offset <= f.body.end)
        })
    }

    pub(crate) fn statement_tree(&self) -> &'a StatementTree {
        self.tree
    }

    /// The innermost node of the subtree whose range contains `offset`: a
    /// node strictly around it wins over one that only touches it, and among
    /// nodes touching it the first (left) one wins. `None` when `offset` is
    /// outside this node.
    pub fn node_at(&self, offset: usize) -> Option<SyntaxNode<'a>> {
        let r = self.range();
        if offset < r.start || offset > r.end {
            return None;
        }
        let mut cur = *self;
        'down: loop {
            let mut touching = None;
            for c in cur.child_nodes() {
                let cr = c.range();
                if cr.start < offset && offset < cr.end {
                    cur = c;
                    continue 'down;
                }
                if touching.is_none() && cr.start <= offset && offset <= cr.end {
                    touching = Some(c);
                }
            }
            match touching {
                Some(c) => cur = c,
                None => break,
            }
        }
        // Inside the program embedded in a string, the node of that program.
        if let Some(f) = cur.fragment()
            && f.body.start <= offset
            && offset <= f.body.end
        {
            return f.root.node_at(offset).or(Some(cur));
        }
        Some(cur)
    }

    /// The whole text of the subtree, trivia included.
    pub fn print(&self) -> String {
        let mut out = String::new();
        for e in self.preorder() {
            if let SyntaxElement::Token(t) = e {
                out.push_str(t.text());
            }
        }
        out
    }

    /// The subtree as indented text: one line per node (`Kind@start..end`)
    /// and per significant token (`Kind "text"`), expected sets in brackets.
    /// For tests and debugging.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.dump_into(&mut out, 0);
        out
    }

    fn dump_into(&self, out: &mut String, depth: usize) {
        use std::fmt::Write as _;
        let r = self.range();
        let _ = write!(
            out,
            "{:indent$}{:?}@{}..{}",
            "",
            self.kind(),
            r.start,
            r.end,
            indent = depth * 2
        );
        if !self.expected().is_empty() {
            let list: Vec<String> = self.expected().iter().map(|e| e.to_string()).collect();
            let _ = write!(out, " [{}]", list.join(", "));
        }
        out.push('\n');
        for c in self.children() {
            match c {
                SyntaxElement::Node(n) => n.dump_into(out, depth + 1),
                SyntaxElement::Token(t) if !t.is_trivia() => {
                    let _ = writeln!(
                        out,
                        "{:indent$}{:?} {:?}",
                        "",
                        t.kind(),
                        t.text(),
                        indent = (depth + 1) * 2
                    );
                }
                SyntaxElement::Token(_) => {}
            }
        }
    }
}

impl<'a> SyntaxToken<'a> {
    fn tok(&self) -> &'a Token {
        &self.tree.tokens[self.idx as usize]
    }

    pub fn kind(&self) -> TokenKind {
        self.tok().kind
    }

    pub fn range(&self) -> Range<usize> {
        let t = self.tok();
        self.base + t.start..self.base + t.end
    }

    pub fn text(&self) -> &'a str {
        self.src.get(self.range()).unwrap_or("")
    }

    pub fn is_trivia(&self) -> bool {
        self.tok().is_trivia()
    }

    /// A string, quoted identifier or block comment that is not closed.
    pub fn unterminated(&self) -> bool {
        self.tok().unterminated
    }

    pub fn parent(&self) -> Option<SyntaxNode<'a>> {
        let p = self.tree.tok_parent[self.idx as usize];
        (p != NONE).then_some(SyntaxNode {
            tree: self.tree,
            base: self.base,
            src: self.src,
            id: p,
        })
    }
}
