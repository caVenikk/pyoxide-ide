//! The colours of the tree's rows: a data source's own (`gui.color`) and
//! the objects' (`gui.object_colors`, by the node's path), with Apply to
//! Children. The tree paints a coloured row's background across its whole
//! width; the colour is kept with the data source's settings.

use db_core::GuiOptions;
use db_model::NodeRef;
use gpui::{Hsla, hsla, rgb};

use crate::dialog::{COLORS, color_id};

/// An object left uncoloured below a coloured parent.
pub(crate) const NO_COLOR: &str = "none";
/// The menu's Auto: no colour of the node's own, what its parents give.
pub(crate) const AUTO: &str = "auto";

/// Where a row's colour is kept in a data source's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The data source's row (`gui.color`).
    Source,
    /// The database row of a PostgreSQL data source (`@`).
    Database,
    /// A schema or an object, by its path (`schema/table/column`).
    Node(String),
    /// A group under a node: no colour of its own, takes what the node's
    /// children get.
    Group(String),
}

/// The path of a node: the names from its schema down joined with `/`.
pub(crate) fn node_path(node: NodeRef<'_>) -> String {
    let mut names = vec![node.name()];
    let mut parent = node.parent();
    while let Some(p) = parent {
        names.push(p.name());
        parent = p.parent();
    }
    names.reverse();
    names.join("/")
}

/// The keys, nearest first, of what colours the rows below `path` and
/// above it (`path` is a node's path).
fn parents_of(path: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = path;
    while let Some((parent, _)) = rest.rsplit_once('/') {
        keys.push(format!("{parent}/*"));
        rest = parent;
    }
    keys.push("@/*".to_owned());
    keys.push("*".to_owned());
    keys
}

impl Slot {
    /// The key of the row's own colour (the data source's is `gui.color`).
    fn own_key(&self) -> Option<String> {
        match self {
            Slot::Source | Slot::Group(_) => None,
            Slot::Database => Some("@".to_owned()),
            Slot::Node(path) => Some(path.clone()),
        }
    }

    /// The key of the colour of what is below the row.
    fn children_key(&self) -> Option<String> {
        match self {
            Slot::Source => Some("*".to_owned()),
            Slot::Database => Some("@/*".to_owned()),
            Slot::Node(path) => Some(format!("{path}/*")),
            Slot::Group(_) => None,
        }
    }

    /// The keys that colour the row when it has no colour of its own.
    fn inherited_keys(&self) -> Vec<String> {
        match self {
            Slot::Source => Vec::new(),
            Slot::Database => vec!["*".to_owned()],
            Slot::Node(path) => parents_of(path),
            Slot::Group(path) => {
                let mut keys = vec![format!("{path}/*")];
                keys.extend(parents_of(path));
                keys
            }
        }
    }
}

fn inherited<'a>(gui: &'a GuiOptions, slot: &Slot) -> Option<&'a str> {
    slot.inherited_keys()
        .iter()
        .find_map(|k| gui.object_colors.get(k))
        .map(String::as_str)
}

/// What the row has of its own: `none` is a colour left off on purpose.
pub(crate) fn own<'a>(gui: &'a GuiOptions, slot: &Slot) -> Option<&'a str> {
    match slot {
        Slot::Source => gui.color.as_deref(),
        _ => slot
            .own_key()
            .and_then(|k| gui.object_colors.get(&k))
            .map(String::as_str),
    }
}

/// The colour a row is painted with (a name of [`COLORS`]), if any.
pub(crate) fn resolve(gui: &GuiOptions, slot: &Slot) -> Option<&'static str> {
    let name = own(gui, slot).or_else(|| inherited(gui, slot))?;
    let name = color_id(name);
    COLORS
        .iter()
        .map(|(id, _)| *id)
        .find(|id| *id == name && *id != NO_COLOR)
}

/// The background of a coloured row: the file colours of the themes, muted
/// on a dark theme (yellow as measured in the reference), pale on a light
/// one.
pub(crate) fn tint(name: &str, dark: bool) -> Hsla {
    let hue = match color_id(name) {
        "blue" => 215.0,
        "teal" => 175.0,
        "green" => 130.0,
        "yellow" => 37.0,
        "orange" => 18.0,
        "red" => 355.0,
        "purple" => 280.0,
        _ => 220.0,
    };
    let gray = color_id(name) == "gray";
    match (dark, color_id(name)) {
        (true, "yellow") => rgb(0x4c4131).into(),
        (true, _) => hsla(hue / 360.0, if gray { 0.05 } else { 0.22 }, 0.245, 1.0),
        (false, _) => hsla(hue / 360.0, if gray { 0.05 } else { 0.7 }, 0.9, 1.0),
    }
}

/// Sets the row's colour: a name of [`COLORS`], `auto` for none of its own
/// or `none` for no colour (also below a coloured parent).
pub(crate) fn set(gui: &mut GuiOptions, slot: &Slot, color: &str) {
    let named = !matches!(color, AUTO | NO_COLOR);
    let Some(key) = slot.own_key() else {
        if matches!(slot, Slot::Source) {
            gui.color = named.then(|| color.to_owned());
        }
        return;
    };
    let below_colour = inherited(gui, slot).is_some();
    match color {
        AUTO => {
            gui.object_colors.remove(&key);
        }
        NO_COLOR if !below_colour => {
            gui.object_colors.remove(&key);
        }
        _ => {
            gui.object_colors.insert(key, color.to_owned());
        }
    }
}

/// Whether Apply to Children has a colour to apply.
pub(crate) fn can_apply(gui: &GuiOptions, slot: &Slot) -> bool {
    slot.children_key().is_some() && resolve(gui, slot).is_some()
}

/// Gives everything below the row the row's colour, over the colours its
/// own nodes have.
pub(crate) fn apply_to_children(gui: &mut GuiOptions, slot: &Slot) {
    let (Some(colour), Some(key)) = (resolve(gui, slot), slot.children_key()) else {
        return;
    };
    match slot {
        Slot::Source => gui.object_colors.clear(),
        Slot::Database => gui.object_colors.retain(|k, _| k == "@"),
        Slot::Node(path) => {
            let below = format!("{path}/");
            gui.object_colors.retain(|k, _| !k.starts_with(&below));
        }
        Slot::Group(_) => return,
    }
    gui.object_colors.insert(key, colour.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str) -> Slot {
        Slot::Node(path.to_owned())
    }

    #[test]
    fn a_node_has_its_own_colour() {
        let mut gui = GuiOptions::default();
        set(&mut gui, &node("demo/events"), "yellow");
        assert_eq!(resolve(&gui, &node("demo/events")), Some("yellow"));
        assert_eq!(resolve(&gui, &node("demo/orders")), None);
        assert_eq!(resolve(&gui, &Slot::Source), None);
        assert_eq!(own(&gui, &node("demo/events")), Some("yellow"));
    }

    #[test]
    fn the_data_source_keeps_its_colour_apart() {
        let mut gui = GuiOptions::default();
        set(&mut gui, &Slot::Source, "blue");
        assert_eq!(gui.color.as_deref(), Some("blue"));
        assert!(gui.object_colors.is_empty());
        set(&mut gui, &Slot::Source, "none");
        assert_eq!(gui.color, None);
    }

    #[test]
    fn auto_removes_the_colour_and_none_keeps_a_colour_off_below_a_parent() {
        let mut gui = GuiOptions::default();
        set(&mut gui, &node("demo/events"), "red");
        set(&mut gui, &node("demo/events"), AUTO);
        assert!(gui.object_colors.is_empty());
        // Nothing to switch off without a coloured parent.
        set(&mut gui, &node("demo/events"), NO_COLOR);
        assert!(gui.object_colors.is_empty());

        set(&mut gui, &node("demo"), "green");
        apply_to_children(&mut gui, &node("demo"));
        assert_eq!(resolve(&gui, &node("demo/events")), Some("green"));
        set(&mut gui, &node("demo/events"), NO_COLOR);
        assert_eq!(own(&gui, &node("demo/events")), Some(NO_COLOR));
        assert_eq!(resolve(&gui, &node("demo/events")), None);
        assert_eq!(resolve(&gui, &node("demo/orders")), Some("green"));
        set(&mut gui, &node("demo/events"), AUTO);
        assert_eq!(resolve(&gui, &node("demo/events")), Some("green"));
    }

    #[test]
    fn apply_to_children_colours_what_is_below_over_their_own() {
        let mut gui = GuiOptions::default();
        set(&mut gui, &node("demo"), "blue");
        set(&mut gui, &node("demo/events"), "red");
        set(&mut gui, &node("other/events"), "red");
        apply_to_children(&mut gui, &node("demo"));
        assert_eq!(resolve(&gui, &node("demo/events")), Some("blue"));
        assert_eq!(resolve(&gui, &node("demo/events/id")), Some("blue"));
        assert_eq!(resolve(&gui, &Slot::Group("demo".into())), Some("blue"));
        assert_eq!(resolve(&gui, &node("other/events")), Some("red"));
        // The node itself keeps its own.
        assert_eq!(resolve(&gui, &node("demo")), Some("blue"));
        assert_eq!(resolve(&gui, &node("other")), None);
    }

    #[test]
    fn a_nearer_parent_wins() {
        let mut gui = GuiOptions::default();
        set(&mut gui, &Slot::Source, "gray");
        apply_to_children(&mut gui, &Slot::Source);
        set(&mut gui, &node("demo"), "teal");
        apply_to_children(&mut gui, &node("demo"));
        assert_eq!(resolve(&gui, &node("demo/events")), Some("teal"));
        assert_eq!(resolve(&gui, &node("other")), Some("gray"));
        assert_eq!(resolve(&gui, &Slot::Database), Some("gray"));
        // Source's apply replaces everything below.
        apply_to_children(&mut gui, &Slot::Source);
        assert_eq!(resolve(&gui, &node("demo/events")), Some("gray"));
        assert_eq!(gui.object_colors.len(), 1);
    }

    #[test]
    fn apply_needs_a_colour_and_something_below() {
        let mut gui = GuiOptions::default();
        assert!(!can_apply(&gui, &node("demo")));
        set(&mut gui, &node("demo"), "blue");
        assert!(can_apply(&gui, &node("demo")));
        assert!(!can_apply(&gui, &Slot::Group("demo".into())));
        apply_to_children(&mut gui, &Slot::Group("demo".into()));
        assert_eq!(gui.object_colors.len(), 1);
    }

    #[test]
    fn old_names_are_shown_as_the_menus() {
        let with = |name: &str| GuiOptions {
            color: Some(name.into()),
            ..GuiOptions::default()
        };
        assert_eq!(resolve(&with("rose"), &Slot::Source), Some("red"));
        assert_eq!(resolve(&with("unknown"), &Slot::Source), None);
    }
}
