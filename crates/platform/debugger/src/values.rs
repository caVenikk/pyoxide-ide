//! How the variables view shows an adapter's values (the reference's
//! `XValue` presentation of a language, `PyDebugValue` for Python): what
//! the product says of each child an adapter reports ([`ValueNode`]), and
//! the text of a value as the tree draws it
//! (`name = {type: size} value`, `XValueNodeImpl.buildText`).

use dap_client::Variable;

/// What kind of value a node shows; its icon (`AllIcons.Debugger`
/// `Db_primitive`, `Db_array`, `Value`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// A value without children.
    Primitive,
    /// A sequence (Python's lists and tuples).
    Array,
    /// Any other value with children.
    Object,
}

impl ValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueKind::Primitive => "primitive",
            ValueKind::Array => "array",
            ValueKind::Object => "object",
        }
    }
}

/// The icon of a group of values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupIcon {
    /// Special variables (`AllIcons.Debugger.SpecialVar`).
    Special,
    /// Return values of the calls just made
    /// (`AllIcons.Debugger.WatchLastReturnValue`).
    ReturnValues,
    /// Protected attributes (`AllIcons.Nodes.C_protected`, a key).
    Protected,
    /// Any other group (a plain group here).
    Plain,
}

impl GroupIcon {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupIcon::Special => "special",
            GroupIcon::ReturnValues => "return-values",
            GroupIcon::Protected => "protected",
            GroupIcon::Plain => "plain",
        }
    }
}

/// What a child the adapter reports is in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueRole {
    /// A value.
    Value(ValueKind),
    /// A group of other values of the parent (its children are the
    /// group's), shown after the values under `title`.
    Group { title: String, icon: GroupIcon },
    /// A value the parent shows in its group `title` (among the group's
    /// children, before those of the groups the adapter reports under the
    /// same title) instead of among its own, as the reference moves the
    /// protected attributes of a value.
    Grouped {
        kind: ValueKind,
        title: String,
        icon: GroupIcon,
    },
    /// A value a call returned: shown in a group of the parent titled
    /// `group`, after the values.
    ReturnValue { group: String },
    /// More children of the parent, loaded when asked for; `remaining`
    /// when the adapter tells how many.
    More { remaining: Option<usize> },
}

/// A child of a scope or a value as the variables view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueNode {
    /// The name shown.
    pub name: String,
    pub role: ValueRole,
}

impl ValueNode {
    /// A value as the adapter reports it: its own name, primitive without
    /// children, an object with them.
    pub fn plain(variable: &Variable) -> Self {
        Self {
            name: variable.name.clone(),
            role: ValueRole::Value(if variable.reference > 0 {
                ValueKind::Object
            } else {
                ValueKind::Primitive
            }),
        }
    }
}

/// The longest value text a row shows (`PyDebugValue.MAX_VALUE`); the rest
/// is cut.
pub const MAX_VALUE: usize = 256;

/// The type part of a row: `list: 4`, `int`, or nothing.
pub fn type_text(type_name: Option<&str>, shape: Option<&str>) -> Option<String> {
    let type_name = type_name.filter(|t| !t.is_empty())?;
    Some(match shape {
        Some(shape) => format!("{type_name}: {shape}"),
        None => type_name.to_owned(),
    })
}

/// A value's text as a row shows it: at most [`MAX_VALUE`] characters,
/// control characters as their escapes (`XValuePresentationUtil
/// .renderValue`).
pub fn value_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(MAX_VALUE + 8));
    for ch in value.chars().take(MAX_VALUE) {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{7}' => out.push_str("\\a"),
            '\u{b}' => out.push_str("\\v"),
            c => out.push(c),
        }
    }
    out
}

/// A row's whole text, as Copy and the tests read it:
/// `items = {list: 4} [1, 2, 3, 4]`.
pub fn row_text(name: &str, type_name: Option<&str>, shape: Option<&str>, value: &str) -> String {
    let mut out = String::from(name);
    out.push_str(" = ");
    if let Some(t) = type_text(type_name, shape) {
        out.push('{');
        out.push_str(&t);
        out.push_str("} ");
    }
    out.push_str(&value_text(value));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_show_the_type_with_its_size_and_the_value() {
        assert_eq!(
            row_text("items", Some("list"), Some("4"), "[1, 2, 3, 4]"),
            "items = {list: 4} [1, 2, 3, 4]"
        );
        assert_eq!(row_text("value", Some("int"), None, "1"), "value = {int} 1");
        assert_eq!(row_text("x", None, None, "1"), "x = 1");
        assert_eq!(row_text("x", Some(""), None, "1"), "x = 1");
    }

    #[test]
    fn values_are_cut_and_escaped() {
        assert_eq!(value_text("'a\nb\t'"), "'a\\nb\\t'");
        let long = "x".repeat(1000);
        assert_eq!(value_text(&long).len(), MAX_VALUE);
    }
}
