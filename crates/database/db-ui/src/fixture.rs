//! A small metadata snapshot for tests: a `demo` schema with tables (one
//! partitioned), a view, a materialized view, routines, a sequence and a
//! type, and an empty `public` schema.

use db_core::schema::{
    ArgumentInfo, ArgumentMode, ColumnInfo, ConstraintInfo, ConstraintKind, IdentityKind,
    IndexInfo, Level, ObjectDetail, ObjectInfo, ObjectKind, PartitionOf, RelationInfo,
    RoutineDetail, SchemaInfo, SchemaSnapshot, Snapshot, TableDetail, TriggerDetail,
};

fn object(name: &str, kind: ObjectKind) -> ObjectInfo {
    ObjectInfo {
        schema: "demo".into(),
        name: name.into(),
        kind,
        ..ObjectInfo::default()
    }
}

fn column(position: u32, name: &str, data_type: &str, nullable: bool) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        position,
        data_type: data_type.into(),
        nullable,
        ..ColumnInfo::default()
    }
}

fn identity(position: u32, name: &str, data_type: &str) -> ColumnInfo {
    ColumnInfo {
        auto_increment: true,
        identity: Some(IdentityKind::Always),
        ..column(position, name, data_type, false)
    }
}

fn key(name: &str, kind: ConstraintKind, columns: &[&str]) -> ConstraintInfo {
    ConstraintInfo {
        name: name.into(),
        kind,
        columns: columns.iter().map(|c| (*c).into()).collect(),
        ..ConstraintInfo::default()
    }
}

fn index(name: &str, unique: bool, columns: &[&str], predicate: Option<&str>) -> IndexInfo {
    IndexInfo {
        name: name.into(),
        unique,
        columns: columns.iter().map(|c| (*c).into()).collect(),
        predicate: predicate.map(Into::into),
        ..IndexInfo::default()
    }
}

fn routine(name: &str, kind: ObjectKind, args: &[(&str, &str)], ret: Option<&str>) -> ObjectInfo {
    ObjectInfo {
        detail: Some(ObjectDetail::Routine(RoutineDetail {
            arguments: args
                .iter()
                .map(|(n, t)| ArgumentInfo {
                    name: Some((*n).into()),
                    data_type: (*t).into(),
                    mode: ArgumentMode::In,
                    has_default: false,
                })
                .collect(),
            return_type: ret.map(Into::into),
            ..RoutineDetail::default()
        })),
        ..object(name, kind)
    }
}

/// The `demo` schema at [`Level::Structure`] and an empty `public`.
pub fn demo_snapshot() -> Snapshot {
    let mut objects = vec![
        object("customers", ObjectKind::Table),
        object("orders", ObjectKind::Table),
        object("audit_log", ObjectKind::Table),
        ObjectInfo {
            detail: Some(ObjectDetail::Table(TableDetail {
                partitioned: true,
                partition_key: Some("RANGE (taken_at)".into()),
                ..TableDetail::default()
            })),
            ..object("measurements", ObjectKind::Table)
        },
        object("v_order_totals", ObjectKind::View),
        object("mv_daily_sales", ObjectKind::MaterializedView),
        object("orders_id_seq", ObjectKind::Sequence),
        object("address", ObjectKind::Type),
        routine(
            "order_count",
            ObjectKind::Function,
            &[("p_customer", "integer")],
            Some("bigint"),
        ),
        routine(
            "cancel_old_pending",
            ObjectKind::Procedure,
            &[("p_before", "timestamp with time zone")],
            None,
        ),
        routine(
            "touch_updated_at",
            ObjectKind::Function,
            &[],
            Some("trigger"),
        ),
        ObjectInfo {
            parent: Some("customers".into()),
            detail: Some(ObjectDetail::Trigger(TriggerDetail {
                events: vec!["UPDATE".into()],
                timing: Some("BEFORE".into()),
                each_row: true,
                ..TriggerDetail::default()
            })),
            ..object("customers_touch", ObjectKind::Trigger)
        },
    ];
    for (name, bounds) in [
        (
            "measurements_2026_01",
            "FOR VALUES FROM ('2026-01-01') TO ('2026-02-01')",
        ),
        (
            "measurements_2026_02",
            "FOR VALUES FROM ('2026-02-01') TO ('2026-03-01')",
        ),
    ] {
        objects.push(ObjectInfo {
            detail: Some(ObjectDetail::Table(TableDetail {
                partition_of: Some(PartitionOf {
                    schema: "demo".into(),
                    table: "measurements".into(),
                    bounds: Some(bounds.into()),
                }),
                ..TableDetail::default()
            })),
            ..object(name, ObjectKind::Table)
        });
    }
    for (table, name) in [
        ("customers", "customers_pkey"),
        ("customers", "customers_email_lower_idx"),
        ("customers", "customers_country_idx"),
        ("orders", "orders_pkey"),
    ] {
        objects.push(ObjectInfo {
            parent: Some(table.into()),
            ..object(name, ObjectKind::Index)
        });
    }
    let relations = vec![
        RelationInfo {
            name: "customers".into(),
            columns: vec![
                identity(1, "id", "integer"),
                column(2, "email", "demo.email_t", false),
                column(3, "full_name", "text", false),
                ColumnInfo {
                    default: Some("'DE'::bpchar".into()),
                    ..column(4, "country", "char(2)", false)
                },
                ColumnInfo {
                    default: Some("true".into()),
                    ..column(5, "is_active", "boolean", false)
                },
                column(6, "profile", "jsonb", true),
                column(7, "home", "demo.address", true),
                column(8, "notes", "text", true),
            ],
            indexes: vec![
                index(
                    "customers_email_lower_idx",
                    true,
                    &["lower(email::text)"],
                    None,
                ),
                index("customers_pkey", true, &["id"], None),
                index(
                    "customers_country_idx",
                    false,
                    &["country"],
                    Some("is_active"),
                ),
            ],
            constraints: vec![key("customers_pkey", ConstraintKind::PrimaryKey, &["id"])],
        },
        RelationInfo {
            name: "orders".into(),
            columns: vec![
                identity(1, "id", "bigint"),
                column(2, "customer_id", "integer", false),
                column(3, "placed_at", "timestamp with time zone", true),
                ColumnInfo {
                    default: Some("0".into()),
                    ..column(4, "total", "numeric(12,2)", false)
                },
            ],
            indexes: vec![index("orders_pkey", true, &["id"], None)],
            constraints: vec![
                key("orders_pkey", ConstraintKind::PrimaryKey, &["id"]),
                ConstraintInfo {
                    ref_table: Some("customers".into()),
                    ref_schema: Some("demo".into()),
                    ref_columns: vec!["id".into()],
                    ..key(
                        "orders_customer_id_fkey",
                        ConstraintKind::ForeignKey,
                        &["customer_id"],
                    )
                },
                ConstraintInfo {
                    check: Some("total >= 0".into()),
                    ..key("orders_total_check", ConstraintKind::Check, &["total"])
                },
            ],
        },
        RelationInfo {
            name: "audit_log".into(),
            columns: vec![
                identity(1, "id", "bigint"),
                column(2, "message", "text", true),
            ],
            ..RelationInfo::default()
        },
        RelationInfo {
            name: "measurements".into(),
            columns: vec![
                column(1, "taken_at", "timestamp with time zone", false),
                column(2, "value", "double precision", true),
            ],
            ..RelationInfo::default()
        },
        RelationInfo {
            name: "v_order_totals".into(),
            columns: vec![
                column(1, "customer_id", "integer", true),
                column(2, "total", "numeric", true),
            ],
            ..RelationInfo::default()
        },
        RelationInfo {
            name: "mv_daily_sales".into(),
            columns: vec![
                column(1, "day", "date", true),
                column(2, "total", "numeric", true),
            ],
            ..RelationInfo::default()
        },
    ];
    let schema = |name: &str, is_current| SchemaInfo {
        name: name.into(),
        is_current,
        ..SchemaInfo::default()
    };
    Snapshot {
        level: Level::Structure,
        token: None,
        schemas: vec![
            SchemaSnapshot {
                info: schema("demo", false),
                level: Level::Structure,
                objects,
                relations,
                sources: Vec::new(),
            },
            SchemaSnapshot {
                info: schema("public", true),
                level: Level::Structure,
                objects: Vec::new(),
                relations: Vec::new(),
                sources: Vec::new(),
            },
        ],
    }
}
