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
