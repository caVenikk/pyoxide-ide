//! Go to Declaration from SQL text to a database object (the `db` scheme of
//! object targets, ADR 0022): the Database tool window shows, the data
//! source opens, the nodes down to the object expand and the object is
//! selected.

use std::rc::Rc;
use std::sync::Arc;

use db_model::NodeId;
use gpui::{App, Context};
use sql_intel::{DB_SCHEME, parse_object_id};

use super::{Explorer, explorer_of, tree_view::database_name};
use crate::TOOL_WINDOW;
use crate::tree::{NodeKey, RowKey, TreeGroup};

/// Registers the handler of the `db` scheme.
pub(super) fn init(cx: &mut App) {
    workbench::object_targets::register(
        cx,
        DB_SCHEME,
        Rc::new(|target, window, cx| {
            let Some((id, source)) = parse_object_id(&target.id) else {
                return;
            };
            let main = ui::dialog_window::parent_of(window.window_handle(), cx)
                .unwrap_or(window.window_handle());
            // The request comes out of the workbench's own update.
            window.defer(cx, move |window, cx| {
                let Some(explorer) = explorer_of(main, cx) else {
                    return;
                };
                if let Some(workbench) = explorer.read(cx).workbench.upgrade() {
                    workbench.update(cx, |wb, cx| {
                        wb.activate_tool_window(TOOL_WINDOW, window, cx)
                    });
                }
                explorer.update(cx, |e, cx| {
                    e.reveal(&source, id, cx);
                });
            });
        }),
    );
}

impl Explorer {
    /// Selects the object Go to Declaration asked for before the model was
    /// there.
    pub(super) fn reveal_pending(&mut self, cx: &mut Context<Self>) {
        let Some((source, id)) = self.tree.pending_reveal.clone() else {
            return;
        };
        if self.model_of(&source).is_some() {
            self.tree.pending_reveal = None;
            self.reveal(&source, id, cx);
        }
    }

    /// Opens the nodes above `id` of the data source `source` and selects
    /// it; false when the data source or the node is unknown. While the
    /// data source has no model, its row opens and shows the progress.
    pub(super) fn reveal(&mut self, source: &str, id: NodeId, cx: &mut Context<Self>) -> bool {
        let Some(profile) = self
            .sources()
            .iter()
            .find(|s| s.name == source)
            .map(|s| s.profile.clone())
        else {
            return false;
        };
        if !self.tree.lives.contains_key(source) {
            self.open_live(source, cx);
        }
        let name: Arc<str> = Arc::from(source);
        let key = |node| RowKey {
            source: name.clone(),
            node,
        };
        let mut open = vec![RowKey::source(&name)];
        if database_name(&profile).is_some() {
            open.push(key(NodeKey::Database));
        }
        let target = match self.model_of(source) {
            None => {
                self.tree.pending_reveal = Some((source.to_owned(), id));
                None
            }
            Some(model) => {
                let Some(node) = model.node(id) else {
                    return false;
                };
                let mut chain = vec![node];
                while let Some(parent) = chain[chain.len() - 1].parent() {
                    chain.push(parent);
                }
                chain.reverse();
                if chain[0].kind() != db_model::NodeKind::Schema {
                    chain.insert(0, node.schema().root());
                }
                for pair in chain.windows(2) {
                    let (parent, child) = (pair[0], pair[1]);
                    let Some(group) = TreeGroup::of(parent.kind())
                        .iter()
                        .copied()
                        .find(|g| g.nodes(parent).iter().any(|n| n.id() == child.id()))
                    else {
                        return false;
                    };
                    open.push(key(NodeKey::Node(parent.id())));
                    open.push(key(NodeKey::Group(parent.id(), group)));
                }
                Some(key(NodeKey::Node(id)))
            }
        };
        self.tree.expanded.extend(open);
        self.rebuild_rows();
        if let Some(target) = target {
            self.select_row(target.clone(), cx);
            if let Some(ix) = self.tree.rows.iter().position(|r| r.key == target) {
                self.scroll.scroll_to_item(ix, gpui::ScrollStrategy::Center);
            }
        }
        cx.notify();
        true
    }
}
