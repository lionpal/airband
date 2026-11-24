//! The operating system's own programs airband runs, each by its full path so a program of the same name earlier
//! on his PATH is never run in its place. These are the system's, not a coding tool's or a service's: they are
//! not catalogue rows.

/// The service manager that keeps the tower running (macOS).
#[cfg(target_os = "macos")]
pub(crate) const LAUNCHCTL: &str = "/bin/launchctl";
/// Process status: when a process started, to tell a process from a later one given the same number.
pub(crate) const PS: &str = "/bin/ps";
/// Unpacks a downloaded model.
pub(crate) const TAR: &str = "/usr/bin/tar";
/// Runs one command line.
pub(crate) const SH: &str = "/bin/sh";
/// The computer's name as he set it (macOS).
pub(crate) const SCUTIL: &str = "/usr/sbin/scutil";
/// The computer's short host name.
pub(crate) const HOSTNAME: &str = "/bin/hostname";
/// The login keychain (macOS).
pub(crate) const SECURITY: &str = "/usr/bin/security";

/// airband's own name on a Mac: the label the tower runs under in launchd, and the service its keys are kept
/// under in the keychain.
#[cfg(target_os = "macos")]
pub(crate) const MAC_NAME: &str = "dev.airband.tower";

/// When process `pid` started, in seconds since 1970 (UTC), as the system says: what tells it from a later
/// process given the same number. None when there is no such process.
pub(crate) fn started(pid: u32) -> Option<u64> {
    let out = airband_io::child::command(PS).args(["-p", &pid.to_string(), "-o", "lstart="]).env("LC_ALL", "C").env("TZ", "UTC").output().ok().filter(|o| o.status.success())?;
    lstart(&String::from_utf8_lossy(&out.stdout))
}

/// A start time in the form `ps -o lstart=` gives it in the C locale at UTC (`Sun Nov 22 10:01:02 2026`), read
/// as seconds since 1970. Anything not in that form is None: never read as some other time.
pub(crate) fn lstart(text: &str) -> Option<u64> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [_weekday, month, day, time, year] = parts.as_slice() else { return None };
    let month = MONTHS.iter().position(|m| m == month)? as i64 + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let hms: Vec<u64> = time.split(':').map(|n| n.parse().ok()).collect::<Option<_>>()?;
    let [h, m, s] = hms.as_slice() else { return None };
    if !(1..=31).contains(&day) || *h > 23 || *m > 59 || *s > 60 {
        return None;
    }
    // days from 1970-01-01 to this date (Howard Hinnant's days_from_civil)
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days).ok().map(|d| d * 86_400 + h * 3600 + m * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_time_is_read_as_the_moment_it_names_and_anything_else_as_none() {
        assert_eq!(lstart("Thu Jan  1 00:00:00 1970"), Some(0));
        assert_eq!(lstart("Sun Nov 22 10:01:02 2026"), Some(1_795_341_662));
        assert_eq!(lstart("  Sun   Nov 22 10:01:02 2026\n"), lstart("Sun Nov 22 10:01:02 2026"), "spacing is not what it says");
        assert_eq!(lstart("Sun Nov 22 10:01:02"), None);
        assert_eq!(lstart("start 11"), None);
        assert_eq!(lstart(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn this_process_has_a_start_time() {
        assert!(started(std::process::id()).is_some());
    }
}
