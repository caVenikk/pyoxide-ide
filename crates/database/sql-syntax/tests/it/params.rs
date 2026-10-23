use sql_syntax::{Dialect, ParamForms, ParamKind, parameters};

fn scan(src: &str, d: Dialect) -> Vec<(ParamKind, Option<String>, String)> {
    parameters(src, d, d.default_params())
        .into_iter()
        .map(|p| (p.kind, p.name, src[p.range].to_string()))
        .collect()
}

#[test]
fn positional_and_named() {
    let n = |s: &str| Some(s.to_string());
    assert_eq!(
        scan("select * from t where a = ? and b = ?", Dialect::Sqlite),
        [
            (ParamKind::Question, None, "?".into()),
            (ParamKind::Question, None, "?".into())
        ]
    );
    assert_eq!(
        scan("a = ?3", Dialect::Sqlite),
        [(ParamKind::Numbered(3), None, "?3".into())]
    );
    assert_eq!(
        scan("a = $2 or b = $1", Dialect::Postgres)[0].0,
        ParamKind::Numbered(2)
    );
    assert_eq!(
        scan("a = :id", Dialect::Oracle),
        [(ParamKind::Named, n("id"), ":id".into())]
    );
    assert_eq!(
        scan("a = :1", Dialect::Oracle),
        [(ParamKind::Numbered(1), None, ":1".into())]
    );
    assert_eq!(
        scan("a = ${user_id}", Dialect::Postgres),
        [(ParamKind::Named, n("user_id"), "${user_id}".into())]
    );
    assert_eq!(
        scan("a = @p", Dialect::Sqlite),
        [(ParamKind::Named, n("p"), "@p".into())]
    );
}

#[test]
fn what_is_not_a_parameter() {
    let none = |src: &str| {
        for d in Dialect::ALL {
            assert!(scan(src, d).is_empty(), "{src} {d:?}");
        }
    };
    none("select '?', ':a', '$1', \"?\" from t");
    none("select 1 -- ? :a $1\n");
    none("select 1 /* ? ${x} */");
    assert!(scan("select $$ ? :a $$", Dialect::Postgres).is_empty());
    none("select x::int, y::text");
    none("select a->>'b', a->'c'");
    none("select a @> b");
    none("n := 1");
    // `?` followed by a quote still ends at the quote.
    assert_eq!(scan("a=?'x'", Dialect::Sqlite).len(), 1);
}

#[test]
fn python_driver_forms_are_configured() {
    let p = parameters(
        "a = %s and b = %(who)s and c like 'x%s'",
        Dialect::Postgres,
        ParamForms::PYTHON_DRIVERS,
    );
    assert_eq!(p.len(), 2);
    assert_eq!(
        (p[0].kind, p[1].kind),
        (ParamKind::Format, ParamKind::Named)
    );
    assert_eq!(p[1].name.as_deref(), Some("who"));
    assert!(
        parameters(
            "a = %s",
            Dialect::Postgres,
            Dialect::Postgres.default_params()
        )
        .is_empty()
    );
}
