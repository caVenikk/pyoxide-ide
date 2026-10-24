//! Snapshots for the tests and the benchmark: small hand-made ones, random
//! ones for the property tests and a synthetic large schema.
#![allow(dead_code)]

use db_core::Dialect;
use db_core::schema::{
    ArgumentInfo, ArgumentMode, ChangeToken, ColumnInfo, ConstraintInfo, ConstraintKind, IndexInfo,
    Level, ObjectDetail, ObjectInfo, ObjectKind, PartitionOf, RelationInfo, RoutineDetail,
    SchemaInfo, SchemaSnapshot, Snapshot, TableDetail, TriggerDetail,
};

pub fn schema_info(name: &str) -> SchemaInfo {
    SchemaInfo {
        name: name.into(),
        is_system: false,
        is_current: false,
        owner: Some("owner".into()),
        comment: None,
    }
}

pub fn object(schema: &str, name: &str, kind: ObjectKind) -> ObjectInfo {
    ObjectInfo {
        schema: schema.into(),
        name: name.into(),
        kind,
        ..ObjectInfo::default()
    }
}

pub fn child(schema: &str, name: &str, kind: ObjectKind, parent: &str) -> ObjectInfo {
    ObjectInfo {
        parent: Some(parent.into()),
        detail: (kind == ObjectKind::Trigger).then(|| {
            ObjectDetail::Trigger(TriggerDetail {
                events: vec!["INSERT".into()],
                timing: Some("BEFORE".into()),
                each_row: true,
                ..TriggerDetail::default()
            })
        }),
        ..object(schema, name, kind)
    }
}

pub fn partition(schema: &str, name: &str, parent_schema: &str, parent: &str) -> ObjectInfo {
    ObjectInfo {
        detail: Some(ObjectDetail::Table(TableDetail {
            partition_of: Some(PartitionOf {
                schema: parent_schema.into(),
                table: parent.into(),
                bounds: Some(format!("FOR VALUES IN ('{name}')")),
            }),
            ..TableDetail::default()
        })),
        ..object(schema, name, ObjectKind::Table)
    }
}

pub fn partitioned(schema: &str, name: &str) -> ObjectInfo {
    ObjectInfo {
        detail: Some(ObjectDetail::Table(TableDetail {
            partitioned: true,
            partition_key: Some("LIST (region)".into()),
            ..TableDetail::default()
        })),
        ..object(schema, name, ObjectKind::Table)
    }
}

pub fn function(schema: &str, name: &str, signature: &str, args: &[(&str, &str)]) -> ObjectInfo {
    ObjectInfo {
        detail: Some(ObjectDetail::Routine(RoutineDetail {
            signature: signature.into(),
            arguments: args
                .iter()
                .map(|(n, t)| ArgumentInfo {
                    name: (!n.is_empty()).then(|| n.to_string()),
                    data_type: t.to_string(),
                    mode: ArgumentMode::In,
                    has_default: false,
                })
                .collect(),
            return_type: Some("integer".into()),
            language: Some("sql".into()),
            characteristics: Vec::new(),
        })),
        ..object(schema, name, ObjectKind::Function)
    }
}

pub fn column(name: &str, position: u32, data_type: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        position,
        data_type: data_type.into(),
        nullable: position > 1,
        default: None,
        auto_increment: position == 1,
        comment: None,
        identity: None,
        generated: None,
    }
}

pub fn constraint(name: &str, kind: ConstraintKind, columns: &[&str]) -> ConstraintInfo {
    ConstraintInfo {
        name: name.into(),
        kind,
        columns: columns.iter().map(|c| c.to_string()).collect(),
        ..ConstraintInfo::default()
    }
}

pub fn foreign_key(name: &str, column: &str, schema: &str, table: &str) -> ConstraintInfo {
    ConstraintInfo {
        ref_schema: Some(schema.into()),
        ref_table: Some(table.into()),
        ref_columns: vec!["id".into()],
        on_delete: Some("CASCADE".into()),
        ..constraint(name, ConstraintKind::ForeignKey, &[column])
    }
}

pub fn index(name: &str, columns: &[&str]) -> IndexInfo {
    IndexInfo {
        name: name.into(),
        unique: false,
        columns: columns.iter().map(|c| c.to_string()).collect(),
        method: Some("btree".into()),
        ..IndexInfo::default()
    }
}

pub fn relation(
    name: &str,
    columns: Vec<ColumnInfo>,
    indexes: Vec<IndexInfo>,
    constraints: Vec<ConstraintInfo>,
) -> RelationInfo {
    RelationInfo {
        name: name.into(),
        columns,
        indexes,
        constraints,
    }
}

pub fn schema(
    name: &str,
    level: Level,
    objects: Vec<ObjectInfo>,
    relations: Vec<RelationInfo>,
) -> SchemaSnapshot {
    SchemaSnapshot {
        info: schema_info(name),
        level,
        objects,
        relations,
        sources: Vec::new(),
    }
}

pub fn token(value: &str) -> ChangeToken {
    ChangeToken {
        dialect: Dialect::Postgres,
        value: value.into(),
    }
}

/// A PostgreSQL-like database: `public` with customers, orders (keys, an
/// index listed as an object and read with the table, a trigger), a
/// partitioned table with two partitions, a view, a sequence and an
/// overloaded function; `app` at names only.
pub fn shop() -> Snapshot {
    let public = schema(
        "public",
        Level::Structure,
        vec![
            object("public", "orders", ObjectKind::Table),
            object("public", "customers", ObjectKind::Table),
            child("public", "orders_customer_idx", ObjectKind::Index, "orders"),
            child("public", "orders_audit", ObjectKind::Trigger, "orders"),
            partitioned("public", "events"),
            partition("public", "events_eu", "public", "events"),
            partition("public", "events_us", "public", "events"),
            object("public", "order_totals", ObjectKind::View),
            object("public", "orders_id_seq", ObjectKind::Sequence),
            function("public", "total", "integer", &[("order_id", "integer")]),
            function(
                "public",
                "total",
                "integer, text",
                &[("order_id", "integer"), ("", "text")],
            ),
        ],
        vec![
            relation(
                "orders",
                vec![
                    column("total", 3, "numeric(10,2)"),
                    column("id", 1, "integer"),
                    column("customer_id", 2, "integer"),
                ],
                vec![
                    index("orders_customer_idx", &["customer_id"]),
                    index("orders_pkey", &["id"]),
                ],
                vec![
                    foreign_key("orders_customer_fk", "customer_id", "public", "customers"),
                    constraint("orders_pkey", ConstraintKind::PrimaryKey, &["id"]),
                    ConstraintInfo {
                        check: Some("total >= 0".into()),
                        ..constraint("orders_total_check", ConstraintKind::Check, &["total"])
                    },
                ],
            ),
            relation(
                "customers",
                vec![column("id", 1, "integer"), column("name", 2, "text")],
                vec![],
                vec![constraint(
                    "customers_pkey",
                    ConstraintKind::PrimaryKey,
                    &["id"],
                )],
            ),
            relation("events", vec![column("region", 1, "text")], vec![], vec![]),
            relation(
                "events_eu",
                vec![column("region", 1, "text")],
                vec![],
                vec![],
            ),
            relation(
                "events_us",
                vec![column("region", 1, "text")],
                vec![],
                vec![],
            ),
            relation(
                "order_totals",
                vec![
                    column("customer_id", 1, "integer"),
                    column("sum", 2, "numeric"),
                ],
                vec![],
                vec![],
            ),
        ],
    );
    let app = schema(
        "app",
        Level::Names,
        vec![
            object("app", "Users", ObjectKind::Table),
            object("app", "users", ObjectKind::Table),
        ],
        vec![],
    );
    Snapshot {
        level: Level::Structure,
        token: Some(token("1")),
        schemas: vec![public, app],
    }
}

/// One schema `app` with `tables` tables of eight columns, a primary key, a
/// foreign key to the table before, an index (listed and read with the
/// table) and every tenth a trigger; plus a small `audit` schema.
pub fn synthetic(tables: usize) -> Snapshot {
    let mut objects = Vec::with_capacity(tables * 3);
    let mut relations = Vec::with_capacity(tables);
    for t in 0..tables {
        let name = format!("table_{t:05}");
        objects.push(object("app", &name, ObjectKind::Table));
        let idx = format!("{name}_created_idx");
        objects.push(child("app", &idx, ObjectKind::Index, &name));
        if t % 10 == 0 {
            objects.push(child(
                "app",
                &format!("{name}_audit"),
                ObjectKind::Trigger,
                &name,
            ));
        }
        let columns = vec![
            column("id", 1, "bigint"),
            column("parent_id", 2, "bigint"),
            column("name", 3, "character varying(200)"),
            column("description", 4, "text"),
            column("amount", 5, "numeric(12,2)"),
            column("status", 6, "character varying(20)"),
            column("created_at", 7, "timestamp with time zone"),
            column("updated_at", 8, "timestamp with time zone"),
        ];
        let mut constraints = vec![constraint(
            &format!("{name}_pkey"),
            ConstraintKind::PrimaryKey,
            &["id"],
        )];
        if t > 0 {
            constraints.push(foreign_key(
                &format!("{name}_parent_fk"),
                "parent_id",
                "app",
                &format!("table_{:05}", t - 1),
            ));
        }
        relations.push(relation(
            &name,
            columns,
            vec![
                index(&idx, &["created_at"]),
                index(&format!("{name}_pkey"), &["id"]),
            ],
            constraints,
        ));
    }
    let audit = schema(
        "audit",
        Level::Structure,
        vec![object("audit", "log", ObjectKind::Table)],
        vec![relation(
            "log",
            vec![column("id", 1, "bigint"), column("entry", 2, "jsonb")],
            vec![],
            vec![],
        )],
    );
    Snapshot {
        level: Level::Structure,
        token: Some(token("42")),
        schemas: vec![schema("app", Level::Structure, objects, relations), audit],
    }
}

/// A small deterministic generator (SplitMix64).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// In `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

const SCHEMAS: &[&str] = &["public", "app", "App", "sales", "Ünïcode"];
const TABLES: &[&str] = &[
    "orders",
    "Orders",
    "customers",
    "items",
    "events",
    "events_a",
    "events_b",
    "log",
    "T1",
    "t1",
    "users",
    "accounts",
];
const VIEWS: &[&str] = &["v_orders", "V_Orders", "v_totals"];
const FUNCTIONS: &[&str] = &["total", "f_tax", "F_Tax"];
const TYPES: &[&str] = &["integer", "text", "bigint", "numeric(10,2)", "jsonb"];

fn random_relation(rng: &mut Rng, name: &str, tables: &[String]) -> RelationInfo {
    let mut names: Vec<String> = Vec::new();
    for _ in 0..1 + rng.below(5) {
        let n = format!("c{}", rng.below(8));
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let columns: Vec<ColumnInfo> = names
        .iter()
        .enumerate()
        .map(|(i, n)| column(n, i as u32 + 1, rng.pick(TYPES)))
        .collect();
    let mut constraints = Vec::new();
    if rng.chance(70) {
        constraints.push(constraint(
            &format!("{name}_pkey"),
            ConstraintKind::PrimaryKey,
            &[&columns[0].name],
        ));
    }
    if rng.chance(30) {
        constraints.push(constraint(
            &format!("{name}_uq"),
            ConstraintKind::Unique,
            &[&columns[columns.len() - 1].name],
        ));
    }
    if rng.chance(40) && !tables.is_empty() {
        let target = rng.pick(tables).clone();
        constraints.push(foreign_key(
            &format!("{name}_fk"),
            &columns[0].name,
            "public",
            &target,
        ));
    }
    // Unnamed checks (SQLite) get distinct ids all the same.
    for i in 0..rng.below(3) {
        constraints.push(ConstraintInfo {
            check: Some(format!("c{i} > 0")),
            ..constraint("", ConstraintKind::Check, &[])
        });
    }
    let indexes = (0..rng.below(3))
        .map(|i| index(&format!("{name}_idx{i}"), &[&columns[0].name]))
        .collect();
    relation(name, columns, indexes, constraints)
}

fn random_schema(rng: &mut Rng, name: &str, level: Level) -> SchemaSnapshot {
    let mut objects = Vec::new();
    let mut relations = Vec::new();
    let mut tables: Vec<String> = Vec::new();
    // At least one table, so a schema new in a delta shows its level.
    let mut names: Vec<&str> = TABLES.to_vec();
    rng.shuffle(&mut names);
    for table in names.into_iter().take(1 + rng.below(6)) {
        let mut info = if !tables.is_empty() && rng.chance(25) {
            let parent = rng.pick(&tables).clone();
            partition(name, table, name, &parent)
        } else {
            object(name, table, ObjectKind::Table)
        };
        if rng.chance(20) {
            info.comment = Some(format!("comment {}", rng.below(3)));
        }
        objects.push(info);
        for i in 0..rng.below(3) {
            objects.push(child(
                name,
                &format!("{table}_idx{i}"),
                ObjectKind::Index,
                table,
            ));
        }
        if rng.chance(30) {
            objects.push(child(
                name,
                &format!("{table}_trg"),
                ObjectKind::Trigger,
                table,
            ));
        }
        if level >= Level::Structure {
            relations.push(random_relation(rng, table, &tables));
        }
        tables.push(table.to_string());
    }
    for view in VIEWS {
        if !rng.chance(40) {
            continue;
        }
        objects.push(object(name, view, ObjectKind::View));
        if level >= Level::Structure {
            relations.push(random_relation(rng, view, &[]));
        }
    }
    for f in FUNCTIONS {
        if !rng.chance(40) {
            continue;
        }
        if level >= Level::Structure {
            let args: Vec<(String, String)> = (0..rng.below(3))
                .map(|i| {
                    let n = if rng.chance(70) {
                        format!("a{i}")
                    } else {
                        String::new()
                    };
                    (n, rng.pick(TYPES).to_string())
                })
                .collect();
            let args: Vec<(&str, &str)> =
                args.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            objects.push(function(name, f, "", &args));
        } else {
            objects.push(object(name, f, ObjectKind::Function));
        }
    }
    if rng.chance(50) {
        objects.push(object(name, "seq", ObjectKind::Sequence));
    }
    schema(name, level, objects, relations)
}

pub fn random_snapshot(rng: &mut Rng, level: Level) -> Snapshot {
    let mut names: Vec<&str> = SCHEMAS.to_vec();
    rng.shuffle(&mut names);
    let schemas = names
        .into_iter()
        .take(1 + rng.below(3))
        .map(|n| random_schema(rng, n, level))
        .collect();
    Snapshot {
        level,
        token: Some(token(&rng.next().to_string())),
        schemas,
    }
}

/// `snapshot` with a few random changes: a table dropped with its indexes,
/// triggers and partitions, a column type changed, a comment, a schema
/// dropped or added.
pub fn mutate(rng: &mut Rng, snapshot: &Snapshot) -> Snapshot {
    let mut next = snapshot.clone();
    next.token = Some(token(&rng.next().to_string()));
    for s in &mut next.schemas {
        if rng.chance(40) {
            let tables: Vec<String> = s
                .objects
                .iter()
                .filter(|o| o.kind == ObjectKind::Table)
                .map(|o| o.name.clone())
                .collect();
            if tables.len() > 1 {
                let gone = rng.pick(&tables).clone();
                let is_partition_of_gone = |o: &ObjectInfo| match &o.detail {
                    Some(ObjectDetail::Table(t)) => {
                        t.partition_of.as_ref().is_some_and(|p| p.table == gone)
                    }
                    _ => false,
                };
                // Partitions of the dropped table become plain tables.
                for o in &mut s.objects {
                    if is_partition_of_gone(o) {
                        o.detail = None;
                    }
                }
                s.objects
                    .retain(|o| o.name != gone && o.parent.as_deref() != Some(gone.as_str()));
                s.relations.retain(|r| r.name != gone);
            }
        }
        if let Some(r) = s.relations.first_mut()
            && rng.chance(50)
            && let Some(c) = r.columns.first_mut()
        {
            c.data_type = rng.pick(TYPES).to_string();
        }
        if rng.chance(30)
            && let Some(o) = s.objects.first_mut()
        {
            o.comment = Some(format!("changed {}", rng.below(5)));
        }
    }
    if next.schemas.len() > 1 && rng.chance(20) {
        let i = rng.below(next.schemas.len());
        next.schemas.remove(i);
    }
    if rng.chance(20) {
        let name = *rng.pick(SCHEMAS);
        if next.schema(name).is_none() {
            let level = next.level;
            next.schemas.push(random_schema(rng, name, level));
        }
    }
    next
}
