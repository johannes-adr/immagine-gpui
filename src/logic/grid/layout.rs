//! Timeline grid geometry (pure). Swift source: `Immagine/UI/Timeline/TimelineGridLayout.swift`
//! (the static math, zoom frames and zoom-start arithmetic; no UICollectionView state).
//! Spec: `ImmagineTests/GeometryTests.swift` (grid section).
//!
//! `Point`, `Size` and `Rect` are the shared geometry types for the logic layer (also used by
//! `crate::logic::crop_geometry`). All values are `f64`, y points down, like UIKit.

use std::ops::Range;

/// Column counts a pinch steps through, fewest columns (largest cells) first.
pub const ZOOM_LEVELS: [usize; 3] = [3, 5, 15];
/// Columns when nothing was stored yet.
pub const DEFAULT_COLUMNS: usize = 5;
/// 5 px on 3x screens (measured in Photos).
pub const DEFAULT_SPACING: f64 = 5.0 / 3.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// Origin plus size, like `CGRect`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn min_x(&self) -> f64 {
        self.x
    }
    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }
    pub fn min_y(&self) -> f64 {
        self.y
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }
    pub fn mid_x(&self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn mid_y(&self) -> f64 {
        self.y + self.height / 2.0
    }

    /// Like `CGRect.contains`: the left and top edges belong to the rect, the right and bottom do not.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min_x() && p.x < self.max_x() && p.y >= self.min_y() && p.y < self.max_y()
    }

    /// Like `CGRect.intersects`: empty rects never intersect; touching edges do not count.
    pub fn intersects(&self, other: &Rect) -> bool {
        if self.width <= 0.0 || self.height <= 0.0 || other.width <= 0.0 || other.height <= 0.0 {
            return false;
        }
        self.min_x() < other.max_x()
            && other.min_x() < self.max_x()
            && self.min_y() < other.max_y()
            && other.min_y() < self.max_y()
    }

    pub fn offset_by(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

/// Zoom in progress: the grid goes from `from` to `to` columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub from: usize,
    pub to: usize,
    /// 0 = source grid, 1 = target grid.
    pub progress: f64,
    /// Pinch location in content coordinates (grid at `from`).
    pub anchor: Point,
    /// Shift mapping `to`-grid content coordinates into the current (`from`) scroll position:
    /// `y_from = y_to + delta_y`.
    pub delta_y: f64,
    /// Content offset to apply when the zoom completes.
    pub final_offset_y: f64,
}

/// The next zoom level for a pinch step, or `None` at the end of the ladder.
/// A column count that is not a level starts again at the first level.
pub fn zoom_level_step(from: usize, zooming_out: bool) -> Option<usize> {
    let Some(i) = ZOOM_LEVELS.iter().position(|&level| level == from) else {
        return Some(ZOOM_LEVELS[0]);
    };
    let next = if zooming_out { i.checked_add(1) } else { i.checked_sub(1) }?;
    ZOOM_LEVELS.get(next).copied()
}

/// Pinch scale to transition progress: the finger spread that turns `from`-sized cells into
/// `to`-sized cells is progress 1. Clamped to 0..=1.
pub fn zoom_progress(scale: f64, from: usize, to: usize, width: f64, spacing: f64) -> f64 {
    let ratio = item_size(width, to, spacing) / item_size(width, from, spacing);
    if (ratio - 1.0).abs() <= 0.001 {
        return 1.0;
    }
    ((scale - 1.0) / (ratio - 1.0)).clamp(0.0, 1.0)
}

/// Side length of one square cell. Never below 1 pt so the stride stays positive.
pub fn item_size(width: f64, columns: usize, spacing: f64) -> f64 {
    let gaps = columns.saturating_sub(1) as f64;
    ((width - spacing * gaps) / columns as f64).max(1.0)
}

/// Frame of one item. With `scale > 0` edges snap to the pixel grid (`scale` = display scale),
/// so neighbouring cells never leave a blurred seam.
pub fn frame(index: usize, width: f64, columns: usize, spacing: f64, scale: f64) -> Rect {
    let size = item_size(width, columns, spacing);
    let row = index / columns;
    let col = index % columns;
    let x = col as f64 * (size + spacing);
    let y = row as f64 * (size + spacing);
    if scale <= 0.0 {
        return Rect::new(x, y, size, size);
    }
    let snap = |v: f64| (v * scale).round() / scale;
    let x0 = snap(x);
    let y0 = snap(y);
    Rect::new(x0, y0, snap(x + size) - x0, snap(y + size) - y0)
}

pub fn content_height(count: usize, width: f64, columns: usize, spacing: f64) -> f64 {
    if count == 0 {
        return 0.0;
    }
    let rows = count.div_ceil(columns);
    let size = item_size(width, columns, spacing);
    rows as f64 * size + (rows - 1) as f64 * spacing
}

/// Index range whose rows intersect `[min_y, max_y]`.
pub fn visible_range(min_y: f64, max_y: f64, count: usize, width: f64, columns: usize, spacing: f64) -> Range<usize> {
    if count == 0 || max_y <= min_y {
        return 0..0;
    }
    let stride = item_size(width, columns, spacing) + spacing;
    let first_row = (min_y / stride).floor().max(0.0) as usize;
    let last_row = ((max_y / stride).floor().max(0.0) as usize).max(first_row);
    let lower = first_row.saturating_mul(columns).min(count);
    let upper = last_row.saturating_add(1).saturating_mul(columns).min(count);
    lower..upper
}

/// Nearest item to a content point.
pub fn index_at(point: Point, count: usize, width: f64, columns: usize, spacing: f64) -> usize {
    if count == 0 {
        return 0;
    }
    let stride = item_size(width, columns, spacing) + spacing;
    let row = (point.y / stride).floor().max(0.0) as usize;
    let col = ((point.x / stride).floor().max(0.0) as usize).min(columns - 1);
    row.saturating_mul(columns).saturating_add(col).min(count - 1)
}

fn scaled(r: Rect, about: Point, by: f64) -> Rect {
    Rect::new(
        about.x + (r.min_x() - about.x) * by,
        about.y + (r.min_y() - about.y) * by,
        r.width * by,
        r.height * by,
    )
}

fn lerp(a: Rect, b: Rect, t: f64) -> Rect {
    Rect::new(
        a.min_x() + (b.min_x() - a.min_x()) * t,
        a.min_y() + (b.min_y() - a.min_y()) * t,
        a.width + (b.width - a.width) * t,
        a.height + (b.height - a.height) * t,
    )
}

/// The grid state the geometry depends on (what `TimelineGridLayout` keeps besides UIKit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub width: f64,
    pub count: usize,
    pub columns: usize,
    pub spacing: f64,
    /// Display scale for pixel snapping; `0` disables snapping.
    pub scale: f64,
}

impl Grid {
    pub fn frame_of(&self, index: usize) -> Rect {
        frame(index, self.width, self.columns, self.spacing, self.scale)
    }

    /// Starts an interactive zoom anchored at `anchor` (content coordinates). `None` when there is
    /// nothing to zoom: same count, no items, or no width.
    pub fn begin_zoom(
        &self,
        target: usize,
        anchor: Point,
        offset_y: f64,
        view_height: f64,
        inset_top: f64,
        inset_bottom: f64,
    ) -> Option<Zoom> {
        if target == self.columns || self.count == 0 || self.width <= 0.0 {
            return None;
        }
        let from = self.columns;
        let anchor_index = index_at(anchor, self.count, self.width, from, self.spacing);
        let from_frame = frame(anchor_index, self.width, from, self.spacing, 0.0);
        let to_frame = frame(anchor_index, self.width, target, self.spacing, 0.0);
        let u = ((anchor.y - from_frame.min_y()) / from_frame.height).clamp(0.0, 1.0);
        let anchor_in_to = to_frame.min_y() + u * to_frame.height;
        let ideal_offset = offset_y - (anchor.y - anchor_in_to);
        let max_offset = (-inset_top)
            .max(content_height(self.count, self.width, target, self.spacing) + inset_bottom - view_height);
        let final_offset = ideal_offset.max(-inset_top).min(max_offset);
        Some(Zoom {
            from,
            to: target,
            progress: 0.0,
            anchor,
            delta_y: offset_y - final_offset,
            final_offset_y: final_offset,
        })
    }

    /// Frame and alpha of one item at the zoom's progress. Items visible in both grids glide to
    /// their new place; items only in the target grid grow in from a scaled-up copy (fading in);
    /// items only in the source grid shrink away (fading out).
    pub fn zoom_frame(&self, index: usize, z: &Zoom, viewport: Rect) -> (Rect, f64) {
        let from_frame = frame(index, self.width, z.from, self.spacing, self.scale);
        let to_frame = frame(index, self.width, z.to, self.spacing, self.scale).offset_by(0.0, z.delta_y);
        let in_from = from_frame.intersects(&viewport);
        let in_to = to_frame.intersects(&viewport);
        let s = item_size(self.width, z.from, self.spacing) / item_size(self.width, z.to, self.spacing);
        let t = z.progress;
        match (in_from, in_to) {
            (true, true) | (false, false) => (lerp(from_frame, to_frame, t), 1.0),
            (false, true) => (lerp(scaled(to_frame, z.anchor, s), to_frame, t), t),
            (true, false) => (lerp(from_frame, scaled(from_frame, z.anchor, 1.0 / s), t), 1.0 - t),
        }
    }

    /// Indices that can be on screen during a zoom: the union of what the source grid and the
    /// target grid show in `viewport`. The caller still filters by the alpha and frame from `zoom_frame`.
    pub fn zoom_range(&self, z: &Zoom, viewport: Rect) -> Range<usize> {
        let from_range = visible_range(viewport.min_y(), viewport.max_y(), self.count, self.width, z.from, self.spacing);
        let to_top = viewport.min_y() - z.delta_y;
        let to_range = visible_range(to_top, to_top + viewport.height, self.count, self.width, z.to, self.spacing);
        let lower = from_range.start.min(to_range.start);
        let upper = from_range.end.max(to_range.end);
        lower..upper
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn assert_near(a: f64, b: f64, accuracy: f64) {
        assert!((a - b).abs() <= accuracy, "{a} != {b} (accuracy {accuracy})");
    }

    #[test]
    fn grid_frames_are_contiguous_and_pixel_aligned() {
        let width = 402.0;
        for columns in ZOOM_LEVELS {
            let a = frame(0, width, columns, 2.0, 3.0);
            let b = frame(1, width, columns, 2.0, 3.0);
            let last = frame(columns - 1, width, columns, 2.0, 3.0);
            assert_near(a.min_x(), 0.0, EPS);
            assert_near(b.min_x() - a.max_x(), 2.0, 0.34); // spacing within a pixel
            assert_near(last.max_x(), width, 0.34); // rows span the full width
            assert_near((a.width * 3.0).round(), a.width * 3.0, 1e-6); // pixel aligned
            let next_row = frame(columns, width, columns, 2.0, 3.0);
            assert_near(next_row.min_x(), 0.0, EPS);
            assert!(next_row.min_y() > a.max_y());
        }
    }

    #[test]
    fn visible_range_and_hit_index() {
        let width = 300.0; // 3 columns of ~98.67 pt + 2 pt spacing
        assert_eq!(visible_range(0.0, 150.0, 100, width, 3, 2.0), 0..6);
        assert_eq!(visible_range(0.0, 150.0, 4, width, 3, 2.0), 0..4);
        assert_eq!(visible_range(-50.0, -10.0, 10, width, 3, 2.0), 0..3);
        assert_eq!(index_at(Point::new(250.0, 120.0), 100, width, 3, 2.0), 5);
        assert_eq!(index_at(Point::new(999.0, 99_999.0), 10, width, 3, 2.0), 9);
        assert_eq!(content_height(0, width, 3, 2.0), 0.0);
    }

    #[test]
    fn zoom_levels_and_direction() {
        assert_eq!(zoom_level_step(3, true), Some(5));
        assert_eq!(zoom_level_step(5, true), Some(15));
        assert_eq!(zoom_level_step(15, true), None);
        assert_eq!(zoom_level_step(15, false), Some(5));
        assert_eq!(zoom_level_step(5, false), Some(3));
        assert_eq!(zoom_level_step(3, false), None);
        assert_eq!(zoom_level_step(7, true), Some(3), "unknown levels start at the first level");
    }

    #[test]
    fn pinch_scale_to_progress() {
        let w = 402.0;
        let sp = 2.0;
        // Fingers together (scale < 1) zooms out; reaching the size ratio completes it.
        let ratio = item_size(w, 5, sp) / item_size(w, 3, sp);
        assert_near(zoom_progress(1.0, 3, 5, w, sp), 0.0, EPS);
        assert_near(zoom_progress(ratio, 3, 5, w, sp), 1.0, EPS);
        assert_near(zoom_progress((1.0 + ratio) / 2.0, 3, 5, w, sp), 0.5, EPS);
        assert_near(zoom_progress(1.4, 3, 5, w, sp), 0.0, EPS); // spreading cannot advance a zoom-out
        assert_near(zoom_progress(1.0 / ratio, 5, 3, w, sp), 1.0, EPS);
        assert_near(zoom_progress(0.1, 5, 15, w, sp), 1.0, EPS); // clamped
    }

    #[test]
    fn interactive_zoom_keeps_anchor_and_ends_aligned() {
        let sp = DEFAULT_SPACING;
        let grid = Grid { width: 402.0, count: 1000, columns: 3, spacing: sp, scale: 3.0 };
        let anchor = Point::new(200.0, 2000.0);
        let z = grid.begin_zoom(5, anchor, 1600.0, 874.0, 0.0, 0.0).expect("zoom starts");
        assert_eq!(z.from, 3);
        assert_eq!(z.to, 5);
        // The anchored item's point sits under the fingers in the target grid.
        let index = index_at(anchor, 1000, 402.0, 3, sp);
        let to_frame = frame(index, 402.0, 5, sp, 0.0);
        let from_frame = frame(index, 402.0, 3, sp, 0.0);
        let u = (anchor.y - from_frame.min_y()) / from_frame.height;
        assert_near(to_frame.min_y() + u * to_frame.height + z.delta_y, anchor.y, 0.5);
        assert_near(z.final_offset_y, 1600.0 - z.delta_y, 1e-6);
        assert!(grid.begin_zoom(3, anchor, 0.0, 874.0, 0.0, 0.0).is_none(), "same level is no zoom");
    }

    #[test]
    fn zoom_offset_is_clamped_at_the_top() {
        // A short library stays pinned to the top.
        let grid = Grid { width: 402.0, count: 30, columns: 5, spacing: DEFAULT_SPACING, scale: 3.0 };
        let z = grid
            .begin_zoom(15, Point::new(300.0, 300.0), -100.0, 874.0, 100.0, 0.0)
            .expect("zoom starts");
        assert_near(z.final_offset_y, -100.0, EPS);
    }

    #[test]
    fn zoom_frame_at_rest_matches_the_source_grid() {
        let grid = Grid { width: 402.0, count: 100, columns: 3, spacing: 2.0, scale: 0.0 };
        let z = grid.begin_zoom(5, Point::new(100.0, 50.0), 0.0, 874.0, 0.0, 0.0).expect("zoom");
        let viewport = Rect::new(0.0, 0.0, 402.0, 874.0);
        let (rect, alpha) = grid.zoom_frame(0, &z, viewport);
        assert_near(alpha, 1.0, EPS);
        assert_near(rect.min_x(), 0.0, EPS);
        assert!(grid.zoom_range(&z, viewport).contains(&0));
    }
}
