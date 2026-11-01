//! Geometry of the timeline's scrollbar thumb: where it sits for a scroll offset, which offset a dragged thumb
//! means, how it fades, and which photo (and month label) is at a thumb position. No gpui, no platform.
//!
//! Scroll offsets follow GPUI: `offset <= 0`, content moves up, `-max_offset` is the end. The track is the span
//! between the header's bottom and the tab bar's top (screen y).

use super::hit::HitGeom;
use crate::logic::range_groups::parse_bucket;

/// Shortest thumb (pt), however long the timeline is.
pub const MIN_THUMB: f32 = 40.;
/// Width of the thumb at rest and while grabbed.
pub const WIDTH: f32 = 3.5;
pub const WIDTH_GRABBED: f32 = 7.;
/// Gap between the thumb and the right screen edge.
pub const EDGE_GAP: f32 = 2.;
/// Touch/click area measured from the right edge, and extra reach above/below the thumb.
pub const HIT_WIDTH: f32 = 44.;
pub const HIT_REACH: f32 = 10.;
/// The thumb stays fully visible this long after the last movement, then fades over `FADE_MS`.
pub const HOLD_MS: f64 = 1200.;
pub const FADE_MS: f64 = 250.;

/// Thumb (top, length) for the scroll position, `None` when nothing scrolls. The visible part of the list is
/// taken as the track's length, so the thumb's share of the track is the share of the content on screen.
pub fn thumb(track_top: f32, track_bottom: f32, max_offset: f32, offset: f32) -> Option<(f32, f32)> {
    let track = track_bottom - track_top;
    if !(max_offset > 0.) || !(track > 0.) {
        return None;
    }
    let len = (track * track / (track + max_offset)).clamp(MIN_THUMB.min(track), track);
    let frac = (-offset / max_offset).clamp(0., 1.);
    Some((track_top + frac * (track - len), len))
}

/// Scroll offset for a pointer at screen `y` holding the thumb `grab` pt below its top edge.
pub fn offset_for(y: f32, grab: f32, track_top: f32, track_bottom: f32, len: f32, max_offset: f32) -> f32 {
    let free = (track_bottom - track_top - len).max(0.);
    if free <= 0. || !(max_offset > 0.) {
        return 0.;
    }
    let frac = ((y - grab - track_top) / free).clamp(0., 1.);
    -(frac * max_offset)
}

/// Whether a point is on the (generous) grab area of a thumb at (top, len) in a screen `width` wide.
pub fn hit(x: f32, y: f32, width: f32, top: f32, len: f32) -> bool {
    x >= width - HIT_WIDTH && y >= top - HIT_REACH && y <= top + len + HIT_REACH
}

/// Opacity (0..1) `idle_ms` after the last scroll movement.
pub fn alpha(idle_ms: f64) -> f32 {
    if idle_ms <= HOLD_MS {
        1.
    } else {
        (1. - (idle_ms - HOLD_MS) / FADE_MS).clamp(0., 1.) as f32
    }
}

/// Index of the first photo of the row at screen `y` (clamped to the photos), for the scroll offset.
pub fn item_at(g: &HitGeom, offset: f32, y: f32) -> Option<usize> {
    if g.count == 0 || g.columns == 0 || g.row_h() <= 0. {
        return None;
    }
    let row = (g.content_y(y, offset) / g.row_h()).floor();
    let row = (row - g.top_rows as f32).max(0.) as usize;
    Some((row * g.columns).min(g.count - 1))
}

const MONTHS: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];

/// "März 2024" for a `YYYY-MM-DD` bucket.
pub fn month_year(bucket: &str) -> Option<String> {
    let (y, m, _) = parse_bucket(bucket)?;
    Some(format!("{} {y}", MONTHS[m as usize - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn thumb_follows_offset_and_stays_in_the_track() {
        // Track 100..700, content three screens long.
        let (top, len) = thumb(100., 700., 1200., 0.).unwrap();
        assert_eq!(top, 100.);
        assert!((len - 200.).abs() < 0.01);
        let (top, _) = thumb(100., 700., 1200., -600.).unwrap();
        assert!((top - 300.).abs() < 0.01, "half way");
        let (top, len) = thumb(100., 700., 1200., -1200.).unwrap();
        assert!((top + len - 700.).abs() < 0.01, "ends at the track's end");
        // Overscroll (rubber band) and bad offsets clamp.
        assert_eq!(thumb(100., 700., 1200., 80.).unwrap().0, 100.);
        let (top, len) = thumb(100., 700., 1200., -5000.).unwrap();
        assert!((top + len - 700.).abs() < 0.01);
        assert_eq!(thumb(100., 700., 0., 0.), None);
        assert_eq!(thumb(700., 100., 10., 0.), None);
    }

    #[::core::prelude::v1::test]
    fn drag_inverts_the_mapping() {
        for off in [0., -1., -333.3, -600., -1199., -1200.] {
            let (top, len) = thumb(100., 700., 1200., off).unwrap();
            let back = offset_for(top + 7., 7., 100., 700., len, 1200.);
            assert!((back - off).abs() < 0.5, "{off} -> {back}");
        }
        let len = thumb(100., 700., 1200., 0.).unwrap().1;
        assert_eq!(offset_for(0., 0., 100., 700., len, 1200.), 0., "above the track");
        assert_eq!(offset_for(9999., 0., 100., 700., len, 1200.), -1200., "below the track");
        assert_eq!(offset_for(300., 0., 100., 700., 600., 1200.), 0., "thumb as long as the track");
    }

    #[::core::prelude::v1::test]
    fn long_timelines_keep_the_minimum_length_and_resolution() {
        // 100 000 photos, 3 columns, 130 pt rows: about 4.3 million pt of content.
        let max = 100_000. / 3. * 130.;
        let (top, len) = thumb(100., 700., max, 0.).unwrap();
        assert_eq!((top, len), (100., MIN_THUMB));
        let (top, len) = thumb(100., 700., max, -max).unwrap();
        assert!((top + len - 700.).abs() < 0.01);
        // One pt of thumb travel is thousands of pt of scrolling, and the mapping is still monotonic.
        let a = offset_for(300., 0., 100., 700., MIN_THUMB, max);
        let b = offset_for(301., 0., 100., 700., MIN_THUMB, max);
        assert!(b < a && (a - b) > 1000.);
        let (t, _) = thumb(100., 700., max, a).unwrap();
        assert!((t - 300.).abs() < 0.05, "round trip at 4M pt: {t}");
    }

    #[::core::prelude::v1::test]
    fn grab_offset_keeps_the_thumb_under_the_finger() {
        let (top, len) = thumb(100., 700., 1200., -300.).unwrap();
        let off = offset_for(top + 30., 30., 100., 700., len, 1200.);
        assert!((off + 300.).abs() < 0.5);
        let off2 = offset_for(top + 30. + 50., 30., 100., 700., len, 1200.);
        assert!(off2 < off);
    }

    #[::core::prelude::v1::test]
    fn hit_area_is_wide_but_bound_to_the_thumb() {
        assert!(hit(380., 300., 400., 290., 60.));
        assert!(hit(399., 289., 400., 290., 60.), "reach above");
        assert!(!hit(340., 300., 400., 290., 60.), "too far from the edge");
        assert!(!hit(390., 200., 400., 290., 60.), "off the thumb");
    }

    #[::core::prelude::v1::test]
    fn fades_after_the_hold() {
        assert_eq!(alpha(0.), 1.);
        assert_eq!(alpha(HOLD_MS), 1.);
        assert!((alpha(HOLD_MS + FADE_MS / 2.) - 0.5).abs() < 1e-3);
        assert_eq!(alpha(HOLD_MS + FADE_MS), 0.);
        assert_eq!(alpha(1e9), 0.);
    }

    #[::core::prelude::v1::test]
    fn photo_at_a_screen_position() {
        // 3 columns, 100 + 2 pt rows, 2 empty rows above, list top at -150 (shift), 10 photos (4 rows).
        let g = HitGeom { width: 300., count: 10, columns: 3, cell: 100., gap: 2., top_rows: 2, top: -104. };
        // Scrolled to the start: screen y 100 is content 204 = row 2 = photo row 0.
        assert_eq!(item_at(&g, 0., 100.), Some(0));
        // 102 pt further down: next photo row.
        assert_eq!(item_at(&g, -102., 100.), Some(3));
        // Far past the end and above the start clamp.
        assert_eq!(item_at(&g, -9999., 100.), Some(9));
        assert_eq!(item_at(&g, 500., 100.), Some(0));
        assert_eq!(item_at(&HitGeom::default(), 0., 0.), None);
    }

    #[::core::prelude::v1::test]
    fn german_month_labels() {
        assert_eq!(month_year("2024-03-17").as_deref(), Some("März 2024"));
        assert_eq!(month_year("2026-12-01T00:00:00.000Z").as_deref(), Some("Dezember 2026"));
        assert_eq!(month_year("kaputt"), None);
    }
}
