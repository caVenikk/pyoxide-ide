//! Oracle introspection (`db_core::schema::OracleIntrospector`) through the
//! thin driver. `docker compose up -d oracle`, then
//! `cargo test -p db-oracle-thin -- --ignored`.

mod common;

use common::*;
use db_core::schema::{ConstraintKind, ObjectKind, introspector};

#[tokio::test]
#[ignore]
async fn introspects_a_test_schema() {
    let c = connect().await;
    let p = prefix("intro");
    let ddl = [
        format!(
            "CREATE TABLE {p}PARENT (id NUMBER CONSTRAINT {p}PARENT_PK PRIMARY KEY, \
             code VARCHAR2(10 CHAR) NOT NULL CONSTRAINT {p}PARENT_UK UNIQUE)"
        ),
        format!(
            "CREATE TABLE {p}CHILD (\
             id NUMBER GENERATED ALWAYS AS IDENTITY CONSTRAINT {p}CHILD_PK PRIMARY KEY, \
             parent_id NUMBER CONSTRAINT {p}CHILD_FK REFERENCES {p}PARENT (id) ON DELETE CASCADE, \
             name VARCHAR2(50) DEFAULT 'x' NOT NULL, \
             amount NUMBER(12,2) CONSTRAINT {p}CHILD_CK CHECK (amount >= 0), \
             created TIMESTAMP(6) WITH TIME ZONE DEFAULT SYSTIMESTAMP)"
        ),
        format!("COMMENT ON TABLE {p}CHILD IS 'child rows'"),
        format!("COMMENT ON COLUMN {p}CHILD.name IS 'the name'"),
        format!("CREATE INDEX {p}CHILD_UPPER ON {p}CHILD (UPPER(name))"),
        format!("CREATE INDEX {p}CHILD_DESC ON {p}CHILD (amount DESC, parent_id)"),
        format!("CREATE VIEW {p}V AS SELECT id, name FROM {p}CHILD"),
        format!("CREATE SEQUENCE {p}SEQ START WITH 5"),
        format!("CREATE PROCEDURE {p}PROC AS BEGIN NULL; END;"),
        format!("CREATE PACKAGE {p}PKG AS FUNCTION f RETURN NUMBER; END;"),
        format!(
            "CREATE PACKAGE BODY {p}PKG AS FUNCTION f RETURN NUMBER IS BEGIN RETURN 1; END; END;"
        ),
        format!(
            "CREATE TRIGGER {p}TRG BEFORE INSERT ON {p}CHILD FOR EACH ROW \
             BEGIN :new.name := UPPER(:new.name); END;"
        ),
        format!("CREATE SYNONYM {p}SYN FOR {p}CHILD"),
        format!("CREATE TYPE {p}T AS OBJECT (a NUMBER)"),
    ];
    for stmt in &ddl {
        exec(&*c, stmt, &[]).await;
    }

    let intro = introspector(&*c);
    let schema = intro.current_schema().await.unwrap();
    assert_eq!(schema, "PYOXIDE");

    let schemas = intro.schemas().await.unwrap();
    let me = schemas.iter().find(|s| s.name == schema).unwrap();
    assert!(me.is_current && !me.is_system);
    assert!(
        schemas
            .iter()
            .any(|s| s.name == "SYS" && s.is_system && !s.is_current)
    );

    let objects = intro.objects(&schema, &[]).await.unwrap();
    let find = |name: &str, kind: ObjectKind| {
        objects
            .iter()
            .find(|o| o.name == format!("{p}{name}") && o.kind == kind)
            .unwrap_or_else(|| panic!("{name} {kind:?} missing"))
    };
    assert_eq!(
        find("CHILD", ObjectKind::Table).comment.as_deref(),
        Some("child rows")
    );
    find("PARENT", ObjectKind::Table);
    find("V", ObjectKind::View);
    find("SEQ", ObjectKind::Sequence);
    find("PROC", ObjectKind::Procedure);
    find("PKG", ObjectKind::Package);
    find("SYN", ObjectKind::Synonym);
    find("T", ObjectKind::Type);
    let child = format!("{p}CHILD");
    assert_eq!(
        find("CHILD_UPPER", ObjectKind::Index).parent.as_deref(),
        Some(child.as_str())
    );
    assert_eq!(
        find("TRG", ObjectKind::Trigger).parent.as_deref(),
        Some(child.as_str())
    );
    let only_views = intro.objects(&schema, &[ObjectKind::View]).await.unwrap();
    assert!(only_views.iter().all(|o| o.kind == ObjectKind::View));
    assert!(only_views.iter().any(|o| o.name == format!("{p}V")));

    let cols = intro.columns(&schema, &child).await.unwrap();
    let names: Vec<_> = cols.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["ID", "PARENT_ID", "NAME", "AMOUNT", "CREATED"]);
    assert!(cols[0].auto_increment && !cols[0].nullable);
    assert_eq!(cols[0].data_type, "NUMBER");
    assert_eq!(cols[2].data_type, "VARCHAR2(50)");
    assert_eq!(cols[2].default.as_deref(), Some("'x'"));
    assert_eq!(cols[2].comment.as_deref(), Some("the name"));
    assert!(!cols[2].nullable);
    assert_eq!(cols[3].data_type, "NUMBER(12,2)");
    assert!(cols[3].nullable && !cols[3].auto_increment);
    assert_eq!(cols[4].data_type, "TIMESTAMP(6) WITH TIME ZONE");
    assert_eq!(cols[4].default.as_deref(), Some("SYSTIMESTAMP"));
    let parent_cols = intro.columns(&schema, &format!("{p}PARENT")).await.unwrap();
    assert_eq!(parent_cols[1].data_type, "VARCHAR2(10 CHAR)");

    let indexes = intro.indexes(&schema, &child).await.unwrap();
    let ix = |name: &str| {
        indexes
            .iter()
            .find(|i| i.name == format!("{p}{name}"))
            .unwrap_or_else(|| panic!("index {name} missing: {indexes:?}"))
    };
    assert!(ix("CHILD_PK").unique);
    assert_eq!(ix("CHILD_PK").columns, ["ID"]);
    assert_eq!(ix("CHILD_UPPER").columns, ["UPPER(\"NAME\")"]);
    assert_eq!(
        ix("CHILD_UPPER").method.as_deref(),
        Some("FUNCTION-BASED NORMAL")
    );
    assert_eq!(ix("CHILD_DESC").columns, ["AMOUNT DESC", "PARENT_ID"]);
    assert!(!ix("CHILD_DESC").unique);

    let cons = intro.constraints(&schema, &child).await.unwrap();
    let con = |name: &str| {
        cons.iter()
            .find(|c| c.name == format!("{p}{name}"))
            .unwrap_or_else(|| panic!("constraint {name} missing: {cons:?}"))
    };
    assert_eq!(con("CHILD_PK").kind, ConstraintKind::PrimaryKey);
    assert_eq!(con("CHILD_PK").columns, ["ID"]);
    let fk = con("CHILD_FK");
    assert_eq!(fk.kind, ConstraintKind::ForeignKey);
    assert_eq!(fk.columns, ["PARENT_ID"]);
    assert_eq!(fk.ref_schema.as_deref(), Some(schema.as_str()));
    assert_eq!(fk.ref_table, Some(format!("{p}PARENT")));
    assert_eq!(fk.ref_columns, ["ID"]);
    assert_eq!(fk.on_delete.as_deref(), Some("CASCADE"));
    let ck = con("CHILD_CK");
    assert_eq!(ck.kind, ConstraintKind::Check);
    assert_eq!(ck.check.as_deref(), Some("amount >= 0"));
    // Generated NOT NULL checks are skipped.
    assert!(cons.iter().all(|c| {
        c.check
            .as_deref()
            .is_none_or(|t| !t.contains("IS NOT NULL"))
    }));
    let parent_cons = intro
        .constraints(&schema, &format!("{p}PARENT"))
        .await
        .unwrap();
    let kinds: Vec<_> = parent_cons.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [ConstraintKind::PrimaryKey, ConstraintKind::Unique]);

    let src = |name: &str, kind| {
        let intro = &intro;
        let full = format!("{p}{name}");
        let schema = schema.clone();
        async move { intro.source(&schema, &full, kind).await.unwrap() }
    };
    let view = src("V", ObjectKind::View).await.unwrap();
    assert!(
        view.starts_with("CREATE OR REPLACE VIEW \"PYOXIDE\"."),
        "{view}"
    );
    assert!(view.contains("SELECT id, name"), "{view}");
    let pkg = src("PKG", ObjectKind::Package).await.unwrap();
    assert!(pkg.starts_with("CREATE OR REPLACE PACKAGE"), "{pkg}");
    assert!(pkg.contains("CREATE OR REPLACE PACKAGE BODY"), "{pkg}");
    assert!(
        src("PROC", ObjectKind::Procedure)
            .await
            .unwrap()
            .contains("PROCEDURE")
    );
    assert!(
        src("TRG", ObjectKind::Trigger)
            .await
            .unwrap()
            .contains(":new.name")
    );
    assert!(src("T", ObjectKind::Type).await.unwrap().contains("OBJECT"));
    let syn = src("SYN", ObjectKind::Synonym).await.unwrap();
    assert!(
        syn.contains(&format!("FOR \"PYOXIDE\".\"{p}CHILD\"")),
        "{syn}"
    );
    let seq = src("SEQ", ObjectKind::Sequence).await.unwrap();
    assert!(seq.contains("CREATE SEQUENCE"), "{seq}");
    let table = src("CHILD", ObjectKind::Table).await.unwrap();
    assert!(table.contains("CREATE TABLE"), "{table}");
    let index = src("CHILD_UPPER", ObjectKind::Index).await.unwrap();
    assert!(index.contains("CREATE INDEX"), "{index}");
    assert_eq!(src("NO_SUCH", ObjectKind::Table).await, None);
    assert_eq!(src("NO_SUCH", ObjectKind::Procedure).await, None);

    for stmt in [
        format!("DROP TYPE {p}T"),
        format!("DROP SYNONYM {p}SYN"),
        format!("DROP PACKAGE {p}PKG"),
        format!("DROP PROCEDURE {p}PROC"),
        format!("DROP SEQUENCE {p}SEQ"),
        format!("DROP VIEW {p}V"),
        format!("DROP TABLE {p}CHILD PURGE"),
        format!("DROP TABLE {p}PARENT PURGE"),
    ] {
        exec(&*c, &stmt, &[]).await;
    }
}
