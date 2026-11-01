//! The "show photos from" date: calendar maths for the settings picker and the cutoff the store filters with.
//!
//! Dates are plain `YYYY-MM-DD` (UTC, the same calendar the timeline sorts and groups by: `sort_millis` is the
//! capture instant in UTC and `date_bucket` is the date prefix of the stored ISO string). No chrono.

use super::iso_date;

/// Earliest year the picker offers.
pub const MIN_YEAR: i32 = 1971;

pub const MONTHS: [&str; 12] = [
    "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Day,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ymd {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 31,
    }
}

impl Ymd {
    /// Strict `YYYY-MM-DD` of a real calendar day from `MIN_YEAR` on.
    pub fn parse(raw: &str) -> Option<Ymd> {
        let b = raw.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !raw.is_ascii() {
            return None;
        }
        let year: i32 = raw[0..4].parse().ok()?;
        let month: u32 = raw[5..7].parse().ok()?;
        let day: u32 = raw[8..10].parse().ok()?;
        if year < MIN_YEAR || !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Ymd { year, month, day })
    }

    pub fn format(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// "3. März 2024".
    pub fn german(&self) -> String {
        format!("{}. {} {}", self.day, MONTHS[(self.month.clamp(1, 12) - 1) as usize], self.year)
    }

    /// Nearest valid day to the given fields that is not after `today`.
    pub fn clamped(year: i32, month: i64, day: i64, today: Ymd) -> Ymd {
        let year = year.clamp(MIN_YEAR, today.year.max(MIN_YEAR));
        let mut month = month.clamp(1, 12) as u32;
        if year == today.year {
            month = month.min(today.month);
        }
        let mut max_day = days_in_month(year, month);
        if year == today.year && month == today.month {
            max_day = max_day.min(today.day);
        }
        Ymd { year, month, day: day.clamp(1, max_day as i64) as u32 }
    }

    /// One step of a picker column. Day and month wrap around inside their valid range (the day range follows
    /// month and year, neither passes `today`); the year stops at `MIN_YEAR` and the current year.
    pub fn stepped(self, field: Field, delta: i32, today: Ymd) -> Ymd {
        let delta = i64::from(delta);
        match field {
            Field::Day => {
                let max = i64::from(Ymd::clamped(self.year, i64::from(self.month), 31, today).day);
                let day = (i64::from(self.day) - 1 + delta).rem_euclid(max) + 1;
                Ymd::clamped(self.year, i64::from(self.month), day, today)
            }
            Field::Month => {
                let max = i64::from(if self.year == today.year { today.month } else { 12 });
                let month = (i64::from(self.month) - 1 + delta).rem_euclid(max) + 1;
                Ymd::clamped(self.year, month, i64::from(self.day), today)
            }
            Field::Year => Ymd::clamped(self.year + delta as i32, i64::from(self.month), i64::from(self.day), today),
        }
    }

    /// The same day `years` years earlier (29 February falls back to 28), clamped like the picker.
    pub fn years_ago(today: Ymd, years: i32) -> Ymd {
        Ymd::clamped(today.year - years, i64::from(today.month), i64::from(today.day), today)
    }

    /// Start of this day (UTC) in epoch milliseconds: photos with an earlier capture instant are hidden.
    pub fn cutoff_millis(&self) -> i64 {
        iso_date::parse(&format!("{}T00:00:00Z", self.format())).unwrap_or(0)
    }

    /// The UTC day containing an epoch-millisecond instant.
    pub fn from_millis(ms: i64) -> Ymd {
        // Howard Hinnant's civil_from_days.
        let z = ms.div_euclid(86_400_000) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Ymd { year, month, day }
    }
}

/// Whether a photo with capture instant `sort_millis` is hidden by `cutoff` (0 = no limit).
/// Photos without a capture date (`sort_millis == 0`) are never hidden.
pub fn is_hidden(sort_millis: i64, cutoff: i64) -> bool {
    cutoff > 0 && sort_millis != 0 && sort_millis < cutoff
}

/// [`is_hidden`] for a stored/server ISO date string (`None` or unparsable = no date = shown).
pub fn hides_iso(date: Option<&str>, cutoff: i64) -> bool {
    cutoff > 0 && date.and_then(iso_date::parse).is_some_and(|ms| is_hidden(ms, cutoff))
}

/// The ISO instant sent to Immich as `takenAfter` for a cutoff date.
pub fn taken_after(date: Ymd) -> String {
    format!("{}T00:00:00.000Z", date.format())
}

/// [`taken_after`] for a cutoff in epoch milliseconds (`None` when there is no limit).
pub fn taken_after_of(cutoff: i64) -> Option<String> {
    (cutoff > 0).then(|| taken_after(Ymd::from_millis(cutoff)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: Ymd = Ymd { year: 2026, month: 11, day: 1 };

    #[test]
    fn parses_only_real_days() {
        assert_eq!(Ymd::parse("2024-02-29"), Some(Ymd { year: 2024, month: 2, day: 29 }));
        assert_eq!(Ymd::parse("2023-02-29"), None);
        assert_eq!(Ymd::parse("2023-13-01"), None);
        assert_eq!(Ymd::parse("2023-1-01"), None);
        assert_eq!(Ymd::parse("1900-01-01"), None);
        assert_eq!(Ymd::parse(""), None);
    }

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2025, 4), 30);
    }

    #[test]
    fn clamps_day_month_year() {
        assert_eq!(Ymd::clamped(2023, 2, 31, TODAY), Ymd { year: 2023, month: 2, day: 28 });
        assert_eq!(Ymd::clamped(2030, 12, 31, TODAY), TODAY);
        assert_eq!(Ymd::clamped(2026, 12, 5, TODAY), Ymd { year: 2026, month: 11, day: 1 });
        assert_eq!(Ymd::clamped(1800, 0, 0, TODAY), Ymd { year: MIN_YEAR, month: 1, day: 1 });
    }

    #[test]
    fn cutoff_and_from_millis_roundtrip() {
        let d = Ymd { year: 2024, month: 3, day: 9 };
        assert_eq!(d.cutoff_millis(), 1_709_942_400_000);
        assert_eq!(Ymd::from_millis(d.cutoff_millis()), d);
        assert_eq!(Ymd::from_millis(d.cutoff_millis() - 1), Ymd { year: 2024, month: 3, day: 8 });
        assert_eq!(Ymd::from_millis(0), Ymd { year: 1970, month: 1, day: 1 });
    }

    #[test]
    fn hidden_predicate() {
        let cut = Ymd { year: 2024, month: 1, day: 1 }.cutoff_millis();
        assert!(is_hidden(cut - 1, cut));
        assert!(!is_hidden(cut, cut));
        assert!(!is_hidden(cut + 1, cut));
        assert!(!is_hidden(0, cut), "no date stays visible");
        assert!(!is_hidden(5, 0), "no limit");
    }

    #[test]
    fn steps_wrap_and_clamp() {
        let d = Ymd { year: 2024, month: 1, day: 31 };
        assert_eq!(d.stepped(Field::Day, 1, TODAY), Ymd { year: 2024, month: 1, day: 1 }, "day wraps");
        assert_eq!(d.stepped(Field::Month, 1, TODAY), Ymd { year: 2024, month: 2, day: 29 }, "day clamps to the new month");
        assert_eq!(d.stepped(Field::Month, -1, TODAY), Ymd { year: 2024, month: 12, day: 31 });
        let t = Ymd { year: 2026, month: 11, day: 1 };
        assert_eq!(t.stepped(Field::Day, 1, TODAY), t, "cannot pass today");
        assert_eq!(t.stepped(Field::Year, 1, TODAY), t);
        assert_eq!(t.stepped(Field::Month, 1, TODAY), Ymd { year: 2026, month: 1, day: 1 }, "months stop at today's");
        assert_eq!(Ymd { year: MIN_YEAR, month: 1, day: 1 }.stepped(Field::Year, -1, TODAY).year, MIN_YEAR);
        assert_eq!(Ymd { year: 2024, month: 2, day: 29 }.stepped(Field::Year, 1, TODAY), Ymd { year: 2025, month: 2, day: 28 });
        assert_eq!(Ymd::years_ago(TODAY, 5), Ymd { year: 2021, month: 11, day: 1 });
    }

    #[test]
    fn iso_strings() {
        let cut = Ymd { year: 2024, month: 1, day: 1 }.cutoff_millis();
        assert!(hides_iso(Some("2023-12-31T23:59:59.999Z"), cut));
        assert!(!hides_iso(Some("2024-01-01T00:00:00.000Z"), cut));
        assert!(!hides_iso(None, cut));
        assert!(!hides_iso(Some("garbage"), cut));
    }

    #[test]
    fn formats() {
        let d = Ymd { year: 2024, month: 3, day: 9 };
        assert_eq!(d.format(), "2024-03-09");
        assert_eq!(d.german(), "9. März 2024");
        assert_eq!(taken_after(d), "2024-03-09T00:00:00.000Z");
    }
}
