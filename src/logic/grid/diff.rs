//! Grid diff for animated inserts and removals. Swift source: `GridDiff` in
//! `Immagine/UI/Timeline/TimelineGrid.swift`. Spec: `ImmagineTests/GridInteractionTests.swift`.
//!
//! Only inserts and removals are diffable; a reorder returns `None` and the grid reloads.

use std::collections::HashSet;

/// Items the grid diffs by identity.
pub trait HasId {
    fn id(&self) -> &str;
}

/// Inserts and removals between two item lists, as indices into the old and the new list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridDiff {
    /// Indices into the old list that are gone.
    pub deleted: Vec<usize>,
    /// Indices into the new list that are new.
    pub inserted: Vec<usize>,
}

impl GridDiff {
    /// `None` when the items that survive are not in the same order (something moved).
    pub fn new<T: HasId>(old: &[T], new: &[T]) -> Option<GridDiff> {
        let old_ids: HashSet<&str> = old.iter().map(|t| t.id()).collect();
        let new_ids: HashSet<&str> = new.iter().map(|t| t.id()).collect();
        let kept_old = old.iter().map(|t| t.id()).filter(|id| new_ids.contains(id));
        let kept_new = new.iter().map(|t| t.id()).filter(|id| old_ids.contains(id));
        if !kept_old.eq(kept_new) {
            return None;
        }
        let deleted = old
            .iter()
            .enumerate()
            .filter(|(_, t)| !new_ids.contains(t.id()))
            .map(|(i, _)| i)
            .collect();
        let inserted = new
            .iter()
            .enumerate()
            .filter(|(_, t)| !old_ids.contains(t.id()))
            .map(|(i, _)| i)
            .collect();
        Some(GridDiff { deleted, inserted })
    }

    pub fn count(&self) -> usize {
        self.deleted.len() + self.inserted.len()
    }

    /// Whether the change reaches above the first visible item, which sits at `old_anchor`
    /// before and `new_anchor` after (`None`: it was removed).
    pub fn touches_above(&self, old_anchor: usize, new_anchor: Option<usize>) -> bool {
        let Some(new_anchor) = new_anchor else {
            return true;
        };
        self.deleted.iter().any(|&i| i < old_anchor) || self.inserted.iter().any(|&i| i < new_anchor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug)]
    struct Item(&'static str);

    impl HasId for Item {
        fn id(&self) -> &str {
            self.0
        }
    }

    fn items(ids: &[&'static str]) -> Vec<Item> {
        ids.iter().map(|id| Item(id)).collect()
    }

    #[test]
    fn diff_of_new_uploads_and_deletes() {
        let diff = GridDiff::new(&items(&["a", "b", "c", "d"]), &items(&["n1", "n2", "a", "c", "d"])).unwrap();
        assert_eq!(diff.inserted, vec![0, 1]);
        assert_eq!(diff.deleted, vec![1]);
        assert_eq!(diff.count(), 3);
    }

    #[test]
    fn diff_rejects_moves() {
        assert!(GridDiff::new(&items(&["a", "b", "c"]), &items(&["b", "a", "c"])).is_none(), "a reorder needs a reload");
    }

    #[test]
    fn diff_of_identical_and_empty_lists() {
        assert_eq!(GridDiff::new(&items(&["a"]), &items(&["a"])).unwrap().count(), 0);
        assert_eq!(GridDiff::new(&items(&[]), &items(&["a", "b"])).unwrap().inserted, vec![0, 1]);
        assert_eq!(GridDiff::new(&items(&["a", "b"]), &items(&[])).unwrap().deleted, vec![0, 1]);
    }

    #[test]
    fn changes_above_the_visible_photos() {
        // The user looks at "c" (old index 2).
        let insert_top = GridDiff::new(&items(&["a", "b", "c", "d"]), &items(&["n", "a", "b", "c", "d"])).unwrap();
        assert!(insert_top.touches_above(2, Some(3)));
        let insert_below = GridDiff::new(&items(&["a", "b", "c", "d"]), &items(&["a", "b", "c", "n", "d"])).unwrap();
        assert!(!insert_below.touches_above(2, Some(2)));
        let delete_above = GridDiff::new(&items(&["a", "b", "c", "d"]), &items(&["b", "c", "d"])).unwrap();
        assert!(delete_above.touches_above(2, Some(1)));
        let delete_anchor = GridDiff::new(&items(&["a", "b", "c", "d"]), &items(&["a", "b", "d"])).unwrap();
        assert!(delete_anchor.touches_above(2, None), "the anchor itself vanished");
    }
}
