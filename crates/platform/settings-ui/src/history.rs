//! Back and Forward of the Settings window (the reference's
//! `ui.navigation.History` with the selected page as its place): every
//! page selected is recorded, Back and Forward go to the pages before and
//! after the current one, and selecting a page after going back drops the
//! pages Forward would go to.

#[derive(Debug, Default)]
pub(crate) struct History {
    places: Vec<&'static str>,
    current: usize,
}

impl History {
    /// Records the page just selected.
    pub fn push(&mut self, page: &'static str) {
        if !self.places.is_empty() {
            self.places.truncate(self.current + 1);
        }
        if self.places.last() == Some(&page) {
            return;
        }
        self.places.push(page);
        self.current = self.places.len() - 1;
    }

    /// The page Back goes to (the nearest one before the current that
    /// still exists), now the current one.
    pub fn back(&mut self, exists: impl Fn(&str) -> bool) -> Option<&'static str> {
        let at = (0..self.current).rev().find(|&i| exists(self.places[i]))?;
        self.current = at;
        Some(self.places[at])
    }

    /// The page Forward goes to, now the current one.
    pub fn forward(&mut self, exists: impl Fn(&str) -> bool) -> Option<&'static str> {
        let at = (self.current + 1..self.places.len()).find(|&i| exists(self.places[i]))?;
        self.current = at;
        Some(self.places[at])
    }

    pub fn can_back(&self, exists: impl Fn(&str) -> bool) -> bool {
        (0..self.current).any(|i| exists(self.places[i]))
    }

    pub fn can_forward(&self, exists: impl Fn(&str) -> bool) -> bool {
        (self.current + 1..self.places.len()).any(|i| exists(self.places[i]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_forward_and_a_new_page_after_going_back() {
        let all = |_: &str| true;
        let mut h = History::default();
        assert!(!h.can_back(all));
        h.push("a");
        h.push("b");
        h.push("b");
        h.push("c");
        assert_eq!(h.back(all), Some("b"));
        assert_eq!(h.back(all), Some("a"));
        assert_eq!(h.back(all), None);
        assert_eq!(h.forward(all), Some("b"));
        assert!(h.can_forward(all));
        // A new page drops "c".
        h.push("d");
        assert!(!h.can_forward(all));
        assert_eq!(h.back(all), Some("b"));
        // Pages that are gone are skipped.
        assert_eq!(h.back(|p| p != "a"), None);
        assert_eq!(h.forward(|p| p != "d"), None);
    }
}
