//! Show Context Actions (Alt+Enter) and the bulb, as the reference's
//! intention list shows the actions of the services
//! (`LspIntentionActionService`, `IntentionHintComponent`): the quick
//! fixes first, in the services' order, with a bulb (red for the fixes of
//! an error), then a separator and the other actions without an icon,
//! sorted by name; sub-actions open next to their row (Right, or the `⋮`
//! button of the selected row); Enter applies the selected action as one
//! undo step. A bulb at the start of the caret line shows that actions are
//! there; a click on it opens the list.

use std::time::Duration;

use code_intel::{ActionTrigger, CodeAction, CodeActionContext, Severity};
use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, Context, Entity, EntityId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Styled, Task, WeakEntity, Window, anchored, deferred, div, point, px,
};
use serde_json::{Value as Json, json};
use ui::pointer::PointerExt as _;
use ui::{Icon, IconName};

use crate::doc;
use crate::list_popup::{ListItem, ListOptions, ListRow};
use crate::navigation::ListTargets;
use crate::view::CodeIntelView;

/// How long the caret rests before the actions at it are asked for (the
/// bulb), the reference's highlighting delay.
pub const BULB_DELAY: Duration = Duration::from_millis(300);
const BULB_SIZE: f32 = 16.0;

/// The bulb at the start of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bulb {
    editor: EntityId,
    line: usize,
    icon: IconName,
}

/// The intention actions state of a window.
pub(crate) struct IntentionState {
    owner: WeakEntity<CodeIntelView>,
    request: Option<Task<()>>,
    /// The actions of the list shown, in the order of its rows, the row
    /// of each, and the version of the text their edits are in.
    shown: Vec<CodeAction>,
    rows: Vec<Option<usize>>,
    version: u64,
    bulb: Option<Bulb>,
    bulb_request: Option<Task<()>>,
}

impl IntentionState {
    pub(crate) fn new(owner: WeakEntity<CodeIntelView>) -> Self {
        Self {
            owner,
            request: None,
            shown: Vec::new(),
            rows: Vec::new(),
            version: 0,
            bulb: None,
            bulb_request: None,
        }
    }
}

/// The actions in the order of the list: quick fixes first as the
/// services sent them, then the others by name.
pub fn order(actions: Vec<CodeAction>) -> Vec<CodeAction> {
    let (mut fixes, mut others): (Vec<_>, Vec<_>) =
        actions.into_iter().partition(|a| a.kind.is_quick_fix());
    others.sort_by_cached_key(|a| a.title.to_lowercase());
    fixes.append(&mut others);
    fixes
}

/// The bulb of a quick fix: red for a fix of an error, else yellow.
fn fix_icon(action: &CodeAction) -> IconName {
    if action
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        IconName::QUICKFIX_BULB
    } else {
        IconName::INTENTION_BULB
    }
}

/// The rows of ordered actions and, for each row, its action (`None` for
/// the separator between the quick fixes and the others).
pub fn rows(actions: &[CodeAction]) -> (Vec<ListItem>, Vec<Option<usize>>) {
    let mut items = Vec::new();
    let mut map = Vec::new();
    for (i, action) in actions.iter().enumerate() {
        let fix = action.kind.is_quick_fix();
        if i > 0 && !fix && actions[i - 1].kind.is_quick_fix() {
            items.push(ListItem::Separator);
            map.push(None);
        }
        items.push(ListItem::Row(ListRow {
            icon: fix.then(|| fix_icon(action)),
            children: action
                .children
                .iter()
                .map(|c| ListItem::Row(ListRow::new(c.title.clone())))
                .collect(),
            ..ListRow::new(action.title.clone())
        }));
        map.push(Some(i));
    }
    (items, map)
}

/// The bulb of a set of actions: none without actions, red when a quick
/// fix fixes an error.
fn bulb_icon(actions: &[CodeAction]) -> Option<IconName> {
    if actions.is_empty() {
        return None;
    }
    let error_fix = actions
        .iter()
        .any(|a| a.kind.is_quick_fix() && fix_icon(a) == IconName::QUICKFIX_BULB);
    Some(if error_fix {
        IconName::QUICKFIX_BULB
    } else {
        IconName::INTENTION_BULB
    })
}

impl CodeIntelView {
    /// The context of a code action request at the caret of an editor: the
    /// range (the problem at the caret, else the selection) and the
    /// problems there.
    fn actions_request(
        &self,
        editor: &Entity<Editor>,
        trigger: ActionTrigger,
        cx: &App,
    ) -> Option<(code_intel::Intel<Vec<CodeAction>>, u64, usize)> {
        let query = self.caret_query(editor, cx)?;
        let services = self.services()?.clone();
        let document = self.document_of(editor)?;
        let e = editor.read(cx);
        let (_, mark) = doc::problems_at(e.diagnostics(), query.offset, true);
        let range = mark.unwrap_or_else(|| e.selections().primary().range());
        let points = document.versions.point(range.start)..document.versions.point(range.end);
        let context = CodeActionContext {
            only: None,
            diagnostics: self.diagnostics_in(&query.doc, range, cx),
            trigger,
        };
        let request = services.code_actions(&query.doc, points, context);
        Some((request, query.version, query.offset))
    }

    /// Show Context Actions at the caret of an editor.
    pub(crate) fn show_intentions_at_caret(
        &mut self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        let Some((request, version, caret)) =
            self.actions_request(editor, ActionTrigger::Invoked, cx)
        else {
            return;
        };
        let editor = editor.clone();
        self.intentions.request = Some(cx.spawn(async move |this, cx| {
            let actions = order(request.await.unwrap_or_default());
            let _ = this.update(cx, |v, cx| {
                v.intentions.request = None;
                if actions.is_empty() {
                    return;
                }
                let (items, map) = rows(&actions);
                v.intentions.shown = actions;
                v.intentions.rows = map;
                v.intentions.version = version;
                let options = ListOptions {
                    more_button: true,
                    name: "intentions",
                    ..ListOptions::default()
                };
                v.open_list(&editor, caret, items, options, ListTargets::Intentions, cx);
            });
        }));
    }

    /// A row of the intention list was chosen: its action, or one of its
    /// sub-actions, is applied.
    pub(crate) fn intention_chosen(
        &mut self,
        editor: &Entity<Editor>,
        path: &[usize],
        cx: &mut Context<Self>,
    ) {
        let Some(action) = path
            .first()
            .and_then(|&row| self.intentions.rows.get(row).copied().flatten())
            .and_then(|i| self.intentions.shown.get(i))
        else {
            return;
        };
        let action = match path.get(1) {
            Some(&child) => action.children.get(child).cloned(),
            None => Some(action.clone()),
        };
        if let Some(action) = action {
            let version = self.intentions.version;
            self.apply_code_action(editor, version, action, cx);
        }
    }

    /// Events of a document's editor: the bulb follows the caret.
    pub(crate) fn intention_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            event,
            EditorEvent::SelectionsChanged | EditorEvent::Edited(_) | EditorEvent::Reloaded
        ) {
            return;
        }
        let line = editor.read(cx).caret_point().line;
        let moved = self
            .intentions
            .bulb
            .is_some_and(|b| b.editor != editor.entity_id() || b.line != line);
        if moved || matches!(event, EditorEvent::Edited(_) | EditorEvent::Reloaded) {
            self.clear_bulb(cx);
        }
        if self.services().is_none() {
            return;
        }
        let editor = editor.clone();
        self.intentions.bulb_request = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BULB_DELAY).await;
            let request = this
                .update(cx, |v, cx| {
                    v.actions_request(&editor, ActionTrigger::Automatic, cx)
                })
                .ok()
                .flatten();
            let Some((request, _, _)) = request else {
                return;
            };
            let actions = request.await.unwrap_or_default();
            let _ = this.update(cx, |v, cx| {
                v.intentions.bulb_request = None;
                let line = editor.read(cx).caret_point().line;
                v.intentions.bulb = bulb_icon(&actions).map(|icon| Bulb {
                    editor: editor.entity_id(),
                    line,
                    icon,
                });
                cx.notify();
            });
        }));
    }

    pub(crate) fn clear_bulb(&mut self, cx: &mut Context<Self>) {
        self.intentions.bulb_request = None;
        if self.intentions.bulb.take().is_some() {
            cx.notify();
        }
    }

    /// The bulb at the start of the caret line, left of the text.
    pub(crate) fn render_bulb(&self, cx: &App) -> Option<AnyElement> {
        let bulb = self.intentions.bulb?;
        let editor = self.active_document()?.editor.clone();
        if editor.entity_id() != bulb.editor {
            return None;
        }
        let e = editor.read(cx);
        if !e.visible_lines()?.contains(&bulb.line) {
            return None;
        }
        let start = e.buffer().line_start(bulb.line);
        let line = e.bounds_for_range(start..start)?;
        let (_, line_height, _) = e.text_geometry()?;
        let y = line.top() + (line_height - px(BULB_SIZE)) / 2.0;
        let x = line.left() - px(BULB_SIZE + 2.0);
        let owner = self.intentions.owner.clone();
        Some(
            deferred(
                anchored().position(point(x, y)).child(
                    div()
                        .id("intention-bulb")
                        .debug_selector(|| "intention-bulb".into())
                        .size(px(BULB_SIZE))
                        // Over the editor: its own request for the hand
                        // wins over the editor's text cursor.
                        .pointing_hand()
                        .on_mouse_down(MouseButton::Left, move |_, _: &mut Window, cx| {
                            cx.stop_propagation();
                            if let Some(owner) = owner.upgrade() {
                                owner.update(cx, |v, cx| {
                                    if let Some(editor) =
                                        v.active_document().map(|d| d.editor.clone())
                                    {
                                        v.show_intentions_at_caret(&editor, cx);
                                    }
                                });
                            }
                        })
                        .child(Icon::new(bulb.icon).size(px(BULB_SIZE))),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The bulb's state for `code.state`.
    pub(crate) fn bulb_state_json(&self) -> Json {
        self.intentions
            .bulb
            .map(|b| json!({ "line": b.line, "icon": b.icon.0 }))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use code_intel::{CodeActionKind, Diagnostic, Point};

    use super::*;

    fn action(title: &str, kind: CodeActionKind) -> CodeAction {
        CodeAction::new(title, kind)
    }

    #[test]
    fn quick_fixes_first_then_the_others_by_name() {
        let mut fix = action(
            "Ruff (unused-import): Remove unused import",
            CodeActionKind::QuickFix,
        );
        fix.children = vec![action("Disable for this file", CodeActionKind::QuickFix)];
        let mut error_fix = action("Import 'Path'", CodeActionKind::QuickFix);
        error_fix.diagnostics = vec![Diagnostic::new(
            Point::new(0, 0)..Point::new(0, 1),
            Severity::Error,
            "unresolved",
        )];
        let actions = order(vec![
            action("Ruff: Organize imports", CodeActionKind::organize_imports()),
            fix,
            action("add type hints", CodeActionKind::Refactor(String::new())),
            error_fix,
            action(
                "Ruff: Fix all auto-fixable problems",
                CodeActionKind::Source("fixAll.ruff".into()),
            ),
        ]);
        let titles: Vec<&str> = actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Ruff (unused-import): Remove unused import",
                "Import 'Path'",
                "add type hints",
                "Ruff: Fix all auto-fixable problems",
                "Ruff: Organize imports",
            ]
        );
        let (items, map) = rows(&actions);
        assert_eq!(items.len(), 6);
        assert_eq!(items[2], ListItem::Separator);
        assert_eq!(map, [Some(0), Some(1), None, Some(2), Some(3), Some(4)]);
        let ListItem::Row(first) = &items[0] else {
            panic!("a row");
        };
        assert_eq!(first.icon, Some(IconName::INTENTION_BULB));
        assert_eq!(first.children.len(), 1);
        let ListItem::Row(second) = &items[1] else {
            panic!("a row");
        };
        assert_eq!(second.icon, Some(IconName::QUICKFIX_BULB));
        let ListItem::Row(other) = &items[3] else {
            panic!("a row");
        };
        assert_eq!(other.icon, None);
        assert_eq!(bulb_icon(&actions), Some(IconName::QUICKFIX_BULB));
        assert_eq!(bulb_icon(&[]), None);
        assert_eq!(bulb_icon(&actions[2..]), Some(IconName::INTENTION_BULB));
    }
}
