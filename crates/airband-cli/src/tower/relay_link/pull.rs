//! The pull loop: the channel opened on the relay, and that radio's sealed requests pulled from it one at a time,
//! each answered on a task of its own ([`super::answer`]).
use super::super::log;
use super::said::RelaySaid;
use super::{Ended, Serve};
use airband_core::relay::{ADMISSION_HEADER, Handing, KEPT_HEADER, PullRefused, Pulling, RADIO_HASH_HEADER, REQUEST_HEADER, Route, Side};
use airband_io::http::Ask;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// What comes to the pull loop: a pull's outcome, under its number, or word that the kept message the pull in
/// flight names was let go without its answer reaching the relay.
pub(super) enum Turn {
    Pulled(u64, Pull),
    LetGo,
}

/// How one pull went.
pub(super) enum Pull {
    /// A request handed over: its id, whether the relay kept it, and its body or why it could not be read whole.
    Handed { id: Option<String>, kept: bool, body: Result<Vec<u8>, String> },
    /// Nothing waiting.
    Nothing,
    /// The relay no longer knows the channel, or it changed hands.
    Gone(RelaySaid),
    /// Refused, with why, or why its reason could not be read.
    Refused(Result<PullRefused, String>),
    /// Any other answer.
    Other(RelaySaid),
    /// The relay was not reached.
    Lost(String),
}

/// The work on one request handed over by a pull, for as long as it lasts: however it ends, its answer sent or
/// not, a panic included, the pulls stop saying the tower is answering for it. One whose answer did not reach the
/// relay cuts the pull in flight short, so the kept line is not held for the rest of that pull. Only the work on
/// the handing the pulls now name lets a kept message go ([`Handing`]), never the work on one before it.
pub(super) struct Answering {
    pub(super) pulling: Arc<Mutex<Pulling>>,
    /// The handing of a kept message this work answers for; nothing for a radio's request.
    pub(super) handing: Option<Handing>,
    /// The relay has its answer.
    pub(super) delivered: bool,
    pub(super) turns: mpsc::Sender<Turn>,
}

impl Drop for Answering {
    fn drop(&mut self) {
        let Some(handing) = &self.handing else { return };
        let let_go = self.pulling.lock().unwrap_or_else(|e| e.into_inner()).answered(handing);
        if let_go && !self.delivered {
            self.turns.send(Turn::LetGo).ok();
        }
    }
}

/// What he is told when the relay will not take a pull, said once for each reason.
pub(super) fn refused_pull(relay: &str, why: Result<PullRefused, &str>) -> String {
    match why {
        Ok(PullRefused::OtherVersion { older: Side::Tower }) => format!("{relay} needs a newer airband than this computer has: update airband here"),
        Ok(PullRefused::OtherVersion { older: Side::Relay }) => format!("this computer's airband is newer than {relay}: update the relay"),
        Ok(PullRefused::AnotherTower) => format!("another computer is serving this pairing through {relay}; stop airband there, or pair this computer again on its own"),
        Ok(PullRefused::AskAgain) => format!("{relay} let another computer's pull go to see whether it is still there; asking again"),
        Err(e) => format!("{relay} refused this computer's pull, and its answer saying why could not be read ({e}); retrying"),
    }
}

/// The pull loop's own bookkeeping: whether the channel is open, how long to wait before trying again, and what
/// he has been told (each thing once, not every few seconds).
struct Loop {
    open: bool,
    wait: u64,
    /// How long to wait after a kept message was let go without its answer reaching the relay, before the pull
    /// that has it handed over again: nothing the first time, longer each time in a row, and back to nothing
    /// once a pull finds nothing waiting. An answer that keeps failing to get there is not handed and failed
    /// again as fast as the two sides can go.
    let_go: Duration,
    told: bool,
    /// Each reason a pull was refused, once told; `None` is a reason that could not be read.
    told_refused: Vec<Option<PullRefused>>,
}

impl Loop {
    fn nap(&mut self) {
        std::thread::sleep(Duration::from_secs(self.wait));
        self.wait = (self.wait * 2).min(15);
    }
    /// Asked to ask again ([`PullRefused::AskAgain`]): nothing for him. The relay has let the other tower's pull
    /// go, and only that tower pulling again says it is still there; nothing tells this one when it would have,
    /// so it gives it a second, the one pause here that stands for no fact.
    fn again(&self) {
        std::thread::sleep(Duration::from_secs(1));
    }
    /// A kept message was let go without its answer reaching the relay: wait before it is handed over again.
    fn let_go(&mut self) {
        std::thread::sleep(self.let_go);
        self.let_go = (self.let_go * 2).clamp(Duration::from_secs(1), Duration::from_secs(15));
    }
    /// A refusal, said once for each reason, then a pause. `None` is a reason that could not be read.
    fn refused(&mut self, relay: &str, why: Result<PullRefused, &str>) {
        let reason = why.ok();
        if !self.told_refused.contains(&reason) {
            log(refused_pull(relay, why));
            self.told_refused.push(reason);
        }
        self.nap();
    }
}

impl Serve {
    pub(super) fn run(self) -> Ended {
        // with no relay there is nothing to pull: requests come straight (`answer_here`) until this is stopped
        let Some(relay) = self.relay.clone() else {
            log(format!("serving {} at this computer's own addresses, with no relay", self.tag));
            while !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
            }
            return Ended::Stopped;
        };
        let inbox = self.inbox.clone();
        let inbox = inbox.lock().unwrap_or_else(|e| e.into_inner());
        let mut at = Loop { open: false, wait: 1, let_go: Duration::ZERO, told: false, told_refused: Vec::new() };
        // the number of the last pull made, and of the one in flight
        let (mut pulls, mut in_flight) = (0u64, None);
        while !self.stop.load(Ordering::SeqCst) {
            // a pairing undone since this started is never opened again, and nothing more it pulls is answered:
            // `airband unpair` has just closed its channel, and opening it would put it back on record for a
            // phone that was cut off
            if !self.still_paired() {
                log(format!("{} is no longer paired; its channel is not opened again", self.tag));
                return Ended::Unpaired;
            }
            if !at.open {
                if let Some(ended) = self.open(&relay, &mut at) {
                    return ended;
                }
                continue;
            }
            let n = *in_flight.get_or_insert_with(|| {
                pulls += 1;
                self.pull_apart(pulls, &relay);
                pulls
            });
            match inbox.recv() {
                // one a newer pull took the place of: the relay has ended it, and what it says is not news
                Ok(Turn::Pulled(m, _)) if m != n => {}
                Ok(Turn::Pulled(_, pull)) => {
                    in_flight = None;
                    self.pulled(pull, &relay, &mut at);
                }
                // the kept message the pull in flight names was let go: a new pull, which does not name it, takes
                // that one's place, and the relay hands it over again: the first time at once, and again in a
                // row after a wait that grows
                Ok(Turn::LetGo) => {
                    at.let_go();
                    in_flight = None;
                }
                Err(_) => return Ended::Stopped,
            }
        }
        Ended::Stopped
    }

    /// Open the channel. What ended this link for good, if anything did.
    fn open(&self, relay: &str, at: &mut Loop) -> Option<Ended> {
        // who its radio is goes with the opening, so the relay knows from the first moment whose channel it is, and
        // which tower this is and what it still answers for, so the relay neither kicks another computer's pull
        // for it nor hands again what is still being answered
        let said = self.pulling.lock().unwrap_or_else(|e| e.into_inner()).headers();
        let mut req = Ask::put(self.keys.to(relay, Route::Channel)).header("authorization", self.keys.as_tower()).header(RADIO_HASH_HEADER, airband_core::relay::token_hash(&self.keys.radio_token)).headers(said);
        // the admission key as the settings hold it now: a new one is used without a restart
        if let Some(a) = &self.app.cfg().relay_admission {
            req = req.header(ADMISSION_HEADER, a);
        }
        // an opening refused as a pull is (another tower pulling, or ask again) says why in the same body
        let answered = self.pulls.send(req).map(|mut r| {
            let said = RelaySaid::of(r.status().as_u16());
            let why = (said == RelaySaid::Refused).then(|| r.body_mut().read_json::<PullRefused>().map_err(|e| e.to_string()));
            (said, why)
        });
        if let Ok((_, Some(why))) = &answered {
            match why {
                Ok(PullRefused::AskAgain) => at.again(),
                why => at.refused(relay, why.as_ref().copied().map_err(String::as_str)),
            }
            return None;
        }
        match answered.map(|(said, _)| said) {
            Ok(RelaySaid::Done | RelaySaid::Nothing) => {
                log(format!("serving {} through {relay}", self.tag));
                // opened again, the relay hands again whatever it had handed and is not still answered for
                self.pulling.lock().unwrap_or_else(|e| e.into_inner()).reopened();
                (at.open, at.wait, at.told) = (true, 1, false);
            }
            // the relay keeps this channel closed for good: it was unpaired, and only a new pairing opens one
            Ok(RelaySaid::Closed) => {
                log(format!("{relay} keeps the channel for {} closed; pair again to use it", self.tag));
                return Some(Ended::Closed);
            }
            other => {
                if !at.told {
                    log(match other {
                        Ok(RelaySaid::NeedsAdmission) => format!("{relay} needs an admission key: airband relay <url> --admission <key>"),
                        Ok(RelaySaid::NotYours) => format!("{relay} says the channel for {} belongs to another computer; pair again", self.tag),
                        Ok(said) => format!("{relay} refused to open a channel ({said:?})"),
                        Err(e) => format!("cannot reach {relay}: {e}; retrying"),
                    });
                    at.told = true;
                }
                at.nap();
            }
        }
        None
    }

    /// Make pull `n` on a thread of its own, its outcome sent to the loop: a pull is held up to 25 s, and one
    /// whose kept message was let go is replaced without waiting for it.
    fn pull_apart(&self, n: u64, relay: &str) {
        let said = self.pulling.lock().unwrap_or_else(|e| e.into_inner()).headers();
        let ask = Ask::get(self.keys.to(relay, Route::Pull)).header("authorization", self.keys.as_tower()).headers(said);
        let (client, turns) = (self.pulls.clone(), self.turns.clone());
        std::thread::spawn(move || {
            let pull = match client.send(ask) {
                Ok(mut res) => match RelaySaid::of(res.status().as_u16()) {
                    RelaySaid::Done => Pull::Handed {
                        id: res.header(REQUEST_HEADER).map(String::from),
                        // the relay says outright when it had been keeping this one; nothing is read from the id
                        kept: res.has_header(KEPT_HEADER),
                        body: res.body_mut().read_to_vec().map_err(|e| e.to_string()),
                    },
                    RelaySaid::Nothing => Pull::Nothing,
                    said @ (RelaySaid::NotYours | RelaySaid::Unknown) => Pull::Gone(said),
                    RelaySaid::OtherVersion | RelaySaid::Refused => Pull::Refused(res.body_mut().read_json::<PullRefused>().map_err(|e| e.to_string())),
                    said => Pull::Other(said),
                },
                Err(e) => Pull::Lost(e.to_string()),
            };
            turns.send(Turn::Pulled(n, pull)).ok();
        });
    }

    /// What a pull's outcome means for the loop.
    fn pulled(&self, pull: Pull, relay: &str, at: &mut Loop) {
        match pull {
            Pull::Handed { id, kept, body } => {
                // the relay has taken this pull, and so heard what it got
                self.pulling.lock().unwrap_or_else(|e| e.into_inner()).taken();
                at.wait = 1;
                // a request with no id could never be answered: the relay has broken its side of the contract
                let Some(id) = id else {
                    return log(format!("{relay} handed over a request for {} with no id; it cannot be answered", self.tag));
                };
                match body {
                    // read whole: the next pull says it was got, and it is answered for from here
                    Ok(sealed) => {
                        let handing = self.pulling.lock().unwrap_or_else(|e| e.into_inner()).handed(&id, kept);
                        self.answer_apart(id, kept, handing, sealed);
                    }
                    // one cut off on the way is not named as got, so the relay hands it over again
                    Err(e) => log(format!("a request for {} was cut off on the way ({e}); the relay hands it over again", self.tag)),
                }
            }
            Pull::Nothing => {
                self.pulling.lock().unwrap_or_else(|e| e.into_inner()).taken();
                // nothing waiting, kept or not: whatever kept failing is over
                (at.wait, at.let_go) = (1, Duration::ZERO);
            }
            // the relay restarted without its data, or the channel changed hands: open it again
            Pull::Gone(said) => {
                log(format!("{relay} no longer knows the channel for {} ({said:?}); opening it again", self.tag));
                at.open = false;
            }
            // refused, saying why: which side is the older, or that another computer pulls with this pairing. A
            // refused pull told the relay nothing, so what it got is said again on the next. A reason that could
            // not be read is said as that, never guessed
            // asked to ask again: nothing for him, and no pause beyond the shortest
            Pull::Refused(Ok(PullRefused::AskAgain)) => at.again(),
            Pull::Refused(why) => at.refused(relay, why.as_ref().copied().map_err(String::as_str)),
            Pull::Other(said) => {
                log(format!("{relay} answered {said:?} when asked for {}'s requests; retrying", self.tag));
                at.nap();
            }
            Pull::Lost(e) => {
                if !at.told {
                    log(format!("lost {relay}: {e}; retrying"));
                    at.told = true;
                }
                at.open = false;
                at.nap();
            }
        }
    }
}
