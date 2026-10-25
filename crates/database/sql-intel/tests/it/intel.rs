//! `SqlCodeIntel`: diagnostics published in the background, code actions
//! applied, suppression, disabling, context changes.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use code_intel::{
    CodeAction, CodeActionContext, CodeIntel, Diagnostic, DocumentId, IntelEvent, Point, Rope,
    Severity, positions,
};
use sql_intel::{Inspection, InspectionProfile, ScopeProvider, SqlCodeIntel, SqlContext};
use sql_syntax::Dialect;

use crate::fixture::*;

const WAIT: Duration = Duration::from_secs(10);

/// The next diagnostics of `doc` at `version`.
fn diagnostics(
    rx: &flume::Receiver<IntelEvent>,
    doc: &DocumentId,
    version: u64,
) -> Vec<Diagnostic> {
    loop {
        match rx.recv_timeout(WAIT).expect("diagnostics in time") {
            IntelEvent::Diagnostics {
                doc: d,
                version: v,
                items,
            } if &d == doc && v == version => return items,
            _ => {}
        }
    }
}

fn block<T>(f: code_intel::Intel<T>) -> T {
    futures::executor::block_on(f).expect("an answer")
}

fn apply_action(text: &str, action: &CodeAction) -> String {
    let rope = Rope::from_str(text);
    let mut edits: Vec<(usize, usize, String)> = action
        .edit
        .as_ref()
        .expect("an edit")
        .files
        .iter()
        .flat_map(|f| f.edits.iter())
        .map(|e| {
            let s = rope.char_to_byte(positions::point_to_offset(&rope, e.range.start));
            let t = rope.char_to_byte(positions::point_to_offset(&rope, e.range.end));
            (s, t, e.text.clone())
        })
        .collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = text.to_string();
    for (s, t, new) in edits {
        out.replace_range(s..t, &new);
    }
    out
}

fn actions_at(intel: &SqlCodeIntel, doc: &DocumentId, at: Point) -> Vec<CodeAction> {
    block(intel.code_actions(doc, at..at, CodeActionContext::default()))
}

/// A provider whose scope can be switched on.
struct Switch(AtomicBool);

impl ScopeProvider for Switch {
    fn context(&self, _: &DocumentId) -> SqlContext {
        if self.0.load(Ordering::SeqCst) {
            scoped(Dialect::Postgres)
        } else {
            unscoped(Dialect::Postgres)
        }
    }
}

#[test]
fn diagnostics_fixes_suppression_and_disabling() {
    let intel = SqlCodeIntel::new(Arc::new(sql_intel::FixedScope(scoped(Dialect::Postgres))));
    let events = intel.events();
    let doc = DocumentId::new("/p/q.sql");
    let mut text = "select * from custmers;\ndelete from orders;\n".to_string();
    intel.did_open(&doc, "sql", 1, &Rope::from_str(&text));
    let items = diagnostics(&events, &doc, 1);
    let summary: Vec<(Point, Severity, &str, Option<&str>)> = items
        .iter()
        .map(|d| {
            (
                d.range.start,
                d.severity,
                d.message.as_str(),
                d.code.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                Point::new(0, 14),
                Severity::Error,
                "Cannot resolve table 'custmers'",
                Some("SqlResolve")
            ),
            (
                Point::new(1, 0),
                Severity::Warning,
                "DELETE without WHERE deletes every row of the table",
                Some("SqlWithoutWhere")
            ),
        ]
    );
    assert_eq!(intel.status()[0].documents, std::slice::from_ref(&doc));

    // The similar name, with suppress and disable behind it.
    let actions = actions_at(&intel, &doc, Point::new(0, 16));
    let titles: Vec<&str> = actions.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["Change to 'customers'"]);
    assert!(actions[0].preferred);
    let children: Vec<&str> = actions[0]
        .children
        .iter()
        .map(|a| a.title.as_str())
        .collect();
    assert_eq!(
        children,
        [
            "Suppress for statement",
            "Disable inspection 'Unresolved reference'"
        ]
    );
    text = apply_action(&text, &actions[0]);
    assert!(text.starts_with("select * from customers;"));
    intel.did_change(&doc, 2, &[], &Rope::from_str(&text));
    let items = diagnostics(&events, &doc, 2);
    assert_eq!(items.len(), 1);

    // Suppress the DELETE warning for its statement.
    let actions = actions_at(&intel, &doc, Point::new(1, 3));
    assert_eq!(actions[0].title, "Add WHERE clause");
    let suppress = actions[0].children[0].clone();
    text = apply_action(&text, &suppress);
    assert_eq!(
        text,
        "select * from customers;\n-- noinspection SqlWithoutWhere\ndelete from orders;\n"
    );
    intel.did_change(&doc, 3, &[], &Rope::from_str(&text));
    assert!(diagnostics(&events, &doc, 3).is_empty());

    // Disable an inspection: the setting is written and marks go.
    let written: Arc<Mutex<Option<InspectionProfile>>> = Arc::default();
    let sink = written.clone();
    intel.on_profile_change(move |p| *sink.lock().unwrap() = Some(p.clone()));
    text.push_str("select * from nope;\n");
    intel.did_change(&doc, 4, &[], &Rope::from_str(&text));
    assert_eq!(diagnostics(&events, &doc, 4).len(), 1);
    let actions = actions_at(&intel, &doc, Point::new(3, 15));
    let disable = actions
        .iter()
        .flat_map(|a| a.children.iter().chain(std::iter::once(a)))
        .find(|a| a.title.starts_with("Disable inspection"))
        .unwrap()
        .clone();
    block(intel.execute_command(disable.command.unwrap()));
    assert!(diagnostics(&events, &doc, 4).is_empty());
    assert!(
        !written
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_enabled(Inspection::Resolve)
    );

    intel.did_close(&doc);
    assert!(diagnostics(&events, &doc, 0).is_empty());
    block(intel.shutdown());
}

#[test]
fn an_edit_analyses_one_statement() {
    let intel = SqlCodeIntel::new(Arc::new(sql_intel::FixedScope(scoped(Dialect::Postgres))));
    let events = intel.events();
    let doc = DocumentId::new("/p/many.sql");
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("select id, total from orders where id = {i};\n"));
    }
    intel.did_open(&doc, "sql", 1, &Rope::from_str(&text));
    assert!(diagnostics(&events, &doc, 1).is_empty());
    assert_eq!(intel.analysed(), 200);
    let at = text.find("= 100;").unwrap();
    text.replace_range(at..at + 1, "= nope +");
    intel.did_change(&doc, 2, &[], &Rope::from_str(&text));
    let items = diagnostics(&events, &doc, 2);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].range.start.line, 100);
    assert_eq!(intel.analysed(), 201);
}

#[test]
fn context_changes_analyse_again() {
    let switch = Arc::new(Switch(AtomicBool::new(false)));
    let intel = SqlCodeIntel::new(switch.clone());
    let events = intel.events();
    let doc = DocumentId::new("/p/s.sql");
    intel.did_open(&doc, "sql", 1, &Rope::from_str("select * from nothing"));
    // No scope: nothing unresolved.
    assert!(diagnostics(&events, &doc, 1).is_empty());
    switch.0.store(true, Ordering::SeqCst);
    intel.context_changed(Some(&doc));
    let items = diagnostics(&events, &doc, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].message, "Cannot resolve table 'nothing'");
}

#[test]
fn unused_cte_is_greyed_and_profile_levels_apply() {
    let mut profile = InspectionProfile::default();
    profile.set(Inspection::UnusedCte, sql_intel::InspectionLevel::Warning);
    let intel = SqlCodeIntel::with_profile(
        Arc::new(sql_intel::FixedScope(unscoped(Dialect::Postgres))),
        profile,
    );
    let events = intel.events();
    let doc = DocumentId::new("/p/c.sql");
    intel.did_open(
        &doc,
        "sql",
        1,
        &Rope::from_str("with unused as (select 1) select 2"),
    );
    let items = diagnostics(&events, &doc, 1);
    assert_eq!(items.len(), 1);
    assert!(items[0].unnecessary);
    assert_eq!(items[0].severity, Severity::Warning);
    intel.set_profile(InspectionProfile::default());
    let items = diagnostics(&events, &doc, 1);
    assert_eq!(items[0].severity, Severity::WeakWarning);
}
