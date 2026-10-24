//! Lookups by name, schema and kind; name search.
use db_core::DataSourceId;
use db_model::{Group, Model, NameMatch, NodeKind};
use name_match::{Matcher, MatchingMode};

use crate::allocations;
use crate::fixture::{shop, synthetic};

#[test]
fn exact_and_case_insensitive_lookups() {
    let model = Model::from_snapshot(DataSourceId::new("shop"), shop());
    let exact: Vec<_> = model
        .lookup("app", "Users", NameMatch::Exact, &[])
        .map(|n| n.name())
        .collect();
    assert_eq!(exact, ["Users"]);
    let any: Vec<_> = model
        .lookup("APP", "USERS", NameMatch::IgnoreCase, &[])
        .map(|n| n.name())
        .collect();
    assert_eq!(any, ["Users", "users"]);
    assert!(
        model
            .find("public", "orders", NameMatch::Exact, &[NodeKind::View])
            .is_none()
    );
    assert!(
        model
            .find("nope", "orders", NameMatch::Exact, &[])
            .is_none()
    );
    assert!(
        model
            .find("public", "nope", NameMatch::Exact, &[])
            .is_none()
    );
    // Indexes and triggers are schema objects too.
    let idx = model
        .find(
            "public",
            "orders_customer_idx",
            NameMatch::Exact,
            &[NodeKind::Index],
        )
        .unwrap();
    assert_eq!(idx.parent().unwrap().name(), "orders");
    // Every schema, in tree order (an unqualified name on a search path is
    // the SQL layer's business; this is the plain walk).
    let users: Vec<_> = model
        .lookup_everywhere("users", NameMatch::IgnoreCase, &NodeKind::RELATIONS)
        .map(|n| (n.schema().name(), n.name()))
        .collect();
    assert_eq!(users, [("app", "Users"), ("app", "users")]);
}

#[test]
fn objects_by_kind_through_groups() {
    let model = Model::from_snapshot(DataSourceId::new("shop"), shop());
    let public = model.schema("public", NameMatch::Exact).unwrap();
    let views: Vec<_> = public
        .group(Group::Views)
        .nodes()
        .map(|n| n.name())
        .collect();
    assert_eq!(views, ["order_totals"]);
    let routines: usize = NodeKind::ROUTINES
        .iter()
        .map(|k| {
            public
                .group(db_model::Group::of(NodeKind::Schema, *k))
                .len()
        })
        .sum();
    assert_eq!(routines, 2);
}

#[test]
fn lookups_do_not_allocate() {
    let model = Model::from_snapshot(DataSourceId::new("big"), synthetic(2_000));
    let names: Vec<String> = (0..2_000).map(|t| format!("table_{t:05}")).collect();
    let upper: Vec<String> = names.iter().map(|n| n.to_uppercase()).collect();
    let (found, count) = allocations(|| {
        let mut found = 0;
        for (name, upper) in names.iter().zip(&upper) {
            let table = model
                .find("app", name, NameMatch::Exact, &NodeKind::RELATIONS)
                .unwrap();
            found += 1;
            if model
                .find("APP", upper, NameMatch::IgnoreCase, &[NodeKind::Table])
                .is_some()
            {
                found += 1;
            }
            if table
                .child("created_at", NameMatch::Exact, &[NodeKind::Column])
                .is_some()
            {
                found += 1;
            }
            if model.node(table.id()).is_some() {
                found += 1;
            }
        }
        found
    });
    assert_eq!(found, 8_000);
    assert_eq!(count, 0, "lookups allocated {count} times");
}

#[test]
fn name_search_ranks_by_matching_degree() {
    let model = Model::from_snapshot(DataSourceId::new("shop"), shop());
    let matcher = Matcher::new("ord", MatchingMode::IgnoreCase);
    let found: Vec<_> = model
        .search(&matcher, &[], 10)
        .into_iter()
        .map(|f| f.node.name())
        .collect();
    assert!(found.contains(&"orders"));
    assert!(found.contains(&"order_totals"));
    assert!(!found.contains(&"customers"));
    assert!(found.contains(&"orders_customer_idx"));
    let columns = model.search(
        &Matcher::new("cusid", MatchingMode::IgnoreCase),
        &[NodeKind::Column],
        10,
    );
    assert!(!columns.is_empty());
    assert!(columns.iter().all(|f| f.node.name() == "customer_id"));
    assert_eq!(model.search(&matcher, &[], 1).len(), 1);
}
