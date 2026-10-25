//! What the console view keeps of the model's lines to draw them: style runs
//! and links in chars, for every line of the editor's text.
//!
//! A line with output of a few characters costs a handful of bytes here, not
//! a heap allocation per line: the runs and the links of all lines are two
//! flat vectors with one end offset per line (a flood of short lines fills
//! the cycle buffer with hundreds of thousands of lines).

use std::ops::Range;

use console::{ContentKind, LinkTarget, Style};

/// One style run of a console line, in chars.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    pub start: u32,
    pub end: u32,
    pub kind: ContentKind,
    pub style: Style,
}

impl Run {
    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// A link of a console line, in chars.
pub(crate) type LinkSpan = (Range<usize>, LinkTarget);

/// Items of consecutive lines in one vector, `ends[i]` being where line
/// `i`'s items stop.
#[derive(Debug, Clone)]
struct PerLine<T> {
    ends: Vec<u32>,
    items: Vec<T>,
}

impl<T> PerLine<T> {
    fn new() -> Self {
        Self {
            ends: Vec::new(),
            items: Vec::new(),
        }
    }

    fn lines(&self) -> usize {
        self.ends.len()
    }

    fn line(&self, index: usize) -> &[T] {
        let Some(&end) = self.ends.get(index) else {
            return &[];
        };
        let start = index.checked_sub(1).map_or(0, |i| self.ends[i]);
        &self.items[start as usize..end as usize]
    }

    fn push(&mut self, items: impl IntoIterator<Item = T>) {
        self.items.extend(items);
        self.ends
            .push(u32::try_from(self.items.len()).unwrap_or(u32::MAX));
    }

    fn truncate(&mut self, lines: usize) {
        if lines >= self.ends.len() {
            return;
        }
        let keep = lines.checked_sub(1).map_or(0, |i| self.ends[i]);
        self.items.truncate(keep as usize);
        self.ends.truncate(lines);
    }

    fn drain_front(&mut self, lines: usize) {
        let lines = lines.min(self.ends.len());
        let Some(base) = lines.checked_sub(1).map(|i| self.ends[i]) else {
            return;
        };
        self.items.drain(..base as usize);
        self.ends.drain(..lines);
        for end in &mut self.ends {
            *end -= base;
        }
    }

    fn replace_first(&mut self, items: Vec<T>) {
        let Some(&old_end) = self.ends.first() else {
            return;
        };
        let new_end = items.len() as i64;
        self.items.splice(..old_end as usize, items);
        let delta = new_end - i64::from(old_end);
        for end in &mut self.ends {
            *end = (i64::from(*end) + delta) as u32;
        }
    }

    /// Forgets the lines but keeps the room: a flood takes the whole text
    /// anew every frame, and growing the vectors again would leave a trail
    /// of freed large blocks.
    fn clear(&mut self) {
        self.ends.clear();
        self.items.clear();
    }

    /// Gives back room that is much larger than the lines in it.
    fn trim_capacity(&mut self) {
        if self.items.capacity() > 2 * self.items.len().max(1024) {
            self.items
                .shrink_to(self.items.len() + self.items.len() / 4);
        }
        if self.ends.capacity() > 2 * self.ends.len().max(1024) {
            self.ends.shrink_to(self.ends.len() + self.ends.len() / 4);
        }
    }
}

/// Styles and links of the lines of the editor's text, in chars.
#[derive(Debug, Clone)]
pub(crate) struct Decor {
    runs: PerLine<Run>,
    links: PerLine<LinkSpan>,
}

impl Default for Decor {
    fn default() -> Self {
        let mut decor = Self {
            runs: PerLine::new(),
            links: PerLine::new(),
        };
        decor.push_empty();
        decor
    }
}

impl Decor {
    /// The style runs of line `index` (none past the end).
    pub fn runs(&self, index: usize) -> &[Run] {
        self.runs.line(index)
    }

    /// The links of line `index` (none past the end).
    pub fn links(&self, index: usize) -> &[LinkSpan] {
        self.links.line(index)
    }

    /// Some line has a link.
    pub fn has_links(&self) -> bool {
        !self.links.items.is_empty()
    }

    /// Every link with its line, in order.
    pub fn all_links(&self) -> impl Iterator<Item = (usize, &LinkSpan)> + '_ {
        (0..self.links.lines()).flat_map(|i| self.links.line(i).iter().map(move |l| (i, l)))
    }

    /// Forgets all lines and takes these.
    pub fn reset<'a>(&mut self, lines: impl Iterator<Item = &'a console::Line>) {
        self.runs.clear();
        self.links.clear();
        self.extend(lines);
        self.runs.trim_capacity();
        self.links.trim_capacity();
    }

    /// Drops the first `lines` lines.
    pub fn drain_front(&mut self, lines: usize) {
        self.runs.drain_front(lines);
        self.links.drain_front(lines);
    }

    /// Takes line 0 anew (the cycle buffer cut its start).
    pub fn replace_first(&mut self, line: &console::Line) {
        let (runs, links) = convert(line);
        self.runs.replace_first(runs);
        self.links.replace_first(links);
    }

    /// Keeps the first `lines` lines.
    pub fn truncate(&mut self, lines: usize) {
        self.runs.truncate(lines);
        self.links.truncate(lines);
    }

    /// Appends lines.
    pub fn extend<'a>(&mut self, lines: impl Iterator<Item = &'a console::Line>) {
        for line in lines {
            let text = line.text();
            let ascii = text.is_ascii();
            let to_char = |byte: usize| char_offset(text, ascii, byte);
            self.runs.push(line.runs().iter().map(|r| Run {
                start: to_char(r.range.start),
                end: to_char(r.range.end),
                kind: r.kind,
                style: r.style,
            }));
            self.links.push(line.links().iter().map(|l| {
                (
                    to_char(l.range.start) as usize..to_char(l.range.end) as usize,
                    l.target.clone(),
                )
            }));
        }
    }

    fn push_empty(&mut self) {
        self.runs.push([]);
        self.links.push([]);
    }
}

fn char_offset(text: &str, ascii: bool, byte: usize) -> u32 {
    let byte = byte.min(text.len());
    let chars = if ascii {
        byte
    } else {
        text[..byte].chars().count()
    };
    u32::try_from(chars).unwrap_or(u32::MAX)
}

fn convert(line: &console::Line) -> (Vec<Run>, Vec<LinkSpan>) {
    let mut decor = Decor {
        runs: PerLine::new(),
        links: PerLine::new(),
    };
    decor.extend(std::iter::once(line));
    (decor.runs.items, decor.links.items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(per_line: &PerLine<u32>) -> Vec<Vec<u32>> {
        (0..per_line.lines())
            .map(|i| per_line.line(i).to_vec())
            .collect()
    }

    fn sample() -> PerLine<u32> {
        let mut p = PerLine::new();
        p.push([1, 2]);
        p.push([]);
        p.push([3]);
        p.push([4, 5, 6]);
        p
    }

    #[test]
    fn lines_keep_their_items() {
        let p = sample();
        assert_eq!(lines(&p), [vec![1, 2], vec![], vec![3], vec![4, 5, 6]]);
        assert!(p.line(9).is_empty());
    }

    #[test]
    fn drain_front_renumbers() {
        let mut p = sample();
        p.drain_front(2);
        assert_eq!(lines(&p), [vec![3], vec![4, 5, 6]]);
        p.drain_front(9);
        assert_eq!(p.lines(), 0);
        assert!(p.items.is_empty());
    }

    #[test]
    fn truncate_keeps_the_head() {
        let mut p = sample();
        p.truncate(3);
        assert_eq!(lines(&p), [vec![1, 2], vec![], vec![3]]);
        p.truncate(0);
        assert_eq!(p.lines(), 0);
        assert!(p.items.is_empty());
    }

    #[test]
    fn replace_first_moves_the_rest() {
        let mut p = sample();
        p.replace_first(vec![7]);
        assert_eq!(lines(&p), [vec![7], vec![], vec![3], vec![4, 5, 6]]);
        p.replace_first(vec![8, 9, 10]);
        assert_eq!(lines(&p), [vec![8, 9, 10], vec![], vec![3], vec![4, 5, 6]]);
        p.replace_first(vec![]);
        assert_eq!(lines(&p), [vec![], vec![], vec![3], vec![4, 5, 6]]);
    }
}
