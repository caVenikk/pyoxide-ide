//! Property tests of the syntax tree: random input never panics, the tree
//! prints back to its input, node ranges nest, and a re-parse after a random
//! edit equals a full parse.
use sql_syntax::{Dialect, HOLE, Script, SyntaxElement, SyntaxNode, TextEdit, parse};

use crate::Rng;

const VOCAB: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "not",
    "in",
    "is",
    "null",
    "between",
    "like",
    "ilike",
    "case",
    "when",
    "then",
    "else",
    "end",
    "cast",
    "as",
    "exists",
    "group",
    "by",
    "having",
    "order",
    "asc",
    "desc",
    "nulls",
    "first",
    "limit",
    "offset",
    "fetch",
    "next",
    "rows",
    "only",
    "union",
    "all",
    "intersect",
    "except",
    "minus",
    "join",
    "left",
    "inner",
    "cross",
    "natural",
    "on",
    "using",
    "lateral",
    "with",
    "recursive",
    "values",
    "distinct",
    "over",
    "partition",
    "window",
    "filter",
    "within",
    "connect",
    "prior",
    "start",
    "pivot",
    "for",
    "update",
    "array",
    "interval",
    "extract",
    "substring",
    "trim",
    "position",
    "insert",
    "into",
    "create",
    "table",
    "begin",
    "declare",
    "a",
    "b",
    "t",
    "c.",
    "c.*",
    "s.t",
    "f(",
    "count(*)",
    "(",
    ")",
    "((",
    "))",
    ",",
    ";",
    ".",
    "*",
    "=",
    "<>",
    "+",
    "-",
    "||",
    "::",
    "->>",
    "@>",
    "(+)",
    "[",
    "]",
    "[1]",
    ":",
    "=>",
    "1",
    "2.5",
    "'x'",
    "'it''s'",
    "\"Q\"",
    "$1",
    "?",
    ":n",
    "$$ x $$",
    "-- c\n",
    "/* c */",
    "/*",
    "'",
    "\"",
    "\n",
    "\n/\n",
    "  ",
    "é",
    "日本",
    "\u{e000}",
    "date '2024-01-01'",
];

const CHARS: &[char] = &[
    'a', 'z', '_', '1', ' ', '\n', '\'', '"', '(', ')', '[', ']', ',', ';', '.', ':', '$', '?',
    '-', '+', '*', '/', '<', '>', '=', '|', 'é', '日', '\u{e000}',
];

fn vocab_soup(rng: &mut Rng, max: usize) -> String {
    let len = 1 + rng.below(max);
    let mut s = String::new();
    for _ in 0..len {
        s.push_str(VOCAB[rng.below(VOCAB.len())]);
        if rng.below(3) > 0 {
            s.push(' ');
        }
    }
    s
}

fn char_soup(rng: &mut Rng, max: usize) -> String {
    (0..rng.below(max))
        .map(|_| CHARS[rng.below(CHARS.len())])
        .collect()
}

/// Ranges nest: every child lies inside its parent, children follow each
/// other, and every range falls on character boundaries.
fn check_node(src: &str, node: SyntaxNode<'_>) {
    let r = node.range();
    assert!(
        r.start <= r.end && r.end <= src.len(),
        "{node:?} in {src:?}"
    );
    assert!(src.is_char_boundary(r.start) && src.is_char_boundary(r.end));
    let mut prev_end = r.start;
    for c in node.children() {
        let cr = c.range();
        if let SyntaxElement::Token(t) = c
            && t.is_trivia()
        {
            continue;
        }
        assert!(
            cr.start >= prev_end && cr.end <= r.end && cr.start >= r.start,
            "{c:?} outside {node:?} or before {prev_end} in {src:?}"
        );
        prev_end = cr.end;
        if let SyntaxElement::Node(n) = c {
            assert_eq!(n.parent(), Some(node));
            check_node(src, n);
        }
    }
}

fn check_script(src: &str, d: Dialect, rng: &mut Rng) -> Script {
    let s = parse(src, d);
    assert_eq!(s.print(), src, "{d:?}");
    let mut end = 0;
    for root in s.statements() {
        assert_eq!(root.range().start, end, "units are adjacent in {src:?}");
        end = root.range().end;
        check_node(src, root);
    }
    assert_eq!(end, src.len());
    for e in s.errors() {
        assert!(src.is_char_boundary(e.range.start) && src.is_char_boundary(e.range.end));
        let _ = e.message(src);
    }
    for _ in 0..3 {
        let at = rng.below(src.len() + 1);
        let _ = s.expected_at(at);
        if let Some(n) = s.node_at(at) {
            assert!(n.range().start <= at && at <= n.range().end);
        }
        let _ = s.token_at(at);
    }
    s
}

fn floor(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A random edit of `src`: an insertion, a deletion or a replacement.
fn random_edit(src: &str, rng: &mut Rng) -> TextEdit {
    let a = floor(src, rng.below(src.len() + 1));
    let b = floor(src, a + rng.below(12)).max(a);
    let text = match rng.below(4) {
        0 => String::new(),
        1 => VOCAB[rng.below(VOCAB.len())].to_string(),
        2 => char_soup(rng, 4),
        _ => format!(" {} ", vocab_soup(rng, 3)),
    };
    let range = if rng.below(3) == 0 { a..a } else { a..b };
    TextEdit { range, text }
}

fn check_reparse(src: &str, d: Dialect, rng: &mut Rng) {
    let mut cur = parse(src, d);
    // A chain of edits, each compared with a full parse.
    for _ in 0..4 {
        let edit = random_edit(cur.text(), rng);
        let mut text = cur.text().to_string();
        text.replace_range(edit.range.clone(), &edit.text);
        let next = cur.reparse(&edit);
        let full = parse(&text, d);
        assert!(
            next == full,
            "{d:?}: {:?} with {edit:?}\nreparse: {:#?}\nfull: {:#?}",
            cur.text(),
            next.statements().map(|r| r.dump()).collect::<String>(),
            full.statements().map(|r| r.dump()).collect::<String>(),
        );
        cur = next;
    }
}

#[test]
fn vocabulary_soup_never_panics_and_keeps_the_invariants() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..1500 {
        let src = vocab_soup(&mut rng, 40);
        for d in Dialect::ALL {
            check_script(&src, d, &mut rng);
        }
    }
}

#[test]
fn character_soup_never_panics_and_keeps_the_invariants() {
    let mut rng = Rng(0x0F1E_2D3C_4B5A_6978);
    for _ in 0..2000 {
        let src = char_soup(&mut rng, 60);
        for d in Dialect::ALL {
            check_script(&src, d, &mut rng);
        }
    }
}

#[test]
fn every_prefix_of_a_script_is_safe() {
    let script = "with r as (select a, b::int from s.t where x in (1, 2) and y like 'é%')\n\
                  select r.a, count(*) over (partition by b order by a rows between 1 preceding and current row)\n\
                  from r left join u on u.id = r.a(+) where exists (select 1) group by 1 order by 2 desc nulls last;\n\
                  select case when a then b else c end, cast(d as varchar(2)) from v limit 3;";
    let mut rng = Rng(7);
    for end in 0..=script.len() {
        if script.is_char_boundary(end) {
            for d in Dialect::ALL {
                check_script(&script[..end], d, &mut rng);
            }
        }
    }
}

#[test]
fn reparse_after_random_edits_equals_a_full_parse() {
    let base = [
        "select a from t;\nselect b, c from u where x = 1;\n\nselect 1;",
        "with x as (select 1) select * from x;\ninsert into t values (1);\nselect 'a;b' from dual;\n",
        "begin\n  null;\nend;\n/\nselect 1 from dual;\ncreate procedure p is begin null; end;\n/\n",
        "select $$ ; $$; select /* ; */ 1; -- ;\nselect \"a;\" from t",
    ];
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    for _ in 0..300 {
        for src in base {
            for d in Dialect::ALL {
                check_reparse(src, d, &mut rng);
            }
        }
    }
    for _ in 0..300 {
        let src = format!(
            "{};\n{};\n{}",
            vocab_soup(&mut rng, 12),
            vocab_soup(&mut rng, 12),
            vocab_soup(&mut rng, 12)
        );
        for d in Dialect::ALL {
            check_reparse(&src, d, &mut rng);
        }
    }
}

#[test]
fn holes_never_panic() {
    let mut rng = Rng(99);
    for _ in 0..500 {
        let mut src = vocab_soup(&mut rng, 20);
        let at = floor(&src, rng.below(src.len() + 1));
        src.insert(at, HOLE);
        for d in Dialect::ALL {
            check_script(&src, d, &mut rng);
        }
    }
}
