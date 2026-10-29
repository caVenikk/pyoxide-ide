//! The editor's context menu request: a right click on the text (or the
//! `editor.context-menu` command) moves the caret as the reference does and
//! tells the owner where to open its menu ([`EditorEvent::ContextMenu`]);
//! the menu itself is the owner's, as it needs the file, the project and
//! the product's commands.

use gpui::{Context, Pixels, Point, Window};
use text::Selection;

use crate::commands::ShowContextMenu;
use crate::editor::{Editor, EditorEvent};

impl Editor {
    /// A right click at `position`: the caret goes to the click unless it
    /// is inside a selection (the selection stays for Cut and Copy), the
    /// editor takes the focus, and the owner is asked for the menu.
    /// `false` when the click is not on the text.
    pub(crate) fn context_menu_press(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let over_text = self.layout.as_ref().is_some_and(|l| l.over_text(position));
        if !over_text || self.single_line {
            return false;
        }
        window.focus(self.focus_handle_ref());
        let (offset, lean) = self.position_for_point(position);
        let inside = self
            .selections
            .iter()
            .any(|s| !s.is_empty() && s.range().contains(&offset));
        if !inside {
            self.set_selections(text::SelectionSet::single(Selection::caret(offset)), cx);
            if lean {
                self.lean_back.insert(offset);
            }
        }
        cx.emit(EditorEvent::ContextMenu { position });
        true
    }

    /// `editor.context-menu`: the menu at the caret, below its line.
    pub(crate) fn show_context_menu(
        &mut self,
        _: &ShowContextMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let head = self.selections.primary().head;
        let Some(bounds) = self.bounds_for_offset(head) else {
            return;
        };
        cx.emit(EditorEvent::ContextMenu {
            position: Point::new(bounds.left(), bounds.bottom()),
        });
    }
}
