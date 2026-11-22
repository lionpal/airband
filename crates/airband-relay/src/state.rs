//! What the relay holds in memory: each channel it knows, and the relay itself with who may do what on them.
use super::*;

/// How long a tower's pull is held open.
pub(super) const HOLD: Duration = Duration::from_secs(25);

/// A tower that has not pulled for this long is offline: radios are told at once instead of waiting.
pub(super) const AIRBAND_GONE: Duration = Duration::from_secs(45);

/// How long a radio's call waits for the tower's answer. Longer than the 25 s a tower holds `next` open.
pub(super) const ANSWER_WAIT: Duration = Duration::from_secs(40);

/// Requests in flight per channel; more are refused.
pub(super) const MAX_IN_FLIGHT: usize = 16;

pub(super) struct Channel {
    pub(super) pull_hash: String,
    pub(super) queue: VecDeque<(String, Bytes)>,
    pub(super) waiting: HashMap<String, oneshot::Sender<Bytes>>,
    pub(super) last_pull: Option<Instant>,
    pub(super) wake: Arc<Notify>,
    /// There may be messages kept for this tower from while it was away. Assumed after a restart, until looked.
    pub(super) has_held: bool,
    /// How many messages have been kept for it since the relay started: a pull that found nothing kept unmarks
    /// `has_held` only when none came in while it looked.
    pub(super) keeps: u64,
    /// The kept message the tower has been handed and has not yet said it took, and when.
    pub(super) holding: Option<(String, Instant)>,
    /// The hash of the token the paired radio proves itself with, as the tower gave it when it opened the
    /// channel.
    pub(super) radio_hash: String,
    /// How many notifications it has sent lately, and since when.
    pub(super) pushed: Option<(Instant, u32)>,
}

impl Channel {
    #[cfg(test)]
    pub(super) fn new(pull_hash: String, radio_hash: String) -> Channel {
        Channel::from(store::Saved { pull_hash, radio_hash })
    }
    pub(super) fn from(saved: store::Saved) -> Channel {
        Channel { pull_hash: saved.pull_hash, queue: VecDeque::new(), waiting: HashMap::new(), last_pull: None, wake: Arc::new(Notify::new()), has_held: true, keeps: 0, holding: None, radio_hash: saved.radio_hash, pushed: None }
    }
}

impl Channel {
    /// Is a kept message to be handed to the tower now? The count of keeps seen so far, when it is: there may be
    /// one, and none has been handed and left unanswered for less than [`HAND_AGAIN`].
    pub(super) fn kept_due(&self) -> Option<u64> {
        (self.has_held && self.holding.as_ref().is_none_or(|(_, at)| at.elapsed() > HAND_AGAIN)).then_some(self.keeps)
    }
    /// The store had nothing kept, as looked at when `seen` keeps had come in. Unmarked only if none came since:
    /// one kept while the store was being read is still there, and newer ordered words must not pass it.
    pub(super) fn found_nothing_kept(&mut self, seen: u64) {
        if self.keeps == seen {
            self.has_held = false;
        }
    }
    /// A message was kept for it: marked, and the tower woken for it.
    pub(super) fn kept_one(&mut self) {
        self.keeps += 1;
        self.has_held = true;
        self.wake.notify_one();
    }
}

/// A kept message the tower was handed and never answered for is handed over again after this long.
pub(super) const HAND_AGAIN: Duration = Duration::from_secs(60);

pub(super) struct Relay {
    /// The push services this relay can wake phones through.
    pub(super) push: push::Push,
    pub(super) channels: Mutex<HashMap<String, Channel>>,
    pub(super) store: store::Store,
    pub(super) admission: Option<String>,
    /// How many sign-ups the waiting list has taken lately.
    pub(super) signups: Mutex<Signups>,
    /// At most two sign-ups are being written to the database at any moment.
    pub(super) writing: tokio::sync::Semaphore,
    /// How much each channel and each internet address has asked for lately.
    pub(super) limits: Mutex<Limits>,
}

/// How busy a channel was when a request on it failed: (requests in flight, messages kept for its tower, its
/// tower pulling).
pub(super) type Load = (usize, bool, bool);

/// The line for a radio's request that failed: see [`Relay::note_failed`].
pub(super) fn failed_line(route: &str, channel: &str, status: u16, took: Duration, bytes: usize, (in_flight, has_held, pulling): Load) -> String {
    // six characters tell one channel's lines from another's in the log and are not enough to use the channel
    let short: String = channel.chars().filter(char::is_ascii_hexdigit).take(6).collect();
    format!("airband-relay: {route} on {short} answered {status} after {} ms ({bytes} bytes sent; {in_flight} in flight; kept messages: {}; tower pulling: {})", took.as_millis(), if has_held { "yes" } else { "no" }, if pulling { "yes" } else { "no" })
}

impl Relay {
    pub(super) fn new(push: push::Push, channels: HashMap<String, Channel>, store: store::Store, admission: Option<String>) -> Relay {
        Relay { push, channels: Mutex::new(channels), store, admission, signups: Mutex::new(Signups::default()), writing: tokio::sync::Semaphore::new(2), limits: Mutex::new(Limits::new()) }
    }

    /// Is this request from the channel's radio, and within what a radio may ask? Every route a radio uses
    /// starts here.
    ///
    /// The radio shows a token made from the pairing secret (`Authorization: Bearer`), whose hash the channel's
    /// tower gave when it opened the channel. Without that token nothing is taken.
    pub(super) fn admit(&self, asks: Asks, channel: &str, headers: &HeaderMap) -> Result<(), Box<Response>> {
        let now = Instant::now();
        if !self.limits.lock().unwrap().address(asks, &caller(headers), now) {
            return Err(Box::new(say(StatusCode::TOO_MANY_REQUESTS, "too many requests from this address; try again in a minute")));
        }
        let shown = token_hash(bearer(headers));
        if !self.channels.lock().unwrap().get(channel).is_some_and(|ch| same(&shown, &ch.radio_hash)) {
            return Err(Box::new(not_the_radio()));
        }
        // counted only now, so a stranger who knows a channel's name cannot use up its radio's share
        if !self.limits.lock().unwrap().channel(asks, channel, now) {
            return Err(Box::new(say(StatusCode::TOO_MANY_REQUESTS, "too many requests on this channel; try again in a minute")));
        }
        Ok(())
    }

    /// A radio's request did not come to what it asked for: one line saying so. It has what the relay itself
    /// handled and nothing else: the route, the status, how long it took, how much was sent, a shortened name
    /// for the channel, how many requests were in flight on it, whether messages are kept for its tower, and
    /// whether its tower has been pulling. Never what was sent (it is sealed, and the relay has no key to it),
    /// and never a token or an internet address.
    pub(super) fn note_failed(&self, route: &str, channel: &str, status: StatusCode, began: Instant, bytes: usize, wanted: StatusCode) {
        if status == wanted {
            return;
        }
        let (in_flight, has_held, pulling) = self.channels.lock().unwrap().get(channel).map(|ch| (ch.waiting.len(), ch.has_held, ch.last_pull.is_some_and(|t| t.elapsed() <= AIRBAND_GONE))).unwrap_or_default();
        eprintln!("{}", failed_line(route, channel, status.as_u16(), began.elapsed(), bytes, (in_flight, has_held, pulling)));
    }

    /// The channel, if this request carries its tower's pull token.
    /// Do this to the channel, if this request carries its tower's pull token: what it came to, or the answer
    /// for a request that may not.
    pub(super) fn with_tower<R>(&self, channel: &str, headers: &HeaderMap, then: impl FnOnce(&mut Channel) -> R) -> Result<R, Box<Response>> {
        self.as_tower(&mut self.channels.lock().unwrap(), channel, headers).map(then)
    }

    pub(super) fn as_tower<'a>(&self, channels: &'a mut HashMap<String, Channel>, channel: &str, headers: &HeaderMap) -> Result<&'a mut Channel, Box<Response>> {
        let ch = channels.get_mut(channel).ok_or_else(|| Box::new(say(StatusCode::NOT_FOUND, "no such channel: open it first")))?;
        if same(&token_hash(bearer(headers)), &ch.pull_hash) { Ok(ch) } else { Err(Box::new(say(StatusCode::FORBIDDEN, "not this channel's tower"))) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_request_is_one_line_with_nothing_of_the_channel_but_its_first_six_characters() {
        let channel = "0123456789abcdef0123456789abcdef";
        let line = failed_line("call", channel, 504, Duration::from_millis(40_012), 412, (3, true, true));
        assert_eq!(line, "airband-relay: call on 012345 answered 504 after 40012 ms (412 bytes sent; 3 in flight; kept messages: yes; tower pulling: yes)");
        assert!(!line.contains(&channel[..7]));
        // a name that is not a channel's at all says nothing of itself either
        assert!(failed_line("queue", "../../etc/passwd", 400, Duration::ZERO, 0, (0, false, false)).contains("queue on ecad answered 400"));
    }

    #[test]
    fn a_message_kept_while_the_store_is_read_keeps_the_channel_marked() {
        // N18: a pull looks, a message is kept meanwhile, then the pull's read comes back empty
        let mut ch = Channel::new("p".into(), "r".into());
        let seen = ch.kept_due().unwrap();
        ch.kept_one();
        ch.found_nothing_kept(seen);
        assert!(ch.has_held && ch.kept_due().is_some());
        // with nothing kept meanwhile, an empty store unmarks it
        let seen = ch.kept_due().unwrap();
        ch.found_nothing_kept(seen);
        assert!(!ch.has_held && ch.kept_due().is_none());
    }
}
