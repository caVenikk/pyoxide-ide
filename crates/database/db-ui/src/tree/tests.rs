use std::collections::HashSet;
use std::sync::Arc;

use db_core::DataSourceId;
use db_model::{Model, NameMatch, NodeId, NodeKind};

use super::*;
use crate::fixture::demo_snapshot;

fn model() -> Model {
    Model::from_snapshot(DataSourceId::from("local"), demo_snapshot())
}

fn node<'a>(model: &'a Model, name: &str, kind: NodeKind) -> NodeRef<'a> {
    model
        .schema("demo", NameMatch::Exact)
        .unwrap()
        .find(name, NameMatch::Exact, &[kind])
        .unwrap_or_else(|| panic!("{name}"))
}

/// The rows as indented names, `v ` before an open one.
fn lines(model: &Model, rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            let text = match r.key.node {
                NodeKey::Source => r.key.source.to_string(),
                NodeKey::Database => "pyoxide".into(),
                NodeKey::Status => "status".into(),
                NodeKey::Node(id) => model.node(id).unwrap().name().to_owned(),
                NodeKey::Group(_, g) => g.label().to_owned(),
            };
            let open = if r.expanded { "v " } else { "" };
            format!("{}{open}{text}", "  ".repeat(r.depth))
        })
        .collect()
}

fn open(source: &Arc<str>, node: NodeKey) -> RowKey {
    RowKey {
        source: source.clone(),
        node,
    }
}

#[test]
fn levels_groups_and_nested_partitions() {
    let model = model();
    let name: Arc<str> = "local".into();
    let demo = model.schema("demo", NameMatch::Exact).unwrap().id();
    let measurements = node(&model, "measurements", NodeKind::Table).id();
    let customers = node(&model, "customers", NodeKind::Table).id();
    let expanded: HashSet<RowKey> = [
        open(&name, NodeKey::Source),
        open(&name, NodeKey::Database),
        open(&name, NodeKey::Node(demo)),
        open(&name, NodeKey::Group(demo, TreeGroup::Tables)),
        open(&name, NodeKey::Node(measurements)),
        open(&name, NodeKey::Group(measurements, TreeGroup::Partitions)),
        open(&name, NodeKey::Node(customers)),
    ]
    .into_iter()
    .collect();
    let sources = [SourceTree {
        name: &name,
        database: Some("pyoxide"),
        model: Some(&model),
        status: None,
    }];
    let rows = rows(&sources, &expanded);
    let got = lines(&model, &rows);
    let want = [
        "v local",
        "  v pyoxide",
        "    v demo",
        "      v tables",
        "        audit_log",
        "        v customers",
        "          columns",
        "          keys",
        "          indexes",
        "          triggers",
        "        v measurements",
        "          columns",
        "          v partitions",
        "            measurements_2026_01",
        "            measurements_2026_02",
        "        orders",
        "      materialized views",
        "      views",
        "      routines",
        "      sequences",
        "      object types",
        "    public",
    ];
    assert_eq!(got, want);
    // An empty schema still opens; a schema's empty groups are hidden.
    assert!(rows.last().unwrap().expandable);
}

#[test]
fn a_source_without_a_model_shows_its_status() {
    let name: Arc<str> = "local".into();
    let expanded: HashSet<RowKey> = [RowKey::source(&name)].into_iter().collect();
    let sources = [SourceTree {
        name: &name,
        database: Some("pyoxide"),
        model: None,
        status: Some(Status::Connecting),
    }];
    let got = rows(&sources, &expanded);
    assert_eq!(got.len(), 2);
    assert_eq!(got[1].key.node, NodeKey::Status);
    assert_eq!(got[1].depth, 1);
    // Closed: only the data source.
    assert_eq!(rows(&sources, &HashSet::new()).len(), 1);
}

#[test]
fn row_texts() {
    let model = model();
    let customers = node(&model, "customers", NodeKind::Table);
    let col = |name: &str| {
        let c = customers
            .group(db_model::Group::Columns)
            .nodes()
            .find(|c| c.name() == name)
            .unwrap();
        let v = node_view(c);
        (v.icon, v.detail.unwrap())
    };
    assert_eq!(
        col("id"),
        (RowIcon::ColumnKey, "integer (auto increment)".into())
    );
    assert_eq!(
        col("country"),
        (RowIcon::ColumnNotNull, "char(2) = 'DE'::bpchar".into())
    );
    assert_eq!(col("profile"), (RowIcon::Column, "jsonb".into()));
    assert_eq!(short_type("character(2)"), "char(2)");
    assert_eq!(short_type("character varying(100)[]"), "varchar(100)[]");
    assert_eq!(short_type("character"), "char");
    assert_eq!(short_type("characteristic"), "characteristic");
    assert_eq!(short_type("VARCHAR2(10)"), "VARCHAR2(10)");

    let idx = |name: &str| node_view(node(&model, name, NodeKind::Index));
    let v = idx("customers_email_lower_idx");
    assert_eq!(
        (v.icon, v.detail.as_deref()),
        (RowIcon::UniqueIndex, Some("(lower(email::text)) UNIQUE"))
    );
    assert_eq!(
        idx("customers_country_idx").detail.as_deref(),
        Some("(country) where is_active")
    );

    let orders = node(&model, "orders", NodeKind::Table);
    let fk = orders.group(db_model::Group::ForeignKeys).get(0).unwrap();
    assert_eq!(
        node_view(fk).detail.as_deref(),
        Some("(customer_id) \u{2192} customers(id)")
    );

    let routine = |name: &str, kind| node_view(node(&model, name, kind)).detail;
    assert_eq!(
        routine("order_count", NodeKind::Function).as_deref(),
        Some("(p_customer: integer): bigint")
    );
    assert_eq!(
        routine("touch_updated_at", NodeKind::Function).as_deref(),
        Some(": trigger")
    );
    assert_eq!(
        routine("cancel_old_pending", NodeKind::Procedure).as_deref(),
        Some("(p_before: timestamp with time zone)")
    );

    let demo = model.schema("demo", NameMatch::Exact).unwrap().root();
    let routines = group_view(demo, TreeGroup::Routines);
    assert_eq!(
        (routines.name.as_str(), routines.count),
        ("routines", Some(3))
    );
    let names: Vec<_> = TreeGroup::Routines
        .nodes(demo)
        .iter()
        .map(|n| n.name())
        .collect();
    assert_eq!(
        names,
        ["cancel_old_pending", "order_count", "touch_updated_at"]
    );
}

#[test]
fn ten_thousand_tables_flatten_within_a_frame() {
    let mut snapshot = demo_snapshot();
    let schema = &mut snapshot.schemas[0];
    for i in 0..10_000 {
        schema.objects.push(db_core::schema::ObjectInfo {
            schema: "demo".into(),
            name: format!("t{i:05}"),
            kind: db_core::schema::ObjectKind::Table,
            ..Default::default()
        });
    }
    let model = Model::from_snapshot(DataSourceId::from("big"), snapshot);
    let name: Arc<str> = "big".into();
    let demo: NodeId = model.schema("demo", NameMatch::Exact).unwrap().id();
    let expanded: HashSet<RowKey> = [
        RowKey::source(&name),
        open(&name, NodeKey::Database),
        open(&name, NodeKey::Node(demo)),
        open(&name, NodeKey::Group(demo, TreeGroup::Tables)),
    ]
    .into_iter()
    .collect();
    let sources = [SourceTree {
        name: &name,
        database: Some("big"),
        model: Some(&model),
        status: None,
    }];
    let started = std::time::Instant::now();
    let got = rows(&sources, &expanded);
    let took = started.elapsed();
    assert!(got.len() > 10_000);
    // One frame at 60 Hz in a release build; debug builds get room.
    let budget = if cfg!(debug_assertions) { 200 } else { 16 };
    assert!(took < std::time::Duration::from_millis(budget), "{took:?}");
}
