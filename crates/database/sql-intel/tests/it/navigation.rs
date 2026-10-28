//! Go to declaration, usages, highlights, rename and the structure through
//! `SqlCodeIntel` (the scenario of `tests/scenarios/sql/navigation.md`), on
//! the shop fixture.
use std::sync::Arc;

use code_intel::{
    CodeIntel, DefinitionTarget, DocumentHighlight, DocumentId, DocumentSymbol, HighlightKind,
    IntelError, Location, Point, Rope, SymbolKind, WorkspaceEdit, positions,
};
use sql_intel::{DB_SCHEME, FixedScope, SqlCodeIntel, SqlContext, parse_object_id};
use sql_syntax::Dialect;

use crate::fixture::*;

fn block<T>(f: code_intel::Intel<T>) -> Result<T, IntelError> {
    futures::executor::block_on(f)
}

fn split(marked: &str) -> (String, Point) {
    let at = marked.find('|').expect("a caret");
    let text = marked.replace('|', "");
    let rope = Rope::from_str(&text);
    (
        text,
        positions::offset_to_point(&rope, rope.byte_to_char(at)),
    )
}

/// The services with `marked` open (the caret is the `|`).
struct Session {
    intel: SqlCodeIntel,
    doc: DocumentId,
    text: String,
    at: Point,
}

fn session(context: SqlContext, marked: &str) -> Session {
    let (text, at) = split(marked);
    let intel = SqlCodeIntel::new(Arc::new(FixedScope(context)));
    let doc = DocumentId::new("/p/q.sql");
    intel.did_open(&doc, "sql", 1, &Rope::from_str(&text));
    Session {
        intel,
        doc,
        text,
        at,
    }
}

impl Session {
    fn slice(&self, range: &std::ops::Range<Point>) -> String {
        let rope = Rope::from_str(&self.text);
        let start = rope.char_to_byte(positions::point_to_offset(&rope, range.start));
        let end = rope.char_to_byte(positions::point_to_offset(&rope, range.end));
        self.text[start..end].to_string()
    }

    fn byte(&self, p: Point) -> usize {
        let rope = Rope::from_str(&self.text);
        rope.char_to_byte(positions::point_to_offset(&rope, p))
    }

    fn targets(&self) -> Vec<DefinitionTarget> {
        block(self.intel.definition_targets(&self.doc, self.at)).expect("an answer")
    }

    /// The byte offsets of the file targets.
    fn declared_at(&self) -> Vec<usize> {
        self.targets()
            .into_iter()
            .filter_map(|t| match t {
                DefinitionTarget::File(Location { range, .. }) => Some(self.byte(range.start)),
                DefinitionTarget::Object(_) => None,
            })
            .collect()
    }

    fn usages(&self, with_declaration: bool) -> Vec<usize> {
        let mut found: Vec<usize> =
            block(self.intel.references(&self.doc, self.at, with_declaration))
                .expect("an answer")
                .iter()
                .map(|l| self.byte(l.range.start))
                .collect();
        found.sort();
        found
    }

    fn highlights(&self) -> Vec<DocumentHighlight> {
        block(self.intel.document_highlights(&self.doc, self.at)).expect("an answer")
    }

    fn rename(&self, to: &str) -> Result<String, IntelError> {
        let edit = block(self.intel.rename(&self.doc, self.at, to.into()))?;
        Ok(apply(&self.text, &edit))
    }

    fn symbols(&self) -> Vec<DocumentSymbol> {
        block(self.intel.document_symbols(&self.doc)).expect("an answer")
    }
}

fn apply(text: &str, edit: &WorkspaceEdit) -> String {
    let rope = Rope::from_str(text);
    let mut edits: Vec<(usize, usize, String)> = edit
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

/// Where the text of `needle` starts (its `nth` occurrence, from 0).
fn nth(text: &str, needle: &str, nth: usize) -> usize {
    text.match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no {needle:?} #{nth} in {text:?}"))
        .0
}

fn pg() -> SqlContext {
    scoped(Dialect::Postgres)
}

// ---- go to declaration ----------------------------------------------------------

#[test]
fn an_alias_goes_to_its_place_in_from() {
    for d in RESOLVING {
        let s = session(scoped(d), "select c|.name from customers c where c.id = 1");
        assert_eq!(s.declared_at(), [nth(&s.text, " c ", 0) + 1], "{d:?}");
    }
}

#[test]
fn a_column_of_an_alias_goes_to_the_database_column() {
    let s = session(pg(), "select c.na|me from customers c");
    let targets = s.targets();
    let [DefinitionTarget::Object(o)] = targets.as_slice() else {
        panic!("one object target: {targets:?}");
    };
    assert_eq!(o.scheme, DB_SCHEME);
    assert_eq!(o.title, "customers.name");
    let (id, source) = parse_object_id(&o.id).expect("an id");
    assert_eq!(source, "shop");
    let model = shop(Dialect::Postgres);
    let node = model.node(id).expect("the node");
    assert_eq!(
        (node.name(), node.parent().map(|p| p.name().to_string())),
        ("name", Some("customers".to_string()))
    );
}

#[test]
fn a_table_goes_to_the_database_object() {
    let s = session(pg(), "select * from cust|omers");
    let targets = s.targets();
    let [DefinitionTarget::Object(o)] = targets.as_slice() else {
        panic!("one object target: {targets:?}");
    };
    assert_eq!(o.title, "public.customers");
    // A schema qualifier goes to the schema.
    let s = session(pg(), "select * from ap|p.users");
    let targets = s.targets();
    let [DefinitionTarget::Object(o)] = targets.as_slice() else {
        panic!("one object target: {targets:?}");
    };
    assert_eq!(o.title, "app");
}

#[test]
fn the_definition_of_the_default_provider_keeps_file_targets_only() {
    let s = session(pg(), "select * from cust|omers");
    let locations = block(s.intel.definition(&s.doc, s.at)).unwrap();
    assert!(locations.is_empty());
    let s = session(pg(), "select c|.id from customers c");
    let locations = block(s.intel.definition(&s.doc, s.at)).unwrap();
    assert_eq!(locations.len(), 1);
}

#[test]
fn a_cte_goes_to_its_definition_and_its_columns_to_the_select_item() {
    let s = session(
        pg(),
        "with recent as (select id, total as amount from orders)\nselect r.amount, id from re|cent r",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "recent", 0)]);
    let s = session(
        pg(),
        "with recent as (select id, total as amount from orders)\nselect r.am|ount from recent r",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "amount", 0)]);
    let s = session(
        pg(),
        "with recent(k, v) as (select id, total from orders)\nselect r.|v from recent r",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "v", 0)]);
}

#[test]
fn a_select_alias_in_order_by_goes_to_the_item() {
    let s = session(pg(), "select name as label from customers order by lab|el");
    assert_eq!(s.declared_at(), [nth(&s.text, "label", 0)]);
}

#[test]
fn an_alias_of_a_subquery_and_its_columns() {
    let s = session(pg(), "select s.t| from (select total as t from orders) s");
    assert_eq!(s.declared_at(), [nth(&s.text, "t from orders", 0)]);
}

#[test]
fn objects_the_script_creates_go_to_their_statement() {
    let s = session(
        pg(),
        "create table staging (id int, note text);\ninsert into stag|ing (id) values (1);",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "staging", 0)]);
    let s = session(
        pg(),
        "create table staging (id int, note text);\nselect st.no|te from staging st;",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "note", 0)]);
}

#[test]
fn on_a_declaration_the_target_is_the_caret_itself() {
    let s = session(pg(), "select c.id from customers c| where c.id = 1");
    assert_eq!(s.declared_at(), [nth(&s.text, " c ", 0) + 1]);
}

#[test]
fn variables_and_parameters_go_to_their_declaration() {
    let s = session(
        scoped(Dialect::Oracle),
        "create procedure p(amount number) is\n  v_total number := 1;\nbegin\n  v_total := amount + 1;\n  dbms_output.put_line(v_to|tal);\nend;",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "v_total", 0)]);
    let s = session(
        scoped(Dialect::Oracle),
        "create procedure p(amount number) is\nbegin\n  null;\n  x := amo|unt + 1;\nend;",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "amount", 0)]);
}

#[test]
fn variables_in_a_pl_pgsql_body() {
    let s = session(
        pg(),
        "create function f(a int) returns int language plpgsql as $$\ndeclare v int := 1;\nbegin\n  v := a + 1;\n  return |v;\nend;\n$$;",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "v int", 0)]);
    let s = session(
        pg(),
        "create function f(a int) returns int language plpgsql as $$\nbegin\n  return |a;\nend;\n$$;",
    );
    assert_eq!(s.declared_at(), [nth(&s.text, "a int", 0)]);
}

#[test]
fn a_column_of_the_model_wins_over_a_variable_of_the_same_name() {
    let s = session(
        scoped(Dialect::Oracle),
        "create procedure p is\n  id number;\nbegin\n  update customers set name = 'x' where i|d = 1;\nend;",
    );
    let targets = s.targets();
    assert!(
        matches!(targets.as_slice(), [DefinitionTarget::Object(_)]),
        "{targets:?}"
    );
}

#[test]
fn nothing_to_go_to_for_keywords_and_unresolved_names() {
    let s = session(pg(), "sel|ect 1");
    assert!(s.targets().is_empty());
    let s = session(pg(), "select * from nowhe|re");
    assert!(s.targets().is_empty());
    let s = session(pg(), "select 1 -- co|mment");
    assert!(s.targets().is_empty());
}

// ---- usages and highlights --------------------------------------------------------

#[test]
fn usages_of_an_alias() {
    let s = session(
        pg(),
        "select c|.name from customers c join orders o on o.customer_id = c.id where c.id > 1 order by c.name",
    );
    let decl = nth(&s.text, " c ", 0) + 1;
    let mut uses: Vec<usize> = s.text.match_indices("c.").map(|(i, _)| i).collect();
    // `customer_id = c.id` is matched by "c." inside "customers"? No: only the
    // alias qualifiers start a "c." here.
    uses.retain(|&i| !s.text[..i].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_'));
    let mut all = uses.clone();
    all.push(decl);
    all.sort();
    assert_eq!(s.usages(true), all);
    assert_eq!(s.usages(false), uses);
}

#[test]
fn highlights_mark_the_declaration_as_a_write() {
    let s = session(pg(), "select c.id from customers c| where c.id = 1");
    let h = s.highlights();
    assert_eq!(h.len(), 3, "{h:?}");
    assert_eq!(
        h.iter().filter(|h| h.kind == HighlightKind::Write).count(),
        1
    );
    assert_eq!(s.slice(&h[0].range), "c");
}

#[test]
fn usages_of_a_column_of_the_model_in_the_file() {
    let s = session(
        pg(),
        "select c.id from customers c join orders o on o.customer_id = c.i|d;\nselect id from customers where id > 1;\nselect id from orders;",
    );
    // `c.id` twice and the unqualified `id` of customers; not orders' id.
    assert_eq!(
        s.usages(false),
        [
            nth(&s.text, "id from customers c", 0),
            nth(&s.text, "c.id;", 0) + 2,
            nth(&s.text, "id from customers where", 0),
            nth(&s.text, "id > 1", 0),
        ]
    );
}

#[test]
fn usages_of_a_table_are_all_its_names() {
    let s = session(
        pg(),
        "select * from custo|mers;\ninsert into customers (id) values (1);\nselect * from app.users;",
    );
    assert_eq!(
        s.usages(false),
        [nth(&s.text, "customers", 0), nth(&s.text, "customers", 1)]
    );
}

#[test]
fn usages_of_a_variable_stay_in_its_scope() {
    let s = session(
        scoped(Dialect::Oracle),
        "create procedure p is\n  n number := 1;\nbegin\n  n := n + 1;\n  for n in 1..3 loop\n    null;\n  end loop;\n  dbms_output.put_line(|n);\nend;",
    );
    // The loop variable `n` is another one.
    let all: Vec<usize> = s.text.match_indices("n number").map(|(i, _)| i).collect();
    let uses = s.usages(true);
    assert_eq!(uses.len(), 4, "{uses:?}");
    assert_eq!(uses[0], all[0]);
    assert!(!uses.contains(&nth(&s.text, "for n", 0).saturating_add(4)));
}

#[test]
fn no_usages_without_a_name() {
    let s = session(pg(), "select 1 +| 2");
    assert!(s.usages(true).is_empty());
    assert!(s.highlights().is_empty());
}

// ---- rename ---------------------------------------------------------------------

#[test]
fn rename_a_table_alias_used_in_order_by() {
    for d in RESOLVING {
        let s = session(
            scoped(d),
            "select c.name from customers c where c.id > 1 order by c|.name",
        );
        assert_eq!(
            s.rename("cust").unwrap(),
            "select cust.name from customers cust where cust.id > 1 order by cust.name",
            "{d:?}"
        );
    }
}

#[test]
fn rename_a_select_alias_used_in_order_by() {
    let s = session(pg(), "select name as lab|el from customers order by label");
    assert_eq!(
        s.rename("title").unwrap(),
        "select name as title from customers order by title"
    );
}

#[test]
fn rename_from_the_declaration_and_from_a_use() {
    let from_decl = session(pg(), "select c.id from customers c| where c.id = 1");
    let from_use = session(pg(), "select c.id from customers c where c|.id = 1");
    let want = "select k.id from customers k where k.id = 1";
    assert_eq!(from_decl.rename("k").unwrap(), want);
    assert_eq!(from_use.rename("k").unwrap(), want);
}

#[test]
fn rename_a_cte_and_its_columns() {
    let s = session(
        pg(),
        "with recent as (select id from orders)\nselect * from rec|ent r join recent q on q.id = r.id",
    );
    assert_eq!(
        s.rename("fresh").unwrap(),
        "with fresh as (select id from orders)\nselect * from fresh r join fresh q on q.id = r.id"
    );
    let s = session(
        pg(),
        "with recent as (select total as amount from orders)\nselect r.amo|unt from recent r where r.amount > 1",
    );
    assert_eq!(
        s.rename("sum").unwrap(),
        "with recent as (select total as sum from orders)\nselect r.sum from recent r where r.sum > 1"
    );
}

#[test]
fn rename_a_script_table_and_its_column() {
    let s = session(
        pg(),
        "create table stag|ing (id int, note text);\ninsert into staging (id) values (1);\nselect s.note from staging s;",
    );
    assert_eq!(
        s.rename("landing").unwrap(),
        "create table landing (id int, note text);\ninsert into landing (id) values (1);\nselect s.note from landing s;"
    );
    let s = session(
        pg(),
        "create table staging (id int, note text);\nselect s.no|te from staging s where s.note > '';",
    );
    assert_eq!(
        s.rename("memo").unwrap(),
        "create table staging (id int, memo text);\nselect s.memo from staging s where s.memo > '';"
    );
}

#[test]
fn rename_a_variable_and_a_parameter() {
    let s = session(
        scoped(Dialect::Oracle),
        "create procedure p(amount number) is\n  v_total number := 1;\nbegin\n  v_total := amo|unt + v_total;\nend;",
    );
    assert_eq!(
        s.rename("qty").unwrap(),
        "create procedure p(qty number) is\n  v_total number := 1;\nbegin\n  v_total := qty + v_total;\nend;"
    );
    let s = session(
        pg(),
        "create function f(a int) returns int language plpgsql as $$\ndeclare v int := 1;\nbegin\n  v := a + 1;\n  return |v;\nend;\n$$;",
    );
    assert_eq!(
        s.rename("res").unwrap(),
        "create function f(a int) returns int language plpgsql as $$\ndeclare res int := 1;\nbegin\n  res := a + 1;\n  return res;\nend;\n$$;"
    );
}

#[test]
fn the_new_name_is_quoted_when_it_needs_to_be() {
    let s = session(pg(), "select c|.id from customers c");
    assert_eq!(
        s.rename("My Alias").unwrap(),
        "select \"My Alias\".id from customers \"My Alias\""
    );
    let s = session(pg(), "select c|.id from customers c");
    assert_eq!(
        s.rename("select").unwrap(),
        "select \"select\".id from customers \"select\""
    );
    let s = session(pg(), "select c|.id from customers \"c\"");
    assert_eq!(
        s.rename("d").unwrap(),
        "select \"d\".id from customers \"d\""
    );
}

#[test]
fn database_objects_are_not_renamed_here() {
    let s = session(pg(), "select * from cust|omers");
    assert!(matches!(s.rename("clients"), Err(IntelError::Other(_))));
    let s = session(pg(), "select c.na|me from customers c");
    assert!(matches!(s.rename("title"), Err(IntelError::Other(_))));
    let s = session(pg(), "select 1 +| 2");
    assert!(matches!(s.rename("x"), Err(IntelError::Other(_))));
    let s = session(pg(), "select c|.id from customers c");
    assert!(matches!(s.rename(" "), Err(IntelError::Other(_))));
}

// ---- structure ------------------------------------------------------------------

fn names(symbols: &[DocumentSymbol]) -> Vec<String> {
    symbols.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn the_structure_lists_statements_with_their_kind_and_name() {
    let s = session(
        pg(),
        "create table staging (id int, note text);\ncreate view v as select 1;\nwith a as (select 1), b as (select 2) select * from a, b;\ninsert into staging (id) values (1);\nupdate staging set id = 2;\ndelete from staging;\ndrop table staging;|",
    );
    let top = s.symbols();
    assert_eq!(
        names(&top),
        [
            "staging",
            "v",
            "select",
            "insert into staging",
            "update staging",
            "delete from staging",
            "drop table staging"
        ]
    );
    assert_eq!(top[0].kind, SymbolKind::Struct);
    assert_eq!(top[0].detail.as_deref(), Some("table"));
    assert_eq!(names(&top[0].children), ["id", "note"]);
    assert_eq!(top[0].children[1].detail.as_deref(), Some("text"));
    assert_eq!(s.slice(&top[0].selection_range), "staging");
    assert_eq!(names(&top[2].children), ["a", "b"]);
    assert_eq!(top[2].detail.as_deref(), Some("from a, b"));
    assert!(s.slice(&top[0].range).starts_with("create table"));
}

#[test]
fn the_structure_of_a_package_has_its_members() {
    let s = session(
        scoped(Dialect::Oracle),
        "create or replace package pk is\n  g number := 1;\n  procedure p1(a number);\n  function f1 return number;\nend pk;\ncreate or replace package body pk is\n  function f1 return number is begin return 1; end;\n  procedure p1(a number) is\n    procedure inner1 is begin null; end;\n  begin null; end;\nend pk;|",
    );
    let top = s.symbols();
    assert_eq!(names(&top), ["pk", "pk"]);
    assert_eq!(top[0].detail.as_deref(), Some("package"));
    assert_eq!(top[1].detail.as_deref(), Some("package body"));
    assert_eq!(names(&top[0].children), ["g", "p1", "f1"]);
    assert_eq!(names(&top[1].children), ["f1", "p1"]);
    assert_eq!(names(&top[1].children[1].children), ["inner1"]);
}

#[test]
fn the_structure_of_a_broken_script_skips_what_it_cannot_name() {
    let s = session(pg(), "create table (;\nselect 1;|");
    assert_eq!(names(&s.symbols()), ["select"]);
}

#[test]
fn navigation_follows_edits() {
    let (intel, doc) = {
        let s = session(pg(), "select c|.id from customers c");
        (s.intel, s.doc)
    };
    let text = "select k.id from customers k";
    intel.did_change(&doc, 2, &[], &Rope::from_str(text));
    let at = Point::new(0, 8);
    let found = block(intel.references(&doc, at, true)).unwrap();
    assert_eq!(found.len(), 2);
}
