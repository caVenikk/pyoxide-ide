//! The layout of the editor area's split windows: a binary tree whose
//! leaves are tab groups (the reference's `EditorWindow`s) and whose inner
//! nodes are splitters (`Splitter`, created by `EditorWindow.split` with the
//! proportion 0.5 limited to 0.1..0.9). No toolkit and no tabs: ids only.

use serde::{Deserialize, Serialize};

/// A tab group of the editor area.
pub type GroupId = u32;

/// Where a splitter may be dragged: the smallest and the largest share of
/// its first side (`createSplitter(.., 0.5f, 0.1f, 0.9f)`).
pub const MIN_PROPORTION: f32 = 0.1;
pub const MAX_PROPORTION: f32 = 0.9;

/// A leaf or a splitter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    Leaf(GroupId),
    Split(Box<Splitter>),
}

/// Two sides divided by a line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Splitter {
    /// The sides are stacked (the second is below the first: Split Down);
    /// else they are side by side (Split Right).
    pub vertical: bool,
    /// The share of the first side.
    pub proportion: f32,
    pub first: Node,
    pub second: Node,
}

/// The way from the root to a splitter: `false` takes the first side.
pub type Path = Vec<bool>;

impl Node {
    /// The groups in tree order (first side before second): the order of
    /// Go to Next Splitter (`getOrderedWindows`).
    pub fn leaves(&self) -> Vec<GroupId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<GroupId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split(s) => {
                s.first.collect(out);
                s.second.collect(out);
            }
        }
    }

    pub fn contains(&self, id: GroupId) -> bool {
        match self {
            Node::Leaf(leaf) => *leaf == id,
            Node::Split(s) => s.first.contains(id) || s.second.contains(id),
        }
    }

    /// The tree is one group (nothing is split).
    pub fn is_single(&self) -> bool {
        matches!(self, Node::Leaf(_))
    }

    /// Splits group `target` into itself and `new`: a splitter takes its
    /// place with `target` on the first side and `new` on the second
    /// (below or right of it), or the other way round with `new_first`.
    /// `false` when there is no such group.
    pub fn split(
        &mut self,
        target: GroupId,
        vertical: bool,
        new: GroupId,
        new_first: bool,
    ) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let (first, second) = if new_first {
                    (Node::Leaf(new), Node::Leaf(target))
                } else {
                    (Node::Leaf(target), Node::Leaf(new))
                };
                *self = Node::Split(Box::new(Splitter {
                    vertical,
                    proportion: 0.5,
                    first,
                    second,
                }));
                true
            }
            Node::Leaf(_) => false,
            Node::Split(s) => {
                s.first.split(target, vertical, new, new_first)
                    || s.second.split(target, vertical, new, new_first)
            }
        }
    }

    /// Takes group `id` out: its splitter is replaced by the other side.
    /// `None` when `id` was the only group (nothing is left).
    pub fn without(self, id: GroupId) -> Option<Node> {
        match self {
            Node::Leaf(leaf) => (leaf != id).then_some(Node::Leaf(leaf)),
            Node::Split(s) => {
                let Splitter {
                    vertical,
                    proportion,
                    first,
                    second,
                } = *s;
                match (first.without(id), second.without(id)) {
                    (Some(first), Some(second)) => Some(Node::Split(Box::new(Splitter {
                        vertical,
                        proportion,
                        first,
                        second,
                    }))),
                    (one, other) => one.or(other),
                }
            }
        }
    }

    /// The groups beside `id` in its splitter (the other side's leaves):
    /// the reference's `siblings()`. Empty when `id` is not in a splitter.
    pub fn siblings(&self, id: GroupId) -> Vec<GroupId> {
        match self {
            Node::Leaf(_) => Vec::new(),
            Node::Split(s) => {
                if matches!(s.first, Node::Leaf(l) if l == id) {
                    return s.second.leaves();
                }
                if matches!(s.second, Node::Leaf(l) if l == id) {
                    return s.first.leaves();
                }
                let beside = s.first.siblings(id);
                if !beside.is_empty() {
                    return beside;
                }
                s.second.siblings(id)
            }
        }
    }

    /// The first group of the second side, when group `id` is the first
    /// side of its splitter (`EditorsSplitters.openInRightSplit`: the
    /// window's parent splitter has another second component). `None` when
    /// `id` is not in a splitter or is its second side.
    pub fn second_side_of(&self, id: GroupId) -> Option<GroupId> {
        let Node::Split(s) = self else {
            return None;
        };
        if matches!(s.first, Node::Leaf(l) if l == id) {
            return s.second.leaves().first().copied();
        }
        if matches!(s.second, Node::Leaf(l) if l == id) {
            return None;
        }
        if s.first.contains(id) {
            s.first.second_side_of(id)
        } else {
            s.second.second_side_of(id)
        }
    }

    /// The splitter whose side is group `id` itself.
    pub fn parent_mut(&mut self, id: GroupId) -> Option<&mut Splitter> {
        let Node::Split(s) = self else {
            return None;
        };
        if matches!(s.first, Node::Leaf(l) if l == id)
            || matches!(s.second, Node::Leaf(l) if l == id)
        {
            return Some(s);
        }
        // Borrowed in turn: the first side's answer must not hold `s`.
        if s.first.contains(id) {
            s.first.parent_mut(id)
        } else {
            s.second.parent_mut(id)
        }
    }

    /// Replaces the splitter that has group `id` as a side by `id` alone
    /// (Unsplit): the groups of the other side are returned.
    pub fn collapse(&mut self, id: GroupId) -> Vec<GroupId> {
        let Node::Split(s) = self else {
            return Vec::new();
        };
        let direct = matches!(s.first, Node::Leaf(l) if l == id)
            || matches!(s.second, Node::Leaf(l) if l == id);
        if direct {
            let gone = if matches!(s.first, Node::Leaf(l) if l == id) {
                s.second.leaves()
            } else {
                s.first.leaves()
            };
            *self = Node::Leaf(id);
            return gone;
        }
        if s.first.contains(id) {
            s.first.collapse(id)
        } else {
            s.second.collapse(id)
        }
    }

    /// The splitter at a path from the root.
    pub fn at_mut(&mut self, path: &[bool]) -> Option<&mut Splitter> {
        let Node::Split(s) = self else {
            return None;
        };
        match path.split_first() {
            None => Some(s),
            Some((false, rest)) => s.first.at_mut(rest),
            Some((true, rest)) => s.second.at_mut(rest),
        }
    }

    /// A valid tree has every group once and proportions in range (a state
    /// read from a file may not). For the saved layout, which comes later.
    #[cfg(test)]
    pub fn is_valid(&self) -> bool {
        let leaves = self.leaves();
        let mut seen = leaves.clone();
        seen.sort_unstable();
        seen.dedup();
        seen.len() == leaves.len() && self.proportions_valid()
    }

    #[cfg(test)]
    fn proportions_valid(&self) -> bool {
        match self {
            Node::Leaf(_) => true,
            Node::Split(s) => {
                (MIN_PROPORTION..=MAX_PROPORTION).contains(&s.proportion)
                    && s.first.proportions_valid()
                    && s.second.proportions_valid()
            }
        }
    }
}

/// A proportion kept in range.
pub fn clamp_proportion(p: f32) -> f32 {
    p.clamp(MIN_PROPORTION, MAX_PROPORTION)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Node {
        let mut n = Node::Leaf(0);
        assert!(n.split(0, false, 1, false));
        n
    }

    #[test]
    fn split_puts_the_new_group_second() {
        let n = two();
        assert_eq!(n.leaves(), vec![0, 1]);
        let Node::Split(s) = &n else { panic!() };
        assert!(!s.vertical);
        assert_eq!(s.proportion, 0.5);
    }

    #[test]
    fn the_second_side_is_beside_the_first_sides_group_only() {
        let mut n = two();
        assert_eq!(n.second_side_of(0), Some(1));
        assert_eq!(n.second_side_of(1), None);
        // (0 | (1 / 2)): 0 has the first of the second side; 1 is the
        // first side of its own splitter.
        assert!(n.split(1, true, 2, false));
        assert_eq!(n.second_side_of(0), Some(1));
        assert_eq!(n.second_side_of(1), Some(2));
        assert_eq!(n.second_side_of(2), None);
        assert_eq!(Node::Leaf(0).second_side_of(0), None);
    }

    #[test]
    fn split_first_puts_the_new_group_first() {
        let mut n = Node::Leaf(0);
        n.split(0, true, 1, true);
        assert_eq!(n.leaves(), vec![1, 0]);
    }

    #[test]
    fn nested_splits_keep_tree_order() {
        let mut n = two();
        assert!(n.split(0, true, 2, false));
        assert!(n.split(1, true, 3, false));
        assert_eq!(n.leaves(), vec![0, 2, 1, 3]);
        assert!(!n.split(9, true, 4, false));
    }

    #[test]
    fn removing_a_group_lets_its_sibling_take_the_place() {
        let mut n = two();
        n.split(1, true, 2, false);
        let n = n.without(1).unwrap();
        assert_eq!(n.leaves(), vec![0, 2]);
        let n = n.without(0).unwrap();
        assert_eq!(n, Node::Leaf(2));
        assert_eq!(n.without(2), None);
    }

    #[test]
    fn siblings_are_the_other_side_of_the_splitter() {
        let mut n = two();
        n.split(1, true, 2, false);
        // 0 | (1 over 2)
        assert_eq!(n.siblings(0), vec![1, 2]);
        assert_eq!(n.siblings(1), vec![2]);
        assert_eq!(n.siblings(2), vec![1]);
        assert!(Node::Leaf(0).siblings(0).is_empty());
    }

    #[test]
    fn collapse_leaves_the_group_alone_in_its_place() {
        let mut n = two();
        n.split(1, true, 2, false);
        assert_eq!(n.collapse(1), vec![2]);
        assert_eq!(n.leaves(), vec![0, 1]);
        assert_eq!(n.collapse(0), vec![1]);
        assert_eq!(n, Node::Leaf(0));
    }

    #[test]
    fn parent_of_a_group_changes_orientation() {
        let mut n = two();
        n.split(1, true, 2, false);
        n.parent_mut(2).unwrap().vertical = false;
        let Node::Split(root) = &n else { panic!() };
        let Node::Split(inner) = &root.second else {
            panic!()
        };
        assert!(!inner.vertical);
        assert!(!root.vertical);
        assert!(n.parent_mut(7).is_none());
    }

    #[test]
    fn paths_reach_splitters() {
        let mut n = two();
        n.split(1, true, 2, false);
        assert!(n.at_mut(&[]).is_some());
        assert!(n.at_mut(&[true]).is_some());
        assert!(n.at_mut(&[false]).is_none());
        n.at_mut(&[true]).unwrap().proportion = 0.3;
        assert!(n.is_valid());
        n.at_mut(&[true]).unwrap().proportion = 0.95;
        assert!(!n.is_valid());
    }

    #[test]
    fn a_tree_with_a_group_twice_is_not_valid() {
        let n = Node::Split(Box::new(Splitter {
            vertical: false,
            proportion: 0.5,
            first: Node::Leaf(1),
            second: Node::Leaf(1),
        }));
        assert!(!n.is_valid());
    }

    #[test]
    fn the_tree_round_trips_through_json() {
        let mut n = two();
        n.split(1, true, 2, false);
        let text = serde_json::to_string(&n).unwrap();
        assert_eq!(serde_json::from_str::<Node>(&text).unwrap(), n);
    }
}
