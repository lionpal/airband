//! How much one caller may ask of the relay, and how often.
use super::*;

/// At most so many requests per key in each stretch of time. A key is a channel or an internet address.
pub(super) struct Limiter {
    pub(super) most: u32,
    pub(super) window: Duration,
    pub(super) by: HashMap<String, (Instant, u32)>,
    pub(super) swept: Option<Instant>,
}

impl Limiter {
    pub(super) fn new(most: u32, seconds: u64) -> Limiter {
        Limiter { most, window: Duration::from_secs(seconds), by: HashMap::new(), swept: None }
    }
    /// May this one be taken? Counts it if so.
    pub(super) fn allow(&mut self, key: &str, now: Instant) -> bool {
        // forget stretches that are over: now and then, and at once when there is no room left
        if self.by.len() >= KEYS_KEPT || self.swept.is_none_or(|at| now.duration_since(at) >= self.window) {
            let window = self.window;
            self.by.retain(|_, (began, _)| now.duration_since(*began) < window);
            self.swept = Some(now);
        }
        let full = self.by.len() >= KEYS_KEPT;
        match self.by.get_mut(key) {
            Some((began, count)) if now.duration_since(*began) < self.window => {
                if *count >= self.most {
                    return false;
                }
                *count += 1;
            }
            Some(slot) => *slot = (now, 1),
            // a key not seen before, with the table full: refuse rather than forget what others have used
            None if full => return false,
            None => {
                self.by.insert(key.to_string(), (now, 1));
            }
        }
        true
    }
}

/// The two things a radio asks of the relay.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Asks {
    Call,
    Queue,
}

/// How much is taken in a minute: on one channel, and from one internet address whatever channels it names.
/// A phone sending a large file makes many calls quickly, six at a time, so calls are allowed generously; how
/// many may wait at once (`MAX_IN_FLIGHT`) is what bounds them. Messages to keep are written to the database,
/// so those are few. An address gets twice a channel's share: a household's phones come from one address.
pub(super) const CALLS_A_MINUTE: u32 = 3000;

pub(super) const KEPT_A_MINUTE: u32 = 60;

/// Notifications asked for from one address in ten minutes, across its channels (`push::PUSHES` is one channel's).
pub(super) const PUSHES_AN_ADDRESS: u32 = push::PUSHES * 4;

/// New channels: how many one internet address may open in a day, and how many the relay opens in an hour
/// whoever asks. A channel is a pairing of one computer with one phone, so a person opens a handful in all;
/// the hourly figure is what a script with many addresses is held to.
pub(super) const OPENED_AN_ADDRESS_A_DAY: u32 = 10;

pub(super) const OPENED_AN_HOUR: u32 = 300;

pub(super) struct Limits {
    /// For each of [`Asks`]: by channel, then by address.
    pub(super) radio: [(Limiter, Limiter); 2],
    pub(super) push_by_address: Limiter,
    pub(super) opened_by_address: Limiter,
    pub(super) opened: Limiter,
}

impl Limits {
    pub(super) fn new() -> Limits {
        let pair = |most: u32| (Limiter::new(most, 60), Limiter::new(most * 2, 60));
        Limits { radio: [pair(CALLS_A_MINUTE), pair(KEPT_A_MINUTE)], push_by_address: Limiter::new(PUSHES_AN_ADDRESS, 600), opened_by_address: Limiter::new(OPENED_AN_ADDRESS_A_DAY, 86_400), opened: Limiter::new(OPENED_AN_HOUR, 3600) }
    }
    pub(super) fn channel(&mut self, asks: Asks, channel: &str, now: Instant) -> bool {
        self.radio[asks as usize].0.allow(channel, now)
    }
    pub(super) fn address(&mut self, asks: Asks, address: &str, now: Instant) -> bool {
        self.radio[asks as usize].1.allow(address, now)
    }
}
