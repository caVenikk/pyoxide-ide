//! The tree built from a snapshot: groups, nesting, order, ids.
use db_core::DataSourceId;
use db_core::schema::{ConstraintInfo, ConstraintKind, Level, ObjectKind, Snapshot};
use db_model::{Group, Model, NameMatch, NodeId, NodeKind, NodeRef};

use crate::fixture::{
    child, column, constraint, object, partition, partitioned, relation, schema, shop, token,
};

fn ds() -> DataSourceId {
    DataSourceId::new("shop")
}

fn names<'a>(nodes: impl Iterator<Item = NodeRef<'a>>) -> Vec<&'a str> {
    nodes.map(|n| n.name()).collect()
}

#[test]
fn schemas_are_sorted_and_have_their_level() {
    let model = Model::from_snapshot(ds(), shop());
    assert_eq!(model.data_source().as_str(), "shop");
    assert_eq!(model.token(), Some(&token("1")));
    let schemas: Vec<_> = model.schemas().map(|s| (s.name(), s.level())).collect();
    assert_eq!(
        schemas,
        [("app", Level::Names), ("public", Level::Structure)]
    );
    let public = model.schema("PUBLIC", NameMatch::IgnoreCase).unwrap();
    assert_eq!(public.root().kind(), NodeKind::Schema);
    assert_eq!(public.root().id(), NodeId::of_schema("public"));
    assert!(model.schema("PUBLIC", NameMatch::Exact).is_none());
}

#[test]
fn schema_groups_list_every_kind_and_report_empty_ones() {
    let model = Model::from_snapshot(ds(), shop());
    let public = model.schema("public", NameMatch::Exact).unwrap();
    let groups: Vec<_> = public
        .root()
        .groups()
        .map(|g| (g.group(), g.len(), g.is_loaded()))
        .collect();
    assert_eq!(
        groups,
        [
            (Group::Tables, 3, true),
            (Group::Views, 1, true),
            (Group::MaterializedViews, 0, true),
            (Group::Sequences, 1, true),
            (Group::Functions, 2, true),
            (Group::Procedures, 0, true),
            (Group::Packages, 0, true),
            (Group::Types, 0, true),
            (Group::Synonyms, 0, true),
            (Group::Indexes, 0, true),
            (Group::Triggers, 0, true),
        ]
    );
    // Partitions are not among the schema's tables: they hang under theirs.
    assert_eq!(
        names(public.group(Group::Tables).nodes()),
        ["customers", "events", "orders"]
    );
}

#[test]
fn partitions_hang_under_their_table() {
    let model = Model::from_snapshot(ds(), shop());
    let events = model
        .find("public", "events", NameMatch::Exact, &[NodeKind::Table])
        .unwrap();
    let partitions = events.group(Group::Partitions);
    assert_eq!(names(partitions.nodes()), ["events_eu", "events_us"]);
    let eu = partitions.get(0).unwrap();
    assert!(eu.is_partition());
    assert_eq!(eu.parent(), Some(events));
    assert_eq!(eu.partition_of().unwrap().table, "events");
    assert!(!events.is_partition());
    // Name lookup finds a partition like any table.
    let found = model
        .find(
            "public",
            "events_us",
            NameMatch::Exact,
            &NodeKind::RELATIONS,
        )
        .unwrap();
    assert!(found.is_partition());
    // Its columns are its own.
    assert_eq!(names(found.group(Group::Columns).nodes()), ["region"]);
}

#[test]
fn a_partition_of_a_table_in_another_schema_stays_in_its_schema() {
    let snapshot = Snapshot {
        level: Level::Names,
        token: None,
        schemas: vec![
            schema("a", Level::Names, vec![partitioned("a", "parent")], vec![]),
            schema(
                "b",
                Level::Names,
                vec![
                    partition("b", "part", "a", "parent"),
                    // A loop of partitions (never in a real database) is cut.
                    partition("b", "x", "b", "y"),
                    partition("b", "y", "b", "x"),
                ],
                vec![],
            ),
        ],
    };
    let model = Model::from_snapshot(ds(), snapshot);
    let b = model.schema("b", NameMatch::Exact).unwrap();
    assert_eq!(names(b.group(Group::Tables).nodes()), ["part", "x"]);
    let part = b.find("part", NameMatch::Exact, &[]).unwrap();
    assert!(!part.is_partition());
    assert_eq!(part.partition_of().unwrap().schema, "a");
    let x = b.find("x", NameMatch::Exact, &[]).unwrap();
    assert_eq!(names(x.group(Group::Partitions).nodes()), ["y"]);
}

#[test]
fn table_children_in_groups_and_order() {
    let model = Model::from_snapshot(ds(), shop());
    let orders = model
        .find("public", "orders", NameMatch::Exact, &[NodeKind::Table])
        .unwrap();
    assert!(orders.structure_loaded());
    let groups: Vec<_> = orders
        .groups()
        .map(|g| (g.group(), names(g.nodes())))
        .collect();
    assert_eq!(
        groups,
        [
            (Group::Columns, vec!["id", "customer_id", "total"]),
            (Group::Keys, vec!["orders_pkey"]),
            (Group::ForeignKeys, vec!["orders_customer_fk"]),
            (Group::Checks, vec!["orders_total_check"]),
            (Group::Indexes, vec!["orders_customer_idx", "orders_pkey"]),
            (Group::Triggers, vec!["orders_audit"]),
            (Group::Partitions, vec![]),
        ]
    );
    let id = orders
        .child("ID", NameMatch::IgnoreCase, &[NodeKind::Column])
        .unwrap();
    assert_eq!(id.column().unwrap().data_type, "integer");
    assert!(orders.child("ID", NameMatch::Exact, &[]).is_none());
    // The index listed as an object and the one read with the table are one
    // node with both.
    let idx = orders.group(Group::Indexes).get(0).unwrap();
    assert_eq!(idx.kind(), NodeKind::Index);
    assert_eq!(idx.object().unwrap().parent.as_deref(), Some("orders"));
    assert_eq!(idx.index().unwrap().columns, ["customer_id"]);
    let pkey_index = orders.group(Group::Indexes).get(1).unwrap();
    assert!(pkey_index.object().is_none() && pkey_index.index().is_some());
    let fk = orders.group(Group::ForeignKeys).get(0).unwrap();
    assert_eq!(
        fk.constraint().unwrap().ref_table.as_deref(),
        Some("customers")
    );
    assert_eq!(fk.parent(), Some(orders));
}

#[test]
fn routines_have_their_arguments_and_overloads() {
    let model = Model::from_snapshot(ds(), shop());
    let overloads: Vec<_> = model
        .lookup("public", "total", NameMatch::Exact, &NodeKind::ROUTINES)
        .map(|n| n.signature())
        .collect();
    assert_eq!(overloads, ["integer", "integer, text"]);
    let two = model
        .lookup("public", "total", NameMatch::Exact, &[])
        .nth(1)
        .unwrap();
    let args = two.group(Group::Arguments);
    assert!(args.is_loaded());
    assert_eq!(names(args.nodes()), ["order_id", "$2"]);
    assert_eq!(args.get(1).unwrap().argument().unwrap().data_type, "text");
    assert_ne!(
        model
            .lookup("public", "total", NameMatch::Exact, &[])
            .next()
            .unwrap()
            .id(),
        two.id()
    );
}

#[test]
fn names_only_schemas_report_structure_not_loaded() {
    let model = Model::from_snapshot(ds(), shop());
    let users = model.find("app", "users", NameMatch::Exact, &[]).unwrap();
    assert!(!users.structure_loaded());
    let columns = users.group(Group::Columns);
    assert!(columns.is_empty() && !columns.is_loaded());
    // Indexes and triggers come with names: known to be empty.
    let indexes = users.group(Group::Indexes);
    assert!(indexes.is_empty() && indexes.is_loaded());
}

#[test]
fn ids_are_unique_and_found_again() {
    let model = Model::from_snapshot(ds(), shop());
    let mut ids = Vec::new();
    for schema in model.schemas() {
        for node in schema.nodes() {
            assert_eq!(model.node(node.id()), Some(node), "{node:?}");
            ids.push(node.id());
        }
    }
    let count = ids.len();
    assert_eq!(count, model.len());
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count);
    let missing: NodeId = "0000000000000000000000000000000f".parse().unwrap();
    assert!(model.node(missing).is_none());
}

#[test]
fn ids_depend_on_kind_and_qualified_name_only() {
    let model = Model::from_snapshot(ds(), shop());
    let id = |m: &Model, schema: &str, name: &str| {
        m.find(schema, name, NameMatch::Exact, &[]).unwrap().id()
    };
    // Same name, other schema or other case: other id.
    let upper = id(&model, "app", "Users");
    let lower = id(&model, "app", "users");
    assert_ne!(upper, lower);
    assert_eq!(upper.schema_id(), NodeId::of_schema("app"));

    // Another database with the same table, among other content.
    let other = Snapshot {
        level: Level::Names,
        token: None,
        schemas: vec![schema(
            "app",
            Level::Names,
            vec![
                object("app", "aaa", ObjectKind::Table),
                object("app", "users", ObjectKind::Table),
                object("app", "v", ObjectKind::View),
            ],
            vec![],
        )],
    };
    let other = Model::from_snapshot(DataSourceId::new("other"), other);
    assert_eq!(id(&other, "app", "users"), lower);
    // The id is printable and parses back (tree state can be stored).
    let text = lower.to_string();
    assert_eq!(text.parse::<NodeId>().unwrap(), lower);
}

#[test]
fn orphans_and_unnamed_constraints() {
    let snapshot = Snapshot {
        level: Level::Structure,
        token: None,
        schemas: vec![schema(
            "main",
            Level::Structure,
            vec![
                object("main", "t", ObjectKind::Table),
                child("main", "lost_idx", ObjectKind::Index, "gone"),
            ],
            vec![
                relation(
                    "t",
                    vec![column("a", 1, "integer")],
                    vec![],
                    vec![
                        ConstraintInfo {
                            check: Some("a > 0".into()),
                            ..constraint("", ConstraintKind::Check, &[])
                        },
                        ConstraintInfo {
                            check: Some("a < 9".into()),
                            ..constraint("", ConstraintKind::Check, &[])
                        },
                    ],
                ),
                // Structure of an object the snapshot does not list is dropped.
                relation("ghost", vec![column("x", 1, "int")], vec![], vec![]),
            ],
        )],
    };
    let model = Model::from_snapshot(ds(), snapshot);
    let main = model.schema("main", NameMatch::Exact).unwrap();
    assert_eq!(names(main.group(Group::Indexes).nodes()), ["lost_idx"]);
    let t = main.find("t", NameMatch::Exact, &[]).unwrap();
    let checks: Vec<_> = t.group(Group::Checks).nodes().map(|n| n.id()).collect();
    assert_eq!(checks.len(), 2);
    assert_ne!(checks[0], checks[1]);
    assert!(main.find("ghost", NameMatch::Exact, &[]).is_none());
    assert_eq!(main.to_snapshot().relations.len(), 1);
}

#[test]
fn the_model_is_shared_across_threads() {
    fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<Model>();
    is_send_sync::<std::sync::Arc<Model>>();
    let model = std::sync::Arc::new(Model::from_snapshot(ds(), shop()));
    let reader = {
        let model = model.clone();
        std::thread::spawn(move || {
            model
                .find("public", "orders", NameMatch::Exact, &[])
                .map(|n| n.id())
        })
    };
    let id = reader.join().unwrap().unwrap();
    assert_eq!(model.node(id).unwrap().name(), "orders");
}
