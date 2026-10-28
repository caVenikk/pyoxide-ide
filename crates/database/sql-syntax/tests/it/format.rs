//! The formatter: golden fixtures (`tests/it/format/*.sql`, one per layout
//! rule and per option), and the invariants of ADR 0022 on generated scripts:
//! the tokens are the same apart from whitespace and the case of words, the
//! comments are untouched, formatting twice is formatting once, a statement
//! with an error is left as written.
use std::path::PathBuf;

use sql_syntax::{CodeStyle, Dialect, TokenKind, format, format_range, lex, parse};

use crate::Rng;

fn dialect(name: &str) -> Dialect {
    match name {
        "postgres" => Dialect::Postgres,
        "oracle" => Dialect::Oracle,
        "sqlite" => Dialect::Sqlite,
        "generic" => Dialect::Generic,
        other => panic!("unknown dialect {other}"),
    }
}

fn style_of(pairs: &str) -> CodeStyle {
    let mut style = CodeStyle::default();
    for kv in pairs.split_whitespace() {
        let (k, v) = kv.split_once('=').expect("key=value");
        style.set(k, v).unwrap_or_else(|e| panic!("{e}"));
    }
    style
}

/// The significant tokens as (kind, text), words in lower case, and the
/// comments in order.
fn signature(text: &str, d: Dialect) -> (Vec<(TokenKind, String)>, Vec<String>) {
    let mut toks = Vec::new();
    let mut comments = Vec::new();
    for t in lex(text, d) {
        let s = &text[t.range()];
        match t.kind {
            TokenKind::Whitespace => {}
            TokenKind::LineComment | TokenKind::BlockComment => comments.push(s.to_string()),
            TokenKind::Word => toks.push((t.kind, s.to_ascii_lowercase())),
            _ => toks.push((t.kind, s.to_string())),
        }
    }
    (toks, comments)
}

/// Formats `text` and checks the invariants; returns the result.
fn check(text: &str, d: Dialect, style: &CodeStyle) -> String {
    let once = format(text, d, style);
    let (a, ac) = signature(text, d);
    let (b, bc) = signature(&once, d);
    assert_eq!(
        a, b,
        "tokens changed\n--- input\n{text}\n--- output\n{once}"
    );
    assert_eq!(
        ac, bc,
        "comments changed\n--- input\n{text}\n--- output\n{once}"
    );
    let twice = format(&once, d, style);
    if once != twice {
        let (a, b): (Vec<&str>, Vec<&str>) = (once.lines().collect(), twice.lines().collect());
        let at = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        let from = at.saturating_sub(4);
        panic!(
            "not idempotent at line {at}\n--- once\n{}\n--- twice\n{}",
            a[from..(at + 4).min(a.len())].join("\n"),
            b[from..(at + 4).min(b.len())].join("\n")
        );
    }
    once
}

fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/it/format");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    files
}

const SEPARATOR: &str = "\n=====\n";

/// `# dialect: x` and `# style: k=v ..` lines, the input, `=====`, the
/// expected output. `BLESS=1` rewrites the expected output.
#[test]
fn golden_fixtures() {
    let bless = std::env::var_os("BLESS").is_some();
    let files = fixtures();
    assert!(files.len() >= 20, "{} fixtures", files.len());
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let raw = std::fs::read_to_string(&path).unwrap();
        let (head, expected) = match raw.split_once(SEPARATOR) {
            Some((h, e)) => (h.to_string(), Some(e.to_string())),
            None => (raw.trim_end_matches('\n').to_string(), None),
        };
        let mut d = Dialect::Postgres;
        let mut style = CodeStyle::default();
        let mut input = String::new();
        for line in head.lines() {
            if let Some(v) = line.strip_prefix("# dialect:") {
                d = dialect(v.trim());
            } else if let Some(v) = line.strip_prefix("# style:") {
                style = style_of(v);
            } else {
                input.push_str(line);
                input.push('\n');
            }
        }
        let got = check(&input, d, &style);
        if bless {
            std::fs::write(&path, format!("{head}{SEPARATOR}{got}")).unwrap();
        } else {
            let expected = expected.unwrap_or_else(|| panic!("{name}: no expected output"));
            assert_eq!(got, expected, "{name}");
        }
    }
}

// Generated scripts.

fn pick<'a>(rng: &mut Rng, items: &[&'a str]) -> &'a str {
    items[rng.below(items.len())]
}

fn ident(rng: &mut Rng) -> String {
    let names = [
        "a",
        "b",
        "id",
        "name",
        "total",
        "created",
        "Customer_Id",
        "x1",
    ];
    let n = pick(rng, &names).to_string();
    match rng.below(5) {
        0 => format!("t.{n}"),
        1 => format!("\"{n} q\""),
        _ => n,
    }
}

fn expr(rng: &mut Rng, depth: usize, out: &mut Vec<String>) {
    let leaf = depth == 0;
    match rng.below(if leaf { 4 } else { 15 }) {
        0 => out.push(ident(rng)),
        1 => out.push(rng.below(1000).to_string()),
        2 => out.push(pick(rng, &["'x'", "'a;b'", "null", "true", "'it''s'", ":p", "?"]).into()),
        3 => out.push(ident(rng)),
        4 => {
            out.push(pick(rng, &["count", "max", "lower", "coalesce", "my_fn"]).into());
            out.push("(".into());
            if rng.below(4) == 0 {
                out.push("*".into());
            } else {
                for i in 0..=rng.below(3) {
                    if i > 0 {
                        out.push(",".into());
                    }
                    expr(rng, depth - 1, out);
                }
            }
            out.push(")".into());
        }
        5 | 6 => {
            expr(rng, depth - 1, out);
            out.push(pick(rng, &["+", "-", "*", "/", "||", "=", "<>", "<", ">="]).into());
            expr(rng, depth - 1, out);
        }
        7 => {
            out.push("(".into());
            expr(rng, depth - 1, out);
            out.push(")".into());
        }
        8 => {
            out.push("case".into());
            for _ in 0..=rng.below(2) {
                out.push("when".into());
                expr(rng, depth - 1, out);
                out.push("then".into());
                expr(rng, depth - 1, out);
            }
            if rng.below(2) == 0 {
                out.push("else".into());
                expr(rng, depth - 1, out);
            }
            out.push("end".into());
        }
        9 => {
            expr(rng, depth - 1, out);
            out.push("::".into());
            out.push(pick(rng, &["int", "varchar", "text"]).into());
        }
        10 => {
            expr(rng, depth - 1, out);
            out.push("in".into());
            out.push("(".into());
            if rng.below(2) == 0 {
                select(rng, depth - 1, out);
            } else {
                for i in 0..=rng.below(4) {
                    if i > 0 {
                        out.push(",".into());
                    }
                    expr(rng, 0, out);
                }
            }
            out.push(")".into());
        }
        11 => {
            out.push("exists".into());
            out.push("(".into());
            select(rng, depth - 1, out);
            out.push(")".into());
        }
        12 => {
            expr(rng, depth - 1, out);
            for t in ["between", "1", "and", "5"] {
                out.push(t.into());
            }
        }
        13 => {
            expr(rng, depth - 1, out);
            for t in ["is", "not", "null"] {
                out.push(t.into());
            }
        }
        _ => {
            out.push("not".into());
            expr(rng, depth - 1, out);
        }
    }
}

fn cond(rng: &mut Rng, depth: usize, out: &mut Vec<String>) {
    expr(rng, depth, out);
    for _ in 0..rng.below(4) {
        out.push(pick(rng, &["and", "or"]).into());
        expr(rng, depth, out);
    }
}

fn select(rng: &mut Rng, depth: usize, out: &mut Vec<String>) {
    if depth > 0 && rng.below(4) == 0 {
        out.push("with".into());
        out.push("c".into());
        out.push("as".into());
        out.push("(".into());
        select(rng, depth - 1, out);
        out.push(")".into());
    }
    out.push("select".into());
    if rng.below(5) == 0 {
        out.push("distinct".into());
    }
    for i in 0..=rng.below(4) {
        if i > 0 {
            out.push(",".into());
        }
        expr(rng, depth, out);
        if rng.below(3) == 0 {
            out.push("as".into());
            out.push("al".into());
        }
    }
    if rng.below(8) != 0 {
        out.push("from".into());
        for i in 0..=rng.below(2) {
            if i > 0 {
                out.push(",".into());
            }
            out.push(pick(rng, &["customers", "orders", "s.items"]).into());
            if rng.below(2) == 0 {
                out.push("x".into());
            }
            for _ in 0..rng.below(3) {
                out.push(pick(rng, &["join", "left", "inner"]).into());
                if out.last().is_some_and(|l| l != "join") {
                    out.push("join".into());
                }
                out.push("u".into());
                out.push("on".into());
                cond(rng, 1, out);
            }
        }
    }
    if rng.below(2) == 0 {
        out.push("where".into());
        cond(rng, depth, out);
    }
    if rng.below(4) == 0 {
        for t in ["group", "by"] {
            out.push(t.into());
        }
        out.push(ident(rng));
        if rng.below(2) == 0 {
            out.push("having".into());
            cond(rng, 1, out);
        }
    }
    if rng.below(3) == 0 {
        for t in ["order", "by"] {
            out.push(t.into());
        }
        out.push(ident(rng));
        out.push(pick(rng, &["asc", "desc"]).into());
    }
    if rng.below(4) == 0 {
        out.push("limit".into());
        out.push(rng.below(100).to_string());
    }
    if rng.below(6) == 0 {
        out.push(pick(rng, &["union", "union all", "except"]).into());
        select(rng, 0, out);
    }
}

fn statement(rng: &mut Rng, out: &mut Vec<String>) {
    match rng.below(8) {
        0 => {
            for t in [
                "insert", "into", "t", "(", "a", ",", "b", ")", "values", "(", "1", ",", "2", ")",
            ] {
                out.push(t.into());
            }
            if rng.below(2) == 0 {
                for t in [",", "(", "3", ",", "4", ")"] {
                    out.push(t.into());
                }
            }
        }
        1 => {
            for t in ["insert", "into", "t", "(", "a", ")"] {
                out.push(t.into());
            }
            select(rng, 1, out);
        }
        2 => {
            for t in ["update", "t", "set", "a", "=", "1", ",", "b", "="] {
                out.push(t.into());
            }
            expr(rng, 1, out);
            if rng.below(2) == 0 {
                out.push("where".into());
                cond(rng, 1, out);
            }
        }
        3 => {
            for t in ["delete", "from", "t", "where"] {
                out.push(t.into());
            }
            cond(rng, 2, out);
        }
        4 => {
            for t in [
                "create",
                "table",
                "t",
                "(",
                "id",
                "int",
                "not",
                "null",
                "primary",
                "key",
                ",",
                "name",
                "varchar",
                "(",
                "40",
                ")",
                "default",
                "'x'",
                ",",
                "constraint",
                "u",
                "unique",
                "(",
                "a",
                ",",
                "b",
                ")",
                ")",
            ] {
                out.push(t.into());
            }
        }
        _ => select(rng, 2, out),
    }
    out.push(";".into());
}

/// Tokens joined by random whitespace and comments.
fn script(rng: &mut Rng, statements: usize) -> String {
    let mut text = String::new();
    for _ in 0..statements {
        let mut toks = Vec::new();
        statement(rng, &mut toks);
        for t in toks {
            // Keyword case and spelling vary, names stay.
            let t = if rng.below(4) == 0 {
                t.to_uppercase()
            } else {
                t
            };
            text.push_str(&t);
            text.push_str(match rng.below(20) {
                0..=10 => " ",
                11 | 12 => "\n",
                13 | 14 => "\n    ",
                15 => "  ",
                16 => " /* c */ ",
                17 => " -- c\n",
                18 => "\n\n",
                _ => " ",
            });
            if t == ";" {
                text.push('\n');
            }
        }
    }
    text
}

fn styles() -> Vec<CodeStyle> {
    [
        "",
        "keyword-case=upper builtin-type-case=lower",
        "keyword-case=lower comma=leading keep-line-breaks=false",
        "and-or=line-end keep-line-breaks=false select-items=one-per-line",
        "right-margin=40 indent=2 collapse-short-statement=never",
        "right-margin=60 clause-elements=new-line comma=trailing use-tab=true",
        "collapse-short-statement=always right-margin=200 identifier-case=upper",
        "parentheses-padding=true spaces-around-operators=false space-before-parenthesis=false",
        "blank-lines-between-statements=1 right-margin=0",
    ]
    .iter()
    .map(|s| style_of(s))
    .collect()
}

#[test]
fn generated_scripts_keep_tokens_comments_and_are_idempotent() {
    let styles = styles();
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let text = script(&mut rng, 6);
        for d in [Dialect::Postgres, Dialect::Generic, Dialect::Sqlite] {
            for style in &styles {
                check(&text, d, style);
            }
        }
    }
}

/// The generator mostly makes valid statements, so the layout is exercised
/// and not only the fallback for statements with errors.
#[test]
fn most_generated_statements_are_valid() {
    let mut clean = 0;
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let text = script(&mut rng, 1);
        if parse(&text, Dialect::Postgres).errors().is_empty() {
            clean += 1;
        }
    }
    assert!(clean >= 240, "{clean} of 300 statements parse");
}

#[test]
fn single_statements_of_a_generated_script_are_idempotent_each() {
    let mut rng = Rng(0xC0FF_EE11);
    for _ in 0..100 {
        let text = script(&mut rng, 1);
        for style in styles() {
            check(&text, Dialect::Postgres, &style);
        }
    }
}

#[test]
fn statements_with_errors_are_left_as_written() {
    let invalid = include_str!("corpus/invalid.sql");
    let style = CodeStyle::default();
    let mut seen = 0;
    for block in invalid.split("\n\n") {
        let block = block.trim();
        if block.is_empty() || block.lines().all(|l| l.starts_with("--")) {
            continue;
        }
        let out = check(block, Dialect::Postgres, &style);
        assert_eq!(out, block, "an invalid statement was changed");
        seen += 1;
    }
    assert!(seen > 10);
    // A bad statement between good ones stays, the others are formatted.
    let out = format(
        "select  1;\nselect a from where;\nselect  2;",
        Dialect::Postgres,
        &style,
    );
    assert_eq!(out, "select 1;\nselect a from where;\nselect 2;");
}

#[test]
fn soup_never_panics_and_keeps_tokens() {
    let vocab = [
        "select", "from", "where", "(", ")", ",", "a", "1", "'s'", "--c\n", "/*c*/", "case",
        "when", "end", "join", "on", "and", ";", "$$", "values", "insert", "into", "set", "::",
        "[", "]", "\n", "union", "with", "as", "<<", "q'[x]'",
    ];
    let mut rng = Rng(77);
    for _ in 0..400 {
        let mut text = String::new();
        for _ in 0..rng.below(30) {
            text.push_str(vocab[rng.below(vocab.len())]);
            text.push(' ');
        }
        for d in [Dialect::Postgres, Dialect::Oracle] {
            check(&text, d, &CodeStyle::default());
        }
    }
}

#[test]
fn every_prefix_of_a_script_is_safe() {
    let mut rng = Rng(5);
    let text = script(&mut rng, 3);
    let style = CodeStyle::default();
    for end in (0..text.len()).filter(|&i| text.is_char_boundary(i)) {
        check(&text[..end], Dialect::Postgres, &style);
    }
}

#[test]
fn the_corpus_formats() {
    let sources = [
        (include_str!("corpus/common.sql"), Dialect::Generic),
        (include_str!("corpus/postgres.sql"), Dialect::Postgres),
        (include_str!("corpus/oracle.sql"), Dialect::Oracle),
        (include_str!("corpus/sqlite.sql"), Dialect::Sqlite),
        (include_str!("corpus/ddl.sql"), Dialect::Postgres),
        (include_str!("corpus/dml.sql"), Dialect::Postgres),
        (include_str!("corpus/plpgsql.sql"), Dialect::Postgres),
        (include_str!("corpus/plsql.sql"), Dialect::Oracle),
    ];
    for (src, d) in sources {
        for style in styles() {
            check(src, d, &style);
            // Block by block as well: the blocks are statements.
            for block in src.split("\n\n") {
                check(block, d, &style);
            }
        }
    }
}

#[test]
fn disabled_formatting_changes_nothing() {
    let style = style_of("disable-formatting=true");
    let text = "select  1  from   t ;";
    assert_eq!(format(text, Dialect::Postgres, &style), text);
}

#[test]
fn crlf_line_ends_stay() {
    let out = format(
        "select a,\r\n b from t\r\nwhere x = 1;\r\n",
        Dialect::Postgres,
        &CodeStyle::default(),
    );
    assert_eq!(out, "select a,\r\n       b\r\nfrom t\r\nwhere x = 1;\r\n");
}

#[test]
fn range_formatting_takes_whole_statements() {
    let text = "select  1;\nselect  2 ,3;\nselect  4;\n";
    let style = CodeStyle::default();
    let at = text.find("2 ,").unwrap();
    let edit = format_range(text, at..at + 1, Dialect::Postgres, &style).unwrap();
    assert_eq!(&text[edit.range.clone()], "select  2 ,3;\n");
    assert_eq!(edit.text, "select 2, 3;\n");
    // Nothing to do on formatted text.
    let done = "select 1;\n";
    assert!(format_range(done, 0..3, Dialect::Postgres, &style).is_none());
    // A range over two statements.
    let edit = format_range(text, 0..text.len(), Dialect::Postgres, &style).unwrap();
    assert_eq!(edit.text, "select 1;\nselect 2, 3;\nselect 4;\n");
}

#[test]
fn the_style_takes_options_by_name() {
    let mut s = CodeStyle::default();
    assert!(s.set("keyword-case", "upper").is_ok());
    assert!(s.set("keyword-case", "shout").is_err());
    assert!(s.set("no-such-option", "1").is_err());
    assert!(s.set("right-margin", "-3").is_err());
    assert_eq!(s.keyword_case, sql_syntax::Case::Upper);
    let errors = s.set_all([("indent", "2"), ("indent", "x"), ("comma", "leading")]);
    assert_eq!(errors.len(), 1);
    assert_eq!(s.indent, 2);
    assert_eq!(s.comma, sql_syntax::CommaPlacement::Leading);
}

#[test]
fn a_long_script_is_formatted_in_one_pass_each_way() {
    let mut rng = Rng(9);
    let text = script(&mut rng, 300);
    let style = CodeStyle::default();
    let once = check(&text, Dialect::Postgres, &style);
    assert!(once.lines().count() > 300);
}
