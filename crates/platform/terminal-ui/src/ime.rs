//! The view as the window's input handler: typed and composed text goes
//! to the session; text an input method is still composing is kept by the
//! view and drawn at the cursor (the reference's
//! `TerminalOutputEditorInputMethodSupport`). The terminal has no text the
//! input method could edit, so ranges are relative to the composition.

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window, point, size};

use crate::view::SessionView;

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The part of `text` in a UTF-16 range.
fn utf16_slice(text: &str, range: &Range<usize>) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let start = range.start.min(units.len());
    let end = range.end.clamp(start, units.len());
    String::from_utf16_lossy(&units[start..end])
}

impl EntityInputHandler for SessionView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let marked = self.marked.as_deref().unwrap_or("");
        let len = utf16_len(marked);
        let range = range_utf16.start.min(len)..range_utf16.end.min(len);
        actual_range.replace(range.clone());
        Some(utf16_slice(marked, &range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked.as_deref().map_or(0, utf16_len);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|m| 0..utf16_len(m))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        self.type_text(text, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The composition starts at the cursor; the candidate window goes
        // below the character the range starts at.
        let geometry = self.geometry?;
        let cursor = self.snapshot.cursor?.point;
        let x =
            geometry.origin.x + geometry.cell.width * (cursor.column + range_utf16.start) as f32;
        let y = geometry.origin.y + geometry.cell.height * cursor.line as f32;
        Some(Bounds::new(
            point(x, y),
            size(geometry.cell.width, geometry.cell.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_ranges() {
        assert_eq!(utf16_len("aé😀"), 4);
        assert_eq!(utf16_slice("aé😀b", &(1..4)), "é😀");
        assert_eq!(utf16_slice("ab", &(1..9)), "b");
        assert_eq!(utf16_slice("ab", &(5..9)), "");
    }
}
