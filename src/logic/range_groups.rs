//! Groups the timeline into days, months and years for the Jahre / Monate views.
//!
//! The items are all in memory (the store syncs the whole timeline), so the views are derived from
//! `TimelineItem::date_bucket` (`YYYY-MM-DD`) instead of asking the server for time buckets. Items are read in
//! the order given (the app keeps them oldest first); a group ends when the month changes. Items without a date
//! are left out. All `first` fields are indexes into the item slice.

use super::models::timeline::TimelineItem;

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub day: u32,
    /// Index of the day's first photo (a still image when there is one).
    pub first: usize,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Month {
    pub year: i32,
    pub month: u32,
    pub days: Vec<Day>,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Year {
    pub year: i32,
    /// Cover photo: the first photo of the year's busiest day.
    pub cover: usize,
    /// Index into `Groups::months` of the year's first month.
    pub first_month: usize,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Groups {
    pub years: Vec<Year>,
    pub months: Vec<Month>,
}

/// `YYYY-MM-DD` to (year, month, day).
fn parse_bucket(s: &str) -> Option<(i32, u32, u32)> {
    let mut parts = s.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.get(..2)?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

pub fn group(items: &[TimelineItem]) -> Groups {
    let mut groups = Groups::default();
    // Busiest day seen so far per year: (count, first), for the cover.
    let mut best: Vec<(usize, usize)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let Some((y, m, d)) = parse_bucket(&item.date_bucket) else { continue };
        if !groups.months.last().is_some_and(|g| g.year == y && g.month == m) {
            if !groups.years.last().is_some_and(|g| g.year == y) {
                groups.years.push(Year { year: y, cover: i, first_month: groups.months.len(), count: 0 });
                best.push((0, i));
            }
            groups.months.push(Month { year: y, month: m, days: Vec::new(), count: 0 });
        }
        let month = groups.months.last_mut().unwrap();
        month.count += 1;
        match month.days.last_mut() {
            Some(day) if day.day == d => {
                day.count += 1;
                if items[day.first].is_video() && item.is_image() {
                    day.first = i;
                }
            }
            _ => month.days.push(Day { day: d, first: i, count: 1 }),
        }
        let year = groups.years.last_mut().unwrap();
        year.count += 1;
        let day = month.days.last().unwrap();
        let top = best.last_mut().unwrap();
        if day.count > top.0 {
            *top = (day.count, day.first);
            year.cover = day.first;
        }
    }
    groups
}

impl Month {
    /// Up to four days to show: the busiest first (it gets the big card), then the next busiest in date order.
    pub fn highlights(&self) -> Vec<&Day> {
        let mut by_count: Vec<&Day> = self.days.iter().collect();
        // Stable sort: ties keep the earlier day first.
        by_count.sort_by(|a, b| b.count.cmp(&a.count));
        by_count.truncate(4);
        let Some(&big) = by_count.first() else { return Vec::new() };
        let mut rest: Vec<&Day> = by_count[1..].to_vec();
        rest.sort_by_key(|d| d.day);
        let mut out = vec![big];
        out.extend(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(date: &str, video: bool) -> TimelineItem {
        TimelineItem {
            id: date.into(),
            asset_type: if video { "VIDEO" } else { "IMAGE" }.into(),
            thumbhash: None,
            ratio: None,
            is_favorite: false,
            is_trashed: false,
            date_bucket: date.into(),
            sort_date: None,
            duration: None,
        }
    }

    #[test]
    fn groups_by_month_and_year() {
        let items = vec![
            item("2025-12-30", false),
            item("2026-09-18", false),
            item("2026-09-18", false),
            item("2026-09-20", false),
            item("", false),
            item("2026-10-04", false),
        ];
        let g = group(&items);
        assert_eq!(g.years.len(), 2);
        assert_eq!(g.months.len(), 3);
        assert_eq!((g.years[1].year, g.years[1].first_month, g.years[1].count), (2026, 1, 4));
        // Busiest day of 2026 is 18 Sept, first photo at index 1.
        assert_eq!(g.years[1].cover, 1);
        assert_eq!(g.months[1].days, vec![Day { day: 18, first: 1, count: 2 }, Day { day: 20, first: 3, count: 1 }]);
    }

    #[test]
    fn highlights_put_busiest_first() {
        let mut items = Vec::new();
        for d in [2, 5, 5, 5, 9, 9, 12, 20] {
            items.push(item(&format!("2026-01-{d:02}"), false));
        }
        let g = group(&items);
        let days: Vec<u32> = g.months[0].highlights().iter().map(|d| d.day).collect();
        assert_eq!(days, vec![5, 2, 9, 12]);
    }

    #[test]
    fn day_prefers_a_still_image() {
        let items = vec![item("2026-01-02", true), item("2026-01-02", false)];
        assert_eq!(group(&items).months[0].days[0].first, 1);
    }
}
