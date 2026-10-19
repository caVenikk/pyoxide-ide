use crate::common::{memory, run};
use db_core::ErrorKind;
use db_core::schema::{ConstraintInfo, ConstraintKind, ObjectKind, introspector};

const SCHEMA: &[&str] = &[
    "CREATE TABLE \"Parent Table\" (\
        \"Id\" INTEGER PRIMARY KEY, \
        \"Full Name\" TEXT NOT NULL, \
        Code TEXT NOT NULL DEFAULT 'x', \
        CONSTRAINT \"uq code\" UNIQUE (Code))",
    "CREATE TABLE \"Order Items\" (\
        \"Order Id\" INTEGER NOT NULL, \
        Line INTEGER NOT NULL, \
        \"Parent Id\" INTEGER REFERENCES \"Parent Table\"(\"Id\") ON DELETE CASCADE, \
        qty INTEGER DEFAULT 1 CONSTRAINT qty_positive CHECK (qty > 0), \
        price REAL, \
        tag TEXT UNIQUE, \
        PRIMARY KEY (\"Order Id\", Line), \
        CHECK (price IS NULL OR price >= 0))",
    "CREATE INDEX \"Idx Mixed\" ON \"Order Items\"(\"Parent Id\", lower(tag) COLLATE NOCASE)",
    "CREATE VIEW \"Item View\" AS SELECT \"Order Id\", qty FROM \"Order Items\"",
    "CREATE TRIGGER \"trg Log\" AFTER INSERT ON \"Order Items\" BEGIN SELECT 1; END",
    "CREATE TABLE autoinc (id INTEGER PRIMARY KEY AUTOINCREMENT, v)",
    "CREATE TABLE wr (a INTEGER PRIMARY KEY, b TEXT) WITHOUT ROWID",
    "CREATE TABLE plain (a, b)",
    "CREATE TABLE \"we\"\"ird\" (\"col \"\"q\"\"\" TEXT)",
    "CREATE TABLE many_fk (\
        a INTEGER, b INTEGER, c INTEGER, \
        CONSTRAINT first_fk FOREIGN KEY (a) REFERENCES plain_parent, \
        CONSTRAINT second_fk FOREIGN KEY (b, c) REFERENCES pair_parent(x, y) ON DELETE SET NULL)",
    "CREATE TABLE plain_parent (id INTEGER PRIMARY KEY)",
    "CREATE TABLE pair_parent (x INTEGER, y INTEGER, PRIMARY KEY (x, y))",
    "INSERT INTO autoinc(v) VALUES (1)",
];

async fn fixture() -> std::sync::Arc<dyn db_core::Connection> {
    let conn = memory().await;
    for sql in SCHEMA {
        run(&conn, sql).await;
    }
    conn
}

fn names(items: &[db_core::schema::ObjectInfo]) -> Vec<&str> {
    items.iter().map(|o| o.name.as_str()).collect()
}

#[tokio::test]
async fn schemas_and_current_schema() {
    let conn = fixture().await;
    let i = introspector(&*conn);
    assert_eq!(i.current_schema().await.unwrap(), "main");
    let schemas = i.schemas().await.unwrap();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].name, "main");
    assert!(schemas[0].is_current);
    assert!(!schemas[0].is_system);
}

#[tokio::test]
async fn lists_objects_by_kind() {
    let conn = fixture().await;
    let i = introspector(&*conn);

    let all = i.objects("main", &[]).await.unwrap();
    let tables: Vec<_> = all
        .iter()
        .filter(|o| o.kind == ObjectKind::Table)
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(
        tables,
        [
            "Order Items",
            "Parent Table",
            "autoinc",
            "many_fk",
            "pair_parent",
            "plain",
            "plain_parent",
            "we\"ird",
            "wr"
        ]
    );
    assert!(
        !all.iter().any(|o| o.name == "sqlite_sequence"),
        "internal tables are hidden"
    );
    assert!(
        all.iter()
            .all(|o| o.schema == "main" && o.comment.is_none())
    );

    let views = i.objects("main", &[ObjectKind::View]).await.unwrap();
    assert_eq!(names(&views), ["Item View"]);
    assert_eq!(views[0].parent, None);

    let triggers = i.objects("main", &[ObjectKind::Trigger]).await.unwrap();
    assert_eq!(names(&triggers), ["trg Log"]);
    assert_eq!(triggers[0].parent.as_deref(), Some("Order Items"));

    // Named and automatic indexes, each pointing at its table.
    let indexes = i.objects("main", &[ObjectKind::Index]).await.unwrap();
    let mixed = indexes.iter().find(|o| o.name == "Idx Mixed").unwrap();
    assert_eq!(mixed.parent.as_deref(), Some("Order Items"));
    assert!(
        indexes
            .iter()
            .any(|o| o.name.starts_with("sqlite_autoindex_") && o.parent.is_some())
    );

    let both = i
        .objects("main", &[ObjectKind::View, ObjectKind::Trigger])
        .await
        .unwrap();
    assert_eq!(names(&both), ["Item View", "trg Log"]);

    // Kinds SQLite does not have are simply absent.
    assert!(
        i.objects("main", &[ObjectKind::Sequence])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn columns_with_types_defaults_and_keys() {
    let conn = fixture().await;
    let i = introspector(&*conn);

    let cols = i.columns("main", "Parent Table").await.unwrap();
    assert_eq!(
        cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Id", "Full Name", "Code"]
    );
    assert_eq!(
        cols.iter().map(|c| c.position).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(cols[0].data_type, "INTEGER");
    assert!(!cols[0].nullable, "a rowid alias is never NULL");
    assert!(cols[0].auto_increment);
    assert!(!cols[1].nullable);
    assert!(!cols[1].auto_increment);
    assert_eq!(cols[2].default.as_deref(), Some("'x'"));

    let items = i.columns("main", "Order Items").await.unwrap();
    let qty = items.iter().find(|c| c.name == "qty").unwrap();
    assert_eq!(qty.default.as_deref(), Some("1"));
    assert!(qty.nullable);
    let price = items.iter().find(|c| c.name == "price").unwrap();
    assert_eq!(price.data_type, "REAL");
    assert!(
        items.iter().all(|c| !c.auto_increment),
        "a composite key has no rowid alias"
    );

    let auto = i.columns("main", "autoinc").await.unwrap();
    assert!(auto[0].auto_increment);
    assert!(auto[1].nullable);
    assert_eq!(auto[1].data_type, "", "no declared type");

    // WITHOUT ROWID: an INTEGER PRIMARY KEY is a real column, not the rowid.
    let wr = i.columns("main", "wr").await.unwrap();
    assert!(!wr[0].auto_increment);
    assert!(!wr[0].nullable);

    let weird = i.columns("main", "we\"ird").await.unwrap();
    assert_eq!(weird.len(), 1);
    assert_eq!(weird[0].name, "col \"q\"");

    // A view has columns too, and no rowid alias.
    let view = i.columns("main", "Item View").await.unwrap();
    assert_eq!(
        view.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Order Id", "qty"]
    );

    // Names match the way SQLite matches them, ignoring case.
    assert_eq!(i.columns("MAIN", "parent table").await.unwrap().len(), 3);
    // An unknown table has no columns.
    assert!(i.columns("main", "nope").await.unwrap().is_empty());
}

#[tokio::test]
async fn indexes_with_columns_and_expressions() {
    let conn = fixture().await;
    let i = introspector(&*conn);
    let indexes = i.indexes("main", "Order Items").await.unwrap();

    let mixed = indexes.iter().find(|x| x.name == "Idx Mixed").unwrap();
    assert!(!mixed.unique);
    assert_eq!(mixed.columns, ["Parent Id", "lower(tag)"]);
    assert_eq!(mixed.method, None);

    let unique: Vec<_> = indexes
        .iter()
        .filter(|x| x.unique)
        .map(|x| x.columns.clone())
        .collect();
    assert!(unique.contains(&vec!["Order Id".to_owned(), "Line".to_owned()]));
    assert!(unique.contains(&vec!["tag".to_owned()]));
    assert_eq!(indexes.len(), 3);

    assert!(i.indexes("main", "plain").await.unwrap().is_empty());
}

fn find(items: &[ConstraintInfo], kind: ConstraintKind) -> Vec<&ConstraintInfo> {
    items.iter().filter(|c| c.kind == kind).collect()
}

#[tokio::test]
async fn constraints_primary_unique_foreign_and_check() {
    let conn = fixture().await;
    let i = introspector(&*conn);

    let cs = i.constraints("main", "Order Items").await.unwrap();
    let pk = find(&cs, ConstraintKind::PrimaryKey);
    assert_eq!(pk.len(), 1);
    assert_eq!(pk[0].columns, ["Order Id", "Line"]);
    assert_eq!(pk[0].name, "Order Items_pk");

    let uq = find(&cs, ConstraintKind::Unique);
    assert_eq!(uq.len(), 1);
    assert_eq!(uq[0].columns, ["tag"]);
    assert!(
        uq[0].name.starts_with("sqlite_autoindex_"),
        "{}",
        uq[0].name
    );

    let fk = find(&cs, ConstraintKind::ForeignKey);
    assert_eq!(fk.len(), 1);
    assert_eq!(fk[0].columns, ["Parent Id"]);
    assert_eq!(fk[0].ref_schema.as_deref(), Some("main"));
    assert_eq!(fk[0].ref_table.as_deref(), Some("Parent Table"));
    assert_eq!(fk[0].ref_columns, ["Id"]);
    assert_eq!(fk[0].on_delete.as_deref(), Some("CASCADE"));

    let ck = find(&cs, ConstraintKind::Check);
    assert_eq!(ck.len(), 2);
    assert_eq!(ck[0].name, "qty_positive");
    assert_eq!(ck[0].check.as_deref(), Some("qty > 0"));
    assert_eq!(ck[1].check.as_deref(), Some("price IS NULL OR price >= 0"));
    assert_eq!(ck[1].name, "Order Items_check2");

    let parent = i.constraints("main", "Parent Table").await.unwrap();
    assert_eq!(parent.len(), 2);
    assert_eq!(parent[0].kind, ConstraintKind::PrimaryKey);
    assert_eq!(parent[0].columns, ["Id"]);
    assert_eq!(parent[1].kind, ConstraintKind::Unique);
    assert_eq!(parent[1].name, "uq code");
    assert_eq!(parent[1].columns, ["Code"]);

    assert!(i.constraints("main", "plain").await.unwrap().is_empty());
    assert!(i.constraints("main", "Item View").await.unwrap().is_empty());
}

#[tokio::test]
async fn foreign_keys_in_declaration_order_with_implicit_targets() {
    let conn = fixture().await;
    let i = introspector(&*conn);
    let cs = i.constraints("main", "many_fk").await.unwrap();
    let fk = find(&cs, ConstraintKind::ForeignKey);
    assert_eq!(fk.len(), 2);

    assert_eq!(fk[0].name, "first_fk");
    assert_eq!(fk[0].columns, ["a"]);
    assert_eq!(fk[0].ref_table.as_deref(), Some("plain_parent"));
    // `REFERENCES plain_parent` points at the parent's primary key.
    assert_eq!(fk[0].ref_columns, ["id"]);

    assert_eq!(fk[1].name, "second_fk");
    assert_eq!(fk[1].columns, ["b", "c"]);
    assert_eq!(fk[1].ref_columns, ["x", "y"]);
    assert_eq!(fk[1].on_delete.as_deref(), Some("SET NULL"));
}

#[tokio::test]
async fn source_of_each_object() {
    let conn = fixture().await;
    let i = introspector(&*conn);

    let table = i
        .source("main", "Parent Table", ObjectKind::Table)
        .await
        .unwrap()
        .unwrap();
    assert!(
        table.starts_with("CREATE TABLE \"Parent Table\""),
        "{table}"
    );
    let view = i
        .source("main", "Item View", ObjectKind::View)
        .await
        .unwrap()
        .unwrap();
    assert!(view.contains("SELECT \"Order Id\", qty"), "{view}");
    let trigger = i
        .source("main", "trg Log", ObjectKind::Trigger)
        .await
        .unwrap()
        .unwrap();
    assert!(trigger.contains("AFTER INSERT"), "{trigger}");
    let index = i
        .source("main", "Idx Mixed", ObjectKind::Index)
        .await
        .unwrap()
        .unwrap();
    assert!(index.contains("lower(tag)"), "{index}");

    // A table is not a view, automatic indexes have no text, nothing for
    // kinds SQLite lacks.
    assert!(
        i.source("main", "Parent Table", ObjectKind::View)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        i.source("main", "nope", ObjectKind::Table)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        i.source("main", "Parent Table", ObjectKind::Package)
            .await
            .unwrap()
            .is_none()
    );
    let auto_index = i
        .objects("main", &[ObjectKind::Index])
        .await
        .unwrap()
        .into_iter()
        .find(|o| o.name.starts_with("sqlite_autoindex_"))
        .unwrap();
    assert!(
        i.source("main", &auto_index.name, ObjectKind::Index)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn attached_and_temp_schemas() {
    let dir = tempfile::tempdir().unwrap();
    let conn = fixture().await;
    let path = dir.path().join("second.db");
    run(
        &conn,
        &format!("ATTACH DATABASE '{}' AS \"other db\"", path.display()),
    )
    .await;
    run(
        &conn,
        "CREATE TABLE \"other db\".\"Remote Table\" (\"Key\" INTEGER PRIMARY KEY, Val TEXT)",
    )
    .await;
    run(
        &conn,
        "CREATE INDEX \"other db\".remote_val ON \"Remote Table\"(Val)",
    )
    .await;
    run(&conn, "CREATE TEMP TABLE scratch (n INTEGER)").await;

    let i = introspector(&*conn);
    let schemas = i.schemas().await.unwrap();
    let schema_names: Vec<_> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(schema_names, ["main", "temp", "other db"]);
    assert!(schemas.iter().all(|s| !s.is_system));
    assert_eq!(
        schemas
            .iter()
            .filter(|s| s.is_current)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["main"]
    );

    let remote = i.objects("other db", &[]).await.unwrap();
    assert_eq!(names(&remote), ["Remote Table", "remote_val"]);
    assert!(remote.iter().all(|o| o.schema == "other db"));
    // Schema names match without regard to case and report the stored name.
    let again = i.objects("OTHER DB", &[ObjectKind::Table]).await.unwrap();
    assert_eq!(again[0].schema, "other db");

    let cols = i.columns("other db", "Remote Table").await.unwrap();
    assert_eq!(cols.len(), 2);
    assert!(cols[0].auto_increment);
    assert_eq!(
        i.indexes("other db", "Remote Table").await.unwrap()[0].columns,
        ["Val"]
    );
    assert!(
        i.source("other db", "Remote Table", ObjectKind::Table)
            .await
            .unwrap()
            .unwrap()
            .contains("Remote Table")
    );
    // The same table name in two schemas stays apart.
    assert!(i.columns("main", "Remote Table").await.unwrap().is_empty());

    let temp = i.objects("temp", &[]).await.unwrap();
    assert_eq!(names(&temp), ["scratch"]);
    assert_eq!(i.columns("temp", "scratch").await.unwrap().len(), 1);
}

#[tokio::test]
async fn temp_schema_exists_before_anything_is_stored_in_it() {
    let conn = memory().await;
    let i = introspector(&*conn);
    assert!(i.objects("temp", &[]).await.unwrap().is_empty());
}

#[tokio::test]
async fn schema_names_are_validated_not_spliced() {
    let conn = fixture().await;
    let i = introspector(&*conn);
    for hostile in [
        "main\"; DROP TABLE plain; --",
        "nope",
        "",
        "main\" UNION SELECT 1,2,3 --",
    ] {
        let err = i.objects(hostile, &[]).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Query, "{hostile:?}");
        assert!(err.message.contains("unknown schema"), "{}", err.message);
        assert!(i.source(hostile, "plain", ObjectKind::Table).await.is_err());
    }
    assert_eq!(i.columns("main", "plain").await.unwrap().len(), 2);
    // Table names are bound, so quotes in them are harmless.
    assert!(
        i.columns("main", "x\"; DROP TABLE plain; --")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(i.columns("main", "plain").await.unwrap().len(), 2);
}

#[tokio::test]
async fn introspection_needs_a_free_connection() {
    let conn = fixture().await;
    run(&conn, "INSERT INTO plain VALUES (1, 1), (2, 2)").await;
    let db_core::Outcome::Rows(cursor) = conn
        .execute("SELECT * FROM plain", &[], &Default::default())
        .await
        .unwrap()
    else {
        panic!("rows expected");
    };
    // While a cursor is open on the connection, metadata queries are refused;
    // the IDE uses a second connection for them.
    let i = introspector(&*conn);
    assert_eq!(i.current_schema().await.unwrap(), "main");
    assert_eq!(
        i.columns("main", "plain").await.unwrap_err().kind,
        ErrorKind::Busy
    );
    drop(cursor);
    assert_eq!(i.columns("main", "plain").await.unwrap().len(), 2);
}
