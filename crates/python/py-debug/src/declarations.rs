//! Where a frame's variables are declared, for their values shown inline
//! after the lines (the reference's `PySourcePositionResolver
//! .getSourcePositionForName`): a name is resolved from the execution
//! point scope by scope outwards (`PyResolveUtil.scopeCrawlUp`) to its
//! first binding in the scope that has one: a parameter, an assignment
//! target, a `for` or `with` target, an `except ... as` name, an imported
//! name, a `def` or `class`, a walrus. So a variable's value shows once,
//! after the line that declares it (`n` on its `def` line, `total` after
//! `total = 0`, not after `total += ...`).
//!
//! The text is parsed with `tree-sitter-python`; a class body is a scope
//! only for a frame in it (functions do not see the names of their
//! class), comprehensions and lambdas are scopes of their own. Lines count
//! from 1, as the debugger model counts them.

use tree_sitter::{Node, Parser, Point};

/// A declaration's line (from 1) and column (a byte offset in the line,
/// from 0).
pub type Pos = (u32, u32);

/// The position of the declaration of each of `names` as seen from line
/// `line` of `text` (both from 1); `None` for a name without one.
pub fn declarations(text: &str, line: u32, names: &[String]) -> Vec<Option<Pos>> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
        return vec![None; names.len()];
    }
    let Some(tree) = parser.parse(text, None) else {
        return vec![None; names.len()];
    };
    let root = tree.root_node();
    let source = text.as_bytes();
    let scopes = scopes_at(root, source, line);
    // The bindings of every scope, innermost first, collected once.
    let bindings: Vec<Vec<(String, Pos)>> = scopes
        .iter()
        .map(|scope| {
            let mut out = Vec::new();
            scope_bindings(*scope, source, &mut out);
            out
        })
        .collect();
    let globals: Vec<Vec<String>> = scopes
        .iter()
        .map(|scope| {
            let mut out = Vec::new();
            global_names(*scope, source, &mut out);
            out
        })
        .collect();
    names
        .iter()
        .map(|name| {
            for (i, scope) in bindings.iter().enumerate() {
                // `global x` sends the name to the module.
                if i + 1 < bindings.len() && globals[i].iter().any(|g| g == name) {
                    return first(&bindings[bindings.len() - 1], name);
                }
                if let Some(line) = first(scope, name) {
                    return Some(line);
                }
            }
            None
        })
        .collect()
}

fn first(bindings: &[(String, Pos)], name: &str) -> Option<Pos> {
    bindings.iter().find(|(n, _)| n == name).map(|(_, l)| *l)
}

/// Kinds of nodes with a scope of their own.
fn is_scope(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "class_definition"
            | "lambda"
            | "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    )
}

/// The scopes around line `line` (from 1), innermost first, the module
/// last. A class is left out when the line is in a function inside it.
fn scopes_at<'t>(root: Node<'t>, source: &[u8], line: u32) -> Vec<Node<'t>> {
    let row = line.saturating_sub(1) as usize;
    let column = source.split(|&b| b == b'\n').nth(row).map_or(0, |l| {
        l.iter().take_while(|b| b.is_ascii_whitespace()).count()
    });
    let at = Point { row, column };
    let mut node = root.descendant_for_point_range(at, at).unwrap_or(root);
    let mut scopes = Vec::new();
    let mut in_function = false;
    loop {
        let kind = node.kind();
        if is_scope(kind) && node.start_position().row <= row {
            // The header line of a `def` belongs to the function (its
            // parameters are what a frame stopped there sees).
            let skip = kind == "class_definition" && in_function;
            if !skip {
                scopes.push(node);
            }
            in_function |= kind == "function_definition" || kind == "lambda";
        }
        match node.parent() {
            Some(parent) => node = parent,
            None => break,
        }
    }
    scopes.push(root);
    scopes
}

/// The names a scope binds, in the order of the text, without going into
/// the scopes inside it.
fn scope_bindings(scope: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    match scope.kind() {
        "function_definition" => {
            if let Some(params) = scope.child_by_field_name("parameters") {
                parameters(params, source, out);
            }
            if let Some(body) = scope.child_by_field_name("body") {
                walk(body, source, out);
            }
        }
        "lambda" => {
            if let Some(params) = scope.child_by_field_name("parameters") {
                parameters(params, source, out);
            }
        }
        "class_definition" => {
            if let Some(body) = scope.child_by_field_name("body") {
                walk(body, source, out);
            }
        }
        "module" => walk(scope, source, out),
        // Comprehensions: their `for` targets.
        _ => {
            let mut cursor = scope.walk();
            for child in scope.named_children(&mut cursor) {
                if child.kind() == "for_in_clause"
                    && let Some(left) = child.child_by_field_name("left")
                {
                    targets(left, source, out);
                }
            }
        }
    }
}

fn parameters(params: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    let mut cursor = params.walk();
    for p in params.named_children(&mut cursor) {
        match p.kind() {
            "identifier" => push(p, source, out),
            "default_parameter" | "typed_default_parameter" => {
                if let Some(name) = p.child_by_field_name("name") {
                    targets(name, source, out);
                }
            }
            "typed_parameter" | "list_splat_pattern" | "dictionary_splat_pattern" => {
                let mut c = p.walk();
                for child in p.named_children(&mut c) {
                    match child.kind() {
                        "identifier" => {
                            push(child, source, out);
                            break;
                        }
                        "list_splat_pattern" | "dictionary_splat_pattern" => {
                            targets(child, source, out);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            "tuple_pattern" => targets(p, source, out),
            _ => {}
        }
    }
}

/// The bindings of statements and expressions under `node`, in the order
/// of the text.
fn walk(node: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, out);
    }
}

/// The bindings of one node and of what is under it.
fn visit(node: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    match node.kind() {
        "function_definition" | "class_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                push(name, source, out);
            }
            if node.kind() == "function_definition" {
                // Defaults and annotations are evaluated in the scope
                // around; the body is the function's own.
                if let Some(params) = node.child_by_field_name("parameters") {
                    walk(params, source, out);
                }
            }
        }
        "lambda"
        | "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => {}
        "assignment" => {
            if let Some(left) = node.child_by_field_name("left") {
                targets(left, source, out);
            }
            // `a = b = 1`: the right side is an assignment too.
            if let Some(right) = node.child_by_field_name("right") {
                visit(right, source, out);
            }
        }
        "for_statement" => {
            if let Some(left) = node.child_by_field_name("left") {
                targets(left, source, out);
            }
            for field in ["right", "body", "alternative"] {
                if let Some(part) = node.child_by_field_name(field) {
                    visit(part, source, out);
                }
            }
        }
        "as_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "as_pattern_target" {
                    visit(child, source, out);
                }
            }
            if let Some(alias) = node.child_by_field_name("alias") {
                targets(alias, source, out);
            }
        }
        "except_clause" => {
            let alias = node.child_by_field_name("alias");
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if Some(child) == alias {
                    targets(child, source, out);
                } else {
                    visit(child, source, out);
                }
            }
        }
        "import_statement" | "import_from_statement" => imports(node, source, out),
        "named_expression" => {
            if let Some(name) = node.child_by_field_name("name") {
                push(name, source, out);
            }
            if let Some(value) = node.child_by_field_name("value") {
                visit(value, source, out);
            }
        }
        _ => walk(node, source, out),
    }
}

fn imports(node: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    let from = node.kind() == "import_from_statement";
    let mut cursor = node.walk();
    for name in node.children_by_field_name("name", &mut cursor) {
        match name.kind() {
            "aliased_import" => {
                if let Some(alias) = name.child_by_field_name("alias") {
                    push(alias, source, out);
                }
            }
            "dotted_name" => {
                // `import os.path` binds `os`; `from a import b` binds `b`.
                let mut c = name.walk();
                let parts: Vec<Node> = name.named_children(&mut c).collect();
                let bound = if from { parts.last() } else { parts.first() };
                if let Some(part) = bound {
                    push(*part, source, out);
                }
            }
            _ => {}
        }
    }
}

/// The names an assignment target binds: names, also inside tuples and
/// lists and after `*`; attributes and subscripts bind none.
fn targets(node: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    match node.kind() {
        "identifier" => push(node, source, out),
        "attribute" | "subscript" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                targets(child, source, out);
            }
        }
    }
}

fn global_names(scope: Node, source: &[u8], out: &mut Vec<String>) {
    fn go(node: Node, source: &[u8], out: &mut Vec<String>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if is_scope(child.kind()) {
                continue;
            }
            if child.kind() == "global_statement" {
                let mut c = child.walk();
                for name in child.named_children(&mut c) {
                    if let Ok(text) = name.utf8_text(source) {
                        out.push(text.to_owned());
                    }
                }
            } else {
                go(child, source, out);
            }
        }
    }
    if scope.kind() == "function_definition"
        && let Some(body) = scope.child_by_field_name("body")
    {
        go(body, source, out);
    }
}

fn push(node: Node, source: &[u8], out: &mut Vec<(String, Pos)>) {
    if let Ok(text) = node.utf8_text(source) {
        let at = node.start_position();
        out.push((text.to_owned(), (at.row as u32 + 1, at.column as u32)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample of the reference screenshot: a worker thread stopped in
    /// `worker`.
    const THREADS: &str = "import threading
import time


def worker(n):
    total = 0
    for i in range(1, 600):
        total += i * n
        time.sleep(0.1)  # breakpoint 1
    return total


threads = [
    threading.Thread(target=worker, args=(k,), name=f\"worker-{k}\") for k in range(3)
]
for t in threads:
    t.start()

items = [1, 2, 3, 4]
config = {\"count\": len(threads), \"sum\": sum(items)}
while any(t.is_alive() for t in threads):
    time.sleep(0.5)  # breakpoint 2
print(\"done\", config)
";

    fn lines(text: &str, line: u32, names: &[&str]) -> Vec<Option<u32>> {
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        declarations(text, line, &names)
            .into_iter()
            .map(|p| p.map(|(line, _)| line))
            .collect()
    }

    #[test]
    fn a_function_frame_finds_parameters_and_first_assignments() {
        assert_eq!(
            lines(THREADS, 9, &["i", "n", "total", "time", "nothing"]),
            vec![Some(7), Some(5), Some(6), Some(2), None]
        );
    }

    #[test]
    fn a_module_frame_finds_imports_and_module_names() {
        assert_eq!(
            lines(
                THREADS,
                22,
                &[
                    "threading",
                    "threads",
                    "t",
                    "items",
                    "config",
                    "k",
                    "worker"
                ]
            ),
            vec![
                Some(1),
                Some(13),
                Some(16),
                Some(19),
                Some(20),
                None,
                Some(5)
            ]
        );
    }

    #[test]
    fn targets_of_every_kind_bind() {
        let text = "def f(a, b=1, *args, c: int = 2, **kw):
    x, (y, *z) = 1, (2, 3)
    p = q = 0
    with open('f') as fh:
        pass
    try:
        pass
    except Exception as err:
        pass
    if (w := 3):
        pass
    import os.path
    from sys import argv as av
    obj.attr = 1
    return a
";
        assert_eq!(
            lines(
                text,
                15,
                &[
                    "a", "b", "args", "c", "kw", "x", "y", "z", "p", "q", "fh", "err", "w", "os",
                    "av", "attr"
                ]
            ),
            vec![
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(2),
                Some(2),
                Some(2),
                Some(3),
                Some(3),
                Some(4),
                Some(8),
                Some(10),
                Some(12),
                Some(13),
                None
            ]
        );
    }

    #[test]
    fn a_method_does_not_see_its_class_and_global_names_go_to_the_module() {
        let text = "count = 0

class A:
    size = 1

    def m(self):
        global count
        count = 2
        return size
";
        assert_eq!(
            lines(text, 9, &["self", "count", "size"]),
            vec![Some(6), Some(1), None]
        );
    }
}
