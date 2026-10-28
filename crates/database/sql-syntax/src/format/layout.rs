use std::borrow::Cow;

use crate::dialect::Dialect;
use crate::lexer::TokenKind;
use crate::script::Script;
use crate::tree::{SyntaxElement, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::out::{Blank, Brk, Lead, Out, text_width};
use super::style::{
    AndOr, Case, ClauseElements, CodeStyle, Collapse, CommaPlacement, FunctionCase, SelectItems,
};
use super::words;

type El<'a> = SyntaxElement<'a>;

/// How a list breaks between its items.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    /// Where the text had a line break, and where the line is too long.
    Wrap,
    OnePerLine,
}

enum Role {
    Cond,
    List(Policy),
    Flow,
}

fn role(kind: K) -> Role {
    match kind {
        K::WhereClause | K::HavingClause | K::OnClause => Role::Cond,
        K::WithClause => Role::List(Policy::OnePerLine),
        K::FromClause
        | K::UsingSource
        | K::GroupByClause
        | K::OrderByClause
        | K::SetClause
        | K::ReturningClause
        | K::WindowClause
        | K::IntoClause
        | K::Values => Role::List(Policy::Wrap),
        _ => Role::Flow,
    }
}

/// Clauses that start a line of their own inside a statement.
fn breaks_line(kind: K, parent: K) -> bool {
    matches!(
        kind,
        K::FromClause
            | K::WhereClause
            | K::GroupByClause
            | K::HavingClause
            | K::WindowClause
            | K::OrderByClause
            | K::LimitClause
            | K::OffsetClause
            | K::FetchClause
            | K::LockingClause
            | K::IntoClause
            | K::HierarchicalClause
            | K::ReturningClause
            | K::OnConflictClause
            | K::UsingSource
            | K::PivotClause
            | K::UnpivotClause
            | K::SetClause
    ) || (kind == K::OnClause && parent == K::Merge)
}

/// The statements the formatter lays out; any other kind is left as written.
fn supported(kind: K) -> bool {
    matches!(
        kind,
        K::Query
            | K::Select
            | K::SetOperation
            | K::ParenQuery
            | K::Values
            | K::TableQuery
            | K::Insert
            | K::Update
            | K::Delete
            | K::Merge
            | K::CreateTable
            | K::CreateView
            | K::CreateMaterializedView
            | K::CreateIndex
            | K::CreateSequence
            | K::CreateSchema
            | K::AlterTable
            | K::DropStatement
            | K::Truncate
            | K::CommentStatement
    )
}

fn is_and_or(t: &SyntaxToken<'_>) -> bool {
    t.kind() == TokenKind::Word
        && (t.text().eq_ignore_ascii_case("and") || t.text().eq_ignore_ascii_case("or"))
}

/// Every token of the subtree, trivia included.
fn all_tokens<'a>(n: SyntaxNode<'a>) -> Vec<SyntaxToken<'a>> {
    n.preorder().filter_map(|e| e.as_token()).collect()
}

fn is_trivia(e: &El<'_>) -> bool {
    matches!(e, El::Token(t) if t.is_trivia())
}

enum Class {
    Keyword,
    Type,
    Function,
    Ident,
    Other,
}

pub(crate) struct Layout<'a> {
    out: Out<'a>,
    style: &'a CodeStyle,
    dialect: Dialect,
    src: &'a str,
    /// The column of the statement or subquery being laid out.
    base: usize,
    /// Break before every `AND` / `OR` of the condition being laid out.
    chain_break: bool,
    last_lparen: bool,
    prev_fn: bool,
    prev_op: bool,
    prev_param: bool,
}

/// Formats every statement of `script`.
pub(crate) fn run(script: &Script, style: &CodeStyle) -> String {
    let mut l = Layout {
        out: Out::new(style),
        style,
        dialect: script.dialect(),
        src: script.text(),
        base: 0,
        chain_break: false,
        last_lparen: false,
        prev_fn: false,
        prev_op: false,
        prev_param: false,
    };
    let sep = match style.blank_lines_between_statements {
        Some(n) => Blank::Exactly(n),
        None => Blank::UpTo(style.keep_blank_lines),
    };
    for (i, root) in script.statements().enumerate() {
        l.out.pending = Some(Brk {
            indent: 0,
            blank: if i > 0 { sep } else { Blank::None },
        });
        l.unit(root);
    }
    let ends_with_newline = script.text().ends_with('\n');
    if ends_with_newline && !l.out.is_empty() && !l.out.ends_with_newline() {
        l.out.push_newline();
    }
    l.out.finish()
}

impl<'a> Layout<'a> {
    fn unit(&mut self, root: SyntaxNode<'a>) {
        let verbatim = !self.formattable(root);
        for el in root.children() {
            match el {
                El::Token(t) => {
                    self.out.top = true;
                    self.tok(t);
                }
                El::Node(stmt) => {
                    self.out.top = false;
                    if verbatim {
                        self.verbatim(stmt);
                    } else {
                        self.statement(stmt);
                    }
                    self.out.top = true;
                    self.out.cont = 0;
                }
            }
        }
    }

    /// The statement as it was written (comments inside it included).
    fn verbatim(&mut self, stmt: SyntaxNode<'a>) {
        let mut first = None;
        let mut last = None;
        for t in all_tokens(stmt) {
            if t.kind() != TokenKind::Whitespace {
                first.get_or_insert(t.range().start);
                last = Some(t);
            }
        }
        let (Some(start), Some(last)) = (first, last) else {
            return;
        };
        let text = self.src.get(start..last.range().end).unwrap_or("");
        self.out.token(text, Lead::Space, false, false);
        if last.kind() == TokenKind::LineComment {
            self.out.end_line();
        }
    }

    /// Statements with a syntax error, kinds the formatter does not lay out
    /// and unterminated strings are left as written.
    fn formattable(&self, root: SyntaxNode<'a>) -> bool {
        if root.child_nodes().any(|n| !supported(n.kind())) {
            return false;
        }
        if root
            .descendants()
            .any(|d| matches!(d.kind(), K::Opaque | K::Error | K::Missing | K::Hole))
        {
            return false;
        }
        !all_tokens(root).iter().any(|t| t.unterminated())
    }

    fn statement(&mut self, n: SyntaxNode<'a>) {
        self.base = 0;
        self.out.cont = self.style.indent;
        match n.kind() {
            K::Query | K::Select | K::SetOperation | K::Values | K::TableQuery => {
                if self.style.collapse_short_statement == Collapse::Always
                    && self.try_flat(0, |s| s.block(n, 0))
                {
                    return;
                }
                self.block(n, 0);
            }
            K::Insert | K::Update | K::Delete | K::Merge | K::AlterTable => self.block(n, 0),
            K::CreateTable => self.create_table(n),
            _ => self.flow(n),
        }
    }

    // Tokens.

    fn el(&mut self, e: El<'a>) {
        match e {
            El::Token(t) => self.tok(t),
            El::Node(n) => self.node(n),
        }
    }

    fn tok(&mut self, t: SyntaxToken<'a>) {
        match t.kind() {
            TokenKind::Whitespace => return self.out.whitespace(t.text()),
            TokenKind::LineComment => return self.out.comment(t.text(), true),
            TokenKind::BlockComment => return self.out.comment(t.text(), false),
            TokenKind::SlashLine => {
                self.out.request(Brk::at(0));
                self.out.token(t.text(), Lead::Space, false, false);
                return;
            }
            _ => {}
        }
        let parent = t.parent();
        let pk = parent.map(|p| p.kind());
        let text = self.word_text(&t, pk);
        let (lead, open_after) = self.spacing(&t, parent);
        if is_and_or(&t) && pk == Some(K::BinaryExpr) && self.out.opp_next.is_none() {
            self.out.opp_next = Some(self.out.cont);
        }
        self.out.token(&text, lead, open_after, true);
        self.last_lparen = t.kind() == TokenKind::LParen;
        self.prev_fn = t.kind() == TokenKind::Word && pk == Some(K::FunctionRef);
        self.prev_op = t.kind() == TokenKind::Operator;
        self.prev_param = t.kind() == TokenKind::Parameter;
        if t.kind() == TokenKind::Comma {
            self.out.opp_next = Some(self.out.cont);
        }
    }

    fn spacing(&self, t: &SyntaxToken<'a>, parent: Option<SyntaxNode<'a>>) -> (Lead, bool) {
        let st = self.style;
        let pk = parent.map(|p| p.kind());
        match t.kind() {
            TokenKind::Comma => (
                if st.space_before_comma {
                    Lead::Space
                } else {
                    Lead::No
                },
                !st.space_after_comma,
            ),
            TokenKind::Semicolon => (Lead::No, false),
            TokenKind::Dot => (Lead::No, true),
            TokenKind::RParen => (
                if st.parentheses_padding && !self.last_lparen {
                    Lead::Space
                } else {
                    Lead::No
                },
                false,
            ),
            TokenKind::LBracket => (Lead::No, true),
            TokenKind::RBracket => (Lead::No, false),
            TokenKind::LParen => {
                let call = matches!(
                    pk,
                    Some(
                        K::ArgList
                            | K::ParamList
                            | K::TypeRef
                            | K::CastExpr
                            | K::OuterJoinMark
                            | K::SubscriptExpr
                    )
                );
                let list = !st.space_before_parenthesis
                    && matches!(pk, Some(K::ColumnList | K::TupleExpr));
                let tight = self.out.prev_open() || self.prev_fn || call || list;
                (
                    if tight { Lead::No } else { Lead::Space },
                    !st.parentheses_padding,
                )
            }
            TokenKind::Operator => {
                let text = t.text();
                if self.prev_op || self.prev_param {
                    // `@ -5` must not become `@-5`, nor `- -5` a comment, nor
                    // `? ||` the operator `?||`.
                    return (
                        Lead::Force,
                        !(self.style.spaces_around_operators && pk != Some(K::UnaryExpr)),
                    );
                }
                match pk {
                    Some(K::Star) => (Lead::Space, false),
                    Some(K::ArgList) if text == "*" => (Lead::Space, false),
                    Some(K::OuterJoinMark | K::AttributeExpr | K::SubscriptExpr) => {
                        (Lead::No, true)
                    }
                    Some(K::UnaryExpr)
                        if parent.is_some_and(|p| p.range().start == t.range().start) =>
                    {
                        (Lead::Space, true)
                    }
                    _ if text == "::" => (Lead::No, true),
                    _ if st.spaces_around_operators => (Lead::Space, false),
                    _ => (Lead::No, true),
                }
            }
            TokenKind::Parameter if self.prev_op => (Lead::Force, false),
            _ => (Lead::Space, false),
        }
    }

    fn word_text(&self, t: &SyntaxToken<'a>, parent: Option<K>) -> Cow<'a, str> {
        let text = t.text();
        if t.kind() != TokenKind::Word {
            return Cow::Borrowed(text);
        }
        let st = self.style;
        let keyword = self.dialect.keyword(text).is_some();
        let class = match parent {
            Some(K::TypeRef) if words::is_builtin_type(text) => Class::Type,
            Some(K::TypeRef) if keyword => Class::Keyword,
            Some(K::TypeRef) => Class::Ident,
            Some(K::Literal) => Class::Keyword,
            Some(K::AliasDef) if text.eq_ignore_ascii_case("as") => Class::Keyword,
            Some(K::FunctionRef) if words::is_builtin_function(text) => Class::Function,
            Some(k) if k.is_name() => Class::Ident,
            _ if keyword => Class::Keyword,
            _ => Class::Other,
        };
        let case = match class {
            Class::Keyword => st.keyword_case,
            Class::Type => st.builtin_type_case,
            Class::Function => match st.builtin_function_case {
                FunctionCase::AsKeywords => st.keyword_case,
                FunctionCase::Keep => Case::Keep,
                FunctionCase::Upper => Case::Upper,
                FunctionCase::Lower => Case::Lower,
            },
            Class::Ident => st.identifier_case,
            Class::Other => Case::Keep,
        };
        match case {
            Case::Upper if text.bytes().any(|b| b.is_ascii_lowercase()) => {
                Cow::Owned(text.to_ascii_uppercase())
            }
            Case::Lower if text.bytes().any(|b| b.is_ascii_uppercase()) => {
                Cow::Owned(text.to_ascii_lowercase())
            }
            _ => Cow::Borrowed(text),
        }
    }

    // Nodes.

    /// The children in a line, with the spacing of tokens.
    fn flow(&mut self, n: SyntaxNode<'a>) {
        for e in n.children() {
            self.el(e);
        }
    }

    fn node(&mut self, n: SyntaxNode<'a>) {
        match n.kind() {
            K::CaseExpr => self.case_expr(n),
            K::Query | K::Select | K::SetOperation | K::Values | K::TableQuery => {
                if self.last_lparen {
                    self.subquery_body(n);
                } else {
                    let base = self.base;
                    self.out.request(Brk::at(base));
                    self.block(n, base);
                }
            }
            _ => self.flow(n),
        }
    }

    /// Runs `f` in a one-line mode; if the result does not fit on the line
    /// (or needs a line break) the output is left as it was and `false` is
    /// returned.
    fn try_flat(&mut self, reserve: usize, f: impl Fn(&mut Self)) -> bool {
        if self.out.flat > 0 {
            f(self);
            return true;
        }
        self.out.flush();
        let snap = self.out.snapshot();
        let (base, cont, chain, lp, pf) = (
            self.base,
            self.out.cont,
            self.chain_break,
            self.last_lparen,
            self.prev_fn,
        );
        self.out.flat += 1;
        f(self);
        self.out.flat -= 1;
        let margin = self.style.right_margin;
        let fits = !self.out.forced
            && (margin == 0 || self.out.col() + reserve <= margin)
            && !self.out.since(&snap).contains('\n');
        if fits {
            return true;
        }
        self.out.restore(snap);
        self.base = base;
        self.out.cont = cont;
        self.chain_break = chain;
        self.last_lparen = lp;
        self.prev_fn = pf;
        false
    }

    /// The width of `n` on one line.
    fn measure(&mut self, n: SyntaxNode<'a>) -> usize {
        let snap = self.out.snapshot();
        let (cont, lp, pf) = (self.out.cont, self.last_lparen, self.prev_fn);
        self.out.flat += 1;
        self.flow(n);
        self.out.flat -= 1;
        let w = text_width(self.out.since(&snap).trim_start());
        self.out.restore(snap);
        self.out.cont = cont;
        self.last_lparen = lp;
        self.prev_fn = pf;
        w
    }

    fn has_newline(&self, n: SyntaxNode<'a>) -> bool {
        all_tokens(n)
            .iter()
            .any(|t| t.kind() == TokenKind::Whitespace && t.text().contains('\n'))
    }

    /// The query of a `( .. )`: on the line of the parenthesis when it is
    /// short, else its clauses on lines of their own under the first word.
    fn subquery_body(&mut self, q: SyntaxNode<'a>) {
        if self.out.flat > 0 {
            return self.block(q, 0);
        }
        if self.style.collapse_short_statement != Collapse::Never
            && self.try_flat(1, |s| s.block(q, 0))
        {
            return;
        }
        self.out.clear_opps();
        let base = self.out.col();
        self.block(q, base);
    }

    /// The clauses of a query or data modification statement, at column `base`.
    fn block(&mut self, n: SyntaxNode<'a>, base: usize) {
        if n.kind() == K::Values {
            return self.clause(n, base);
        }
        let saved = std::mem::replace(&mut self.base, base);
        let saved_cont = std::mem::replace(&mut self.out.cont, base);
        let kind = n.kind();
        let mut first = true;
        let mut after_node = false;
        for e in n.children() {
            match e {
                El::Token(t) if t.is_trivia() => self.tok(t),
                El::Token(t) => {
                    if kind == K::SetOperation && after_node {
                        self.out.request(Brk::at(base));
                    }
                    after_node = false;
                    first = false;
                    self.tok(t);
                }
                El::Node(c) => {
                    self.block_child(kind, c, base, first);
                    first = false;
                    after_node = true;
                }
            }
        }
        self.base = saved;
        self.out.cont = saved_cont;
    }

    fn block_child(&mut self, parent: K, c: SyntaxNode<'a>, base: usize, first: bool) {
        let st = self.style;
        match c.kind() {
            K::SelectList => {
                let new_line = st.clause_elements == ClauseElements::NewLine;
                let align = if new_line {
                    base + st.indent
                } else {
                    base + "select".len() + 1
                };
                let policy = if st.select_items == SelectItems::OnePerLine {
                    Policy::OnePerLine
                } else {
                    Policy::Wrap
                };
                let kids: Vec<El<'a>> = c.children().collect();
                self.list_items(&kids, align, policy, new_line);
            }
            K::WithClause => self.clause(c, base),
            K::Select | K::Query | K::SetOperation | K::Values | K::TableQuery => {
                if !first {
                    self.out.request(Brk::at(base));
                }
                self.block(c, base);
            }
            K::ParenQuery => {
                if !first {
                    self.out.request(Brk::at(base));
                }
                self.node(c);
            }
            K::MergeClause => {
                self.out.request(Brk::at(base));
                self.merge_clause(c, base);
            }
            K::AlterAction => {
                self.out.request(Brk::at(base + st.indent));
                self.node(c);
            }
            K::SetClause if parent == K::MergeUpdate => {
                self.clause(c, base + "update".len() + 1);
            }
            k if breaks_line(k, parent) => {
                self.out.request(Brk::at(base));
                self.clause(c, base);
            }
            _ => self.node(c),
        }
    }

    fn merge_clause(&mut self, c: SyntaxNode<'a>, base: usize) {
        let col = base + self.style.indent;
        for e in c.children() {
            match e {
                El::Node(a)
                    if matches!(a.kind(), K::MergeUpdate | K::MergeDelete | K::MergeInsert) =>
                {
                    self.out.request(Brk::at(col));
                    self.block(a, col);
                }
                other => self.el(other),
            }
        }
    }

    /// A clause: its keywords, then the elements.
    fn clause(&mut self, c: SyntaxNode<'a>, base: usize) {
        let st = self.style;
        let kids: Vec<El<'a>> = c.children().collect();
        let mut i = 0;
        let mut kw = 0usize;
        while i < kids.len() {
            match kids[i] {
                El::Token(t) if t.is_trivia() => self.tok(t),
                El::Token(t) if t.kind() == TokenKind::Word => {
                    kw += usize::from(kw > 0) + t.text().len();
                    self.tok(t);
                }
                _ => break,
            }
            i += 1;
        }
        let new_line = st.clause_elements == ClauseElements::NewLine;
        let align = if new_line {
            base + st.indent
        } else {
            base + kw + 1
        };
        let saved = self.out.cont;
        match role(c.kind()) {
            Role::Cond => {
                for &e in &kids[i..] {
                    match e {
                        El::Node(x) => {
                            if new_line {
                                self.out.request(Brk::at(base + st.indent));
                            }
                            self.cond(x, base + st.indent);
                        }
                        other => self.el(other),
                    }
                }
            }
            Role::List(policy) => self.list_items(&kids[i..], align, policy, new_line),
            Role::Flow => {
                self.out.cont = base + st.continuation_indent;
                for &e in &kids[i..] {
                    self.el(e);
                }
            }
        }
        self.out.cont = saved;
    }

    fn leading_commas(&self, kids: &[El<'a>]) -> bool {
        match self.style.comma {
            CommaPlacement::Leading => true,
            CommaPlacement::Trailing => false,
            CommaPlacement::Auto => kids.windows(2).any(|w| {
                matches!((w[0], w[1]), (El::Token(a), El::Token(b))
                    if a.kind() == TokenKind::Whitespace
                        && a.text().contains('\n')
                        && b.kind() == TokenKind::Comma)
            }),
        }
    }

    /// Items separated by commas, the later ones under the first (`align`).
    fn list_items(&mut self, kids: &[El<'a>], align: usize, policy: Policy, first_break: bool) {
        let st = self.style;
        let saved = self.out.cont;
        let leading = self.leading_commas(kids);
        let mut after_comma = false;
        let mut first = true;
        let mut deferred: Option<SyntaxToken<'a>> = None;
        for &e in kids {
            match e {
                El::Token(t) if t.is_trivia() => self.tok(t),
                El::Token(t) if t.kind() == TokenKind::Comma => {
                    if leading {
                        deferred = Some(t);
                    } else {
                        self.out.cont = align + st.indent;
                        self.tok(t);
                        // A line that is too long breaks into a new item.
                        self.out.opp_next = Some(align);
                        // Comments before the next item stand where it will.
                        self.out.cont = align;
                    }
                    after_comma = true;
                }
                El::Token(t) => self.tok(t),
                El::Node(c) => {
                    let want = self.out.flat == 0
                        && ((first && first_break)
                            || (after_comma
                                && (policy == Policy::OnePerLine
                                    || self.out.line_comment
                                    || (st.keep_line_breaks && self.out.nl_seen > 0))));
                    if let Some(comma) = deferred.take() {
                        if want {
                            self.out.request(Brk::at(align.saturating_sub(2)));
                        } else {
                            // A line that is too long breaks before the comma.
                            self.out.opp_next = Some(align.saturating_sub(2));
                        }
                        self.tok(comma);
                        self.out.opp_next = None;
                    } else if want {
                        self.out.request(Brk::at(align));
                    }
                    self.out.cont = align + st.indent;
                    if c.kind() == K::Join {
                        self.join(c);
                    } else {
                        self.node(c);
                    }
                    after_comma = false;
                    first = false;
                }
            }
        }
        if let Some(comma) = deferred {
            self.tok(comma);
        }
        self.out.cont = saved;
    }

    fn join(&mut self, c: SyntaxNode<'a>) {
        let col = self.base + self.style.indent;
        let mut left_seen = false;
        let mut broke = false;
        for e in c.children() {
            match e {
                El::Token(t) if t.is_trivia() => self.tok(t),
                El::Token(t) => {
                    if left_seen && !broke {
                        self.out.request(Brk::at(col));
                        broke = true;
                    }
                    self.tok(t);
                }
                El::Node(k) => match k.kind() {
                    K::Join => {
                        self.join(k);
                        left_seen = true;
                    }
                    K::OnClause => self.on_clause(k, col),
                    _ => {
                        self.node(k);
                        left_seen = true;
                    }
                },
            }
        }
    }

    fn on_clause(&mut self, c: SyntaxNode<'a>, col: usize) {
        let saved = self.out.cont;
        self.out.cont = self.base + self.style.continuation_indent;
        for e in c.children() {
            match e {
                El::Node(x) => self.cond(x, col + self.style.indent),
                other => self.el(other),
            }
        }
        self.out.cont = saved;
    }

    /// A condition: `AND` / `OR` of the top level go to a line of their own
    /// at `col` where the text had a break (or, with `keep_line_breaks` off,
    /// where the condition does not fit a line).
    fn cond(&mut self, e: SyntaxNode<'a>, col: usize) {
        let saved = (self.out.cont, self.chain_break);
        self.out.cont = self.base + self.style.continuation_indent;
        let mut done = false;
        if !self.style.keep_line_breaks && self.out.flat == 0 {
            if self.try_flat(0, |s| s.cond_inner(e, col)) {
                done = true;
            } else {
                self.chain_break = true;
            }
        } else {
            self.chain_break = false;
        }
        if !done {
            self.cond_inner(e, col);
        }
        self.out.cont = saved.0;
        self.chain_break = saved.1;
    }

    fn cond_inner(&mut self, e: SyntaxNode<'a>, col: usize) {
        let has_op = e.kind() == K::BinaryExpr && e.child_tokens().any(|t| is_and_or(&t));
        if !has_op {
            return self.node(e);
        }
        for x in e.children() {
            match x {
                El::Token(t) if is_and_or(&t) => self.and_or(t, col),
                El::Token(t) => self.tok(t),
                El::Node(c) => self.cond_inner(c, col),
            }
        }
    }

    fn and_or(&mut self, t: SyntaxToken<'a>, col: usize) {
        let flat = self.out.flat > 0;
        let keep = self.style.keep_line_breaks;
        let want = !flat
            && if keep {
                self.out.nl_seen > 0
            } else {
                self.chain_break
            };
        match self.style.and_or {
            AndOr::LineStart => {
                if want {
                    self.out.request(Brk::at(col));
                } else {
                    self.out.opp_next = Some(col);
                }
                self.tok(t);
            }
            AndOr::LineEnd => {
                self.tok(t);
                if want {
                    self.out.request(Brk::at(col));
                } else if keep && !flat {
                    self.out.hold = Some(col);
                }
                self.out.opp_next = Some(col);
            }
        }
    }

    fn case_expr(&mut self, n: SyntaxNode<'a>) {
        if self.out.flat > 0 {
            return self.flow(n);
        }
        let multi = self.style.keep_line_breaks && self.has_newline(n);
        if !multi && self.try_flat(0, |s| s.flow(n)) {
            return;
        }
        self.out.clear_opps();
        let indent = self.style.indent;
        let saved = self.out.cont;
        let mut case_col: Option<usize> = None;
        for e in n.children() {
            match e {
                El::Token(t) if t.is_trivia() => self.tok(t),
                El::Token(t) => {
                    self.tok(t);
                    if case_col.is_none() {
                        case_col = Some(self.out.col().saturating_sub(t.text().len()));
                    }
                }
                El::Node(c) if matches!(c.kind(), K::WhenClause | K::ElseClause) => {
                    let col = case_col.unwrap_or(0) + indent;
                    self.out.request(Brk::at(col));
                    self.out.cont = col + indent;
                    self.flow(c);
                    self.out.cont = saved;
                }
                El::Node(c) => self.node(c),
            }
        }
    }

    fn create_table(&mut self, n: SyntaxNode<'a>) {
        let st = self.style;
        let kids: Vec<El<'a>> = n.children().collect();
        let is = |e: &El<'a>, k: TokenKind| matches!(e, El::Token(t) if t.kind() == k);
        let Some(lp) = kids.iter().position(|e| is(e, TokenKind::LParen)) else {
            return self.flow(n);
        };
        let Some(rp) = kids[lp..]
            .iter()
            .position(|e| is(e, TokenKind::RParen))
            .map(|i| i + lp)
        else {
            return self.flow(n);
        };
        for &e in &kids[..lp] {
            self.el(e);
        }
        let (mut name_w, mut type_w) = (0, 0);
        if st.align_ddl_columns {
            for &e in &kids[lp..rp] {
                if let El::Node(c) = e
                    && c.kind() == K::ColumnDef
                    && let (Some(name), Some(ty)) = (c.child(K::DefName), c.child(K::TypeRef))
                {
                    name_w = name_w.max(text_width(name.text()));
                    type_w = type_w.max(self.measure(ty));
                }
            }
        }
        self.out.request(Brk::at(0));
        let saved = self.out.cont;
        if let El::Token(t) = kids[lp] {
            self.tok(t);
        }
        for &e in &kids[lp + 1..rp] {
            match e {
                El::Node(c) => {
                    self.out.request(Brk::at(st.indent));
                    self.out.cont = st.indent + st.indent;
                    if c.kind() == K::ColumnDef && st.align_ddl_columns {
                        self.column_def(c, name_w, type_w);
                    } else {
                        self.node(c);
                    }
                }
                other => self.el(other),
            }
        }
        self.out.cont = saved;
        self.out.request(Brk::at(0));
        if let El::Token(t) = kids[rp] {
            self.tok(t);
        }
        for &e in &kids[rp + 1..] {
            self.el(e);
        }
    }

    /// `name type constraints` with the type and the constraints in columns.
    fn column_def(&mut self, c: SyntaxNode<'a>, name_w: usize, type_w: usize) {
        let ind = self.style.indent;
        let kids: Vec<El<'a>> = c.children().collect();
        let significant: Vec<usize> = (0..kids.len()).filter(|&i| !is_trivia(&kids[i])).collect();
        let aligned = matches!(
            (significant.first().map(|&i| kids[i]), significant.get(1).map(|&i| kids[i])),
            (Some(El::Node(a)), Some(El::Node(b)))
                if a.kind() == K::DefName && b.kind() == K::TypeRef
        );
        if !aligned {
            return self.flow(c);
        }
        for (i, &e) in kids.iter().enumerate() {
            self.el(e);
            if i == significant[0] && significant.len() > 1 {
                self.out.pad_to(ind + name_w + 1);
            } else if i == significant[1] && significant.len() > 2 {
                self.out.pad_to(ind + name_w + 1 + type_w + 1);
            }
        }
    }
}
