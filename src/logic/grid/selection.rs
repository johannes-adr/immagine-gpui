//! Swipe-to-select range painting. Swift source: the range logic of `SelectionPaint` in
//! `Immagine/UI/Timeline/TimelineGrid.swift`. Spec: `ImmagineTests/GridInteractionTests.swift`.

use std::collections::HashSet;
use std::ops::RangeInclusive;

/// Range selection of a swipe-to-select drag: everything between the anchor and the current item
/// takes the anchor's new state; items the range leaves again fall back to their state before the drag.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionPaint {
    anchor: usize,
    selecting: bool,
    base: HashSet<usize>,
    current: usize,
    started: bool,
}

fn span(a: usize, b: usize) -> RangeInclusive<usize> {
    a.min(b)..=a.max(b)
}

impl SelectionPaint {
    /// `selecting`: the drag selects (true) or deselects (false). `base`: the selected indices before the drag.
    pub fn new(anchor: usize, selecting: bool, base: HashSet<usize>) -> Self {
        Self { anchor, selecting, base, current: anchor, started: false }
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn started(&self) -> bool {
        self.started
    }

    /// Moves the drag to `index`; returns the items whose state changes, in ascending index order.
    pub fn move_to(&mut self, index: usize) -> Vec<(usize, bool)> {
        let old = self.started.then(|| span(self.anchor, self.current));
        let new = span(self.anchor, index);
        self.started = true;
        self.current = index;

        let lower = old.as_ref().map_or(*new.start(), |o| *o.start()).min(*new.start());
        let upper = old.as_ref().map_or(*new.end(), |o| *o.end()).max(*new.end());
        let mut changes = Vec::new();
        for i in lower..=upper {
            let was = match &old {
                Some(o) if o.contains(&i) => self.selecting,
                _ => self.base.contains(&i),
            };
            let now = if new.contains(&i) { self.selecting } else { self.base.contains(&i) };
            if was != now {
                changes.push((i, now));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[usize]) -> HashSet<usize> {
        items.iter().copied().collect()
    }

    fn indices(changes: &[(usize, bool)]) -> Vec<usize> {
        changes.iter().map(|c| c.0).collect()
    }

    #[test]
    fn paint_selects_the_range_in_reading_order() {
        let mut paint = SelectionPaint::new(2, true, set(&[]));
        assert_eq!(indices(&paint.move_to(2)), vec![2], "the first photo selects at once");
        assert_eq!(indices(&paint.move_to(7)), vec![3, 4, 5, 6, 7], "down a row: everything in between");
        assert!(paint.move_to(7).is_empty());
        let back = paint.move_to(4);
        assert_eq!(indices(&back), vec![5, 6, 7], "shrinking the range gives photos back");
        assert!(back.iter().all(|c| !c.1));
        let flipped = paint.move_to(0);
        assert_eq!(indices(&flipped), vec![0, 1, 3, 4], "crossing the anchor");
        let selected: Vec<usize> = flipped.iter().filter(|c| c.1).map(|c| c.0).collect();
        assert_eq!(selected, vec![0, 1]);
    }

    #[test]
    fn paint_starting_on_a_selected_photo_deselects() {
        let mut paint = SelectionPaint::new(1, false, set(&[0, 1, 2, 5]));
        let first = paint.move_to(3);
        assert_eq!(indices(&first), vec![1, 2], "3 was not selected, so nothing changes for it");
        assert!(first.iter().all(|c| !c.1));
        let undo = paint.move_to(1);
        assert_eq!(indices(&undo), vec![2], "photos leaving the range get their old state back");
        assert!(undo.iter().all(|c| c.1));
    }
}
