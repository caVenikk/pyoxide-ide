//! Opening a table's data from the object tree: a double click or Enter on
//! a table, view or materialized view, F4 (Cmd+Down on macOS) on the
//! selected one, or `db.grid.open`. The grid gets the data source's handle
//! of the tree (so it shares its data connections) and what the metadata
//! model knows about the columns (key marks, types).

use db_core::schema::ConstraintKind;
use db_model::{Group, Model, NodeKind, NodeRef};
use db_session::TableRef;
use gpui::{AnyWindowHandle, App, Context, Entity};
use ui::notification::{Notification, NotificationKind};

use super::{Explorer, Explorers};
use crate::grid::data::{ColumnMeta, KeyMark};
use crate::grid::{self, GridTarget};
use crate::tree::{NodeKey, RowKey};

/// What the metadata says about a table's columns.
pub(crate) fn column_meta(table: NodeRef<'_>) -> Vec<ColumnMeta> {
    let constraints: Vec<_> = table
        .group(Group::Keys)
        .nodes()
        .filter_map(|k| k.constraint())
        .collect();
    table
        .children()
        .filter_map(|n| n.column())
        .map(|c| {
            let in_kind = |kind| {
                constraints
                    .iter()
                    .any(|k| k.kind == kind && k.columns.contains(&c.name))
            };
            let key = if in_kind(ConstraintKind::PrimaryKey) {
                KeyMark::Primary
            } else if in_kind(ConstraintKind::ForeignKey) {
                KeyMark::Foreign
            } else {
                KeyMark::None
            };
            ColumnMeta {
                name: c.name.clone(),
                data_type: c.data_type.clone(),
                key,
            }
        })
        .collect()
}

fn relation<'a>(model: &'a Model, key: &RowKey) -> Option<NodeRef<'a>> {
    let NodeKey::Node(id) = key.node else {
        return None;
    };
    model.node(id).filter(|n| n.kind().is_relation())
}

impl Explorer {
    /// The window this tool window is in.
    fn window_handle(&self, cx: &Context<Self>) -> Option<AnyWindowHandle> {
        let me = cx.entity_id();
        cx.try_global::<Explorers>()?
            .0
            .iter()
            .find(|(_, e)| e.entity_id() == me)
            .map(|(w, _)| *w)
    }

    /// What the grid of the tree row `key` opens, when it is a table.
    fn grid_target(&self, key: &RowKey) -> Option<GridTarget> {
        let live = self.tree.lives.get(&*key.source)?;
        let model = live.shown_model()?;
        let node = relation(model, key)?;
        Some(GridTarget {
            source: key.source.to_string(),
            backend: live.profile.backend,
            handle: live.handle.clone(),
            table: TableRef::new(Some(node.schema().name()), node.name()),
            columns: column_meta(node),
        })
    }

    /// Opens the grid of the tree row `key` when it is a table, view or
    /// materialized view; `false` for any other row.
    pub(super) fn open_row_table(&mut self, key: &RowKey, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.grid_target(key) else {
            return false;
        };
        self.open_grid(target, cx);
        true
    }

    fn open_grid(&mut self, target: GridTarget, cx: &mut Context<Self>) {
        let (Some(window), Some(workbench)) = (self.window_handle(cx), self.workbench.upgrade())
        else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                grid::open(&workbench, target, window, cx);
            });
        });
    }

    /// Opens the selected row's table (F4).
    pub(super) fn open_selected_table(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.tree.selected.clone() {
            self.open_row_table(&key, cx);
        }
    }

    /// The grid target of a table by name, from the loaded metadata.
    pub(crate) fn named_target(
        &self,
        source: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Option<GridTarget> {
        self.tree.lives.get(source).and_then(|live| {
            let model = live.shown_model()?;
            let node = model
                .schemas()
                .filter(|s| schema.is_none_or(|n| s.name() == n))
                .find_map(|s| {
                    s.find(
                        table,
                        db_model::NameMatch::Exact,
                        &[NodeKind::Table, NodeKind::View, NodeKind::MaterializedView],
                    )
                })?;
            Some(GridTarget {
                source: source.to_owned(),
                backend: live.profile.backend,
                handle: live.handle.clone(),
                table: TableRef::new(Some(node.schema().name()), node.name()),
                columns: column_meta(node),
            })
        })
    }

    /// The table of the selected row.
    pub(crate) fn selected_target(&self) -> Option<GridTarget> {
        self.grid_target(self.tree.selected.as_ref()?)
    }

    /// Opens a table by name (`db.grid.open`); the data source's model is
    /// read, never the database.
    pub(super) fn open_named_table(
        &mut self,
        source: &str,
        schema: Option<&str>,
        table: &str,
        cx: &mut Context<Self>,
    ) {
        let found = self.named_target(source, schema, table);
        match found {
            Some(target) => self.open_grid(target, cx),
            None => self.notify(
                Notification::new(
                    NotificationKind::Warning,
                    source.to_owned(),
                    format!("No table {table} in the loaded metadata of {source}"),
                ),
                cx,
            ),
        }
    }
}

/// The explorer of the active window (else the newest).
fn target_explorer(cx: &App) -> Option<Entity<Explorer>> {
    super::target(cx).map(|(_, e)| e)
}

/// `db.grid.open`.
pub(crate) fn open_table_command(
    source: String,
    schema: Option<String>,
    table: String,
    cx: &mut App,
) {
    let Some(explorer) = target_explorer(cx) else {
        return;
    };
    cx.defer(move |cx| {
        explorer.update(cx, |e, cx| {
            e.open_named_table(&source, schema.as_deref(), &table, cx)
        });
    });
}

/// `db.explorer.open-table`.
pub(crate) fn open_selected_command(cx: &mut App) {
    let Some(explorer) = target_explorer(cx) else {
        return;
    };
    cx.defer(move |cx| explorer.update(cx, |e, cx| e.open_selected_table(cx)));
}

/// Runs `f` with the grid target of the table `table` of `source` (or of
/// the table selected in the tool window when `table` is `None`) and the
/// window of the tool window; a table the metadata does not know gets a
/// warning.
pub(crate) fn with_table(
    source: Option<String>,
    schema: Option<String>,
    table: Option<String>,
    f: impl FnOnce(GridTarget, AnyWindowHandle, &mut App) + 'static,
    cx: &mut App,
) {
    let Some(explorer) = target_explorer(cx) else {
        return;
    };
    cx.defer(move |cx| {
        let found = explorer.update(cx, |e, cx| {
            let window = e.window_handle(cx)?;
            let target = match (&source, &table) {
                (Some(source), Some(table)) => {
                    let target = e.named_target(source, schema.as_deref(), table);
                    if target.is_none() {
                        e.notify(
                            Notification::new(
                                NotificationKind::Warning,
                                source.clone(),
                                format!("No table {table} in the loaded metadata of {source}"),
                            ),
                            cx,
                        );
                    }
                    target
                }
                _ => e.selected_target(),
            }?;
            Some((target, window))
        });
        if let Some((target, window)) = found {
            f(target, window, cx);
        }
    });
}
