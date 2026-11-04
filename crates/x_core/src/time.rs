//! Provider timestamp parsing without a datetime crate.

/// Unix seconds from a provider `created_at` string.
/// Accepts unix seconds/millis, ISO-8601, and the classic X date form.
pub fn parse_created_at(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<i64>() {
        return Some(if n.abs() > 10_000_000_000 {
            n / 1000
        } else {
            n
        });
    }
    parse_iso(s).or_else(|| parse_twitter(s))
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Weekday (0 = Sunday) and hour (0-23) in UTC.
pub fn weekday_hour(unix: i64) -> (u8, u8) {
    let day = unix.div_euclid(86_400);
    let rem = unix.rem_euclid(86_400) as u64;
    let weekday = ((day + 4).rem_euclid(7)) as u8;
    let hour = (rem / 3600) as u8;
    (weekday, hour)
}

pub fn ymd(unix: i64) -> String {
    let (y, m, d) = civil_from_days(unix.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn start_of_day(unix: i64) -> i64 {
    unix.div_euclid(86_400) * 86_400
}

fn parse_iso(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let b = s.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if b[10] != b'T' && b[10] != b' ' {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u32 = s[11..13].parse().ok()?;
    let min: u32 = s[14..16].parse().ok()?;
    let sec: u32 = s[17..19].parse().ok()?;
    ymd_hms_unix(year, month, day, hour, min, sec)
}

fn parse_twitter(s: &str) -> Option<i64> {
    // "Mon Oct 06 13:00:00 +0000 2026"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }
    let month = month_num(parts[1])?;
    let day: u32 = parts[2].parse().ok()?;
    let time = parts[3];
    if time.len() < 8 {
        return None;
    }
    let hour: u32 = time[0..2].parse().ok()?;
    let min: u32 = time[3..5].parse().ok()?;
    let sec: u32 = time[6..8].parse().ok()?;
    let year: i32 = parts[parts.len() - 1].parse().ok()?;
    ymd_hms_unix(year, month, day, hour, min, sec)
}

fn month_num(name: &str) -> Option<u32> {
    Some(match name {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn ymd_hms_unix(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + (hour as i64) * 3600 + (min as i64) * 60 + sec as i64)
}

/// Days since Unix epoch for a civil date (Howard Hinnant).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let mut y = year;
    let m = month as i32;
    let d = day as i32;
    y -= i32::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 }.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u32;
    (era as i64) * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i32::from(m <= 2);
    (y, m, d)
}

/// Newest first. Posts with no parseable timestamp sink to the end.
pub fn sort_newest<T, F>(items: &mut [T], created_at: F)
where
    F: Fn(&T) -> Option<&str>,
{
    items.sort_by_key(|item| {
        std::cmp::Reverse(
            created_at(item)
                .and_then(parse_created_at)
                .unwrap_or(i64::MIN),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_and_unix_and_twitter_parse() {
        let iso = parse_created_at("2026-10-06T13:00:00Z").unwrap();
        assert_eq!(parse_created_at("2026-10-06T13:00:00.000Z"), Some(iso));
        assert_eq!(parse_created_at("2026-10-06 13:00:00"), Some(iso));
        assert_eq!(
            parse_created_at("Tue Oct 06 13:00:00 +0000 2026"),
            Some(iso)
        );
        assert_eq!(parse_created_at(&iso.to_string()), Some(iso));
        assert_eq!(parse_created_at(&format!("{iso}000")), Some(iso));
        assert!(parse_created_at("").is_none());
        assert!(parse_created_at("not a date").is_none());
        // Known epoch: 1970-01-01.
        assert_eq!(parse_created_at("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_created_at("1970-01-01T00:00:01Z"), Some(1));
    }

    #[test]
    fn ymd_and_weekday_round_trip() {
        let unix = parse_created_at("2026-10-06T13:00:00Z").unwrap();
        assert_eq!(ymd(unix), "2026-10-06");
        let (wd, hour) = weekday_hour(unix);
        assert_eq!(hour, 13);
        assert_eq!(wd, 2, "2026-10-06 is a Tuesday");
        assert_eq!(
            start_of_day(unix),
            parse_created_at("2026-10-06T00:00:00Z").unwrap()
        );
    }

    #[test]
    fn sort_newest_puts_undated_last() {
        let mut rows = vec![
            ("old", Some("2026-10-01T00:00:00Z")),
            ("new", Some("2026-10-06T00:00:00Z")),
            ("none", None),
        ];
        sort_newest(&mut rows, |row| row.1);
        assert_eq!(
            rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec!["new", "old", "none"]
        );
    }
}
