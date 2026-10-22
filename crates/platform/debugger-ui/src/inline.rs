//! Values of the selected frame's variables after the lines of its file
//! (the reference's inline debugger: `XValueNodeImpl
//! .updateInlineDebuggerData`, `XDebuggerInlayUtil.createLineEndInlay`,
//! `InlineDebugRenderer`): `name: value` after the line that declares
//! each variable of the frame (where the product resolves the name from
//! the execution point, [`debugger::DebugAdapterProvider::declarations`]),
//! in the order of the declarations on a line, italic in the scheme's
//! `DEBUGGER_INLINED_VALUES` (`..._EXECUTION_LINE` on the line the top
//! frame is suspended on). A value that differs from the one shown before
//! after the same line is drawn in `DEBUGGER_INLINED_VALUES_MODIFIED`,
//! only the changed items of a list or a dict (`LinePainter
//! .computeVariablePresentationWithChanges`). The values are shown at
//! every stop and frame change and taken away when the program runs on
//! (after the variables view's clear delay, so a step does not flicker).
//! Under the mouse a value is a link (`REFERENCE_HYPERLINK_COLOR`, the
//! editor's text colour on the execution line) with the drop-down
//! triangle after it; a click opens its tree in a popup under it
//! ([`crate::value_hint`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use debugger::values::{ValueKind, ValueRole, value_text};
use debugger::{Phase, Session, SessionId, Variable};
use editor::{AfterTextHover, Decoration, DecorationId, Editor, TextPiece};
use gpui::{App, Context, Entity, EntityId, Task};
use themes::TextRole;
use ui::icons::IconName;

use crate::session::execution_point::ExecutionPoint;
use crate::{DebuggerView, DebuggerViewEvent};

/// The decorations' group in an editor.
pub(crate) const GROUP: &str = "debugger-inline-values";
/// The separator after a name (`XDebuggerInlayUtil.INLINE_HINTS_DELIMETER`
/// and a space).
const SEPARATOR: &str = ": ";
/// How long the values stay after the program moved on (the variables
/// view's clear delay).
const CLEAR_DELAY: Duration = Duration::from_millis(100);
/// The items of a list or a dict compared one by one
/// (`VariableValue.ARRAYS_WRAPPERS`, `ARRAY_DELIMITER`).
const WRAPPERS: &[(&str, &str)] = &[("[", "]"), ("{", "}")];
const ITEM_DELIMITER: &str = ", ";

/// The stop and frame whose values the editors show.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    session: SessionId,
    generation: u64,
    frame: i64,
}

/// A value shown after a line before and now (`LinePainter.VariableValue`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct History {
    actual: String,
    old: Option<String>,
}

impl History {
    fn changed(&self) -> bool {
        self.old.as_ref().is_some_and(|old| *old != self.actual)
    }
}

/// The inline values of a window's editors.
#[derive(Default)]
pub(crate) struct InlineValues {
    /// Show values inline (`XDebuggerDataViewSettings.isShowValuesInline`).
    pub disabled: bool,
    shown: Option<Shown>,
    /// The file and its decorations, for the editors opened later, with
    /// the values they show.
    items: Option<(PathBuf, Vec<(usize, Decoration)>)>,
    values: Vec<InlineValue>,
    /// The decorations of the values in each editor, with the index of
    /// the value each shows.
    ids: HashMap<EntityId, Vec<(DecorationId, usize)>>,
    /// Values by file, name and line, for the changed ones.
    history: HashMap<(PathBuf, String, u32), History>,
    task: Option<Task<()>>,
}

/// A variable of the frame, with where it is declared.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InlineValue {
    pub name: String,
    pub value: String,
    /// From 1.
    pub line: u32,
    pub column: u32,
    pub variable: Variable,
    /// The size of a container.
    pub shape: Option<String>,
    pub kind: ValueKind,
    /// The scope the variable is in.
    pub parent: i64,
}

/// A variable of a frame as the variables view shows it.
struct FrameValue {
    name: String,
    variable: Variable,
    shape: Option<String>,
    kind: ValueKind,
    parent: i64,
}

impl DebuggerView {
    /// The selected session's stop and frame changed: the editors show its
    /// values, or none while it runs.
    pub(crate) fn update_inline_values(&mut self, cx: &mut Context<Self>) {
        let target = (!self.inline.disabled)
            .then(|| {
                let tab = self.tool_window.read(cx).selected_tab()?;
                let point = ExecutionPoint::of(&tab.snapshot)?;
                let shown = Shown {
                    session: tab.snapshot.id,
                    generation: tab.snapshot.generation,
                    frame: point.frame,
                };
                Some((tab.session.clone(), shown, point))
            })
            .flatten();
        let Some((session, shown, point)) = target else {
            if self.inline.shown.take().is_some() || self.inline.items.is_some() {
                let delay = !self.inline.disabled;
                self.inline.task = Some(cx.spawn(async move |this, cx| {
                    if delay {
                        cx.background_executor().timer(CLEAR_DELAY).await;
                    }
                    let _ = this.update(cx, |this, cx| {
                        if this.inline.shown.is_none() {
                            this.inline.items = None;
                            this.inline.values.clear();
                            this.show_inline_values_everywhere(cx);
                        }
                    });
                }));
            }
            return;
        };
        if self.inline.shown.as_ref() == Some(&shown) {
            return;
        }
        self.inline.shown = Some(shown.clone());
        let Some(provider) = self.debugger.as_ref().map(|d| d.provider().clone()) else {
            return;
        };
        let text = self
            .editor_of(&point.path, cx)
            .map(|e| e.read(cx).buffer().text());
        self.inline.task = Some(cx.spawn(async move |this, cx| {
            let Some(names) = frame_values(&session, &provider, shown.frame).await else {
                return;
            };
            let path = point.path.clone();
            let line = point.line;
            let positions = cx
                .background_executor()
                .spawn(async move {
                    let text = match text {
                        Some(text) => text,
                        None => std::fs::read_to_string(&path).ok()?,
                    };
                    let keys: Vec<String> = names.iter().map(|v| v.name.clone()).collect();
                    let positions = provider.declarations(&path, &text, line, &keys)?;
                    Some(
                        names
                            .into_iter()
                            .zip(positions)
                            .filter_map(|(v, pos)| {
                                let (line, column) = pos?;
                                Some(InlineValue {
                                    name: v.name,
                                    value: value_text(&v.variable.value),
                                    line,
                                    column,
                                    variable: v.variable,
                                    shape: v.shape,
                                    kind: v.kind,
                                    parent: v.parent,
                                })
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.inline.shown.as_ref() != Some(&shown) {
                    return;
                }
                let values = positions.unwrap_or_default();
                this.set_inline_values(&point, values, cx);
            });
        }));
    }

    /// Show Variable Values in Editor (`XDebugger.Inline`), on or off. The
    /// option is the application's: the window asks to store it
    /// ([`DebuggerViewEvent::ShowValuesInline`]) and every window follows
    /// the stored value ([`Self::set_inline_values_shown`]).
    pub(crate) fn toggle_inline_values(&mut self, on: Option<bool>, cx: &mut Context<Self>) {
        let on = on.unwrap_or(self.inline.disabled);
        if self.set_inline_values_shown(on, cx) {
            cx.emit(DebuggerViewEvent::ShowValuesInline(on));
        }
    }

    /// The stored option (`XDebuggerDataViewSettings.isShowValuesInline`):
    /// the editors show the values of the stop, or none. True when it
    /// changed.
    pub fn set_inline_values_shown(&mut self, on: bool, cx: &mut Context<Self>) -> bool {
        if on == !self.inline.disabled {
            return false;
        }
        self.inline.disabled = !on;
        self.update_inline_values(cx);
        cx.notify();
        true
    }

    /// Values are shown after the lines (the menu's check mark).
    pub fn inline_values_shown(&self) -> bool {
        !self.inline.disabled
    }

    /// The values of the frame: their decorations, remembered for the
    /// changed ones.
    fn set_inline_values(
        &mut self,
        point: &ExecutionPoint,
        mut values: Vec<InlineValue>,
        cx: &mut Context<Self>,
    ) {
        values.sort_by_key(|v| (v.line, v.column));
        let mut items = Vec::with_capacity(values.len());
        for v in &values {
            let key = (point.path.clone(), v.name.clone(), v.line);
            let history = self
                .inline
                .history
                .entry(key)
                .and_modify(|h| {
                    if h.actual != v.value {
                        h.old = Some(std::mem::replace(&mut h.actual, v.value.clone()));
                    }
                })
                .or_insert_with(|| History {
                    actual: v.value.clone(),
                    old: None,
                });
            let execution_line = point.top && v.line == point.line;
            let pieces = presentation(&v.name, history, execution_line);
            // Under the mouse: a link, on the execution line in the
            // execution point's colour (the scheme's text colour).
            let hover = AfterTextHover {
                role: (!execution_line).then_some(TextRole::ReferenceLink),
                icon: Some(IconName::CHEVRON_DOWN),
            };
            items.push((
                v.line as usize - 1,
                Decoration::default()
                    .after_text(pieces, true)
                    .after_text_hover(hover),
            ));
        }
        self.inline.items = Some((point.path.clone(), items));
        self.inline.values = values;
        self.show_inline_values_everywhere(cx);
    }

    fn show_inline_values_everywhere(&mut self, cx: &mut Context<Self>) {
        let editors: Vec<Entity<Editor>> =
            self.gutters.values().map(|g| g.editor.clone()).collect();
        for editor in editors {
            self.show_inline_values_in(&editor, cx);
        }
    }

    /// An editor shows the values when it shows their file.
    pub(crate) fn show_inline_values_in(&mut self, editor: &Entity<Editor>, cx: &mut App) {
        let items = self.inline.items.as_ref().and_then(|(path, items)| {
            let shows = editor.read(cx).path().is_some_and(|p| p == path);
            shows.then(|| items.clone())
        });
        let ids = editor.update(cx, |e, cx| match items {
            Some(items) => {
                let last = e.buffer().len_lines().saturating_sub(1);
                // A value after a line past the end of the text is not
                // shown; the others keep their index.
                let (indices, shown): (Vec<usize>, Vec<_>) = items
                    .into_iter()
                    .enumerate()
                    .filter(|(_, (line, _))| *line <= last)
                    .map(|(i, (line, d))| (i, (line..line + 1, d)))
                    .unzip();
                let ids = e.set_decorations(GROUP, shown, cx);
                ids.into_iter().zip(indices).collect()
            }
            None => {
                e.clear_decorations(GROUP, cx);
                Vec::new()
            }
        });
        if ids.is_empty() {
            self.inline.ids.remove(&editor.entity_id());
        } else {
            self.inline.ids.insert(editor.entity_id(), ids);
        }
    }

    /// The decoration of the value after line `line` (from 1) of an
    /// editor, the one of `name` or the first.
    pub(crate) fn inline_value_id(
        &self,
        editor: EntityId,
        line: u32,
        name: Option<&str>,
    ) -> Option<DecorationId> {
        let ids = self.inline.ids.get(&editor)?;
        ids.iter()
            .find(|(_, i)| {
                self.inline
                    .values
                    .get(*i)
                    .is_some_and(|v| v.line == line && name.is_none_or(|n| n == v.name))
            })
            .map(|(id, _)| *id)
    }

    /// The value a decoration of an editor shows.
    pub(crate) fn inline_value_of(
        &self,
        editor: EntityId,
        id: DecorationId,
    ) -> Option<&InlineValue> {
        let ids = self.inline.ids.get(&editor)?;
        let (_, index) = ids.iter().find(|(d, _)| *d == id)?;
        self.inline.values.get(*index)
    }

    /// The values shown, for `debug.state`.
    pub(crate) fn inline_values_json(&self) -> serde_json::Value {
        match &self.inline.items {
            Some((path, items)) => serde_json::json!({
                "path": path.to_string_lossy(),
                "values": items
                    .iter()
                    .map(|(line, d)| {
                        let text: String = d
                            .after_text
                            .as_ref()
                            .map(|a| a.pieces.iter().map(|p| p.text.as_ref()).collect())
                            .unwrap_or_default();
                        let changed = d.after_text.as_ref().is_some_and(|a| {
                            a.pieces
                                .iter()
                                .any(|p| p.role == TextRole::DebuggerInlinedValuesModified)
                        });
                        serde_json::json!({ "line": line + 1, "text": text, "changed": changed })
                    })
                    .collect::<Vec<_>>(),
            }),
            None => serde_json::Value::Null,
        }
    }
}

/// The variables of a frame, as the variables view shows them at its
/// root (the first scope's values for a product that shows only those,
/// else every scope's); `None` when the stop is over.
async fn frame_values(
    session: &Session,
    provider: &std::sync::Arc<dyn debugger::DebugAdapterProvider>,
    frame: i64,
) -> Option<Vec<FrameValue>> {
    let scopes = session.scopes(frame).await.ok()?;
    let scopes: Vec<_> = if provider.first_scope_only() {
        scopes.into_iter().take(1).collect()
    } else {
        scopes.into_iter().filter(|s| !s.expensive).collect()
    };
    let mut out: Vec<FrameValue> = Vec::new();
    for scope in scopes {
        let children = session.children(scope.reference).await.ok()?;
        for (i, variable) in children.items.iter().enumerate() {
            let node = provider.present_value(variable);
            if let ValueRole::Value(kind) = node.role
                && !out.iter().any(|v| v.name == node.name)
            {
                out.push(FrameValue {
                    name: node.name,
                    variable: variable.clone(),
                    shape: children.shape(i).map(str::to_owned),
                    kind,
                    parent: scope.reference,
                });
            }
        }
    }
    (session.phase() == Phase::Suspended).then_some(out)
}

/// `name: value` as the reference's renderer draws it: the name and the
/// separator in the line's colour, the value too unless it changed; a
/// changed list or dict marks the changed items only.
fn presentation(name: &str, history: &History, execution_line: bool) -> Vec<TextPiece> {
    let normal = if execution_line {
        TextRole::DebuggerInlinedValuesExecutionLine
    } else {
        TextRole::DebuggerInlinedValues
    };
    let mut pieces = vec![TextPiece::new(format!("{name}{SEPARATOR}"), normal).italic()];
    if !history.changed() {
        pieces.push(TextPiece::new(history.actual.clone(), normal).italic());
        return pieces;
    }
    let modified = TextRole::DebuggerInlinedValuesModified;
    let base = TextRole::DebuggerInlinedValues;
    let actual = &history.actual;
    let old = history.old.as_deref().unwrap_or_default();
    match (wrapper(actual), wrapper(old)) {
        (Some((open, close)), Some(_)) => {
            let actual_items = items(actual, open, close);
            let old_items = items(old, open, close);
            pieces.push(TextPiece::new(open, base).italic());
            let count = actual_items.len();
            for (i, item) in actual_items.into_iter().enumerate() {
                let same = old_items.get(i).is_some_and(|o| *o == item);
                pieces.push(
                    TextPiece::new(item.to_owned(), if same { base } else { modified }).italic(),
                );
                if i + 1 != count {
                    pieces.push(TextPiece::new(ITEM_DELIMITER, base).italic());
                }
            }
            pieces.push(TextPiece::new(close, base).italic());
        }
        _ => pieces.push(TextPiece::new(actual.clone(), modified).italic()),
    }
    pieces
}

fn wrapper(text: &str) -> Option<(&'static str, &'static str)> {
    WRAPPERS
        .iter()
        .copied()
        .find(|(open, close)| text.len() >= 2 && text.starts_with(open) && text.ends_with(close))
}

/// The items of `[a, b]` split at the delimiter, as the reference does
/// (`StringUtil.split`, which drops empty parts).
fn items<'a>(text: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    text[open.len()..text.len() - close.len()]
        .split(ITEM_DELIMITER)
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(pieces: &[TextPiece]) -> Vec<(String, TextRole)> {
        pieces
            .iter()
            .map(|p| (p.text.to_string(), p.role))
            .collect()
    }

    #[test]
    fn an_unchanged_value_is_one_colour() {
        let h = History {
            actual: "0".into(),
            old: None,
        };
        assert_eq!(
            texts(&presentation("n", &h, false)),
            vec![
                ("n: ".into(), TextRole::DebuggerInlinedValues),
                ("0".into(), TextRole::DebuggerInlinedValues)
            ]
        );
        assert_eq!(
            presentation("n", &h, true)[0].role,
            TextRole::DebuggerInlinedValuesExecutionLine
        );
    }

    #[test]
    fn a_changed_value_and_the_changed_items_of_a_list_are_marked() {
        let modified = TextRole::DebuggerInlinedValuesModified;
        let normal = TextRole::DebuggerInlinedValues;
        let h = History {
            actual: "2".into(),
            old: Some("1".into()),
        };
        assert_eq!(
            texts(&presentation("i", &h, false))[1],
            ("2".into(), modified)
        );
        let h = History {
            actual: "[1, 5, 3]".into(),
            old: Some("[1, 2]".into()),
        };
        assert_eq!(
            texts(&presentation("items", &h, false)),
            vec![
                ("items: ".into(), normal),
                ("[".into(), normal),
                ("1".into(), normal),
                (", ".into(), normal),
                ("5".into(), modified),
                (", ".into(), normal),
                ("3".into(), modified),
                ("]".into(), normal),
            ]
        );
    }
}
