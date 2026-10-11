//! Text input from the platform: typed characters, dead keys and input
//! method compositions arrive through GPUI's input handler, in UTF-16
//! offsets. The structure follows the input handler of GPUI's own text input
//! example (Apache-2.0, see NOTICE).

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window, point};
use text::{CaretPlacement, Edit, EditKind, Selection, SelectionSet};

use crate::editor::Editor;

impl Editor {
    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.buffer.utf16_to_char(range.start)..self.buffer.utf16_to_char(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.buffer.char_to_utf16(range.start)..self.buffer.char_to_utf16(range.end)
    }

    /// Replaces `range` (or the composition, or every selection) with text;
    /// returns the inserted range of the primary replacement.
    fn replace_for_input(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let target = range.or_else(|| self.marked.clone());
        let kind = if text.contains('\n') {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        match target {
            Some(range) => {
                let start = range.start;
                let end = start + text.chars().count();
                self.edit(
                    vec![Edit::new(range, text)],
                    kind,
                    CaretPlacement::Explicit(SelectionSet::caret(end)),
                    cx,
                );
                Some(start..end)
            }
            None => {
                let start = self.selections.primary().start();
                self.type_str(text, cx);
                let end = self.selections.primary().head;
                Some(start.min(end)..end)
            }
        }
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.buffer.slice(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let s = *self.selections.primary();
        Some(UTF16Selection {
            range: self.range_to_utf16(&s.range()),
            reversed: s.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|r| self.range_from_utf16(&r));
        self.replace_for_input(range, text, cx);
        self.marked = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|r| self.range_from_utf16(&r));
        let Some(inserted) = self.replace_for_input(range, text, cx) else {
            return;
        };
        self.marked = (!inserted.is_empty()).then(|| inserted.clone());
        let selection = match new_selected_range_utf16 {
            Some(r) => {
                // Relative to the start of the composition, in UTF-16.
                let base = self.buffer.char_to_utf16(inserted.start);
                let r = self.range_from_utf16(&(base + r.start..base + r.end));
                Selection::new(r.start, r.end)
            }
            None => Selection::caret(inserted.end),
        };
        self.selections = SelectionSet::single(selection);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.bounds_for_offset(range.start)?;
        let end = self.bounds_for_offset(range.end)?;
        Some(Bounds::from_corners(
            start.origin,
            point(end.left().max(start.right()), start.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.layout.as_ref()?;
        let offset = self.offset_for_position(position);
        Some(self.buffer.char_to_utf16(offset))
    }
}
