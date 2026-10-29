//! Owner: A1. Swift source: `../immagine/Immagine/Support/ISODate.swift`.
//! Spec: `../immagine/ImmagineTests/` -> CoreTests (ISODate*).
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::iso_date` is green.

//! Parser for the ISO-8601 forms Immich emits (`2026-10-01T20:17:39.410Z`, `...923+00:00`,
//! `...923`, `...-05:00`). No chrono: the calendar arithmetic is done here.

/// Parses an ISO-8601 date-time into Unix epoch milliseconds (UTC).
///
/// Accepts `Z`, `+hh:mm` and `-hh:mm` offsets, and optional fractional seconds (only the
/// first three digits are kept). Returns `None` for anything else, including out-of-range
/// fields such as month 13 or 30 February.
pub fn parse(raw: &str) -> Option<i64> {
    let (date_part, mut time) = raw.split_once('T')?;

    let &[year, month, day] = date_part.split('-').collect::<Vec<_>>().as_slice() else {
        return None;
    };
    let year: i64 = year.parse().ok()?;
    let month: i64 = month.parse().ok()?;
    let day: i64 = day.parse().ok()?;

    // Timezone offset, in seconds east of UTC. Same precedence as the Swift original:
    // `Z`, else the last `+`, else the last `-` that is not the first character.
    let mut tz_seconds = 0;
    if let Some(z) = time.find('Z') {
        time = &time[..z];
    } else if let Some(plus) = time.rfind('+') {
        if let Some(off) = parse_offset(&time[plus + 1..]) {
            tz_seconds = off;
            time = &time[..plus];
        }
    } else if let Some(minus) = time.rfind('-').filter(|&i| i > 0) {
        if let Some(off) = parse_offset(&time[minus + 1..]) {
            tz_seconds = -off;
            time = &time[..minus];
        }
    }

    let tp: Vec<&str> = time.split(':').collect();
    if tp.len() < 3 {
        return None;
    }
    let hour: i64 = tp[0].parse().ok()?;
    let minute: i64 = tp[1].parse().ok()?;
    let (second, millis) = parse_seconds(tp[2])?;

    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }

    let local_seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    // Local time minus east-of-UTC offset = UTC instant.
    Some((local_seconds - tz_seconds) * 1_000 + millis)
}

/// `hh:mm` after a sign → seconds east of UTC (the sign is applied by the caller).
fn parse_offset(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split(':').collect();
    let &[h, m] = parts.as_slice() else {
        return None;
    };
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    Some(h * 3_600 + m * 60)
}

/// `SS` or `SS.fff...` → (whole seconds, milliseconds). Parsed as text, so `39.410` is
/// exactly 410 ms without floating-point error.
fn parse_seconds(s: &str) -> Option<(i64, i64)> {
    let (whole, frac) = match s.split_once('.') {
        Some((whole, frac)) => (whole, Some(frac)),
        None => (s, None),
    };
    if !all_digits(whole) {
        return None;
    }
    let second: i64 = whole.parse().ok()?;
    let millis = match frac {
        None => 0,
        Some(frac) => {
            if !all_digits(frac) {
                return None;
            }
            let mut ms = String::from(&frac[..frac.len().min(3)]);
            while ms.len() < 3 {
                ms.push('0');
            }
            ms.parse().ok()?
        }
    };
    Some((second, millis))
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let month_from_march = (month + 9) % 12; // March = 0
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_fractional_zulu() {
        // 2026-10-01T20:17:39.410Z
        assert_eq!(parse("2026-10-01T20:17:39.410Z"), Some(1_790_885_859_410));
    }

    #[test]
    fn iso_date_offset_equals_zulu() {
        let a = parse("2026-10-01T20:17:30.923+00:00");
        let b = parse("2026-10-01T20:17:30.923Z");
        assert_eq!(a, b);
        assert!(a.is_some());
    }

    #[test]
    fn iso_date_negative_offset() {
        // 20:00 UTC-5 == 01:00 UTC next day
        let d = parse("2026-10-01T20:00:00.000-05:00");
        let utc = parse("2026-10-02T01:00:00.000Z");
        assert_eq!(d, utc);
    }

    // Added beyond the Swift suite: the no-fraction form and rejection of malformed input.

    #[test]
    fn iso_date_without_fraction() {
        assert_eq!(parse("2026-10-01T20:17:39Z"), Some(1_790_885_859_000));
    }

    #[test]
    fn iso_date_rejects_malformed_input() {
        assert_eq!(parse("nope"), None);
        assert_eq!(parse("2026-10-01 20:17:39Z"), None);
        assert_eq!(parse("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse("2026-02-30T00:00:00Z"), None);
        assert_eq!(parse("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
    }
}
