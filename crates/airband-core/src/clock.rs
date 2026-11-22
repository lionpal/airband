//! The time, as every protocol here carries it: counted from 1970, in milliseconds where a moment is meant and
//! in seconds where a service asks for them. One place, so nobody has to remember which a `now()` gives.
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since 1970.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Seconds since 1970.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A moment in the calendar everyone uses, in UTC.
struct Civil {
    y: i64,
    m: i64,
    d: i64,
    hour: u64,
    minute: u64,
    second: u64,
    milli: u64,
}

/// Milliseconds since 1970 as a date and time (Howard Hinnant's `civil_from_days`).
fn civil(ms: u64) -> Civil {
    let (days, rest) = ((ms / 86_400_000) as i64, ms % 86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    Civil { y: yoe + era * 400 + i64::from(m <= 2), m, d, hour: rest / 3_600_000, minute: rest / 60_000 % 60, second: rest / 1000 % 60, milli: rest % 1000 }
}

/// Days since 1970 for a date (Howard Hinnant's `days_from_civil`).
fn days_from(y: i64, m: i64, d: i64) -> i64 {
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + d - 1 - 719_468
}

/// A moment as RFC 3339 in UTC with milliseconds: `2026-11-22T09:30:00.000Z`.
pub fn rfc3339(ms: u64) -> String {
    let c = civil(ms);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", c.y, c.m, c.d, c.hour, c.minute, c.second, c.milli)
}

/// A moment in seconds as the compact form request signatures are dated in: `20261122T093000Z`.
pub fn compact(secs: u64) -> String {
    let c = civil(secs * 1000);
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", c.y, c.m, c.d, c.hour, c.minute, c.second)
}

/// An RFC 3339 date and time (`Z` or a `+hh:mm`/`-hh:mm` offset, any fraction of a second), as milliseconds
/// since 1970. `None` for anything that is not one.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (time, offset_minutes) = match rest.rfind(['+', '-', 'Z']) {
        Some(i) if rest.as_bytes()[i] == b'Z' => (&rest[..i], 0),
        Some(i) => {
            let (h, min) = rest[i + 1..].split_once(':')?;
            let minutes = h.parse::<i64>().ok()? * 60 + min.parse::<i64>().ok()?;
            (&rest[..i], if rest.as_bytes()[i] == b'-' { -minutes } else { minutes })
        }
        None => return None,
    };
    let mut t = time.split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec = match t.next() {
        Some(x) => x.parse::<f64>().ok()?,
        None => 0.0,
    };
    let seconds = days_from(y, m, day) * 86_400 + h * 3600 + min * 60 - offset_minutes * 60;
    u64::try_from(seconds * 1000 + (sec * 1000.0).round() as i64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_go_both_ways() {
        assert_eq!(compact(0), "19700101T000000Z");
        assert_eq!(compact(1_440_938_160), "20150830T123600Z");
        assert_eq!(compact(1_709_164_799), "20240228T235959Z");
        assert_eq!(compact(1_709_251_200), "20240301T000000Z"); // the day after a leap day
        assert_eq!(rfc3339(1_795_568_400_487), "2026-11-25T01:00:00.487Z");
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-25T01:00:00.487315+00:00"), Some(1_795_568_400_487));
        assert_eq!(parse_rfc3339("2026-11-25T10:00:00.487+09:00"), Some(1_795_568_400_487));
        assert_eq!(parse_rfc3339("2024-02-29T12:00:00-05:00"), Some(1_709_226_000_000));
        assert_eq!(parse_rfc3339(&rfc3339(1_795_568_400_487)), Some(1_795_568_400_487));
        assert_eq!(parse_rfc3339("next week"), None);
    }
}
