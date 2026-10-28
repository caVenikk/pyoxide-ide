//! The SQL highlighter (ADR 0022, "Highlighting"): the editor's
//! [`Highlighter`] with two layers. The lexical layer colours each line at
//! once from a state carried from the line before (comments, strings and
//! `$tag$` bodies that run over lines); a background pass parses the text
//! with `sql-syntax` and gives the names their roles (table, column, alias,
//! schema, function) from the tree, which are painted over the lexical
//! colours. Edits clear the roles of the lines they touch, so the lexical
//! colours stay right while the pass runs and typing never waits for it.
//! The semantic layer (unresolved names) is the analysis' diagnostics.

use std::any::Any;
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use editor::{Highlighter, Job, JobResult, Span};
use sql_syntax::{Dialect, LineLexer, Script, SyntaxKind, Token, TokenKind};
use syntax::{LexSpan, LineStates, MAX_HIGHLIGHT_LINE};
use text::{AppliedEdit, Buffer, Rope};
use themes::TextRole;

use super::hub::Hub;

/// The lexical layer of a dialect as a `syntax::Lexer`: the highlighter
/// uses it per line, and documentation code blocks use it for `sql`.
pub struct SqlLexer {
    lines: LineLexer,
}

impl SqlLexer {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            lines: LineLexer::new(dialect),
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.lines.dialect()
    }

    /// The role of a token, `None` for plain text.
    fn role(&self, token: &Token, line: &str) -> Option<TextRole> {
        Some(match token.kind {
            TokenKind::Word => {
                let word = &line[token.range()];
                if word.eq_ignore_ascii_case("null")
                    || word.eq_ignore_ascii_case("true")
                    || word.eq_ignore_ascii_case("false")
                {
                    TextRole::ConstantBuiltin
                } else if self.lines.dialect().keyword(word).is_some() {
                    TextRole::Keyword
                } else {
                    return None;
                }
            }
            TokenKind::Number => TextRole::Number,
            TokenKind::String => TextRole::String,
            TokenKind::LineComment | TokenKind::BlockComment => TextRole::Comment,
            TokenKind::Parameter => TextRole::Parameter,
            TokenKind::Operator => TextRole::Operator,
            TokenKind::Semicolon | TokenKind::Comma | TokenKind::Dot | TokenKind::SlashLine => {
                TextRole::Punctuation
            }
            TokenKind::LParen
            | TokenKind::RParen
            | TokenKind::LBracket
            | TokenKind::RBracket
            | TokenKind::LBrace
            | TokenKind::RBrace => TextRole::Bracket,
            TokenKind::Whitespace
            | TokenKind::QuotedIdent
            | TokenKind::Hole
            | TokenKind::Unknown => return None,
        })
    }
}

impl syntax::Lexer for SqlLexer {
    fn initial_state(&self) -> u32 {
        self.lines.initial_state()
    }

    fn lex_line(&self, line: &str, state: u32, out: &mut Vec<LexSpan>) -> u32 {
        let mut tokens = Vec::new();
        let next = self.lines.lex_line(line, state, &mut tokens);
        for token in &tokens {
            if let Some(role) = self.role(token, line) {
                out.push((token.range(), role));
            }
        }
        next
    }
}

/// A line's spans in chars from the start of the line, from spans in bytes.
fn to_chars(line: &str, spans: &[LexSpan], out: &mut Vec<Span>) {
    if line.is_ascii() {
        out.extend(spans.iter().cloned());
        return;
    }
    let mut chars = 0;
    let mut bytes = 0;
    let mut at = |target: usize| {
        for c in line[bytes..].chars() {
            if bytes >= target {
                break;
            }
            bytes += c.len_utf8();
            chars += 1;
        }
        chars
    };
    for (range, role) in spans {
        let start = at(range.start);
        let end = at(range.end);
        out.push((start..end, *role));
    }
}

/// `base` with the spans of `top` painted over it (both sorted and not
/// overlapping).
fn overlay(base: Vec<Span>, top: &[Span]) -> Vec<Span> {
    if top.is_empty() {
        return base;
    }
    let mut out = Vec::with_capacity(base.len() + top.len());
    for (range, role) in base {
        let mut start = range.start;
        for (t, _) in top
            .iter()
            .filter(|(t, _)| t.start < range.end && t.end > range.start)
        {
            if t.start > start {
                out.push((start..t.start, role));
            }
            start = start.max(t.end);
        }
        if start < range.end {
            out.push((start..range.end, role));
        }
    }
    out.extend(top.iter().cloned());
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// The role of a name token from its place in the tree.
fn tree_role(kind: SyntaxKind, parent_of_def: Option<SyntaxKind>) -> Option<TextRole> {
    Some(match kind {
        SyntaxKind::TableRef | SyntaxKind::CteDef => TextRole::SqlTable,
        SyntaxKind::ColumnRef => TextRole::SqlColumn,
        SyntaxKind::SchemaRef | SyntaxKind::CatalogRef => TextRole::SqlSchema,
        SyntaxKind::AliasDef | SyntaxKind::WindowDef | SyntaxKind::WindowRef => TextRole::SqlAlias,
        SyntaxKind::FunctionRef => TextRole::FunctionCall,
        SyntaxKind::TypeRef => TextRole::Keyword,
        SyntaxKind::DefName => match parent_of_def? {
            SyntaxKind::ColumnDef | SyntaxKind::IndexElement => TextRole::SqlColumn,
            _ => return None,
        },
        _ => return None,
    })
}

/// The roles of the names of `text`, per line of `rope`, in chars.
fn tree_roles(rope: &Rope, text: &str, dialect: Dialect) -> Vec<Vec<Span>> {
    let script = Script::parse(text, dialect);
    let mut lines: Vec<Vec<Span>> = vec![Vec::new(); rope.len_lines()];
    for statement in script.statements() {
        // Names that qualify a column and are aliases of this statement.
        let aliases: HashSet<String> = statement
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::AliasDef)
            .map(|n| n.text().trim().to_lowercase())
            .collect();
        for element in statement.preorder() {
            let Some(token) = element.as_token() else {
                continue;
            };
            if !matches!(token.kind(), TokenKind::Word | TokenKind::QuotedIdent) {
                continue;
            }
            let Some(parent) = token.parent() else {
                continue;
            };
            let Some(mut role) = tree_role(parent.kind(), parent.parent().map(|p| p.kind())) else {
                continue;
            };
            if role == TextRole::SqlTable && is_qualifier(&parent) {
                let name = token.text().trim_matches('"').to_lowercase();
                if aliases.contains(&name) {
                    role = TextRole::SqlAlias;
                }
            }
            let range = token.range();
            if range.end > text.len() {
                continue;
            }
            let line = rope.byte_to_line(range.start);
            let line_end = rope.line_to_byte(line + 1).min(text.len());
            if range.end > line_end {
                continue;
            }
            let first = rope.line_to_char(line);
            let start = rope.byte_to_char(range.start) - first;
            let end = rope.byte_to_char(range.end) - first;
            if let Some(spans) = lines.get_mut(line) {
                spans.push((start..end, role));
            }
        }
    }
    for spans in &mut lines {
        spans.sort_by_key(|(r, _)| r.start);
        spans.dedup_by(|b, a| a.0.start == b.0.start);
    }
    lines
}

/// A table reference that is the qualifier of a column (`c` in `c.name`).
fn is_qualifier(node: &sql_syntax::SyntaxNode<'_>) -> bool {
    node.parent()
        .is_some_and(|p| p.kind() == SyntaxKind::ColumnRef)
}

/// The highlighter of an editor of a SQL file.
pub struct SqlHighlighter {
    hub: Arc<Hub>,
    path: Option<PathBuf>,
    /// The hub's epoch when the dialect was read.
    epoch: u64,
    lexer: Arc<SqlLexer>,
    states: LineStates,
    /// The roles of the names, per line; may be shorter than the text.
    roles: Vec<Vec<Span>>,
    /// A parse is due.
    dirty: bool,
    running: Option<Arc<AtomicBool>>,
    /// Edits made while the running pass works on older text, as
    /// (first row, last row before the edit, last row after it).
    during_job: Vec<(usize, usize, usize)>,
    cache: Vec<Vec<Span>>,
    cache_start: usize,
    cache_valid: bool,
}

type Pass = (Vec<Vec<Span>>, LineStates);

impl SqlHighlighter {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self {
            epoch: hub.epoch(),
            hub,
            path: None,
            lexer: Arc::new(SqlLexer::new(Dialect::Generic)),
            states: LineStates::default(),
            roles: Vec::new(),
            dirty: true,
            running: None,
            during_job: Vec::new(),
            cache: Vec::new(),
            cache_start: 0,
            cache_valid: false,
        }
    }

    /// A factory of highlighters over `hub`.
    pub fn factory(hub: Arc<Hub>) -> impl Fn() -> Box<dyn Highlighter> + Send + Sync {
        move || Box::new(Self::new(hub.clone()))
    }

    pub fn dialect(&self) -> Dialect {
        self.lexer.dialect()
    }

    /// Reads the file's dialect again; everything is re-coloured when it
    /// changed.
    fn refresh_context(&mut self) {
        self.epoch = self.hub.epoch();
        let dialect = match &self.path {
            Some(path) => self.hub.dialect(path),
            None => Dialect::Generic,
        };
        if dialect != self.lexer.dialect() {
            self.lexer = Arc::new(SqlLexer::new(dialect));
            self.invalidate();
        }
    }

    fn invalidate(&mut self) {
        if let Some(flag) = self.running.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.states.clear();
        self.roles.clear();
        self.during_job.clear();
        self.dirty = true;
        self.cache_valid = false;
    }

    fn splice(roles: &mut Vec<Vec<Span>>, first: usize, old_last: usize, new_last: usize) {
        if first >= roles.len() {
            return;
        }
        let end = (old_last + 1).min(roles.len());
        roles.splice(first..end, vec![Vec::new(); new_last - first + 1]);
    }
}

impl Highlighter for SqlHighlighter {
    fn set_path(&mut self, path: Option<&Path>) {
        self.path = path.map(Path::to_owned);
        self.refresh_context();
    }

    fn reset(&mut self, _: &Buffer) {
        self.refresh_context();
        self.invalidate();
    }

    fn edited(&mut self, _: &Buffer, edits: &[AppliedEdit]) {
        for e in edits {
            self.states.invalidate_after(e.start_position.0);
            let span = (
                e.start_position.0,
                e.old_end_position.0.max(e.start_position.0),
                e.new_end_position.0.max(e.start_position.0),
            );
            Self::splice(&mut self.roles, span.0, span.1, span.2);
            if self.running.is_some() {
                self.during_job.push(span);
            }
        }
        self.dirty = true;
        self.cache_valid = false;
    }

    fn next_job(&mut self, buffer: &Buffer) -> Option<Job> {
        if self.running.is_some() || !self.dirty {
            return None;
        }
        self.dirty = false;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some(cancel.clone());
        self.during_job.clear();
        let rope = buffer.rope().clone();
        let lexer = self.lexer.clone();
        // The lexer states of every line are known by the time the pass
        // ends, so the UI thread never lexes a long stretch itself.
        let mut states = self.states.clone();
        Some(Box::new(move || {
            let last = rope.len_lines().saturating_sub(1);
            states.ensure(&*lexer, &rope, last);
            let result: Option<Pass> = if cancel.load(Ordering::Relaxed) {
                None
            } else {
                let text = rope.to_string();
                Some((tree_roles(&rope, &text, lexer.dialect()), states))
            };
            Box::new(result) as JobResult
        }))
    }

    fn job_done(&mut self, result: JobResult, _: &Buffer) {
        let cancelled = self
            .running
            .take()
            .is_none_or(|flag| flag.load(Ordering::Relaxed));
        if cancelled {
            return;
        }
        let result: Box<dyn Any + Send> = result;
        let Ok(result) = result.downcast::<Option<Pass>>() else {
            return;
        };
        let Some((mut roles, mut states)) = *result else {
            return;
        };
        for (first, old_last, new_last) in self.during_job.drain(..) {
            Self::splice(&mut roles, first, old_last, new_last);
        }
        if let Some(first) = self.during_job.first() {
            states.invalidate_after(first.0);
        }
        self.roles = roles;
        if states.len() > self.states.len() {
            self.states = states;
        }
        self.cache_valid = false;
    }

    fn prepare(&mut self, buffer: &Buffer, lines: Range<usize>) {
        if self.hub.epoch() != self.epoch {
            let before = self.lexer.dialect();
            self.refresh_context();
            if before == self.lexer.dialect() {
                self.cache_valid = false;
            }
        }
        let total = buffer.len_lines();
        let lines = lines.start.min(total)..lines.end.min(total);
        let cached = self.cache_start..self.cache_start + self.cache.len();
        if self.cache_valid && cached.start <= lines.start && lines.end <= cached.end {
            return;
        }
        self.cache.clear();
        self.cache_start = lines.start;
        self.cache_valid = true;
        if lines.is_empty() {
            return;
        }
        let rope = buffer.rope();
        self.states.ensure(&*self.lexer, rope, lines.end - 1);
        let mut state = self.states.ensure(&*self.lexer, rope, lines.start);
        let mut spans = Vec::new();
        for line in lines {
            let text = buffer.line_text(line);
            spans.clear();
            let next = syntax::Lexer::lex_line(&*self.lexer, &text, state, &mut spans);
            state = next;
            let mut chars = Vec::with_capacity(spans.len());
            if text.len() <= MAX_HIGHLIGHT_LINE {
                to_chars(&text, &spans, &mut chars);
            }
            let top = self.roles.get(line).map(Vec::as_slice).unwrap_or(&[]);
            self.cache.push(overlay(chars, top));
        }
    }

    fn line_spans(&self, line: usize, out: &mut Vec<Span>) {
        if let Some(spans) = line
            .checked_sub(self.cache_start)
            .and_then(|i| self.cache.get(i))
        {
            out.extend_from_slice(spans);
        }
    }

    fn is_current(&self) -> bool {
        !self.dirty && self.running.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::fake::FakeStore;
    use syntax::Lexer as _;

    fn hub() -> Arc<Hub> {
        Arc::new(Hub::new(FakeStore::new("pw")))
    }

    fn lex(dialect: Dialect, text: &str) -> Vec<(String, TextRole)> {
        let lexer = SqlLexer::new(dialect);
        let mut state = lexer.initial_state();
        let mut out = Vec::new();
        for line in text.split('\n') {
            let mut spans = Vec::new();
            state = lexer.lex_line(line, state, &mut spans);
            out.extend(
                spans
                    .into_iter()
                    .map(|(r, role)| (line[r].to_string(), role)),
            );
        }
        out
    }

    #[test]
    fn the_lexical_layer_colours_keywords_literals_and_comments() {
        let got = lex(
            Dialect::Postgres,
            "select 1, 'a' -- c\nfrom t where x is null /* b\nc */;",
        );
        let role = |text: &str| got.iter().find(|(t, _)| t == text).map(|(_, r)| *r);
        assert_eq!(role("select"), Some(TextRole::Keyword));
        assert_eq!(role("1"), Some(TextRole::Number));
        assert_eq!(role("'a'"), Some(TextRole::String));
        assert_eq!(role("-- c"), Some(TextRole::Comment));
        assert_eq!(role("null"), Some(TextRole::ConstantBuiltin));
        assert_eq!(role("c */"), Some(TextRole::Comment));
        assert_eq!(role("t"), None);
    }

    #[test]
    fn spans_are_in_chars() {
        let mut out = Vec::new();
        to_chars("é select", &[(3..9, TextRole::Keyword)], &mut out);
        assert_eq!(out, vec![(2..8, TextRole::Keyword)]);
    }

    #[test]
    fn roles_replace_the_lexical_colours_they_cover() {
        let base = vec![(0..6, TextRole::Keyword), (7..13, TextRole::Keyword)];
        let top = [(7..13, TextRole::SqlColumn), (14..15, TextRole::SqlAlias)];
        assert_eq!(
            overlay(base, &top),
            vec![
                (0..6, TextRole::Keyword),
                (7..13, TextRole::SqlColumn),
                (14..15, TextRole::SqlAlias),
            ]
        );
        let split = overlay(
            vec![(0..10, TextRole::Comment)],
            &[(3..5, TextRole::SqlTable)],
        );
        assert_eq!(
            split,
            vec![
                (0..3, TextRole::Comment),
                (3..5, TextRole::SqlTable),
                (5..10, TextRole::Comment),
            ]
        );
    }

    #[test]
    fn the_tree_names_tables_columns_aliases_and_schemas() {
        let text = "select c.name, status from public.customers c where c.id = 1";
        let rope = Rope::from_str(text);
        let roles = tree_roles(&rope, text, Dialect::Postgres);
        let at = |needle: &str| {
            let start = text.find(needle).unwrap();
            roles[0]
                .iter()
                .find(|(r, _)| r.start == start)
                .map(|(_, role)| *role)
        };
        assert_eq!(at("public"), Some(TextRole::SqlSchema));
        assert_eq!(at("customers"), Some(TextRole::SqlTable));
        assert_eq!(at("name"), Some(TextRole::SqlColumn));
        assert_eq!(at("status"), Some(TextRole::SqlColumn));
        // The alias after the table, and its use as a qualifier.
        assert_eq!(at("c where"), Some(TextRole::SqlAlias));
        assert_eq!(at("c.name"), Some(TextRole::SqlAlias));
    }

    fn prepared(h: &mut SqlHighlighter, buffer: &Buffer, line: usize) -> Vec<Span> {
        h.prepare(buffer, line..line + 1);
        let mut out = Vec::new();
        h.line_spans(line, &mut out);
        out
    }

    #[test]
    fn a_line_is_coloured_before_the_pass_and_named_after_it() {
        let buffer = Buffer::new("select name from customers\n");
        let mut h = SqlHighlighter::new(hub());
        h.reset(&buffer);
        let before = prepared(&mut h, &buffer, 0);
        assert!(before.contains(&(0..6, TextRole::Keyword)));
        // `name` is a keyword for the lexer until the tree says it is a column.
        let job = h.next_job(&buffer).expect("a pass is due");
        assert!(!h.is_current());
        let result = job();
        h.job_done(result, &buffer);
        assert!(h.is_current());
        let after = prepared(&mut h, &buffer, 0);
        assert!(after.contains(&(7..11, TextRole::SqlColumn)), "{after:?}");
        assert!(after.contains(&(17..26, TextRole::SqlTable)), "{after:?}");
    }

    #[test]
    fn an_edit_clears_the_roles_of_its_lines_only() {
        let mut buffer = Buffer::new("select a from t;\nselect b from u;\n");
        let mut h = SqlHighlighter::new(hub());
        h.reset(&buffer);
        let job = h.next_job(&buffer).unwrap();
        h.job_done(job(), &buffer);
        assert!(!prepared(&mut h, &buffer, 1).is_empty());
        let edit = buffer.replace_untracked(7..8, "x");
        h.edited(&buffer, &[edit]);
        let first = prepared(&mut h, &buffer, 0);
        assert!(!first.iter().any(|(_, r)| *r == TextRole::SqlColumn));
        let second = prepared(&mut h, &buffer, 1);
        assert!(second.iter().any(|(_, r)| *r == TextRole::SqlColumn));
    }

    /// 5000 lines of typical SQL: the visible window is coloured at once
    /// and the whole lexical pass (what the background job does first)
    /// takes at most 5 ms (docs/performance.md). Timing needs a release
    /// build.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing needs a release build")]
    fn a_5000_line_file_is_lexed_within_the_budget() {
        let mut text = String::new();
        for i in 0..1000 {
            text.push_str(&format!(
                "select c.id, c.name, 'x{i}' as tag -- row {i}\nfrom public.customers c\n\
                 join orders o on o.customer_id = c.id /* join\nnote */\nwhere c.id > {i};\n"
            ));
        }
        let buffer = Buffer::new(&text);
        assert!(buffer.len_lines() >= 5000);
        let mut h = SqlHighlighter::new(hub());
        h.reset(&buffer);
        let start = std::time::Instant::now();
        h.prepare(&buffer, 0..60);
        let window = start.elapsed();
        let mut states = LineStates::default();
        let lexer = SqlLexer::new(Dialect::Generic);
        let start = std::time::Instant::now();
        states.ensure(&lexer, buffer.rope(), buffer.len_lines() - 1);
        let whole = start.elapsed();
        assert!(window.as_micros() < 1000, "visible window: {window:?}");
        assert!(whole.as_millis() <= 5, "whole pass: {whole:?}");
    }
}
