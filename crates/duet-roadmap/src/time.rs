//! UTC dates and times, to the second, without a date library.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Days since 1970-01-01 to a (year, month, day), by Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A (year, month, day) to days since 1970-01-01, by Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn seconds(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// `YYYY-MM-DD`, in UTC.
pub fn utc_date(at: SystemTime) -> String {
    let (y, m, d) = civil_from_days(seconds(at).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ`, in UTC, to the second.
pub fn utc_time(at: SystemTime) -> String {
    let secs = seconds(at);
    let of_day = secs.rem_euclid(86_400);
    let (h, min, s) = (of_day / 3600, of_day % 3600 / 60, of_day % 60);
    format!("{}T{h:02}:{min:02}:{s:02}Z", utc_date(at))
}

/// Read `YYYY-MM-DDTHH:MM:SSZ`, the form [`utc_time`] writes. Anything else is `None`.
pub fn parse_utc(text: &str) -> Option<SystemTime> {
    let b = text.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<u32>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let days = days_from_civil(i64::from(y), mo, d);
    let secs = days * 86_400 + i64::from(h * 3600 + mi * 60 + s);
    let secs = u64::try_from(secs).ok()?;
    let at = UNIX_EPOCH + Duration::from_secs(secs);
    // A day past the month's end (2026-02-30) would roll over; refuse it rather than move it.
    (utc_time(at) == text).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        for text in [
            "1970-01-01T00:00:00Z",
            "2000-02-29T12:34:56Z",
            "2026-11-19T02:46:14Z",
            "2026-12-31T23:59:59Z",
        ] {
            let at = parse_utc(text).expect(text);
            assert_eq!(utc_time(at), text);
        }
        assert_eq!(
            utc_date(parse_utc("2026-11-19T23:59:59Z").expect("valid")),
            "2026-11-19"
        );
    }

    #[test]
    fn malformed_times_are_refused() {
        for text in [
            "2026-02-30T00:00:00Z",
            "2026-11-19 02:46:14Z",
            "2026-11-19T02:46:14",
            "2026-13-01T00:00:00Z",
        ] {
            assert_eq!(parse_utc(text), None, "{text}");
        }
    }
}
