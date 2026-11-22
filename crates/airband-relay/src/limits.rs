//! How much one caller may ask of the relay, and how often.
use super::*;

/// How many keys (channels or internet addresses) one limit remembers at once.
pub(super) const KEYS_KEPT: usize = 20_000;

/// Who is asking, as the load balancer saw them. It adds the caller's address to the end of this header, so the
/// last entry is the one a caller cannot make up; anything before it is whatever they chose to send.
pub(super) fn caller(headers: &HeaderMap) -> Caller {
    header(headers, "x-forwarded-for").and_then(|v| v.rsplit(',').next()).map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).map_or(Caller::Unseen, Caller::At)
}

/// Who is asking, by internet address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Caller {
    /// The address the load balancer saw.
    At(String),
    /// No load balancer said: every such caller is counted as one.
    Unseen,
}

/// One count in one stretch of time: may another be taken, at most `most` in each `window`? Counts it if so.
/// The one rule every limit here is made of.
pub(super) fn count(slot: &mut Option<(Instant, u32)>, most: u32, window: Duration, now: Instant) -> bool {
    let (began, taken) = slot.filter(|(began, _)| now.duration_since(*began) < window).unwrap_or((now, 0));
    if taken >= most {
        return false;
    }
    *slot = Some((began, taken + 1));
    true
}

/// At most so many requests per key in each stretch of time. A key is a channel or an internet address.
pub(super) struct Limiter<K = String> {
    pub(super) most: u32,
    pub(super) window: Duration,
    pub(super) by: HashMap<K, Option<(Instant, u32)>>,
    pub(super) swept: Option<Instant>,
}

impl<K: std::hash::Hash + Eq + Clone> Limiter<K> {
    pub(super) fn new(most: u32, seconds: u64) -> Limiter<K> {
        Limiter { most, window: Duration::from_secs(seconds), by: HashMap::new(), swept: None }
    }
    /// Would this one be taken now? Counts nothing.
    pub(super) fn room(&mut self, key: &K, now: Instant) -> bool {
        self.sweep(now);
        match self.by.get(key) {
            Some(slot) => count(&mut slot.clone(), self.most, self.window, now),
            // a key not seen before, with the table full: refuse rather than forget what others have used
            None => self.by.len() < KEYS_KEPT && self.most > 0,
        }
    }
    /// May this one be taken? Counts it if so.
    pub(super) fn allow(&mut self, key: &K, now: Instant) -> bool {
        if !self.room(key, now) {
            return false;
        }
        count(self.by.entry(key.clone()).or_default(), self.most, self.window, now)
    }
    /// Forget stretches that are over: now and then, and at once when there is no room left.
    fn sweep(&mut self, now: Instant) {
        if self.by.len() >= KEYS_KEPT || self.swept.is_none_or(|at| now.duration_since(at) >= self.window) {
            let window = self.window;
            self.by.retain(|_, slot| slot.is_some_and(|(began, _)| now.duration_since(began) < window));
            self.swept = Some(now);
        }
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
    pub(super) radio: [(Limiter, Limiter<Caller>); 2],
    pub(super) push_by_address: Limiter<Caller>,
    pub(super) opened_by_address: Limiter<Caller>,
    /// Everyone's, under one key.
    pub(super) opened: Limiter<()>,
}

impl Limits {
    pub(super) fn new() -> Limits {
        let pair = |most: u32| (Limiter::new(most, 60), Limiter::new(most * 2, 60));
        Limits { radio: [pair(CALLS_A_MINUTE), pair(KEPT_A_MINUTE)], push_by_address: Limiter::new(PUSHES_AN_ADDRESS, 600), opened_by_address: Limiter::new(OPENED_AN_ADDRESS_A_DAY, 86_400), opened: Limiter::new(OPENED_AN_HOUR, 3600) }
    }
    pub(super) fn channel(&mut self, asks: Asks, channel: &str, now: Instant) -> bool {
        self.radio[asks as usize].0.allow(&channel.to_string(), now)
    }
    pub(super) fn address(&mut self, asks: Asks, address: &Caller, now: Instant) -> bool {
        self.radio[asks as usize].1.allow(address, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_counts_per_key_and_starts_again_when_its_time_is_up() {
        let k = |s: &str| s.to_string();
        let (mut limit, t0) = (Limiter::new(3, 60), Instant::now());
        assert!((0..3).all(|_| limit.allow(&k("a"), t0)) && !limit.allow(&k("a"), t0 + Duration::from_secs(59)));
        assert!(limit.allow(&k("b"), t0));
        assert!(limit.allow(&k("a"), t0 + Duration::from_secs(60)));
        // a full table turns new keys away and keeps counting the ones it has
        let mut limit = Limiter::new(2, 60);
        for i in 0..KEYS_KEPT {
            assert!(limit.allow(&i.to_string(), t0));
        }
        assert!(!limit.allow(&k("one more"), t0) && limit.allow(&k("7"), t0) && !limit.allow(&k("7"), t0));
        assert!(limit.allow(&k("one more"), t0 + Duration::from_secs(61)));
    }
}
