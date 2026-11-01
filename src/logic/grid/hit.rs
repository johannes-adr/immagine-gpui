//! Hit-testing of the timeline grid as the app lays it out (rows of `columns` tiles, each row centered, `row_h`
//! apart, with `top_rows` empty rows above the first photo row; only the first row can be short, see `head_gap`) and the auto-scroll speed of swipe-to-select.
//! The Swift counterpart is `TimelineGridLayout.index(at:)` and `updateAutoScroll` in `TimelineGrid.swift`.

/// Distance from the visible top/bottom edge within which a painting finger scrolls the grid.
pub const AUTOSCROLL_ZONE: f32 = 80.;
/// Auto-scroll speed (px/s) right at the edge.
pub const AUTOSCROLL_MAX: f32 = 1400.;

/// Geometry of the grid as it was painted last. Coordinates: `x` from the left of the screen, `y` measured from
/// the top of the list's box (add the scroll offset's magnitude to get content coordinates, see `content_y`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitGeom {
    pub width: f32,
    pub count: usize,
    pub columns: usize,
    pub cell: f32,
    pub gap: f32,
    /// Empty rows above the first photo row.
    pub top_rows: usize,
    /// Screen y of the list's top edge.
    pub top: f32,
}

impl HitGeom {
    pub fn row_h(&self) -> f32 {
        self.cell + self.gap
    }

    /// Content y of a screen y: `offset_y` is the list's scroll offset (<= 0 once scrolled down).
    pub fn content_y(&self, screen_y: f32, offset_y: f32) -> f32 {
        screen_y - self.top - offset_y
    }

    fn valid(&self) -> bool {
        self.count > 0 && self.columns > 0 && self.cell > 0. && self.row_h() > 0.
    }

    /// Empty cells before the first photo.
    pub fn skip(&self) -> usize {
        super::layout::head_gap(self.count, self.columns)
    }

    /// Index of the first photo of photo row `row`.
    pub fn first_of_row(&self, row: usize) -> usize {
        (row * self.columns).saturating_sub(self.skip())
    }

    /// Tiles in photo row `row` and the x of the first one (rows are centered).
    fn row_layout(&self, row: usize) -> (usize, f32) {
        let n = if row == 0 { self.columns - self.skip() } else { self.columns }.max(1);
        let w = n as f32 * self.cell + (n - 1) as f32 * self.gap;
        (n, (self.width - w) / 2.)
    }

    /// The photo whose tile contains the point (content y), `None` in gaps, empty rows and beside the tiles.
    pub fn item_at(&self, x: f32, content_y: f32) -> Option<usize> {
        if !self.valid() || content_y < 0. {
            return None;
        }
        let global = (content_y / self.row_h()).floor() as usize;
        let row = global.checked_sub(self.top_rows)?;
        if row * self.columns >= self.count + self.skip() {
            return None;
        }
        if content_y - global as f32 * self.row_h() > self.cell {
            return None;
        }
        let (n, x0) = self.row_layout(row);
        let rel = x - x0;
        if rel < 0. {
            return None;
        }
        let col = (rel / (self.cell + self.gap)).floor() as usize;
        if col >= n || rel - col as f32 * (self.cell + self.gap) > self.cell {
            return None;
        }
        Some(self.first_of_row(row) + col)
    }

    /// The photo nearest to the point: above the first row the first photo, below the last one the last photo,
    /// sideways the nearest column. For painting, where the finger can leave the tiles.
    pub fn nearest(&self, x: f32, content_y: f32) -> Option<usize> {
        if !self.valid() {
            return None;
        }
        let global = (content_y.max(0.) / self.row_h()).floor() as usize;
        let last_row = (self.count + self.skip() - 1) / self.columns;
        let row = global.saturating_sub(self.top_rows).min(last_row);
        let (n, x0) = self.row_layout(row);
        let col = (((x - x0) / (self.cell + self.gap)).floor().max(0.) as usize).min(n - 1);
        Some((self.first_of_row(row) + col).min(self.count - 1))
    }
}

/// Auto-scroll speed in px/s for a finger at screen `y` over the visible area `top..bottom`: 0 in the middle,
/// growing linearly to `AUTOSCROLL_MAX` at the edges (negative = up, towards the start of the list).
pub fn autoscroll_speed(y: f32, top: f32, bottom: f32) -> f32 {
    let mut speed = 0.;
    if y < top + AUTOSCROLL_ZONE {
        speed = -AUTOSCROLL_MAX * ((top + AUTOSCROLL_ZONE - y) / AUTOSCROLL_ZONE).min(1.);
    }
    if y > bottom - AUTOSCROLL_ZONE {
        speed = AUTOSCROLL_MAX * ((y - (bottom - AUTOSCROLL_ZONE)) / AUTOSCROLL_ZONE).min(1.);
    }
    speed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> HitGeom {
        // 100 wide, 3 columns of 32 with 2 gap: 32*3 + 2*2 = 100. 9 photos: rows of 3, 3, 3. Two empty rows above.
        HitGeom { width: 100., count: 9, columns: 3, cell: 32., gap: 2., top_rows: 2, top: 50. }
    }

    #[test]
    fn exact_hits_follow_rows_and_columns() {
        let g = geom();
        let row0 = 2. * g.row_h();
        assert_eq!(g.item_at(5., row0 + 5.), Some(0));
        assert_eq!(g.item_at(40., row0 + 5.), Some(1));
        assert_eq!(g.item_at(95., row0 + 5.), Some(2));
        assert_eq!(g.item_at(5., row0 + g.row_h() + 5.), Some(3));
        assert_eq!(g.item_at(5., 5.), None, "empty rows above the photos");
        assert_eq!(g.item_at(33., row0 + 5.), None, "the gap between two tiles");
        assert_eq!(g.item_at(5., row0 + 33.), None, "the gap between two rows");
        assert_eq!(g.item_at(5., row0 + 3. * g.row_h() + 5.), None, "below the last row");
    }

    #[test]
    fn the_short_first_row_is_centered() {
        let g = HitGeom { count: 8, ..geom() };
        let first = 2. * g.row_h() + 5.;
        // two tiles: 32 + 2 + 32 = 66 wide, starting at x = 17
        assert_eq!(g.item_at(5., first), None);
        assert_eq!(g.item_at(20., first), Some(0));
        assert_eq!(g.item_at(60., first), Some(1));
        assert_eq!(g.item_at(90., first), None);
        assert_eq!(g.nearest(2., first), Some(0));
        assert_eq!(g.nearest(99., first), Some(1));
        // The rows below are full and go on from there.
        assert_eq!(g.item_at(5., first + g.row_h()), Some(2));
        assert_eq!(g.item_at(95., first + 2. * g.row_h()), Some(7));
        assert_eq!(g.item_at(5., first + 3. * g.row_h()), None, "below the last row");
    }

    #[test]
    fn nearest_clamps_to_the_grid() {
        let g = geom();
        assert_eq!(g.nearest(5., -100.), Some(0), "above everything");
        assert_eq!(g.nearest(50., 3. * g.row_h()), Some(4), "a gap resolves to the row's tile");
        assert_eq!(g.nearest(99., 100_000.), Some(8), "below everything: the last photo");
        assert_eq!(g.nearest(-20., 2. * g.row_h() + 5.), Some(0));
        assert_eq!(HitGeom::default().nearest(1., 1.), None);
    }

    #[test]
    fn content_y_adds_the_scroll() {
        let g = geom();
        assert_eq!(g.content_y(150., -300.), 400.);
        assert_eq!(g.content_y(60., 0.), 10.);
    }

    #[test]
    fn autoscroll_grows_towards_the_edges() {
        assert_eq!(autoscroll_speed(300., 100., 700.), 0.);
        assert_eq!(autoscroll_speed(100., 100., 700.), -AUTOSCROLL_MAX);
        assert_eq!(autoscroll_speed(40., 100., 700.), -AUTOSCROLL_MAX, "past the edge it stays at the maximum");
        let half = autoscroll_speed(140., 100., 700.);
        assert!((half + AUTOSCROLL_MAX / 2.).abs() < 1., "{half}");
        assert_eq!(autoscroll_speed(700., 100., 700.), AUTOSCROLL_MAX);
        assert!(autoscroll_speed(650., 100., 700.) > 0. && autoscroll_speed(650., 100., 700.) < AUTOSCROLL_MAX);
    }
}
