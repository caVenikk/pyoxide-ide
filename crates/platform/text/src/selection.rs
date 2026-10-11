//! Carets and selections. Positions are char offsets into the buffer.
//! A [`SelectionSet`] holds one or more selections (multiple carets), kept
//! sorted and without overlaps.

use std::ops::Range;

/// A caret (`anchor == head`) or a selected range. The head is where the
/// caret is drawn; the anchor stays put when the selection is extended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Display column the caret keeps while moving up and down across
    /// shorter lines; `None` until the first vertical move.
    pub goal: Option<usize>,
}

impl Selection {
    pub const fn caret(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
            goal: None,
        }
    }

    pub const fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            goal: None,
        }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The head is before the anchor.
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// Moves the caret to `pos`, dropping the selection.
    pub fn collapse_to(&mut self, pos: usize) {
        self.anchor = pos;
        self.head = pos;
        self.goal = None;
    }

    /// Moves the head to `pos`, keeping the anchor.
    pub fn extend_to(&mut self, pos: usize) {
        self.head = pos;
        self.goal = None;
    }
}

/// One or more selections, sorted by start and never overlapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSet {
    items: Vec<Selection>,
    primary: usize,
}

impl Default for SelectionSet {
    fn default() -> Self {
        Self::caret(0)
    }
}

impl SelectionSet {
    pub fn caret(pos: usize) -> Self {
        Self::single(Selection::caret(pos))
    }

    pub fn single(selection: Selection) -> Self {
        Self {
            items: vec![selection],
            primary: 0,
        }
    }

    /// Builds a set from any selections; overlapping ones are merged. The
    /// last selection given is the primary one.
    pub fn from_vec(items: Vec<Selection>) -> Self {
        assert!(!items.is_empty(), "a selection set is never empty");
        let primary = items[items.len() - 1];
        let mut set = Self { items, primary: 0 };
        set.normalize(Some(primary));
        set
    }

    pub fn primary(&self) -> &Selection {
        &self.items[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.items.iter()
    }

    pub fn as_slice(&self) -> &[Selection] {
        &self.items
    }

    /// Applies `f` to every selection, then merges overlaps.
    pub fn update(&mut self, mut f: impl FnMut(&mut Selection)) {
        let primary = self.items[self.primary];
        let mut new_primary = primary;
        for (i, s) in self.items.iter_mut().enumerate() {
            f(s);
            if i == self.primary {
                new_primary = *s;
            }
        }
        self.normalize(Some(new_primary));
    }

    /// Keeps only the primary selection.
    pub fn keep_primary(&mut self) {
        let primary = self.items[self.primary];
        self.items = vec![primary];
        self.primary = 0;
    }

    /// Clamps every selection to `len` chars.
    pub fn clamp(&mut self, len: usize) {
        self.update(|s| {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
        });
    }

    fn normalize(&mut self, primary: Option<Selection>) {
        self.items.sort_by_key(|s| (s.start(), s.end()));
        let mut merged: Vec<Selection> = Vec::with_capacity(self.items.len());
        for s in self.items.drain(..) {
            match merged.last_mut() {
                // Overlapping, or two carets on the same spot.
                Some(last)
                    if s.start() < last.end()
                        || (s.start() == last.end() && (s.is_empty() || last.is_empty())) =>
                {
                    let start = last.start().min(s.start());
                    let end = last.end().max(s.end());
                    let reversed = last.is_reversed();
                    *last = if reversed {
                        Selection::new(end, start)
                    } else {
                        Selection::new(start, end)
                    };
                }
                _ => merged.push(s),
            }
        }
        self.items = merged;
        self.primary = primary
            .and_then(|p| {
                self.items
                    .iter()
                    .position(|s| s.start() <= p.head && p.head <= s.end())
            })
            .unwrap_or(self.items.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_have_a_direction() {
        let s = Selection::new(5, 2);
        assert_eq!(s.range(), 2..5);
        assert!(s.is_reversed());
        assert!(!s.is_empty());
        assert!(Selection::caret(3).is_empty());
    }

    #[test]
    fn overlapping_selections_merge() {
        let set = SelectionSet::from_vec(vec![
            Selection::new(0, 4),
            Selection::new(3, 8),
            Selection::caret(10),
            Selection::caret(10),
        ]);
        assert_eq!(
            set.as_slice(),
            &[Selection::new(0, 8), Selection::caret(10)]
        );
        assert_eq!(set.primary().head, 10);
    }

    #[test]
    fn update_and_clamp() {
        let mut set = SelectionSet::from_vec(vec![Selection::caret(1), Selection::caret(5)]);
        set.update(|s| s.collapse_to(s.head + 10));
        assert_eq!(set.iter().map(|s| s.head).collect::<Vec<_>>(), [11, 15]);
        set.clamp(12);
        assert_eq!(set.iter().map(|s| s.head).collect::<Vec<_>>(), [11, 12]);
        set.keep_primary();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().head, 12);
    }
}
