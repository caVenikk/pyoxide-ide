use db_core::DataSourceId;
use db_model::{Model, NameMatch, NodeKind, NodeRef};
use sql_syntax::Dialect;

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

const PG: Dialect = Dialect::Postgres;

#[test]
fn table_ddl_has_columns_keys_and_indexes() {
    let model = model();
    let text = node_ddl(PG, node(&model, "orders", NodeKind::Table));
    assert_eq!(
        text,
        "create table demo.orders\n\
         (\n    \
         id bigint generated always as identity not null,\n    \
         customer_id integer not null,\n    \
         placed_at timestamp with time zone,\n    \
         total numeric(12,2) default 0 not null,\n    \
         constraint orders_pkey\n        primary key (id),\n    \
         constraint orders_customer_id_fkey\n        foreign key (customer_id) references demo.customers (id),\n    \
         constraint orders_total_check\n        check (total >= 0)\n\
         );\n"
    );
    let customers = node_ddl(PG, node(&model, "customers", NodeKind::Table));
    assert!(
        customers.contains(
            "create unique index customers_email_lower_idx\n    on demo.customers (lower(email::text));"
        ),
        "{customers}"
    );
    assert!(
        customers.contains(
            "create index customers_country_idx\n    on demo.customers (country)\n    where is_active;"
        ),
        "{customers}"
    );
    // The key's index comes with the key.
    assert!(!customers.contains("index customers_pkey"), "{customers}");
}

#[test]
fn drop_statements_follow_the_options() {
    let model = model();
    let customers = node(&model, "customers", NodeKind::Table);
    assert_eq!(
        drop_statement(PG, customers, DropOptions::default()).unwrap(),
        "drop table demo.customers;"
    );
    let both = DropOptions {
        if_exists: true,
        cascade: true,
    };
    assert_eq!(
        drop_statement(PG, customers, both).unwrap(),
        "drop table if exists demo.customers cascade;"
    );
    assert_eq!(
        drop_statement(Dialect::Oracle, customers, both).unwrap(),
        "drop table if exists \"demo\".\"customers\" cascade constraints;",
        "Oracle quotes names in lower case"
    );
    let email = customers
        .group(Group::Columns)
        .nodes()
        .find(|c| c.name() == "email")
        .unwrap();
    assert_eq!(
        drop_statement(PG, email, DropOptions::default()).unwrap(),
        "alter table demo.customers drop column email;"
    );
    let order_count = node(&model, "order_count", NodeKind::Function);
    assert_eq!(
        drop_statement(PG, order_count, DropOptions::default()).unwrap(),
        "drop function demo.order_count(integer);"
    );
    let demo = model.schema("demo", NameMatch::Exact).unwrap().root();
    assert_eq!(
        drop_statement(PG, demo, both).unwrap(),
        "drop schema if exists demo cascade;"
    );
}

#[test]
fn rename_statements() {
    let model = model();
    let customers = node(&model, "customers", NodeKind::Table);
    assert_eq!(
        rename_statement(PG, customers, "Clients").unwrap(),
        "alter table demo.customers rename to \"Clients\";"
    );
    let pkey = customers.group(Group::Keys).get(0).unwrap();
    assert_eq!(
        rename_statement(PG, pkey, "customers_pk").unwrap(),
        "alter table demo.customers rename constraint customers_pkey to customers_pk;"
    );
    assert_eq!(
        rename_statement(Dialect::Sqlite, pkey, "pk"),
        None,
        "SQLite cannot rename a constraint"
    );
    assert_eq!(
        reference(PG, pkey),
        "demo.customers.customers_pkey",
        "a key after its table"
    );
    assert_eq!(
        truncate_statement(PG, customers),
        "truncate table demo.customers;"
    );
    assert_eq!(
        truncate_statement(Dialect::Sqlite, customers),
        "delete from customers;"
    );
}

#[test]
fn objects_without_their_text_say_so() {
    let model = model();
    let view = node(&model, "v_order_totals", NodeKind::View);
    assert_eq!(
        node_ddl(PG, view),
        "-- view demo.v_order_totals: its definition is not in the local model\n"
    );
}
