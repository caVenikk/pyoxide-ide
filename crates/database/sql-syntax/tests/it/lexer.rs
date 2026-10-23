use sql_syntax::{Dialect, HOLE, ParamForms, Token, TokenKind, attach_trivia, lex, lex_with};

fn kinds(src: &str, dialect: Dialect) -> Vec<(TokenKind, String)> {
    lex(src, dialect)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .map(|t| (t.kind, t.text(src).to_string()))
        .collect()
}

fn significant(src: &str, dialect: Dialect) -> Vec<String> {
    kinds(src, dialect).into_iter().map(|(_, t)| t).collect()
}

fn one(src: &str, dialect: Dialect) -> Token {
    let t = lex(src, dialect);
    assert_eq!(t.len(), 1, "{src:?}: {t:?}");
    t[0]
}

#[test]
fn tokens_cover_the_text() {
    let src = "select \"A b\", 1.5e+3, 'x''y' -- c\nfrom t /* d */ where a >= :p;";
    for d in Dialect::ALL {
        let tokens = lex(src, d);
        let mut end = 0;
        for t in &tokens {
            assert_eq!(t.start, end);
            end = t.end;
        }
        assert_eq!(end, src.len());
        let joined: String = tokens.iter().map(|t| t.text(src)).collect();
        assert_eq!(joined, src);
    }
}

#[test]
fn trivia_is_whitespace_and_comments() {
    let t = lex("a /* b */ -- c\n d", Dialect::Postgres);
    let trivia: Vec<_> = t.iter().filter(|t| t.is_trivia()).map(|t| t.kind).collect();
    assert_eq!(
        trivia,
        [
            TokenKind::Whitespace,
            TokenKind::BlockComment,
            TokenKind::Whitespace,
            TokenKind::LineComment,
            TokenKind::Whitespace
        ]
    );
}

#[test]
fn trivia_attaches_to_the_token_before_on_the_same_line() {
    let src = "  -- head\nselect 1 -- tail\n\n  /* n */ from t \n";
    let tokens = lex(src, Dialect::Generic);
    let items = attach_trivia(src, &tokens);
    assert_eq!(items.len(), 4);
    let mut end = 0;
    let mut joined = String::new();
    for a in &items {
        assert_eq!(a.leading.start, end);
        assert_eq!(a.leading.end, a.token.start);
        assert_eq!(a.trailing.start, a.token.end);
        end = a.trailing.end;
        joined += &src[a.leading.clone()];
        joined += a.token.text(src);
        joined += &src[a.trailing.clone()];
    }
    assert_eq!(joined, src);
    assert_eq!(&src[items[0].leading.clone()], "  -- head\n");
    // `1` owns the comment that follows it on its line; `from` the rest.
    assert_eq!(&src[items[1].trailing.clone()], " -- tail");
    assert_eq!(&src[items[2].leading.clone()], "\n\n  /* n */ ");
    assert_eq!(&src[items[3].trailing.clone()], " \n");
    assert!(attach_trivia("  -- nothing\n", &lex("  -- nothing\n", Dialect::Generic)).is_empty());
}

#[test]
fn strings_of_every_form() {
    let pg = Dialect::Postgres;
    for s in [
        "'it''s'",
        "E'a\\'b'",
        "N'x'",
        "X'1F'",
        "B'101'",
        "U&'d\\0061t'",
        "$$a;b$$",
        "$tag$ $$ ; $tag$",
        "''",
    ] {
        let t = one(s, pg);
        assert_eq!((t.kind, t.unterminated), (TokenKind::String, false), "{s}");
    }
    for s in [
        "q'[it's; x]'",
        "Q'{a ' ; }'",
        "q'(a ';' )'",
        "q'<a;>'",
        "q'!a;'!'",
        "nq'[ñ;]'",
        "NQ'#x;#'",
        "N'x'",
    ] {
        let t = one(s, Dialect::Oracle);
        assert_eq!((t.kind, t.unterminated), (TokenKind::String, false), "{s}");
    }
    // A backslash does not escape in a plain string.
    assert_eq!(significant("'a\\' b", Dialect::Postgres), ["'a\\'", "b"]);
    assert_eq!(one("E'a\\'b'", pg).end, 7);
    // `q` is a name elsewhere.
    assert_eq!(significant("q'x'", Dialect::Postgres), ["q", "'x'"]);
    assert_eq!(significant("q 'a;'", Dialect::Oracle), ["q", "'a;'"]);
}

#[test]
fn unterminated_constructs_run_to_the_end_and_say_so() {
    for (src, kind) in [
        ("'abc", TokenKind::String),
        ("\"abc", TokenKind::QuotedIdent),
        ("/* abc", TokenKind::BlockComment),
        ("$$ abc", TokenKind::String),
        ("$tag$ abc $ta", TokenKind::String),
        ("E'abc\\", TokenKind::String),
        ("E'\\ñ", TokenKind::String),
    ] {
        let t = one(src, Dialect::Postgres);
        assert_eq!(
            (t.kind, t.unterminated, t.end),
            (kind, true, src.len()),
            "{src}"
        );
    }
    let t = one("q'[abc; def", Dialect::Oracle);
    assert!(t.unterminated && t.kind == TokenKind::String);
    assert!(one("[abc", Dialect::Sqlite).unterminated);
    assert!(one("`abc", Dialect::Sqlite).unterminated);
    assert!(!one("-- c", Dialect::Sqlite).unterminated);
}

#[test]
fn comments_nest_on_postgres_only() {
    let src = "/* a /* b */ c */ x";
    assert_eq!(significant(src, Dialect::Postgres), ["x"]);
    assert_eq!(significant(src, Dialect::Sqlite), ["c", "*/", "x"]);
    assert_eq!(significant(src, Dialect::Oracle).len(), 3);
}

#[test]
fn quoted_identifiers_per_dialect() {
    assert_eq!(
        one("\"a\"\"b\"", Dialect::Postgres).kind,
        TokenKind::QuotedIdent
    );
    assert_eq!(one("`a;b`", Dialect::Sqlite).kind, TokenKind::QuotedIdent);
    assert_eq!(one("[a;b]", Dialect::Sqlite).kind, TokenKind::QuotedIdent);
    // Not quotes elsewhere.
    assert_eq!(kinds("`a`", Dialect::Postgres)[0].0, TokenKind::Unknown);
    assert_eq!(kinds("[a]", Dialect::Postgres)[0].0, TokenKind::LBracket);
    // A subscript is not a quoted name.
    let k: Vec<_> = kinds("arr[1]", Dialect::Sqlite)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(
        k,
        [
            TokenKind::Word,
            TokenKind::LBracket,
            TokenKind::Number,
            TokenKind::RBracket
        ]
    );
}

#[test]
fn numbers() {
    for (src, n) in [
        ("1", 1),
        ("1.5", 1),
        ("1e10", 1),
        ("1E+10", 1),
        (".5", 1),
        ("0x1F", 1),
        ("1_000", 1),
        ("1.", 1),
    ] {
        let k = kinds(src, Dialect::Sqlite);
        assert_eq!(k.len(), n, "{src}");
        assert_eq!(k[0].0, TokenKind::Number, "{src}");
    }
    assert_eq!(significant("1..3", Dialect::Oracle), ["1", ".", ".", "3"]);
    assert_eq!(significant("1e", Dialect::Oracle), ["1", "e"]);
    assert_eq!(significant("t.c1", Dialect::Oracle), ["t", ".", "c1"]);
}

#[test]
fn operators_are_whole() {
    let d = Dialect::Postgres;
    assert_eq!(significant("a->>'b'", d), ["a", "->>", "'b'"]);
    assert_eq!(significant("a @> b", d), ["a", "@>", "b"]);
    assert_eq!(significant("x::int", d), ["x", "::", "int"]);
    assert_eq!(significant("a||b", d), ["a", "||", "b"]);
    assert_eq!(significant("a<>b", d), ["a", "<>", "b"]);
    assert_eq!(significant("a=-1", d), ["a", "=", "-", "1"]);
    assert_eq!(significant("a<-1", d), ["a", "<", "-", "1"]);
    assert_eq!(significant("n := 1", Dialect::Oracle), ["n", ":=", "1"]);
    assert_eq!(significant("a=>1", Dialect::Oracle), ["a", "=>", "1"]);
    // Comments end a run.
    assert_eq!(significant("a+-- c\nb", d), ["a", "+", "b"]);
    assert_eq!(significant("a*/**/b", d), ["a", "*", "b"]);
    assert_eq!(significant("a ?| b", d), ["a", "?|", "b"]);
}

#[test]
fn parameters_of_every_form() {
    let p = |src: &str, d: Dialect| -> Vec<(TokenKind, String)> {
        kinds(src, d)
            .into_iter()
            .filter(|(k, _)| *k == TokenKind::Parameter)
            .collect()
    };
    let texts = |src: &str, d: Dialect| p(src, d).into_iter().map(|(_, t)| t).collect::<Vec<_>>();
    assert_eq!(texts("a = ? and b = ?", Dialect::Sqlite), ["?", "?"]);
    assert_eq!(texts("a=?", Dialect::Sqlite), ["?"]);
    assert_eq!(texts("a = ?12", Dialect::Sqlite), ["?12"]);
    assert_eq!(
        texts("a = $1 and b = $23", Dialect::Postgres),
        ["$1", "$23"]
    );
    assert_eq!(
        texts("a = :id and b=:name_2", Dialect::Oracle),
        [":id", ":name_2"]
    );
    assert_eq!(texts("a = :1", Dialect::Oracle), [":1"]);
    assert_eq!(texts("a = ${user.id}", Dialect::Postgres), ["${user.id}"]);
    assert_eq!(texts("a = @v and b = $w", Dialect::Sqlite), ["@v", "$w"]);
    // Operators and casts are not parameters.
    assert!(p("x::int, a->>'b', a @> b, a ?| b, n := 1", Dialect::Postgres).is_empty());
    assert!(p("n := 1; x := y", Dialect::Oracle).is_empty());
    // The trigger correlation names of Oracle are not binds.
    assert_eq!(texts(":new.x := :old.y + :z", Dialect::Oracle), [":z"]);
    // Nothing inside strings, comments or quoted names.
    assert!(p("'?' \"?\" -- ?\n/* :a */ '$1'", Dialect::Sqlite).is_empty());
    // Python driver forms are opt-in.
    assert!(p("a = %s", Dialect::Postgres).is_empty());
    let py = |src| {
        lex_with(src, Dialect::Postgres, ParamForms::PYTHON_DRIVERS)
            .into_iter()
            .filter(|t| t.kind == TokenKind::Parameter)
            .map(|t| t.text(src).to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        py("a = %s and b = %(user_id)s and c % 2 and d %sum"),
        ["%s", "%(user_id)s"]
    );
}

#[test]
fn parameter_forms_follow_the_configuration() {
    let none = lex_with("a = ? and b = :x", Dialect::Sqlite, ParamForms::NONE);
    assert!(none.iter().all(|t| t.kind != TokenKind::Parameter));
    // With `?` off it is an operator character.
    assert_eq!(kinds("a ? b", Dialect::Sqlite)[1].0, TokenKind::Parameter);
    let k: Vec<_> = lex_with("a ? b", Dialect::Sqlite, ParamForms::NONE)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind)
        .collect();
    assert_eq!(k, [TokenKind::Word, TokenKind::Operator, TokenKind::Word]);
}

#[test]
fn the_oracle_slash_line() {
    let src = "begin\n  null;\nend;\n/\nselect 4 / 2 from dual\n  /  \nselect 4\n/ 2";
    let slashes: Vec<_> = lex(src, Dialect::Oracle)
        .into_iter()
        .filter(|t| t.kind == TokenKind::SlashLine)
        .collect();
    assert_eq!(slashes.len(), 2);
    assert_eq!(slashes[0].start, src.find("/\n").unwrap());
    // `/ 2` on its own line starts with a slash but has more on the line.
    assert!(
        lex(src, Dialect::Postgres)
            .iter()
            .all(|t| t.kind != TokenKind::SlashLine)
    );
}

#[test]
fn the_hole_is_a_token_of_its_own() {
    let src =
        format!("select * from t_{HOLE}_x where a = {HOLE} and b = '{HOLE}' -- {HOLE}\n{HOLE}");
    let k = kinds(&src, Dialect::Postgres);
    let holes = k.iter().filter(|(k, _)| *k == TokenKind::Hole).count();
    // In a name, after `=` and the last; not in the string or comment.
    assert_eq!(holes, 3);
    assert!(k.contains(&(TokenKind::Word, "t_".to_string())));
    assert!(k.contains(&(TokenKind::Word, "_x".to_string())));
    let strings: Vec<_> = k.iter().filter(|(k, _)| *k == TokenKind::String).collect();
    assert_eq!(strings[0].1, format!("'{HOLE}'"));
}

#[test]
fn words_and_unknown_characters() {
    assert_eq!(
        significant("año café_1 x$y", Dialect::Postgres),
        ["año", "café_1", "x$y"]
    );
    assert_eq!(significant("a#b", Dialect::Oracle), ["a#b"]);
    assert_eq!(
        kinds("\\ € 🎵", Dialect::Postgres)
            .iter()
            .map(|(k, _)| *k)
            .collect::<Vec<_>>(),
        [TokenKind::Unknown; 3]
    );
    // Unicode white space is white space.
    assert_eq!(
        significant("a\u{a0}b\u{2003}c", Dialect::Generic),
        ["a", "b", "c"]
    );
}

#[test]
fn dollar_quote_or_parameter() {
    let d = Dialect::Postgres;
    assert_eq!(kinds("$1", d)[0].0, TokenKind::Parameter);
    assert_eq!(kinds("$$x$$", d)[0].0, TokenKind::String);
    assert_eq!(kinds("$a1$x$a1$", d)[0].0, TokenKind::String);
    // `$` plus a digit is no tag; Oracle has no dollar quotes.
    assert_eq!(
        kinds("$1$x$1$", d)[0],
        (TokenKind::Parameter, "$1".to_string())
    );
    assert_eq!(kinds("$$x$$", Dialect::Oracle)[0].0, TokenKind::Unknown);
}
