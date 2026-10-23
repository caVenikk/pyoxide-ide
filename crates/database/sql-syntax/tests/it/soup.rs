//! Property tests: random input never panics, and the invariants hold.
use sql_syntax::{
    Dialect, ExplainOptions, HOLE, ParamForms, Token, attach_trivia, candidates_at, classify_text,
    explain_statements, lex, lex_with, parameters, split, statement_at, statements_in,
};

use crate::{Rng, check_ranges};

const VOCAB: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "between",
    "case",
    "when",
    "then",
    "end",
    "group",
    "by",
    "order",
    "left",
    "join",
    "on",
    "union",
    "all",
    "values",
    "update",
    "set",
    "insert",
    "into",
    "with",
    "as",
    "delete",
    "create",
    "table",
    "trigger",
    "function",
    "procedure",
    "package",
    "body",
    "type",
    "begin",
    "declare",
    "if",
    "loop",
    "while",
    "is",
    "atomic",
    "not",
    "do",
    "language",
    "(",
    ")",
    ",",
    ";",
    ".",
    "a",
    "t",
    "date",
    "1",
    "1.5e+3",
    ".5",
    "0x1F",
    "'x y'",
    "'it''s'",
    "\"q\"",
    "\"\"",
    "[b]",
    "`c`",
    "-- note\n",
    "--",
    "/* c */",
    "/*",
    "*/",
    "=",
    "-",
    "*",
    "::",
    ":=",
    "=>",
    "->>",
    "@>",
    "||",
    "<>",
    "$1",
    "?",
    "?1",
    ":name",
    ":new.x",
    "@v",
    "$w",
    "${p}",
    "%s",
    "%(n)s",
    "$$ body $$",
    "$tag$",
    "$a$ x ; $a$",
    "#t",
    "\n",
    "\n\n",
    "\n/\n",
    "/\n",
    "  ",
    "\t",
    "e",
    "n",
    "x",
    "b",
    "q",
    "u&",
    "E'a\\'b'",
    "N'n'",
    "X'1F'",
    "[",
    "]",
    "{",
    "}",
    "<",
    ">",
    "!",
    "/",
    "+",
    "\\",
    "'",
    "\"",
    "$",
    "q'[s; t]'",
    "nq'{u}'",
    "q'!",
    "<<l>>",
    "'p'\n'q'",
    "é",
    "ñandú",
    "🎵",
    "\u{a0}",
    "\u{e000}",
    "ü_1",
    "日本",
];

const CHARS: &[char] = &[
    'a', 'Z', '_', '1', '9', ' ', '\n', '\r', '\t', '\'', '"', '`', '[', ']', '(', ')', ',', ';',
    '.', ':', '$', '?', '@', '%', '#', '-', '+', '*', '/', '\\', '<', '>', '=', '!', '|', '&', '~',
    '^', '{', '}', 'q', 'E', 'N', 'é', 'ñ', '日', '🎵', '\u{a0}', '\u{e000}', '\u{7f}', '\u{0}',
];

fn vocab_soup(rng: &mut Rng) -> String {
    let len = 1 + rng.below(40);
    let mut s = String::new();
    for _ in 0..len {
        s.push_str(VOCAB[rng.below(VOCAB.len())]);
        if rng.below(3) > 0 {
            s.push(' ');
        }
    }
    s
}

fn char_soup(rng: &mut Rng) -> String {
    let len = rng.below(60);
    (0..len).map(|_| CHARS[rng.below(CHARS.len())]).collect()
}

fn all_forms() -> ParamForms {
    ParamForms {
        question: true,
        question_numbered: true,
        dollar_numbered: true,
        dollar_named: true,
        brace_named: true,
        colon_named: true,
        colon_numbered: true,
        at_named: true,
        pyformat: true,
        percent_s: true,
    }
}

fn check_tokens(src: &str, tokens: &[Token]) {
    let mut end = 0;
    let mut joined = String::new();
    for t in tokens {
        assert_eq!(t.start, end, "gap in {src:?}");
        assert!(t.end > t.start);
        assert!(
            src.is_char_boundary(t.start) && src.is_char_boundary(t.end),
            "boundary in {src:?}"
        );
        joined.push_str(t.text(src));
        end = t.end;
    }
    assert_eq!(end, src.len());
    assert_eq!(joined, src, "round trip");
}

fn check_everything(src: &str, rng: &mut Rng) {
    for d in Dialect::ALL {
        // The tokens give the input back, with any parameter forms.
        check_tokens(src, &lex(src, d));
        check_tokens(src, &lex_with(src, d, all_forms()));
        check_tokens(src, &lex_with(src, d, ParamForms::NONE));

        let tokens = lex(src, d);
        let items = attach_trivia(src, &tokens);
        if !items.is_empty() {
            let joined: String = items
                .iter()
                .map(|a| {
                    format!(
                        "{}{}{}",
                        &src[a.leading.clone()],
                        a.token.text(src),
                        &src[a.trailing.clone()]
                    )
                })
                .collect();
            assert_eq!(joined, src);
        }

        let st = split(src, d);
        check_ranges(src, &st);
        // A statement's text lexes back to itself and holds a significant token.
        for s in &st {
            check_tokens(&s.text, &lex(&s.text, d));
            assert!(lex(&s.text, d).iter().any(|t| !t.is_trivia()));
        }
        let caret = rng.below(src.len() + 3);
        match statement_at(src, caret, d) {
            Some(s) => assert!(st.contains(&s)),
            None => assert!(st.is_empty()),
        }
        for c in candidates_at(src, caret, d) {
            assert!(
                src.is_char_boundary(c.range.start)
                    && src.is_char_boundary(c.range.end)
                    && c.range.start < c.range.end
            );
        }
        let a = rng.below(src.len() + 2);
        let b = rng.below(src.len() + 2);
        let part = statements_in(src, a.min(b)..a.max(b), d);
        check_ranges(src, &part);
        let _ = parameters(src, d, all_forms());
        let _ = classify_text(src, d);
        for s in &st {
            let _ = explain_statements(&s.text, d, ExplainOptions { analyze: true });
        }
        // Names.
        let _ = d.fold(src);
        let _ = d.quote(src);
        let _ = d.keyword(src);
    }
}

#[test]
fn vocabulary_soup_never_panics_and_keeps_the_invariants() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for _ in 0..2500 {
        let s = vocab_soup(&mut rng);
        check_everything(&s, &mut rng);
    }
}

#[test]
fn character_soup_never_panics_and_keeps_the_invariants() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..4000 {
        let s = char_soup(&mut rng);
        check_everything(&s, &mut rng);
    }
}

#[test]
fn every_prefix_of_a_script_is_safe() {
    let script = "select 'é;', \"ñ\" -- c;\n/* x */ from t where a = $1 and b = :n; do $$ x; $$;\nbegin\n  null;\nend;\n/\nselect q'[a;]' from dual";
    for end in 0..=script.len() {
        if script.is_char_boundary(end) {
            check_everything(&script[..end], &mut Rng(end as u64 + 1));
        }
    }
}

#[test]
fn the_hole_survives_everywhere() {
    for d in Dialect::ALL {
        let src = format!("select {HOLE} from {HOLE}.{HOLE}t where x = {HOLE}{HOLE}");
        check_tokens(&src, &lex(&src, d));
        assert_eq!(split(&src, d).len(), 1);
    }
}
