//! Crop-box math. Swift source: `Immagine/UI/Editor/CropGeometry.swift`.
//! Spec: `ImmagineTests/GeometryTests.swift` (crop section).
//!
//! Coordinates are normalized display coordinates (0..=1, y down) of the displayed
//! (mirrored/rotated, uncropped) image. `handle_at` takes canvas points.

use super::grid::layout::{Point, Rect, Size};

/// Hit distance around the crop box edges, in canvas points.
pub const HANDLE_TOLERANCE: f64 = 28.0;

/// Aspect presets of the crop tool (Photos' "Seitenverhältnis").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CropAspect {
    Freeform,
    Original,
    Square,
    R16x9,
    R4x3,
    R3x2,
    R9x16,
    R3x4,
    R2x3,
}

impl CropAspect {
    pub const ALL: [CropAspect; 9] = [
        CropAspect::Freeform,
        CropAspect::Original,
        CropAspect::Square,
        CropAspect::R16x9,
        CropAspect::R4x3,
        CropAspect::R3x2,
        CropAspect::R9x16,
        CropAspect::R3x4,
        CropAspect::R2x3,
    ];

    pub fn title(self) -> &'static str {
        match self {
            CropAspect::Freeform => "Freiform",
            CropAspect::Original => "Original",
            CropAspect::Square => "Quadrat",
            CropAspect::R16x9 => "16:9",
            CropAspect::R4x3 => "4:3",
            CropAspect::R3x2 => "3:2",
            CropAspect::R9x16 => "9:16",
            CropAspect::R3x4 => "3:4",
            CropAspect::R2x3 => "2:3",
        }
    }

    /// Width / height in pixels; `None` = unconstrained. `Original` is the displayed full image's ratio.
    pub fn pixel_ratio(self, canvas_pixels: Size) -> Option<f64> {
        match self {
            CropAspect::Freeform => None,
            CropAspect::Original => {
                (canvas_pixels.height > 0.0).then(|| canvas_pixels.width / canvas_pixels.height)
            }
            CropAspect::Square => Some(1.0),
            CropAspect::R16x9 => Some(16.0 / 9.0),
            CropAspect::R4x3 => Some(4.0 / 3.0),
            CropAspect::R3x2 => Some(3.0 / 2.0),
            CropAspect::R9x16 => Some(9.0 / 16.0),
            CropAspect::R3x4 => Some(3.0 / 4.0),
            CropAspect::R2x3 => Some(2.0 / 3.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Move,
}

/// Which handle a touch at `point` grabs; corners win over edges, edges over moving.
/// `rect` and `point` are in canvas points.
pub fn handle_at(point: Point, rect: Rect, tolerance: f64) -> Option<CropHandle> {
    let near_l = (point.x - rect.min_x()).abs() <= tolerance;
    let near_r = (point.x - rect.max_x()).abs() <= tolerance;
    let near_t = (point.y - rect.min_y()).abs() <= tolerance;
    let near_b = (point.y - rect.max_y()).abs() <= tolerance;
    let within_x = point.x >= rect.min_x() - tolerance && point.x <= rect.max_x() + tolerance;
    let within_y = point.y >= rect.min_y() - tolerance && point.y <= rect.max_y() + tolerance;
    if !(within_x && within_y) {
        return None;
    }
    match (near_l, near_r, near_t, near_b) {
        (true, _, true, _) => Some(CropHandle::TopLeft),
        (_, true, true, _) => Some(CropHandle::TopRight),
        (true, _, _, true) => Some(CropHandle::BottomLeft),
        (_, true, _, true) => Some(CropHandle::BottomRight),
        (true, _, _, _) => Some(CropHandle::Left),
        (_, true, _, _) => Some(CropHandle::Right),
        (_, _, true, _) => Some(CropHandle::Top),
        (_, _, _, true) => Some(CropHandle::Bottom),
        _ => rect.contains(point).then_some(CropHandle::Move),
    }
}

/// The crop box after dragging `handle` by `delta` (normalized). `ratio` is the locked width/height
/// in normalized units (see `normalized_ratio`); `min_size` the smallest normalized side.
pub fn drag(start: Rect, handle: CropHandle, delta: Size, ratio: Option<f64>, min_size: f64) -> Rect {
    if handle == CropHandle::Move {
        let x = (start.min_x() + delta.width).max(0.0).min(1.0 - start.width);
        let y = (start.min_y() + delta.height).max(0.0).min(1.0 - start.height);
        return Rect::new(x, y, start.width, start.height);
    }
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (start.min_x(), start.max_x(), start.min_y(), start.max_y());
    let (dw, dh) = (delta.width, delta.height);
    match handle {
        CropHandle::TopLeft => {
            min_x += dw;
            min_y += dh;
        }
        CropHandle::Top => min_y += dh,
        CropHandle::TopRight => {
            max_x += dw;
            min_y += dh;
        }
        CropHandle::Right => max_x += dw,
        CropHandle::BottomRight => {
            max_x += dw;
            max_y += dh;
        }
        CropHandle::Bottom => max_y += dh,
        CropHandle::BottomLeft => {
            min_x += dw;
            max_y += dh;
        }
        CropHandle::Left => min_x += dw,
        CropHandle::Move => {}
    }
    min_x = min_x.min(max_x - min_size).max(0.0);
    max_x = max_x.max(min_x + min_size).min(1.0);
    min_y = min_y.min(max_y - min_size).max(0.0);
    max_y = max_y.max(min_y + min_size).min(1.0);
    let rect = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);

    let ratio = match ratio {
        Some(r) if r > 0.0 => r,
        _ => return rect,
    };

    // Locked ratio: the dragged corner follows the larger relative change; the opposite corner
    // (or the opposite edge's center) stays put.
    let anchor = match handle {
        CropHandle::TopLeft => Point::new(start.max_x(), start.max_y()),
        CropHandle::TopRight => Point::new(start.min_x(), start.max_y()),
        CropHandle::BottomLeft => Point::new(start.max_x(), start.min_y()),
        CropHandle::BottomRight => Point::new(start.min_x(), start.min_y()),
        CropHandle::Left => Point::new(start.max_x(), start.mid_y()),
        CropHandle::Right => Point::new(start.min_x(), start.mid_y()),
        CropHandle::Top => Point::new(start.mid_x(), start.max_y()),
        CropHandle::Bottom => Point::new(start.mid_x(), start.min_y()),
        CropHandle::Move => Point::default(),
    };
    let mut width = rect.width;
    let mut height = rect.height;
    let width_driven = match handle {
        CropHandle::Left | CropHandle::Right => true,
        CropHandle::Top | CropHandle::Bottom => false,
        _ => width / start.width.max(0.0001) >= height / start.height.max(0.0001),
    };
    if width_driven {
        height = width / ratio;
    } else {
        width = height * ratio;
    }

    // Room available on each side of the anchor.
    let goes_left = matches!(handle, CropHandle::TopLeft | CropHandle::BottomLeft | CropHandle::Left);
    let goes_up = matches!(handle, CropHandle::TopLeft | CropHandle::TopRight | CropHandle::Top);
    let horizontal_centered = matches!(handle, CropHandle::Top | CropHandle::Bottom);
    let vertical_centered = matches!(handle, CropHandle::Left | CropHandle::Right);
    let max_width = if horizontal_centered {
        2.0 * anchor.x.min(1.0 - anchor.x)
    } else if goes_left {
        anchor.x
    } else {
        1.0 - anchor.x
    };
    let max_height = if vertical_centered {
        2.0 * anchor.y.min(1.0 - anchor.y)
    } else if goes_up {
        anchor.y
    } else {
        1.0 - anchor.y
    };
    let fit = 1.0_f64.min(max_width / width.max(0.0001)).min(max_height / height.max(0.0001));
    width *= fit;
    height *= fit;
    // Never below the minimum on either side, ratio kept.
    let grow = 1.0_f64.max(min_size / width.max(0.0001)).max(min_size / height.max(0.0001));
    width = (width * grow).min(max_width);
    height = (height * grow).min(max_height);

    let x = if horizontal_centered {
        anchor.x - width / 2.0
    } else if goes_left {
        anchor.x - width
    } else {
        anchor.x
    };
    let y = if vertical_centered {
        anchor.y - height / 2.0
    } else if goes_up {
        anchor.y - height
    } else {
        anchor.y
    };
    Rect::new(x, y, width, height)
}

/// The largest box with `ratio` (normalized) centered in `rect`.
pub fn fit(rect: Rect, ratio: f64) -> Rect {
    if ratio <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return rect;
    }
    let mut width = rect.width;
    let mut height = width / ratio;
    if height > rect.height {
        height = rect.height;
        width = height * ratio;
    }
    Rect::new(rect.mid_x() - width / 2.0, rect.mid_y() - height / 2.0, width, height)
}

/// Normalized width/height for a pixel ratio on a canvas of `canvas_pixels`.
pub fn normalized_ratio(pixel_ratio: f64, canvas_pixels: Size) -> f64 {
    if canvas_pixels.width <= 0.0 {
        return pixel_ratio;
    }
    pixel_ratio * canvas_pixels.height / canvas_pixels.width
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_near(a: f64, b: f64, accuracy: f64) {
        assert!((a - b).abs() <= accuracy, "{a} != {b} (accuracy {accuracy})");
    }

    fn assert_rect_near(a: Rect, b: Rect, accuracy: f64) {
        assert_near(a.min_x(), b.min_x(), accuracy);
        assert_near(a.min_y(), b.min_y(), accuracy);
        assert_near(a.width, b.width, accuracy);
        assert_near(a.height, b.height, accuracy);
    }

    #[test]
    fn crop_handle_hit_testing() {
        let rect = Rect::new(100.0, 100.0, 200.0, 300.0);
        let at = |x, y| handle_at(Point::new(x, y), rect, HANDLE_TOLERANCE);
        assert_eq!(at(102.0, 98.0), Some(CropHandle::TopLeft));
        assert_eq!(at(299.0, 401.0), Some(CropHandle::BottomRight));
        assert_eq!(at(200.0, 101.0), Some(CropHandle::Top));
        assert_eq!(at(299.0, 250.0), Some(CropHandle::Right));
        assert_eq!(at(200.0, 250.0), Some(CropHandle::Move));
        assert_eq!(at(10.0, 10.0), None);
    }

    #[test]
    fn crop_drag_clamps_and_keeps_minimum() {
        let start = Rect::new(0.2, 0.2, 0.6, 0.6);
        let moved = drag(start, CropHandle::Move, Size::new(0.5, -0.5), None, 0.05);
        assert_rect_near(moved, Rect::new(0.4, 0.0, 0.6, 0.6), 1e-9);
        let shrunk = drag(start, CropHandle::TopLeft, Size::new(0.9, 0.9), None, 0.1);
        assert_near(shrunk.width, 0.1, 1e-9);
        assert_near(shrunk.max_x(), start.max_x(), 1e-9); // opposite corner stays fixed
        let grown = drag(start, CropHandle::BottomRight, Size::new(2.0, 2.0), None, 0.1);
        assert_rect_near(grown, Rect::new(0.2, 0.2, 0.8, 0.8), 1e-9);
    }

    #[test]
    fn locked_ratio_drag_stays_inside_and_keeps_ratio() {
        let ratio = 1.5;
        let start = fit(Rect::new(0.0, 0.0, 1.0, 1.0), ratio);
        let cases = [
            (CropHandle::BottomRight, Size::new(-0.2, 0.05)),
            (CropHandle::TopLeft, Size::new(0.3, 0.3)),
            (CropHandle::Right, Size::new(0.4, 0.0)),
            (CropHandle::Top, Size::new(0.0, 0.2)),
        ];
        for (handle, delta) in cases {
            let r = drag(start, handle, delta, Some(ratio), 0.05);
            assert_near(r.width / r.height, ratio, 0.001);
            assert!(r.min_x() >= -1e-9, "{handle:?}");
            assert!(r.min_y() >= -1e-9, "{handle:?}");
            assert!(r.max_x() <= 1.0 + 1e-9, "{handle:?}");
            assert!(r.max_y() <= 1.0 + 1e-9, "{handle:?}");
        }
    }

    #[test]
    fn aspect_fit_and_normalized_ratio() {
        let fitted = fit(Rect::new(0.0, 0.0, 1.0, 1.0), 2.0);
        assert_rect_near(fitted, Rect::new(0.0, 0.25, 1.0, 0.5), 1e-9);
        // 16:9 on a 4000x3000 canvas in normalized units.
        let canvas = Size::new(4000.0, 3000.0);
        assert_near(normalized_ratio(16.0 / 9.0, canvas), 16.0 / 9.0 * 0.75, 1e-9);
        assert_near(CropAspect::Original.pixel_ratio(canvas).unwrap_or(0.0), 4.0 / 3.0, 1e-9);
        assert_eq!(CropAspect::Freeform.pixel_ratio(Size::new(1.0, 1.0)), None);
    }
}
