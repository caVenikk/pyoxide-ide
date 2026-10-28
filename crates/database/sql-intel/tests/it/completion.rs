//! Completion, quick documentation and parameter info through
//! `SqlCodeIntel`, on a fixture model with keys, comments and routines
//! (the scenario of `tests/scenarios/sql/completion.md`).
use std::sync::Arc;

use code_intel::{
    CodeIntel, CompletionItem, CompletionTrigger, Completions, DocumentId, InsertText, Point, Rope,
    SignatureContext, SignatureHelp, SymbolKind, positions,
};
use db_core::DataSourceId;
use db_core::schema::{
    ArgumentInfo, ArgumentMode, ColumnInfo, ConstraintInfo, ConstraintKind, Level, ObjectDetail,
    ObjectInfo, ObjectKind, RelationInfo, RoutineDetail, SchemaInfo, SchemaSnapshot, Snapshot,
};
use db_model::Model;
use sql_intel::{CompletionOptions, FixedScope, KeywordCase, Scope, SqlCodeIntel, SqlContext};
use sql_syntax::Dialect;

fn stored(d: Dialect, name: &str) -> String {
    if d == Dialect::Oracle {
        name.to_uppercase()
    } else {
        name.to_string()
    }
}

fn col(d: Dialect, name: &str, ty: &str, nullable: bool) -> ColumnInfo {
    ColumnInfo {
        name: stored(d, name),
        data_type: ty.into(),
        nullable,
        ..ColumnInfo::default()
    }
}

fn numbered(mut cols: Vec<ColumnInfo>) -> Vec<ColumnInfo> {
    for (i, c) in cols.iter_mut().enumerate() {
        c.position = i as u32 + 1;
    }
    cols
}

fn pk(d: Dialect, table: &str, cols: &[&str]) -> ConstraintInfo {
    ConstraintInfo {
        name: format!("{table}_pkey"),
        kind: ConstraintKind::PrimaryKey,
        columns: cols.iter().map(|c| stored(d, c)).collect(),
        ..ConstraintInfo::default()
    }
}

fn fk(d: Dialect, name: &str, cols: &[&str], table: &str, refs: &[&str]) -> ConstraintInfo {
    ConstraintInfo {
        name: name.into(),
        kind: ConstraintKind::ForeignKey,
        columns: cols.iter().map(|c| stored(d, c)).collect(),
        ref_table: Some(stored(d, table)),
        ref_columns: refs.iter().map(|c| stored(d, c)).collect(),
        ..ConstraintInfo::default()
    }
}

fn table(d: Dialect, name: &str, cols: Vec<ColumnInfo>, cons: Vec<ConstraintInfo>) -> RelationInfo {
    RelationInfo {
        name: stored(d, name),
        columns: numbered(cols),
        indexes: Vec::new(),
        constraints: cons,
    }
}

fn object(d: Dialect, schema: &str, name: &str, kind: ObjectKind) -> ObjectInfo {
    ObjectInfo {
        schema: stored(d, schema),
        name: stored(d, name),
        kind,
        ..ObjectInfo::default()
    }
}

fn function(
    d: Dialect,
    schema: &str,
    name: &str,
    args: &[(&str, &str, bool)],
    ret: &str,
) -> ObjectInfo {
    let mut o = object(d, schema, name, ObjectKind::Function);
    o.comment = Some(format!("Computes {name}."));
    o.detail = Some(ObjectDetail::Routine(RoutineDetail {
        signature: args.iter().map(|a| a.1).collect::<Vec<_>>().join(", "),
        arguments: args
            .iter()
            .map(|(n, t, default)| ArgumentInfo {
                name: Some(stored(d, n)),
                data_type: (*t).into(),
                mode: ArgumentMode::In,
                has_default: *default,
            })
            .collect(),
        return_type: Some(ret.into()),
        language: Some("sql".into()),
        characteristics: Vec::new(),
    }));
    o
}

fn schema(
    d: Dialect,
    name: &str,
    current: bool,
    level: Level,
    objects: Vec<ObjectInfo>,
    relations: Vec<RelationInfo>,
) -> SchemaSnapshot {
    SchemaSnapshot {
        info: SchemaInfo {
            name: stored(d, name),
            is_system: false,
            is_current: current,
            owner: None,
            comment: None,
        },
        level,
        objects,
        relations,
        sources: Vec::new(),
    }
}

/// The shop with keys: `customers`, `orders -> customers`, the mixed-case
/// `Order Items -> orders`, the view `order_totals`, two functions.
fn shop(d: Dialect) -> Arc<Model> {
    let main = match d {
        Dialect::Oracle => "shop",
        Dialect::Sqlite => "main",
        _ => "public",
    };
    let mut customers = object(d, main, "customers", ObjectKind::Table);
    customers.comment = Some("People who buy things.".into());
    let objects = vec![
        customers,
        object(d, main, "orders", ObjectKind::Table),
        object(d, main, "Order Items", ObjectKind::Table),
        object(d, main, "order_totals", ObjectKind::View),
        function(
            d,
            main,
            "discount",
            &[("price", "numeric", false), ("rate", "numeric", true)],
            "numeric",
        ),
        function(
            d,
            main,
            "discount",
            &[("price", "numeric", false)],
            "numeric",
        ),
    ];
    let mut customer_cols = vec![
        col(d, "id", "integer", false),
        col(d, "name", "text", true),
        col(d, "email", "text", true),
    ];
    customer_cols[1].comment = Some("Display name.".into());
    let relations = vec![
        table(
            d,
            "customers",
            customer_cols,
            vec![pk(d, "customers", &["id"])],
        ),
        table(
            d,
            "orders",
            vec![
                col(d, "id", "integer", false),
                col(d, "customer_id", "integer", false),
                col(d, "total", "numeric", true),
            ],
            vec![
                pk(d, "orders", &["id"]),
                fk(
                    d,
                    "orders_customer_fk",
                    &["customer_id"],
                    "customers",
                    &["id"],
                ),
            ],
        ),
        table(
            d,
            "Order Items",
            vec![
                col(d, "id", "integer", false),
                col(d, "order_id", "integer", false),
                col(d, "qty", "integer", true),
            ],
            vec![fk(d, "items_order_fk", &["order_id"], "orders", &["id"])],
        ),
        table(
            d,
            "order_totals",
            vec![
                col(d, "customer_id", "integer", true),
                col(d, "sum", "numeric", true),
            ],
            Vec::new(),
        ),
    ];
    let public = schema(d, main, true, Level::Structure, objects, relations);
    let app = schema(
        d,
        "app",
        false,
        Level::Structure,
        vec![
            object(d, "app", "users", ObjectKind::Table),
            object(d, "app", "invoices", ObjectKind::Table),
            object(d, "app", "orders", ObjectKind::Table),
        ],
        vec![
            table(
                d,
                "users",
                vec![
                    col(d, "id", "integer", false),
                    col(d, "email", "text", true),
                ],
                Vec::new(),
            ),
            table(
                d,
                "invoices",
                vec![col(d, "id", "integer", false)],
                Vec::new(),
            ),
            table(
                d,
                "orders",
                vec![col(d, "id", "integer", false)],
                Vec::new(),
            ),
        ],
    );
    let audit = schema(
        d,
        "audit",
        false,
        Level::Names,
        vec![object(d, "audit", "log", ObjectKind::Table)],
        Vec::new(),
    );
    Arc::new(Model::from_snapshot(
        DataSourceId::new("shop"),
        Snapshot {
            level: Level::Structure,
            token: None,
            schemas: vec![public, app, audit],
        },
    ))
}

fn ctx(d: Dialect) -> SqlContext {
    SqlContext::new(d, Some(Scope::new(shop(d))))
}

fn block<T>(f: code_intel::Intel<T>) -> T {
    futures::executor::block_on(f).expect("an answer")
}

/// The text without its `|` and the caret as a point.
fn split(marked: &str) -> (String, Point) {
    let at = marked.find('|').expect("a caret");
    let text = marked.replace('|', "");
    let rope = Rope::from_str(&text);
    (
        text,
        positions::offset_to_point(&rope, rope.byte_to_char(at)),
    )
}

fn open(context: SqlContext, text: &str) -> (SqlCodeIntel, DocumentId) {
    let intel = SqlCodeIntel::new(Arc::new(FixedScope(context)));
    let doc = DocumentId::new("/p/q.sql");
    intel.did_open(&doc, "sql", 1, &Rope::from_str(text));
    (intel, doc)
}

fn complete_with(context: SqlContext, marked: &str) -> Completions {
    let (text, at) = split(marked);
    let (intel, doc) = open(context, &text);
    block(intel.completion(&doc, at, CompletionTrigger::Invoked))
}

fn complete(d: Dialect, marked: &str) -> Completions {
    complete_with(ctx(d), marked)
}

/// The labels in the order the popup shows them for an empty prefix.
fn shown(c: &Completions) -> Vec<String> {
    let mut items: Vec<&CompletionItem> = c.items.iter().collect();
    items.sort_by(|a, b| a.sort_text().cmp(b.sort_text()));
    items.iter().map(|i| i.label.clone()).collect()
}

fn labels(c: &Completions) -> Vec<String> {
    c.items.iter().map(|i| i.label.clone()).collect()
}

fn item<'a>(c: &'a Completions, label: &str) -> &'a CompletionItem {
    c.items
        .iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("no {label:?} in {:?}", labels(c)))
}

fn has(c: &Completions, label: &str) -> bool {
    c.items.iter().any(|i| i.label == label)
}

fn insert_text(i: &CompletionItem) -> &str {
    i.insert.text()
}

/// The text after the item is chosen.
fn applied(marked: &str, i: &CompletionItem) -> String {
    let (text, _) = split(marked);
    let rope = Rope::from_str(&text);
    let InsertText::Edit {
        insert, text: new, ..
    } = &i.insert
    else {
        panic!("an edit");
    };
    let s = rope.char_to_byte(positions::point_to_offset(&rope, insert.start));
    let e = rope.char_to_byte(positions::point_to_offset(&rope, insert.end));
    let mut out = text.clone();
    out.replace_range(s..e, new);
    out
}

const PG: Dialect = Dialect::Postgres;

#[test]
fn columns_of_an_alias_before_from_is_connected() {
    // Step 2 of the scenario: `FROM` comes after the caret.
    let c = complete(PG, "select c.| from customers c");
    assert_eq!(shown(&c), ["*", "email", "id", "name"].map(String::from));
    assert_eq!(item(&c, "id").type_text.as_deref(), Some("integer"));
    assert_eq!(item(&c, "id").kind, Some(SymbolKind::Field));
    let by = complete(PG, "select c.na| from customers c");
    assert!(has(&by, "name"));
}

#[test]
fn columns_before_from_is_typed_and_in_where() {
    let c = complete(PG, "select | from customers");
    assert!(has(&c, "id") && has(&c, "name") && has(&c, "email"));
    assert!(has(&c, "count"), "functions: {:?}", labels(&c));
    assert!(!has(&c, "total"), "no column of another table");
    let c = complete(PG, "select * from orders o where |");
    assert!(has(&c, "customer_id"));
    let c = complete(PG, "select id, | from customers");
    assert!(has(&c, "email"));
    // Columns come before functions and keywords.
    let order = shown(&complete(PG, "select | from customers"));
    let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
    assert!(pos("id") < pos("count"));
}

#[test]
fn ambiguous_columns_are_inserted_qualified() {
    let c = complete(
        PG,
        "select | from orders o join customers c on o.customer_id = c.id",
    );
    let ids: Vec<&CompletionItem> = c.items.iter().filter(|i| i.label == "id").collect();
    let mut texts: Vec<&str> = ids.iter().map(|i| insert_text(i)).collect();
    texts.sort();
    assert_eq!(texts, ["c.id", "o.id"]);
    assert_eq!(item(&c, "customer_id").insert.text(), "customer_id");
}

#[test]
fn schema_qualified_names() {
    let c = complete(PG, "select * from public.|");
    let mut names = labels(&c);
    names.sort();
    assert_eq!(
        names,
        ["Order Items", "customers", "order_totals", "orders"]
    );
    let c = complete(PG, "select * from app.|");
    assert!(has(&c, "users") && has(&c, "invoices") && has(&c, "orders"));
    assert!(!has(&c, "customers"));
}

#[test]
fn tables_of_the_search_path_first_and_other_schemas_by_prefix() {
    // Step 4.
    let c = complete(PG, "select * from |");
    let order = shown(&c);
    assert!(order.iter().position(|n| n == "customers") < order.iter().position(|n| n == "public"));
    assert!(!has(&c, "users"), "other schemas wait for a prefix");
    assert!(c.incomplete, "typing asks again");
    assert_eq!(item(&c, "customers").type_text.as_deref(), Some("table"));
    assert_eq!(item(&c, "order_totals").type_text.as_deref(), Some("view"));
    assert!(has(&c, "app"), "schemas are offered");

    let c = complete(PG, "select * from inv|");
    let i = item(&c, "invoices");
    assert_eq!(insert_text(i), "app.invoices");
    assert_eq!(i.tail_text.as_deref(), Some(" (app)"));
    assert_eq!(
        applied("select * from inv|", i),
        "select * from app.invoices"
    );
    // The same name in the search path and in another schema.
    let c = complete(PG, "select * from ord|");
    let orders: Vec<&str> = c
        .items
        .iter()
        .filter(|i| i.label == "orders")
        .map(insert_text)
        .collect();
    assert_eq!(orders.len(), 2);
    assert!(orders.contains(&"orders") && orders.contains(&"app.orders"));
}

#[test]
fn mixed_case_names_are_quoted() {
    // Step 6.
    let c = complete(PG, "select * from |");
    let i = item(&c, "Order Items");
    assert_eq!(insert_text(i), "\"Order Items\"");
    assert_eq!(
        applied(
            "select * from Ord|",
            item(&complete(PG, "select * from Ord|"), "Order Items")
        ),
        "select * from \"Order Items\""
    );
    // An opening quote typed already is replaced with the name.
    let marked = "select * from \"Ord|";
    let c = complete(PG, marked);
    assert_eq!(
        applied(marked, item(&c, "Order Items")),
        "select * from \"Order Items\""
    );
    // Columns that need no quotes get none.
    let c = complete(PG, "select o.| from \"Order Items\" o");
    assert_eq!(insert_text(item(&c, "qty")), "qty");
}

#[test]
fn common_table_expressions() {
    let c = complete(PG, "with x as (select id from customers) select * from |");
    assert_eq!(item(&c, "x").type_text.as_deref(), Some("CTE"));
    let c = complete(
        PG,
        "with x as (select id, name as who from customers) select | from x",
    );
    assert!(has(&c, "id") && has(&c, "who"));
    let c = complete(PG, "with x as (select id from customers) select x.| from x");
    assert!(has(&c, "id") && has(&c, "*"));
}

#[test]
fn join_conditions_come_from_foreign_keys() {
    // Step 3.
    let marked = "select * from orders o join customers c on |";
    let c = complete(PG, marked);
    let order = shown(&c);
    assert_eq!(order[0], "o.customer_id = c.id", "{order:?}");
    assert!(has(&c, "customer_id") && has(&c, "email"));
    assert_eq!(
        applied(marked, item(&c, "o.customer_id = c.id")),
        "select * from orders o join customers c on o.customer_id = c.id"
    );
    // The other way round: the key is the earlier table's.
    let c = complete(PG, "select * from customers c join orders o on |");
    assert_eq!(shown(&c)[0], "o.customer_id = c.id");
    // A chain of joins looks at every table before the new one.
    let c = complete(
        PG,
        "select * from customers c join orders o on o.customer_id = c.id join \"Order Items\" i on |",
    );
    assert_eq!(shown(&c)[0], "i.order_id = o.id");
    // No key, no condition.
    let c = complete(PG, "select * from customers c join order_totals t on |");
    assert!(!c.items.iter().any(|i| i.kind == Some(SymbolKind::Snippet)));
}

#[test]
fn insert_and_update_column_lists() {
    // Step 5.
    let c = complete(PG, "insert into customers (|");
    assert_eq!(shown(&c), ["email", "id", "name"].map(String::from));
    let c = complete(PG, "insert into customers (id, |");
    assert_eq!(shown(&c), ["email", "name"].map(String::from));
    let c = complete(PG, "insert into customers (id, na|) values (1, 'x')");
    assert!(has(&c, "name") && !has(&c, "id"));
    let c = complete(PG, "update customers set |");
    assert_eq!(shown(&c), ["email", "id", "name"].map(String::from));
    let c = complete(PG, "update customers set name = 'x', |");
    assert_eq!(shown(&c), ["email", "id"].map(String::from));
    // The right side of an assignment and `WHERE` are expressions.
    let c = complete(PG, "update customers set name = | where id = 1");
    assert!(has(&c, "email") && has(&c, "lower"));
    let c = complete(PG, "update customers set name = 'x' where |");
    assert!(has(&c, "id"));
    let c = complete(PG, "update |");
    assert!(has(&c, "customers"));
    let c = complete(PG, "delete from customers where |");
    assert!(has(&c, "email"));
    let c = complete(PG, "insert into |");
    assert!(has(&c, "orders"));
}

#[test]
fn correlated_subqueries_see_the_outer_sources() {
    let c = complete(
        PG,
        "select * from customers c where exists (select 1 from orders o where o.customer_id = c.|)",
    );
    assert!(has(&c, "email"));
    let c = complete(
        PG,
        "select * from customers c where exists (select 1 from orders o where |)",
    );
    assert!(has(&c, "total") && has(&c, "email"));
    // The inner source comes first.
    let order = shown(&c);
    let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
    assert!(pos("total") < pos("email"));
}

#[test]
fn select_list_aliases_in_order_by() {
    let c = complete(PG, "select total as grand from orders order by |");
    assert!(has(&c, "grand") && has(&c, "total"));
    let c = complete(PG, "select total as grand from orders where |");
    assert!(!has(&c, "grand"));
    let c = complete(Dialect::Sqlite, "select total as grand from orders where |");
    assert!(has(&c, "grand"));
}

#[test]
fn script_created_tables_complete() {
    let c = complete(PG, "create table tmp (a int, b text);\nselect * from |");
    assert!(has(&c, "tmp"));
    let c = complete(
        PG,
        "create table tmp (a int, b text);\nselect t.| from tmp t",
    );
    assert!(has(&c, "a") && has(&c, "b"));
    // Only what is created before the statement.
    let c = complete(PG, "select * from |;\ncreate table later (a int);");
    assert!(!has(&c, "later"));
}

#[test]
fn types_and_constraints_in_ddl() {
    let c = complete(PG, "create table t (id |");
    assert!(has(&c, "integer") && has(&c, "varchar") && has(&c, "uuid"));
    assert_eq!(item(&c, "varchar").tail_text.as_deref(), Some("(length)"));
    let c = complete(PG, "create table t (id int |");
    let l = labels(&c);
    assert!(
        l.contains(&"not null".to_string()) && l.contains(&"primary key".to_string()),
        "{l:?}"
    );
    assert!(!l.contains(&"integer".to_string()));
    let c = complete(PG, "select cast(a as |)");
    assert!(has(&c, "integer"));
    let c = complete(PG, "select a::| from t");
    assert!(has(&c, "text"));
    let c = complete(Dialect::Oracle, "create table t (id |");
    assert!(has(&c, "NUMBER") && has(&c, "VARCHAR2"));
    let c = complete(PG, "alter table customers add column nick |");
    assert!(has(&c, "text"));
    // A new column is named, not completed.
    let c = complete(PG, "alter table customers add |");
    assert!(!has(&c, "email"));
    let c = complete(PG, "alter table customers drop column |");
    assert!(has(&c, "email"));
}

#[test]
fn index_and_constraint_columns() {
    let c = complete(PG, "create index i on customers (|");
    assert!(has(&c, "email"));
    let c = complete(PG, "create index i on |");
    assert!(has(&c, "customers"));
    let c = complete(PG, "create table t (a int, b int, primary key (|");
    assert!(has(&c, "a") && has(&c, "b"));
    let c = complete(PG, "create table t (a int references customers (|");
    assert!(has(&c, "email"));
}

#[test]
fn routine_arguments_by_name() {
    let c = complete(PG, "select discount(|");
    assert!(has(&c, "price =>") && has(&c, "rate =>"));
    assert_eq!(insert_text(item(&c, "rate =>")), "rate => ");
    let c = complete(PG, "select discount(price => 1, |");
    assert!(has(&c, "rate =>") && !has(&c, "price =>"));
}

#[test]
fn functions_from_the_model_and_the_dialect() {
    let c = complete(PG, "select |");
    let f = item(&c, "discount");
    assert_eq!(f.kind, Some(SymbolKind::Function));
    assert!(f.snippet && insert_text(f) == "discount($0)");
    assert_eq!(f.type_text.as_deref(), Some("numeric"));
    assert_eq!(c.items.iter().filter(|i| i.label == "discount").count(), 1);
    assert!(has(&c, "coalesce") && has(&c, "string_agg"));
    let c = complete(Dialect::Oracle, "select |");
    assert!(has(&c, "NVL") && has(&c, "SYSDATE") && !has(&c, "string_agg"));
    // A call that has its parentheses already gets none.
    let (text, at) = split("select disc|(1) from customers");
    let (intel, doc) = open(ctx(PG), &text);
    let c = block(intel.completion(&doc, at, CompletionTrigger::Invoked));
    assert!(!item(&c, "discount").snippet);
}

#[test]
fn keywords_are_what_the_grammar_accepts_in_the_style_case() {
    let c = complete(PG, "select * from customers |");
    let l = labels(&c);
    assert!(
        l.contains(&"where".to_string()) && l.contains(&"join".to_string()),
        "{l:?}"
    );
    assert!(!l.contains(&"select".to_string()));
    // After upper case keywords, upper case.
    let c = complete(PG, "SELECT * FROM customers |");
    assert!(has(&c, "WHERE") && !has(&c, "where"));
    // Typed in lower case, completed in lower case.
    let c = complete(PG, "select * from customers whe|");
    assert!(has(&c, "where") && !has(&c, "WHERE"));
    let upper = CompletionOptions {
        keyword_case: KeywordCase::Upper,
        ..CompletionOptions::default()
    };
    let c = complete_with(
        ctx(PG).with_completion(upper),
        "select * from customers whe|",
    );
    assert!(has(&c, "WHERE"));
    let lower = CompletionOptions {
        keyword_case: KeywordCase::Lower,
        ..CompletionOptions::default()
    };
    let c = complete_with(ctx(PG).with_completion(lower), "select * from customers |");
    assert!(has(&c, "where") && has(&c, "group by"));
    // Statement starts.
    let c = complete(PG, "sel|");
    assert!(has(&c, "select"));
    assert_eq!(item(&c, "select").kind, Some(SymbolKind::Keyword));
    // Expression starts offer expression keywords, not the clauses.
    let c = complete(PG, "select * from customers where |");
    assert!(has(&c, "not") && has(&c, "case") && !has(&c, "group by"));
}

#[test]
fn an_alias_can_be_inserted_with_the_table() {
    let on = CompletionOptions {
        insert_alias: true,
        ..CompletionOptions::default()
    };
    let marked = "select * from |";
    let c = complete_with(ctx(PG).with_completion(on), marked);
    assert_eq!(insert_text(item(&c, "customers")), "customers c");
    assert_eq!(insert_text(item(&c, "order_totals")), "order_totals ot");
    assert_eq!(insert_text(item(&c, "Order Items")), "\"Order Items\" oi");
    // Unique in the statement.
    let c = complete_with(
        ctx(PG).with_completion(on),
        "select * from customers c join |",
    );
    assert_eq!(insert_text(item(&c, "customers")), "customers c2");
    // Off by default, and not in DML.
    assert_eq!(
        insert_text(item(&complete(PG, marked), "customers")),
        "customers"
    );
    let c = complete_with(ctx(PG).with_completion(on), "update |");
    assert_eq!(insert_text(item(&c, "customers")), "customers");
}

#[test]
fn nothing_inside_strings_and_comments() {
    assert!(
        complete(PG, "select 'abc|' from customers")
            .items
            .is_empty()
    );
    assert!(
        complete(PG, "select 1 -- cus|\nfrom customers")
            .items
            .is_empty()
    );
    assert!(complete(PG, "select /* cu| */ 1").items.is_empty());
}

#[test]
fn without_a_scope_only_the_dialect_answers() {
    let c = complete_with(SqlContext::unscoped(PG), "select | from customers c");
    assert!(has(&c, "count") && has(&c, "distinct"));
    assert!(!has(&c, "customers"));
    let c = complete_with(SqlContext::unscoped(PG), "select * from |");
    assert!(c.items.iter().all(|i| i.kind != Some(SymbolKind::Class)));
    let c = complete_with(SqlContext::unscoped(Dialect::Generic), "sel|");
    assert!(has(&c, "select"));
}

#[test]
fn oracle_and_sqlite_names() {
    let c = complete(Dialect::Oracle, "select * from |");
    assert_eq!(insert_text(item(&c, "CUSTOMERS")), "CUSTOMERS");
    assert_eq!(insert_text(item(&c, "ORDER ITEMS")), "\"ORDER ITEMS\"");
    let c = complete(Dialect::Oracle, "select c.| from customers c");
    assert!(has(&c, "ID") && has(&c, "NAME"));
    let c = complete(Dialect::Sqlite, "select c.| from Customers c");
    assert!(has(&c, "id"));
}

#[test]
fn items_replace_the_typed_prefix_and_resolve_lazily() {
    let marked = "select * from cust|";
    let c = complete(PG, marked);
    let i = item(&c, "customers");
    assert_eq!(applied(marked, i), "select * from customers");
    let InsertText::Edit {
        insert, replace, ..
    } = &i.insert
    else {
        panic!("an edit");
    };
    assert_eq!(insert.start, Point::new(0, 14));
    assert_eq!(insert.end, Point::new(0, 18));
    assert_eq!(replace.end, Point::new(0, 18));
    // Tab replaces the rest of the word.
    let (text, at) = split("select * from cu|stom");
    let (intel, doc) = open(ctx(PG), &text);
    let c = block(intel.completion(&doc, at, CompletionTrigger::Invoked));
    let InsertText::Edit { replace, .. } = &item(&c, "customers").insert else {
        panic!("an edit");
    };
    assert_eq!(replace.end, Point::new(0, 20));
    // Documentation on demand.
    let resolved = block(intel.resolve_completion(&doc, item(&c, "customers").clone()));
    let docs = resolved.documentation.expect("documentation").text;
    assert!(docs.contains("People who buy things."), "{docs}");
    assert!(
        docs.contains("`id` integer, primary key, not null"),
        "{docs}"
    );
    let resolved =
        block(intel.resolve_completion(&doc, item(&complete(PG, "select |"), "lower").clone()));
    assert!(resolved.documentation.unwrap().text.contains("lower case"));
}

#[test]
fn a_dot_opens_completion() {
    let (intel, doc) = open(ctx(PG), "");
    assert_eq!(intel.completion_triggers(&doc), ['.']);
}

#[test]
fn completion_follows_edits_and_scope_changes() {
    let (intel, doc) = open(ctx(PG), "select * from custo");
    let c = block(intel.completion(&doc, Point::new(0, 19), CompletionTrigger::Invoked));
    assert!(has(&c, "customers"));
    intel.did_change(&doc, 2, &[], &Rope::from_str("select e. from customers e"));
    let c = block(intel.completion(&doc, Point::new(0, 9), CompletionTrigger::Character('.')));
    assert!(has(&c, "email"));
}

fn hover_at(d: Dialect, marked: &str) -> Option<String> {
    let (text, at) = split(marked);
    let (intel, doc) = open(ctx(d), &text);
    block(intel.hover(&doc, at)).map(|h| h.contents.text)
}

#[test]
fn quick_documentation_of_objects() {
    let t = hover_at(PG, "select * from custo|mers").expect("a table");
    assert!(t.contains("table public.customers"), "{t}");
    assert!(t.contains("People who buy things."));
    assert!(t.contains("- `id` integer, primary key, not null"), "{t}");
    assert!(t.contains("- `name` text"));
    let t = hover_at(PG, "select o.cust|omer_id from orders o").expect("a column");
    assert!(
        t.contains("column public.orders.customer_id integer"),
        "{t}"
    );
    assert!(
        t.contains("not null") && t.contains("references `customers.id`"),
        "{t}"
    );
    let t = hover_at(PG, "select c.na|me from customers c").expect("a column");
    assert!(t.contains("Display name."), "{t}");
    let t = hover_at(PG, "select disc|ount(1)").expect("a routine");
    assert!(t.contains("function public.discount(price numeric"), "{t}");
    assert!(
        t.contains("returns numeric") && t.contains("Computes discount."),
        "{t}"
    );
    let t = hover_at(PG, "select * from app.inv|oices").expect("a table");
    assert!(t.contains("table app.invoices"), "{t}");
}

#[test]
fn quick_documentation_of_aliases_and_ctes() {
    let t = hover_at(PG, "select c|.name from customers c").expect("an alias");
    assert!(t.contains("alias c for customers"), "{t}");
    assert!(t.contains("- `email` text"), "{t}");
    let t = hover_at(PG, "with x as (select id from customers) select * from |x").expect("a CTE");
    assert!(t.contains("with x") && t.contains("- `id`"), "{t}");
}

#[test]
fn quick_documentation_of_keywords_and_builtins() {
    let t = hover_at(PG, "sel|ect 1").expect("a keyword");
    assert!(t.contains("SELECT") && t.contains("Reads rows"), "{t}");
    let t = hover_at(PG, "select coal|esce(null, 1)").expect("a function");
    assert!(
        t.contains("coalesce(value, ...)") && t.contains("first argument"),
        "{t}"
    );
    let t = hover_at(Dialect::Oracle, "select n|vl(null, 1) from dual").expect("a function");
    assert!(t.contains("nvl(value, replacement)"), "{t}");
    assert!(hover_at(PG, "select * from custo|mers_typo").is_none());
    assert!(hover_at(PG, "select 1 |+ 2").is_none());
}

fn help_at(d: Dialect, marked: &str) -> Option<SignatureHelp> {
    let (text, at) = split(marked);
    let (intel, doc) = open(ctx(d), &text);
    block(intel.signature_help(&doc, at, SignatureContext::invoked()))
}

#[test]
fn parameter_info_for_routines_of_the_model() {
    let h = help_at(PG, "select discount(|").expect("help");
    assert_eq!(h.signatures.len(), 2);
    let labels: Vec<&str> = h.signatures.iter().map(|s| s.label.as_str()).collect();
    assert!(
        labels.contains(&"discount(price numeric, rate numeric = default) returns numeric"),
        "{labels:?}"
    );
    assert!(labels.contains(&"discount(price numeric) returns numeric"));
    assert_eq!(h.active_parameter, Some(0));
    let h = help_at(PG, "select discount(1, |").expect("help");
    assert_eq!(h.active_parameter, Some(1));
    // The overload that has a second parameter is the active one.
    let active = &h.signatures[h.active_signature];
    assert!(active.label.contains("rate"), "{}", active.label);
    let p = &active.parameters[1];
    assert_eq!(&active.label[p.label.clone()], "rate numeric = default");
    // Named arguments.
    let h = help_at(PG, "select discount(rate => 2, |").expect("help");
    assert!(h.signatures[h.active_signature].label.contains("rate"));
    let h = help_at(PG, "select discount(rate => |").expect("help");
    let active = &h.signatures[h.active_signature];
    assert_eq!(active.active_parameter, Some(1));
    // Not inside a call, or after it.
    assert!(help_at(PG, "select discount(1) |").is_none());
    assert!(help_at(PG, "select (1 + |2)").is_none());
}

#[test]
fn parameter_info_for_builtins_and_nested_calls() {
    let h = help_at(PG, "select round(1.5, |").expect("help");
    assert_eq!(h.signatures[0].label, "round(number, [digits]) -> number");
    assert_eq!(h.active_parameter, Some(1));
    let p = &h.signatures[0].parameters[1];
    assert_eq!(&h.signatures[0].label[p.label.clone()], "[digits]");
    // The call around the caret, not the one before it.
    let h = help_at(PG, "select round(lower(name), |) from customers").expect("help");
    assert!(h.signatures[0].label.starts_with("round("));
    let h = help_at(PG, "select round(lower(|), 2) from customers").expect("help");
    assert!(h.signatures[0].label.starts_with("lower("));
    // Variable arguments keep the last parameter.
    let h = help_at(PG, "select coalesce(a, b, c, |").expect("help");
    assert_eq!(h.signatures[0].active_parameter, Some(1));
    let h = help_at(Dialect::Oracle, "select nvl(|").expect("help");
    assert!(h.signatures[0].label.starts_with("nvl("));
    assert!(help_at(PG, "select nothing_like_it(|").is_none());
}

/// `public` with `count` tables of two columns.
fn wide(count: usize) -> SqlContext {
    let d = PG;
    let objects = (0..count)
        .map(|i| object(d, "public", &format!("table_{i:05}"), ObjectKind::Table))
        .collect();
    let relations = (0..count)
        .map(|i| {
            table(
                d,
                &format!("table_{i:05}"),
                vec![col(d, "id", "integer", false), col(d, "name", "text", true)],
                Vec::new(),
            )
        })
        .collect();
    let public = schema(d, "public", true, Level::Structure, objects, relations);
    let model = Model::from_snapshot(
        DataSourceId::new("wide"),
        Snapshot {
            level: Level::Structure,
            token: None,
            schemas: vec![public],
        },
    );
    SqlContext::new(d, Some(Scope::new(Arc::new(model))))
}

#[test]
fn a_huge_schema_is_cut_without_a_prefix_and_filtered_with_one() {
    let c = complete_with(wide(5000), "select * from |");
    let tables = c
        .items
        .iter()
        .filter(|i| i.label.starts_with("table_"))
        .count();
    assert!(tables <= 2000, "{tables}");
    assert!(c.incomplete);
    let c = complete_with(wide(5000), "select * from table_042|");
    let mut names: Vec<String> = c
        .items
        .iter()
        .filter(|i| i.label.starts_with("table_"))
        .map(|i| i.label.clone())
        .collect();
    names.sort();
    assert_eq!(names.len(), 100, "{names:?}");
    assert_eq!(names[0], "table_04200");
    // A prefix that matches nearly everything is cut as well.
    let c = complete_with(wide(5000), "select * from t|");
    let tables = c
        .items
        .iter()
        .filter(|i| i.label.starts_with("table_"))
        .count();
    assert_eq!(tables, 2000);
    assert!(c.incomplete);
    // A small schema is offered whole.
    let c = complete_with(wide(30), "select * from |");
    assert_eq!(
        c.items
            .iter()
            .filter(|i| i.label.starts_with("table_"))
            .count(),
        30
    );
}

#[test]
fn next_to_a_placeholder_only_keywords_are_offered() {
    let hole = sql_syntax::HOLE;
    let c = complete(PG, &format!("select * from {hole}|"));
    assert!(!has(&c, "customers"), "{:?}", labels(&c));
    let c = complete(PG, &format!("select * from |{hole}"));
    assert!(!has(&c, "customers"), "{:?}", labels(&c));
    let c = complete(PG, "select * from |");
    assert!(has(&c, "customers"));
}
