//! How the variables view shows Python values from debugpy, as the
//! reference shows its Python values (`PyDebugValue`, `PyStackFrame`):
//! lists and tuples with the array icon, other containers with the value
//! icon; debugpy's `special variables` child as the "Special Variables"
//! group, its `(return) f` children as `f()` in "Return Values"; debugpy's
//! `more` child of a large collection (`MoreItemsRange`, `[100:1000]`) as
//! the rest of the collection; the length of containers shown with their
//! type (`{list: 4}`), as pydevd's `shape` (`pydevd_xml.var_to_xml`: the
//! `len()` of a value with `__len__` that is not a string).

use debugger::Variable;
use debugger::values::{GroupIcon, ValueKind, ValueNode, ValueRole};

/// debugpy's name prefix of a value a call returned (`showReturnValue`).
const RETURN_PREFIX: &str = "(return) ";
/// debugpy's type of the child that holds the rest of a large collection.
const MORE_ITEMS: &str = "MoreItemsRange";
/// The reference's group titles (`PyBundle`
/// `debugger.stack.frame.special.variables`, `.return.values`;
/// `PydevBundle` `pydev.value.protected.attributes.group.name`).
const SPECIAL: &str = "Special Variables";
const RETURN_VALUES: &str = "Return Values";
const PROTECTED: &str = "Protected Attributes";
/// debugpy's name of the child that holds the length of a container, and
/// pydevd's (`PyDebugValueGroupsKt.DUNDER_LEN`).
const LEN: &str = "len()";
const DUNDER_LEN: &str = "__len__";
/// The names that stay among a value's children when its protected
/// attributes are grouped (`PROTECTED_ATTRS_EXCLUDED`).
const NOT_PROTECTED: [&str; 2] = [DUNDER_LEN, "__exception__"];

/// Types shown with the array icon (`NodeTypes.LIST_NODE_TYPE`,
/// `TUPLE_NODE_TYPE`).
const ARRAYS: [&str; 2] = ["list", "tuple"];

pub(crate) fn present(variable: &Variable) -> ValueNode {
    let type_name = variable.type_name.as_deref().unwrap_or_default();
    // debugpy's groups: a child named `<scope> variables` with neither a
    // type nor a value.
    if type_name.is_empty()
        && variable.value.is_empty()
        && variable.reference > 0
        && let Some(scope) = variable.name.strip_suffix(" variables")
    {
        let (title, icon) = match scope {
            "special" => (SPECIAL.to_owned(), GroupIcon::Special),
            "protected" => (PROTECTED.to_owned(), GroupIcon::Plain),
            other => (format!("{} Variables", title_case(other)), GroupIcon::Plain),
        };
        return ValueNode {
            name: title.clone(),
            role: ValueRole::Group { title, icon },
        };
    }
    if type_name == MORE_ITEMS && variable.reference > 0 {
        return ValueNode {
            name: variable.name.clone(),
            role: ValueRole::More {
                remaining: remaining(&variable.value),
            },
        };
    }
    if let Some(name) = variable.name.strip_prefix(RETURN_PREFIX) {
        return ValueNode {
            name: format!("{name}()"),
            role: ValueRole::ReturnValue {
                group: RETURN_VALUES.to_owned(),
            },
        };
    }
    let kind = if variable.reference <= 0 {
        ValueKind::Primitive
    } else if ARRAYS.contains(&type_name) {
        ValueKind::Array
    } else {
        ValueKind::Object
    };
    ValueNode {
        name: variable.name.clone(),
        role: ValueRole::Value(kind),
    }
}

/// A child of a value, or of a frame's scope or a group (`scope`). The children of a value are
/// shown as the reference's pydevd shows them (`PyDebugValue
/// .computeChildren` in the simplified view): the length as `__len__`, the
/// names that start with `_` (debugpy's "special variables" and the
/// protected names it leaves among the values) in a "Protected
/// Attributes" group after the others.
pub(crate) fn present_child(variable: &Variable, scope: bool) -> ValueNode {
    let mut node = present(variable);
    if scope {
        return node;
    }
    match &node.role {
        ValueRole::Group { title, .. } if title == SPECIAL => {
            node.name = PROTECTED.to_owned();
            node.role = ValueRole::Group {
                title: PROTECTED.to_owned(),
                icon: GroupIcon::Protected,
            };
        }
        ValueRole::Group { title, .. } if title == PROTECTED => {
            node.role = ValueRole::Group {
                title: PROTECTED.to_owned(),
                icon: GroupIcon::Protected,
            };
        }
        ValueRole::Value(kind) => {
            if variable.name == LEN {
                node.name = DUNDER_LEN.to_owned();
            } else if node.name.starts_with('_') && !NOT_PROTECTED.contains(&node.name.as_str()) {
                node.role = ValueRole::Grouped {
                    kind: *kind,
                    title: PROTECTED.to_owned(),
                    icon: GroupIcon::Protected,
                };
            }
        }
        _ => {}
    }
    node
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The children left after a `more` child: `[100:1000]` leaves 900.
fn remaining(range: &str) -> Option<usize> {
    let (start, end) = range
        .trim()
        .strip_prefix('[')?
        .strip_suffix(']')?
        .split_once(':')?;
    let start: usize = start.trim().parse().ok()?;
    let end: usize = end.trim().parse().ok()?;
    end.checked_sub(start)
}

/// One Python expression for the sizes of several values: a tuple with the
/// length of each value that has one (`__len__` on its type, not a string
/// or bytes), `None` for the others, as a string (debugpy shows at most
/// about 60 items of a tuple, all of a string). The expression of a value
/// is evaluated again, so one with a call (a watch such as `items.pop()`)
/// gets no size rather than being called again.
pub(crate) fn sizes_expression(expressions: &[&str]) -> Option<String> {
    let calls = |e: &&str| e.contains('(');
    if expressions.iter().all(calls) {
        return None;
    }
    let mut out = String::from("str((");
    for e in expressions {
        if calls(e) {
            out.push_str("None, ");
            continue;
        }
        out.push_str(&format!(
            "len({e}) if hasattr(type({e}), '__len__') and not isinstance({e}, (str, bytes, bytearray)) else None, "
        ));
    }
    out.push_str("))");
    Some(out)
}

/// The sizes in the value of [`sizes_expression`]: `'(4, None, 2)'`.
pub(crate) fn parse_sizes(value: &str, count: usize) -> Vec<Option<String>> {
    let value = value.trim();
    let value = value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .unwrap_or(value);
    let inner = value
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or_default();
    let mut out: Vec<Option<String>> = inner
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u64>().ok().map(|n| n.to_string()))
        .collect();
    if out.len() != count {
        out = vec![None; count];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str, type_name: &str, value: &str, reference: i64) -> Variable {
        Variable {
            name: name.to_owned(),
            value: value.to_owned(),
            type_name: Some(type_name.to_owned()),
            presentation: Default::default(),
            evaluate_name: Some(name.to_owned()),
            reference,
            named: None,
            indexed: None,
        }
    }

    #[test]
    fn kinds_groups_return_values_and_more() {
        let node = present(&variable("items", "list", "[1, 2]", 8));
        assert_eq!(node.role, ValueRole::Value(ValueKind::Array));
        let node = present(&variable("summary", "dict", "{}", 9));
        assert_eq!(node.role, ValueRole::Value(ValueKind::Object));
        let node = present(&variable("value", "int", "1", 0));
        assert_eq!(node.role, ValueRole::Value(ValueKind::Primitive));
        let node = present(&variable("special variables", "", "", 10));
        assert_eq!(node.name, "Special Variables");
        assert_eq!(
            node.role,
            ValueRole::Group {
                title: "Special Variables".into(),
                icon: GroupIcon::Special
            }
        );
        let node = present(&variable("function variables", "", "", 11));
        assert_eq!(node.name, "Function Variables");
        let node = present(&variable("(return) describe", "dict", "{}", 12));
        assert_eq!(node.name, "describe()");
        assert_eq!(
            node.role,
            ValueRole::ReturnValue {
                group: "Return Values".into()
            }
        );
        let node = present(&variable("more", "MoreItemsRange", "[100:1000]", 13));
        assert_eq!(
            node.role,
            ValueRole::More {
                remaining: Some(900)
            }
        );
    }

    #[test]
    fn the_children_of_a_value_group_their_protected_attributes() {
        // A frame's scope (and a group) keeps debugpy's groups and names.
        let node = present_child(&variable("special variables", "", "", 10), true);
        assert_eq!(node.name, "Special Variables");
        let node = present_child(&variable("_cache", "dict", "{}", 0), true);
        assert!(matches!(node.role, ValueRole::Value(_)));
        // A value's children: the group is the protected attributes.
        let node = present_child(&variable("special variables", "", "", 10), false);
        assert_eq!(node.name, "Protected Attributes");
        assert_eq!(
            node.role,
            ValueRole::Group {
                title: "Protected Attributes".into(),
                icon: GroupIcon::Protected
            }
        );
        let node = present_child(&variable("len()", "int", "2", 0), false);
        assert_eq!(node.name, "__len__");
        assert_eq!(node.role, ValueRole::Value(ValueKind::Primitive));
        let node = present_child(&variable("_cache", "list", "[]", 4), false);
        assert_eq!(
            node.role,
            ValueRole::Grouped {
                kind: ValueKind::Array,
                title: "Protected Attributes".into(),
                icon: GroupIcon::Protected
            }
        );
        let node = present_child(&variable("count", "int", "1", 0), false);
        assert_eq!(node.role, ValueRole::Value(ValueKind::Primitive));
        let node = present_child(&variable("__exception__", "tuple", "()", 5), false);
        assert!(matches!(node.role, ValueRole::Value(_)));
    }

    #[test]
    fn sizes_are_asked_for_in_one_tuple() {
        let e = sizes_expression(&["items", "d['a']"]).unwrap();
        assert!(e.starts_with("str((len(items) if hasattr(type(items), '__len__')"));
        assert!(e.contains("len(d['a']) if"));
        assert!(e.ends_with(", ))"));
        assert_eq!(sizes_expression(&[]), None);
        // A call is not made again for its size.
        assert_eq!(sizes_expression(&["items.pop()"]), None);
        let e = sizes_expression(&["f(x)", "items"]).unwrap();
        assert!(e.starts_with("str((None, len(items) if"), "{e}");
        assert_eq!(
            parse_sizes("'(1000, None)'", 2),
            vec![Some("1000".into()), None]
        );
        assert_eq!(
            parse_sizes("(1000, None)", 2),
            vec![Some("1000".into()), None]
        );
        assert_eq!(parse_sizes("(4,)", 1), vec![Some("4".into())]);
        assert_eq!(parse_sizes("oops", 2), vec![None, None]);
    }
}
