//! The analysis of a document: every statement resolved and inspected, the
//! results of a statement kept while its text, the objects the statements
//! before it create and the scope stay the same. An edit of one statement
//! re-analyses that statement only; the statements before the edit come
//! from the cache, so the edited statement is the first one analysed.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use db_model::Model;
use sql_syntax::{Script, SyntaxNode};

use crate::context::SqlContext;
use crate::inspect::{Problem, inspect, shift_range};
use crate::locals::{LocalIndex, LocalObject};
use crate::profile::{Inspection, InspectionProfile};
use crate::reference::{Reference, Resolution, Target};
use crate::resolve::{Env, Resolver};

/// The results of one parse unit, with ranges relative to its start.
#[derive(Debug)]
struct UnitResults {
    refs: Vec<Reference>,
    problems: Vec<Problem>,
    defs: Vec<LocalObject>,
}

#[derive(Debug, Clone)]
struct UnitEntry {
    /// The unit in the document.
    unit: Range<usize>,
    /// From its first to its last significant token.
    statement: Range<usize>,
    results: Arc<UnitResults>,
    /// The inspections a `-- noinspection` comment before the statement
    /// turns off.
    suppressed: Vec<String>,
}

/// The resolution and the problems of a document at one version.
#[derive(Debug, Clone, Default)]
pub struct DocumentAnalysis {
    units: Vec<UnitEntry>,
    /// Inspections turned off for the file (`-- noinspection <id>ForFile`).
    file_suppressed: Vec<String>,
}

fn shift_target(t: &mut Target, delta: isize) {
    match t {
        Target::Local(r) => *r = shift_range(r, delta),
        Target::SourceColumn { source, .. } => *source = shift_range(source, delta),
        _ => {}
    }
}

fn shift_reference(r: &mut Reference, delta: isize) {
    r.range = shift_range(&r.range, delta);
    match &mut r.resolution {
        Resolution::Resolved(t) => shift_target(t, delta),
        Resolution::Ambiguous(ts) => ts.iter_mut().for_each(|t| shift_target(t, delta)),
        _ => {}
    }
}

impl DocumentAnalysis {
    /// Every reference of the document, in order.
    pub fn references(&self) -> Vec<Reference> {
        let mut out = Vec::new();
        for u in &self.units {
            for r in &u.results.refs {
                let mut r = r.clone();
                shift_reference(&mut r, u.unit.start as isize);
                out.push(r);
            }
        }
        out
    }

    /// The innermost reference whose name contains `offset` (or ends at it).
    pub fn reference_at(&self, offset: usize) -> Option<Reference> {
        let u = self.unit_at(offset)?;
        let rel = offset.checked_sub(u.unit.start)?;
        u.results
            .refs
            .iter()
            .find(|r| r.range.start <= rel && rel <= r.range.end)
            .map(|r| {
                let mut r = r.clone();
                shift_reference(&mut r, u.unit.start as isize);
                r
            })
    }

    fn unit_at(&self, offset: usize) -> Option<&UnitEntry> {
        let i = self
            .units
            .partition_point(|u| u.unit.start <= offset)
            .checked_sub(1)?;
        self.units.get(i)
    }

    /// Every problem, without the inspections the profile turns off and
    /// those suppressed by comments.
    pub fn problems(&self, profile: &InspectionProfile) -> Vec<Problem> {
        let mut out = Vec::new();
        for u in &self.units {
            for p in &u.results.problems {
                if !profile.is_enabled(p.inspection) || self.is_suppressed(u, p.inspection) {
                    continue;
                }
                let mut p = p.clone();
                p.shift(u.unit.start as isize);
                out.push(p);
            }
        }
        out
    }

    /// Every problem the inspections found, the profile and suppression
    /// comments aside.
    pub fn all_problems(&self) -> Vec<Problem> {
        self.problems_where(|_, _| true)
    }

    fn problems_where(&self, keep: impl Fn(&UnitEntry, &Problem) -> bool) -> Vec<Problem> {
        let mut out = Vec::new();
        for u in &self.units {
            for p in u.results.problems.iter().filter(|p| keep(u, p)) {
                let mut p = p.clone();
                p.shift(u.unit.start as isize);
                out.push(p);
            }
        }
        out
    }

    fn is_suppressed(&self, u: &UnitEntry, inspection: Inspection) -> bool {
        if !inspection.suppressible() {
            return false;
        }
        let id = inspection.id();
        u.suppressed.iter().any(|s| s == id || s == "ALL")
            || self.file_suppressed.iter().any(|s| s == id || s == "ALL")
    }

    /// The statement (first to last significant token) at `offset`.
    pub fn statement_at(&self, offset: usize) -> Option<Range<usize>> {
        self.unit_at(offset).map(|u| u.statement.clone())
    }

    /// The number of statements.
    pub fn statement_count(&self) -> usize {
        self.units.len()
    }
}

/// The line comments of a piece of trivia: offset and text.
fn line_comments(gap: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in gap.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        let body = trimmed.trim_end_matches(['\n', '\r']);
        if body.starts_with("--") {
            out.push((offset + lead, body));
        }
        offset += line.len();
    }
    out
}

/// The ids of a `-- noinspection A, B` comment.
fn noinspection(comment: &str) -> Option<Vec<String>> {
    let rest = comment.strip_prefix("--")?.trim_start();
    let rest = rest.strip_prefix("noinspection")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(
        rest.split([',', ' ', '\t'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// The range from the first to the last significant token of a unit.
fn statement_range(root: SyntaxNode<'_>) -> Range<usize> {
    let mut first = None;
    let mut last = root.range().start;
    for e in root.children() {
        let r = match e {
            sql_syntax::SyntaxElement::Node(n) => n.range(),
            sql_syntax::SyntaxElement::Token(t) if !t.is_trivia() => t.range(),
            _ => continue,
        };
        if first.is_none() {
            first = Some(r.start);
        }
        last = last.max(r.end);
    }
    let first = first.unwrap_or(root.range().start);
    first..last.max(first)
}

/// Analyses documents, keeping the results of statements between runs.
#[derive(Debug, Default)]
pub struct Analyzer {
    cache: HashMap<u64, Arc<UnitResults>>,
    /// The model the cache was filled with (kept alive so its address
    /// cannot be reused while the cache refers to it).
    model: Option<Arc<Model>>,
    analysed: Arc<AtomicUsize>,
}

impl Analyzer {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many statements were analysed, cache misses only (shared with
    /// the services for tests and diagnostics).
    pub fn counter(&self) -> Arc<AtomicUsize> {
        self.analysed.clone()
    }

    pub fn analysed(&self) -> usize {
        self.analysed.load(Ordering::Relaxed)
    }

    /// Analyses `script` in `ctx`; `None` when `cancel` answered true
    /// between two statements.
    pub fn analyze(
        &mut self,
        script: &Script,
        ctx: &SqlContext,
        cancel: &dyn Fn() -> bool,
    ) -> Option<DocumentAnalysis> {
        let model = ctx.scope.as_ref().map(|s| s.model.clone());
        let same_model = match (&self.model, &model) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same_model {
            self.cache.clear();
            self.model = model;
        }
        let mut h = DefaultHasher::new();
        ctx.dialect.hash(&mut h);
        if let Some(s) = &ctx.scope {
            (Arc::as_ptr(&s.model) as usize).hash(&mut h);
            s.search_path.hash(&mut h);
            s.complete.hash(&mut h);
        }
        let mut fingerprint = h.finish();

        let env = Env::new(ctx);
        let text = script.text();
        let mut locals = LocalIndex::default();
        let mut cache: HashMap<u64, Arc<UnitResults>> = HashMap::with_capacity(self.cache.len());
        let mut units = Vec::new();
        let mut file_suppressed = Vec::new();
        let mut prev_end = 0;
        for (i, root) in script.statements().enumerate() {
            if cancel() {
                return None;
            }
            let unit = root.range();
            let statement = statement_range(root);
            let mut h = DefaultHasher::new();
            fingerprint.hash(&mut h);
            text[unit.clone()].hash(&mut h);
            let key = h.finish();
            let results = match cache.get(&key).or_else(|| self.cache.get(&key)) {
                Some(r) => r.clone(),
                None => {
                    self.analysed.fetch_add(1, Ordering::Relaxed);
                    Arc::new(analyse_unit(root, &env, &locals))
                }
            };
            cache.insert(key, results.clone());
            if !results.defs.is_empty() {
                let mut h = DefaultHasher::new();
                fingerprint.hash(&mut h);
                for d in &results.defs {
                    d.hash_into(&mut h);
                    locals.push(ctx.dialect, d.clone());
                }
                fingerprint = h.finish();
            }
            let gap = &text[prev_end.min(statement.start)..statement.start];
            let mut suppressed = Vec::new();
            if gap.contains("noinspection") {
                for (_, line) in line_comments(gap) {
                    if let Some(ids) = noinspection(line) {
                        for id in ids {
                            match id.strip_suffix("ForFile") {
                                Some(file) if i == 0 => file_suppressed.push(file.to_string()),
                                Some(_) => {}
                                None => suppressed.push(id),
                            }
                        }
                    }
                }
            }
            prev_end = statement.end;
            units.push(UnitEntry {
                unit,
                statement,
                results,
                suppressed,
            });
        }
        self.cache = cache;
        Some(DocumentAnalysis {
            units,
            file_suppressed,
        })
    }
}

/// Resolves and inspects one unit; ranges relative to its start.
fn analyse_unit(root: SyntaxNode<'_>, env: &Env<'_>, locals: &LocalIndex) -> UnitResults {
    let out = Resolver::new(env, locals).unit(root);
    let mut problems = inspect(root, &out);
    let base = -(root.range().start as isize);
    let mut refs = out.refs;
    for r in &mut refs {
        shift_reference(r, base);
    }
    for p in &mut problems {
        p.shift(base);
    }
    UnitResults {
        refs,
        problems,
        defs: out.defs,
    }
}

/// Analyses a text once (tests, one-off checks).
pub fn analyze(text: &str, ctx: &SqlContext) -> DocumentAnalysis {
    let script = sql_syntax::parse(text, ctx.dialect);
    Analyzer::new()
        .analyze(&script, ctx, &|| false)
        .expect("never cancelled")
}
